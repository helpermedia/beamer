//! Bypass handling with smooth crossfading.
//!
//! This module provides utilities for implementing soft bypass in audio plugins.
//! Soft bypass crossfades between wet (processed) and dry (passthrough) signals
//! to avoid clicks when bypass is toggled.
//!
//! # Overview
//!
//! - [`BypassState`] - Current bypass state (Active, Bypassed, or transitioning)
//! - [`BypassAction`] - What the plugin should do this buffer
//! - [`CrossfadeCurve`] - Crossfade curve shape (Linear, EqualPower, SCurve)
//! - [`BypassHandler`] - Main utility for handling bypass with automatic crossfading
//!
//! # Latency
//!
//! A plugin with latency outputs its input `latency_samples()` later. The
//! dry signal has to come out just as late, in the crossfade and while
//! bypassed. Otherwise the two signals comb-filter while they are mixed
//! and the bypassed track plays early against the host's delay
//! compensation. The handler delays the dry signal by the latency given to
//! `begin()`, up to the largest latency given to `new()`.
//!
//! # Warm-up
//!
//! The DSP does not run while bypassed, so what it holds (a lookahead, an
//! envelope) is from before the bypass. When bypass ends, `begin()`
//! returns [`BypassAction::Resume`]: the plugin clears that memory and
//! processes. The handler keeps the delayed dry signal until the DSP has
//! run for the warm-up (the latency, or longer with
//! [`set_warmup_samples()`](BypassHandler::set_warmup_samples)), then
//! crossfades to it.
//!
//! # Example
//!
//! ```ignore
//! use beamer_core::{BypassHandler, BypassAction, CrossfadeCurve, Buffer};
//!
//! // In prepare(), with (SampleRate, MaxBufferSize, MainInputChannels) as Setup:
//! // BypassHandler::new(ramp_samples, CrossfadeCurve::Linear, channels, max_buffer_size, max_latency)
//!
//! struct MyPlugin {
//!     bypass_handler: BypassHandler,
//!     gain: f32,
//! }
//!
//! impl Processor for MyPlugin {
//!     fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
//!         let is_bypassed = self.model.bypass.get();
//!         let latency = self.latency_samples();
//!
//!         match self.bypass_handler.begin(is_bypassed, latency, buffer) {
//!             BypassAction::Passthrough => {
//!                 // Fully bypassed - the buffer holds the input, delayed by the latency
//!             }
//!             BypassAction::Process => {
//!                 // Normal processing
//!                 self.apply_gain(buffer);
//!             }
//!             BypassAction::ProcessAndCrossfade => {
//!                 // Process first, then crossfade
//!                 self.apply_gain(buffer);
//!                 self.bypass_handler.finish(buffer);
//!             }
//!             BypassAction::Resume => {
//!                 // Processing starts again: clear the DSP's memory first.
//!                 self.clear_dsp();
//!                 self.apply_gain(buffer);
//!                 self.bypass_handler.finish(buffer);
//!             }
//!         }
//!     }
//! }
//! ```

use crate::buffer::Buffer;
use crate::sample::Sample;

// =============================================================================
// BypassState
// =============================================================================

/// Current state of the bypass handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BypassState {
    /// Plugin is processing normally (not bypassed).
    Active,
    /// Transitioning from active to bypassed (crossfading to dry).
    RampingToBypassed,
    /// Plugin is fully bypassed (passthrough only).
    Bypassed,
    /// Transitioning from bypassed to active: the dry signal during the
    /// warm-up, then crossfading to wet.
    RampingToActive,
}

// =============================================================================
// BypassAction
// =============================================================================

/// What action the plugin should take for this buffer.
///
/// Returned by [`BypassHandler::begin()`] to tell you what to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BypassAction {
    /// Plugin is fully bypassed. The buffer holds the input, delayed by the
    /// latency given to `begin()`, so there is nothing to do. No DSP
    /// processing needed.
    Passthrough,

    /// Plugin is fully active. Run your DSP normally.
    /// No crossfade needed.
    Process,

    /// Plugin is transitioning. Run your DSP, then call `bypass_handler.finish(buffer)`
    /// to apply the crossfade.
    ProcessAndCrossfade,

    /// Processing starts again after a bypass. The DSP did not run while
    /// bypassed, so its memory (lookahead, delay lines, envelopes) is from
    /// before the bypass: clear it, run your DSP, then call
    /// `bypass_handler.finish(buffer)`. The handler keeps the dry signal
    /// until the DSP has run for the warm-up, then crossfades. Memory no
    /// longer than the warm-up (a lookahead) is flushed by then anyway.
    Resume,
}

// =============================================================================
// CrossfadeCurve
// =============================================================================

/// Crossfade curve shape for bypass transitions.
///
/// A bypass crossfades the processed and the dry version of the same
/// signal. For such related signals the gains have to add up to 1 to keep
/// the level: `Linear` and `SCurve` do. `EqualPower` keeps the power of two
/// unrelated signals instead (a fully wet reverb against its input) and
/// raises related ones by up to 3 dB in the middle of the crossfade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CrossfadeCurve {
    /// Linear crossfade: wet = 1 - position, dry = position. Keeps the level
    /// of related signals: the right curve for a bypass whose output follows
    /// its input. Unrelated signals dip by up to 3 dB in the middle.
    #[default]
    Linear,

    /// Equal-power crossfade: wet = cos(position * PI/2), dry =
    /// sin(position * PI/2). Keeps the level of unrelated signals only;
    /// related ones rise by up to 3 dB in the middle.
    EqualPower,

    /// S-curve crossfade: dry = 3x^2 - 2x^3 (smoothstep), wet = 1 - dry.
    /// Keeps the level of related signals like `Linear`, with a gentler
    /// start and end.
    SCurve,
}

impl CrossfadeCurve {
    /// Calculate wet and dry gains for a given ramp position.
    ///
    /// # Arguments
    /// * `t` - Normalized position (0.0 = fully wet, 1.0 = fully dry)
    ///
    /// # Returns
    /// Tuple of (wet_gain, dry_gain) as the specified sample type
    #[inline]
    pub fn gains<S: Sample>(&self, t: f64) -> (S, S) {
        let (wet, dry) = match self {
            CrossfadeCurve::Linear => (1.0 - t, t),
            CrossfadeCurve::EqualPower => {
                let angle = t * std::f64::consts::FRAC_PI_2;
                (angle.cos(), angle.sin())
            }
            CrossfadeCurve::SCurve => {
                // Smoothstep.
                let smooth = t * t * (3.0 - 2.0 * t);
                (1.0 - smooth, smooth)
            }
        };
        (S::from_f64(wet), S::from_f64(dry))
    }
}

// =============================================================================
// BypassHandler
// =============================================================================

/// Utility for handling bypass with smooth crossfading.
///
/// Maintains bypass state and provides automatic crossfade between
/// wet (processed) and dry (passthrough) signals when bypass is toggled.
///
/// # Usage Pattern
///
/// ```ignore
/// let latency = self.latency_samples();
/// match self.bypass_handler.begin(is_bypassed, latency, buffer) {
///     BypassAction::Passthrough => {}
///     BypassAction::Process => self.process_dsp(buffer),
///     BypassAction::ProcessAndCrossfade => {
///         self.process_dsp(buffer);
///         self.bypass_handler.finish(buffer);
///     }
///     BypassAction::Resume => {
///         self.clear_dsp();
///         self.process_dsp(buffer);
///         self.bypass_handler.finish(buffer);
///     }
/// }
/// ```
///
/// # Sample Type Flexibility
///
/// BypassHandler is not generic over sample type. The `begin()` and
/// `finish()` methods are generic, so a single BypassHandler instance can
/// process both `Buffer<f32>` and `Buffer<f64>` buffers.
///
/// # Dry Signal
///
/// The buffer is processed in place, so the input is gone once the DSP
/// wrote the output. `begin()` copies the input (the dry signal) into a
/// ring per channel: every block when the handler has a largest latency,
/// otherwise only while it crossfades. `finish()` mixes it back in and
/// `begin()` writes it into the buffer while bypassed, both delayed by the
/// latency of the block.
///
/// # Host Bypass
///
/// A host's own bypass button sets the plugin's bypass parameter (VST3
/// hosts and Audio Unit hosts alike), so this handler serves both. The
/// handler covers audio only: the plugin still receives MIDI while
/// bypassed, so a MIDI effect or instrument passes it on or ends its notes
/// itself.
///
/// # Real-Time Safety
///
/// The rings are allocated in [`new()`](Self::new). `begin()` and
/// `finish()` perform no heap allocations and are safe to use in audio
/// processing callbacks.
pub struct BypassHandler {
    /// Current bypass state
    state: BypassState,
    /// Current position in ramp (0 = start, ramp_samples = end)
    ramp_position: u32,
    /// Total ramp length in samples
    ramp_samples: u32,
    /// Crossfade curve to use
    curve: CrossfadeCurve,
    /// The dry signal of the recent blocks: one ring per main input
    /// channel, `ring_len` samples each.
    dry: Vec<Vec<f64>>,
    /// Length of each ring: the largest latency plus the longest block.
    ring_len: usize,
    /// Where the next block goes in the rings.
    write_position: usize,
    /// Where the current block starts in the rings.
    block_start: usize,
    /// The latency of the current block, at most `max_latency`.
    latency: usize,
    /// Number of channels the rings hold for the current block.
    dry_channels: usize,
    /// Number of samples the rings hold for the current block.
    dry_samples: usize,
    /// The longest block `new()` was given.
    max_buffer_size: usize,
    /// The largest latency `new()` was given.
    max_latency: usize,
    /// The shortest warm-up the plugin asked for, used when it is longer
    /// than the latency.
    warmup_samples: usize,
    /// How many samples the DSP has run since processing started again
    /// after a bypass. `usize::MAX` when it has run all along.
    processed_since_resume: usize,
}

impl BypassHandler {
    /// Create a new bypass handler.
    ///
    /// Call this in `prepare()`: it allocates the rings for the dry signal.
    ///
    /// # Arguments
    /// * `ramp_samples` - Number of samples for crossfade (0 = instant bypass)
    /// * `curve` - Crossfade curve shape
    /// * `channels` - Number of main input channels (`MainInputChannels`),
    ///   the channels whose dry signal is kept; the others fade to silence
    /// * `max_buffer_size` - Maximum block length (`MaxBufferSize`)
    /// * `max_latency` - The largest latency the plugin will report with
    ///   this setup, 0 for a plugin without latency. Each ring holds
    ///   `max_latency + max_buffer_size` samples.
    pub fn new(
        ramp_samples: u32,
        curve: CrossfadeCurve,
        channels: usize,
        max_buffer_size: usize,
        max_latency: u32,
    ) -> Self {
        let max_latency = max_latency as usize;
        let ring_len = max_latency + max_buffer_size;
        Self {
            state: BypassState::Active,
            ramp_position: 0,
            ramp_samples,
            curve,
            dry: vec![vec![0.0; ring_len]; channels],
            ring_len,
            write_position: 0,
            block_start: 0,
            latency: 0,
            dry_channels: 0,
            dry_samples: 0,
            max_buffer_size,
            max_latency,
            warmup_samples: 0,
            processed_since_resume: usize::MAX,
        }
    }

    /// Get the current bypass state.
    #[inline]
    pub fn state(&self) -> BypassState {
        self.state
    }

    /// Returns true if currently in a ramping (crossfading) state.
    #[inline]
    pub fn is_ramping(&self) -> bool {
        matches!(
            self.state,
            BypassState::RampingToBypassed | BypassState::RampingToActive
        )
    }

    /// Returns true if fully bypassed (not ramping).
    #[inline]
    pub fn is_bypassed(&self) -> bool {
        self.state == BypassState::Bypassed
    }

    /// Returns true if fully active (not ramping, not bypassed).
    #[inline]
    pub fn is_active(&self) -> bool {
        self.state == BypassState::Active
    }

    /// Get the configured ramp length in samples.
    #[inline]
    pub fn ramp_samples(&self) -> u32 {
        self.ramp_samples
    }

    /// Set the ramp length. Takes effect at once: a ramp in progress
    /// continues from where it is, cut to the new length.
    pub fn set_ramp_samples(&mut self, samples: u32) {
        self.ramp_samples = samples;
        self.ramp_position = self.ramp_position.min(samples);
    }

    /// Set the crossfade curve. Takes effect on next state transition.
    pub fn set_curve(&mut self, curve: CrossfadeCurve) {
        self.curve = curve;
    }

    /// Get the largest latency the handler delays by, as given to
    /// [`new()`](Self::new).
    #[inline]
    pub fn max_latency(&self) -> u32 {
        self.max_latency as u32
    }

    /// Set the shortest warm-up in samples: how long the DSP runs after a
    /// bypass before the handler crossfades to it. The warm-up is the
    /// latency of the block or this value, whichever is longer. Set it for
    /// DSP whose memory reaches further back than its latency (overlapping
    /// FFT windows, a long envelope). Default 0: the latency.
    pub fn set_warmup_samples(&mut self, samples: u32) {
        self.warmup_samples = samples as usize;
    }

    /// Get the shortest warm-up in samples (see
    /// [`set_warmup_samples()`](Self::set_warmup_samples)).
    #[inline]
    pub fn warmup_samples(&self) -> u32 {
        self.warmup_samples as u32
    }

    /// Clear the dry signal and finish any transition at once.
    ///
    /// Call this where the plugin resets its DSP, in
    /// [`Processor::set_active()`](crate::Processor::set_active). It does
    /// not allocate, so it is safe on the audio thread (where an Audio
    /// Unit host's reset runs). The handler goes to the state it was
    /// heading for: bypassed or active. After a reset while bypassed the
    /// output is silent for the latency, until the delayed input arrives.
    pub fn reset(&mut self) {
        for ring in &mut self.dry {
            ring.fill(0.0);
        }
        self.write_position = 0;
        self.block_start = 0;
        self.dry_channels = 0;
        self.dry_samples = 0;
        self.processed_since_resume = usize::MAX;
        match self.state {
            BypassState::Bypassed | BypassState::RampingToBypassed => {
                self.state = BypassState::Bypassed;
                self.ramp_position = self.ramp_samples;
            }
            BypassState::Active | BypassState::RampingToActive => {
                self.state = BypassState::Active;
                self.ramp_position = 0;
            }
        }
    }

    /// Begin bypass processing for this buffer.
    ///
    /// Call this at the start of your `process()` method, before anything
    /// writes to the buffer. It updates the internal state and returns what
    /// action you should take. It copies the input (the dry signal) for
    /// `finish()` and for the blocks to come.
    ///
    /// # Arguments
    /// * `bypassed` - Current bypass parameter state (true = bypassed)
    /// * `latency` - The latency of this block (`latency_samples()`). A
    ///   value above the `max_latency` given to `new()` is clamped, which
    ///   misaligns the dry signal against the latency the host was told
    ///   (debug builds assert). A latency that changes while the handler
    ///   crossfades or is bypassed moves the dry signal in time once: a
    ///   click, at the moment the host aligns the track again.
    /// * `buffer` - The buffer of this block, still holding the input
    ///
    /// # Returns
    /// A [`BypassAction`] telling you what to do:
    /// - `Passthrough`: Nothing, the buffer holds the delayed input
    /// - `Process`: Run your DSP normally
    /// - `ProcessAndCrossfade`: Run your DSP, then call `finish()`
    /// - `Resume`: Clear your DSP's memory, run it, then call `finish()`
    ///
    /// # Example
    ///
    /// ```ignore
    /// let latency = self.latency_samples();
    /// match self.bypass_handler.begin(is_bypassed, latency, buffer) {
    ///     BypassAction::Passthrough => {}
    ///     BypassAction::Process => self.process_dsp(buffer),
    ///     BypassAction::ProcessAndCrossfade => {
    ///         self.process_dsp(buffer);
    ///         self.bypass_handler.finish(buffer);
    ///     }
    ///     BypassAction::Resume => {
    ///         self.clear_dsp();
    ///         self.process_dsp(buffer);
    ///         self.bypass_handler.finish(buffer);
    ///     }
    /// }
    /// ```
    pub fn begin<S: Sample>(
        &mut self,
        bypassed: bool,
        latency: u32,
        buffer: &mut Buffer<S>,
    ) -> BypassAction {
        debug_assert!(
            latency as usize <= self.max_latency,
            "latency {latency} is above the largest latency the BypassHandler was made for ({})",
            self.max_latency
        );
        self.latency = (latency as usize).min(self.max_latency);
        // The warm-up depends on the latency of this block.
        let resuming = self.set_bypass(bypassed);

        // Without a latency only a crossfade needs the input. With one,
        // every block feeds the delay.
        if self.max_latency > 0 || self.is_ramping() {
            self.push_dry(buffer);
        }

        match self.state {
            BypassState::Bypassed => {
                if self.latency > 0 {
                    self.write_delayed_dry(buffer);
                }
                BypassAction::Passthrough
            }
            _ if resuming => BypassAction::Resume,
            BypassState::Active => BypassAction::Process,
            BypassState::RampingToBypassed | BypassState::RampingToActive => BypassAction::ProcessAndCrossfade,
        }
    }

    /// Finish bypass processing by applying the crossfade.
    ///
    /// Call this AFTER your DSP processing when `begin()` returned
    /// `BypassAction::ProcessAndCrossfade` or `BypassAction::Resume`.
    ///
    /// This blends the wet signal (in the buffer) with the dry signal (copied
    /// by `begin()` and delayed by the latency) according to the current
    /// ramp position.
    ///
    /// # Arguments
    /// * `buffer` - The buffer containing the processed (wet) signal
    pub fn finish<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        self.apply_crossfade(buffer);
    }

    /// Copy the channels that hold an input into the rings, after the
    /// blocks before. A block longer than the longest block `new()` was
    /// given does not come from Beamer's wrappers: it is copied up to that
    /// length and the rest keeps what the buffer held. A block without input
    /// channels still has samples to crossfade (to silence).
    fn push_dry<S: Sample>(&mut self, buffer: &Buffer<S>) {
        debug_assert!(
            buffer.num_samples() <= self.max_buffer_size,
            "block of {} samples is longer than the BypassHandler was made for ({})",
            buffer.num_samples(),
            self.max_buffer_size
        );
        let channels = buffer
            .num_input_channels()
            .min(buffer.num_channels())
            .min(self.dry.len());
        let samples = buffer.num_samples().min(self.max_buffer_size);
        for (ring, channel) in self.dry.iter_mut().zip(buffer.channels()).take(channels) {
            let mut position = self.write_position;
            for sample in &channel[..samples] {
                ring[position] = sample.to_f64();
                position += 1;
                if position == self.ring_len {
                    position = 0;
                }
            }
        }
        self.block_start = self.write_position;
        if self.ring_len > 0 {
            self.write_position = (self.write_position + samples) % self.ring_len;
        }
        self.dry_channels = channels;
        self.dry_samples = samples;
    }

    /// Where the dry signal of sample `index` of the current block is in
    /// the rings: `latency` samples before the input of that sample.
    #[inline]
    fn dry_position(&self, index: usize) -> usize {
        (self.block_start + index + self.ring_len - self.latency) % self.ring_len
    }

    /// Write the delayed dry signal into the channels that hold an input.
    /// The other channels are silent already.
    fn write_delayed_dry<S: Sample>(&self, buffer: &mut Buffer<S>) {
        for ch in 0..self.dry_channels {
            let channel = buffer.channel(ch);
            for (index, sample) in channel[..self.dry_samples].iter_mut().enumerate() {
                *sample = S::from_f64(self.dry[ch][self.dry_position(index)]);
            }
        }
    }

    /// The warm-up of the current block: the latency or the plugin's
    /// shortest warm-up, whichever is longer.
    #[inline]
    fn warmup(&self) -> usize {
        self.latency.max(self.warmup_samples)
    }

    /// Update the state for the bypass parameter of this block. Returns
    /// true when processing starts again after a bypass.
    fn set_bypass(&mut self, bypassed: bool) -> bool {
        match (self.state, bypassed) {
            // Start ramping to bypassed, or bypass at once without a ramp.
            (BypassState::Active, true) => {
                if self.ramp_samples == 0 {
                    self.state = BypassState::Bypassed;
                } else {
                    self.state = BypassState::RampingToBypassed;
                }
                self.ramp_position = 0;
                false
            }
            // Reverse: was ramping to bypass, now going back to active. The
            // DSP ran all along, so the ramp goes back from where it is.
            (BypassState::RampingToBypassed, false) => {
                self.state = BypassState::RampingToActive;
                false
            }
            // Processing starts again: the dry signal holds for the
            // warm-up, then the ramp (if any) starts.
            (BypassState::Bypassed, false) => {
                self.processed_since_resume = 0;
                if self.ramp_samples > 0 || self.warmup() > 0 {
                    self.state = BypassState::RampingToActive;
                    self.ramp_position = self.ramp_samples;
                } else {
                    self.state = BypassState::Active;
                    self.ramp_position = 0;
                }
                true
            }
            // Reverse: was ramping to active (or warming up), now going
            // back to bypass from where the ramp is.
            (BypassState::RampingToActive, true) => {
                if self.ramp_samples == 0 {
                    self.state = BypassState::Bypassed;
                } else {
                    self.state = BypassState::RampingToBypassed;
                }
                false
            }
            // Already in correct stable state, or continuing ramp
            _ => false,
        }
    }

    fn apply_crossfade<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        // Nothing to mix once a transition is over (a resume without
        // warm-up or ramp goes straight to active).
        if !self.is_ramping() {
            return;
        }

        // All channels are crossfaded, over the samples begin() copied the
        // dry signal of. A channel without an input (mono to stereo) has a
        // silent dry signal, which Passthrough leaves when bypassed.
        let num_channels = buffer.num_channels();
        let num_samples = self.dry_samples.min(buffer.num_samples());
        if num_channels == 0 {
            return;
        }

        let ramping_to_bypass = self.state == BypassState::RampingToBypassed;
        let warmup = self.warmup();

        // Process sample by sample
        for sample_idx in 0..num_samples {
            // On the way back the dry signal holds until the DSP has run
            // for the warm-up.
            let warming_up = !ramping_to_bypass && self.processed_since_resume < warmup;

            // Calculate normalized position (0.0 = wet, 1.0 = dry). The
            // warm-up is all dry; without a ramp it is all dry or all wet.
            let t = if warming_up {
                1.0
            } else if self.ramp_samples == 0 {
                if ramping_to_bypass { 1.0 } else { 0.0 }
            } else {
                (self.ramp_position as f64) / (self.ramp_samples as f64)
            };
            let (wet_gain, dry_gain): (S, S) = self.curve.gains(t);

            // Apply crossfade to all channels for this sample
            let dry_position = self.dry_position(sample_idx);
            for ch in 0..num_channels {
                let dry = if ch < self.dry_channels {
                    S::from_f64(self.dry[ch][dry_position])
                } else {
                    S::ZERO
                };
                let sample = &mut buffer.channel(ch)[sample_idx];
                *sample = *sample * wet_gain + dry * dry_gain;
            }

            // Advance ramp position (once per sample)
            self.processed_since_resume = self.processed_since_resume.saturating_add(1);
            if ramping_to_bypass {
                self.ramp_position = (self.ramp_position + 1).min(self.ramp_samples);
            } else if !warming_up {
                self.ramp_position = self.ramp_position.saturating_sub(1);
            }
        }

        // Check if ramp complete
        if ramping_to_bypass && self.ramp_position >= self.ramp_samples {
            self.state = BypassState::Bypassed;
        } else if !ramping_to_bypass && self.ramp_position == 0 && self.processed_since_resume >= warmup {
            self.state = BypassState::Active;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn test_crossfade_mixes_the_dry_signal_copied_before_processing() {
        // Mono in, stereo out: channel 0 holds the input, channel 1 starts
        // silent. A linear ramp of 4 samples to bypassed.
        let mut handler = BypassHandler::new(4, CrossfadeCurve::Linear, 1, 8, 0);
        let mut left = [1.0f32; 4];
        let mut right = [0.0f32; 4];
        let mut buffer = Buffer::new([&mut left[..], &mut right[..]], std::iter::empty(), 1, 4);

        assert_eq!(handler.begin(true, 0, &mut buffer), BypassAction::ProcessAndCrossfade);
        // The DSP writes the wet signal over the input, in place.
        for channel in buffer.channels_mut() {
            channel.fill(0.5);
        }
        handler.finish(&mut buffer);

        // Wet 0.5 fades to the dry input 1.0 on channel 0 and to silence on
        // channel 1, which has no input.
        assert_eq!(left, [0.5, 0.625, 0.75, 0.875]);
        assert_eq!(right, [0.5, 0.375, 0.25, 0.125]);
        assert_eq!(handler.state(), BypassState::Bypassed);
    }

    /// A plugin with latency: its wet signal is its input `latency`
    /// samples later. Its delay is fed only while it processes. A silent
    /// one outputs silence after its latency, to show where the handler
    /// holds the dry signal.
    struct DelayPlugin {
        lines: Vec<VecDeque<f32>>,
        silent: bool,
    }

    impl DelayPlugin {
        fn new(channels: usize, latency: usize) -> Self {
            Self {
                lines: vec![VecDeque::from(vec![0.0; latency]); channels],
                silent: false,
            }
        }

        fn silent(channels: usize, latency: usize) -> Self {
            Self { silent: true, ..Self::new(channels, latency) }
        }

        /// Clear the delay, as a plugin does on `Resume`.
        fn clear(&mut self) {
            for line in &mut self.lines {
                line.iter_mut().for_each(|sample| *sample = 0.0);
            }
        }

        fn process(&mut self, buffer: &mut Buffer<f32>) {
            for (line, channel) in self.lines.iter_mut().zip(buffer.channels_mut()) {
                for sample in channel {
                    line.push_back(if self.silent { 0.0 } else { *sample });
                    *sample = line.pop_front().unwrap();
                }
            }
        }
    }

    /// Run a mono `input` through the handler and the plugin in blocks of
    /// `block` samples, with the bypass and the latency of each block.
    /// Returns the output and the action of each block.
    fn run(
        handler: &mut BypassHandler,
        plugin: &mut DelayPlugin,
        input: &[f32],
        block: usize,
        latency: impl Fn(usize) -> u32,
        bypassed: impl Fn(usize) -> bool,
    ) -> (Vec<f32>, Vec<BypassAction>) {
        let mut output = Vec::new();
        let mut actions = Vec::new();
        for (index, chunk) in input.chunks(block).enumerate() {
            let mut samples = chunk.to_vec();
            let len = samples.len();
            let mut buffer = Buffer::new([&mut samples[..]], std::iter::empty(), 1, len);
            let action = handler.begin(bypassed(index), latency(index), &mut buffer);
            match action {
                BypassAction::Passthrough => {}
                BypassAction::Process => plugin.process(&mut buffer),
                BypassAction::ProcessAndCrossfade => {
                    plugin.process(&mut buffer);
                    handler.finish(&mut buffer);
                }
                BypassAction::Resume => {
                    plugin.clear();
                    plugin.process(&mut buffer);
                    handler.finish(&mut buffer);
                }
            }
            output.extend_from_slice(&samples);
            actions.push(action);
        }
        (output, actions)
    }

    /// Assert that `output` is `input` `latency` samples later.
    fn assert_delayed(output: &[f32], input: &[f32], latency: usize, what: &str) {
        for (index, sample) in output.iter().enumerate() {
            let expected = if index < latency { 0.0 } else { input[index - latency] };
            assert!(
                (sample - expected).abs() < 1e-4,
                "{what}, sample {index}: {sample} is not {expected}"
            );
        }
    }

    /// A rising input, so that a sample from the wrong time shows.
    fn rising(len: usize) -> Vec<f32> {
        (1..=len).map(|n| n as f32).collect()
    }

    #[test]
    fn test_dry_signal_is_delayed_by_the_latency_through_the_ramp_and_bypassed() {
        // The wet and the dry signal are the input 3 samples later, so the
        // output is that through the ramp and once bypassed. Blocks that
        // do and do not divide the ramp; a largest latency above the
        // latency.
        let input = rising(48);
        for block in [4, 5] {
            for max_latency in [3, 8] {
                let mut handler = BypassHandler::new(8, CrossfadeCurve::Linear, 1, 8, max_latency);
                let mut plugin = DelayPlugin::new(1, 3);
                let (output, _) = run(&mut handler, &mut plugin, &input, block, |_| 3, |b| b >= 2);
                assert_delayed(&output, &input, 3, &format!("block {block}, largest latency {max_latency}"));
                assert_eq!(handler.state(), BypassState::Bypassed);
            }
        }
    }

    #[test]
    fn test_bypass_and_back_keeps_the_delayed_input_with_and_without_a_ramp() {
        // Bypassed for blocks 2 to 5, then processing again. The plugin
        // clears its delay on Resume; the handler holds the dry signal
        // for the latency, so the output never shows the cleared delay.
        let input = rising(64);
        for ramp in [0, 6] {
            let mut handler = BypassHandler::new(ramp, CrossfadeCurve::Linear, 1, 4, 5);
            let mut plugin = DelayPlugin::new(1, 5);
            let (output, actions) = run(&mut handler, &mut plugin, &input, 4, |_| 5, |b| (2..6).contains(&b));
            assert_delayed(&output, &input, 5, &format!("ramp {ramp}"));
            assert_eq!(actions[6], BypassAction::Resume, "ramp {ramp}");
            assert_ne!(actions[7], BypassAction::Resume, "ramp {ramp}");
            assert_eq!(handler.state(), BypassState::Active, "ramp {ramp}");
        }
    }

    #[test]
    fn test_warmup_holds_the_dry_signal_for_the_latency_or_longer() {
        // A silent plugin with a latency of 3: after Resume the output is
        // the delayed input for the warm-up, then fades out over the ramp
        // (whose first sample is still all dry).
        let input = vec![1.0f32; 40];
        for (warmup, expected_hold) in [(0, 3), (10, 10)] {
            let mut handler = BypassHandler::new(4, CrossfadeCurve::Linear, 1, 2, 3);
            handler.set_warmup_samples(warmup);
            let mut plugin = DelayPlugin::silent(1, 3);
            // Bypassed from the start, processing again from sample 20.
            let (output, _) = run(&mut handler, &mut plugin, &input, 2, |_| 3, |b| b < 10);
            let resumed = &output[20..];
            assert!(
                resumed[..expected_hold].iter().all(|&sample| sample == 1.0),
                "warm-up {warmup}: {resumed:?}"
            );
            assert_eq!(resumed[expected_hold], 1.0, "warm-up {warmup}: {resumed:?}");
            assert!(resumed[expected_hold + 1] < 1.0, "warm-up {warmup}: {resumed:?}");
            assert_eq!(resumed[expected_hold + 4], 0.0, "warm-up {warmup}: {resumed:?}");
        }
    }

    #[test]
    fn test_latency_that_drops_during_the_warmup_ends_it() {
        // Latency 8 when processing starts again, 2 from the next block on:
        // 4 samples have run by then, more than the new warm-up, so the
        // ramp starts at once (its first sample still all dry).
        let input = vec![1.0f32; 40];
        let mut handler = BypassHandler::new(4, CrossfadeCurve::Linear, 1, 4, 8);
        let mut plugin = DelayPlugin::silent(1, 2);
        let (output, _) = run(&mut handler, &mut plugin, &input, 4, |b| if b < 6 { 8 } else { 2 }, |b| b < 5);
        assert_eq!(output[20..25], [1.0; 5]);
        assert!(output[25] < 1.0, "{:?}", &output[20..]);
    }

    #[test]
    fn test_bypass_again_during_the_warmup_stays_on_the_dry_signal() {
        // Processing again in block 3 with a warm-up of 8, bypassed again
        // in block 4 and processing again in block 6: the output is the
        // delayed input throughout. With a ramp the bypass in block 4 ends
        // within the block, without one it is at once.
        let input = rising(48);
        for (ramp, block_4) in [(4, BypassAction::ProcessAndCrossfade), (0, BypassAction::Passthrough)] {
            let mut handler = BypassHandler::new(ramp, CrossfadeCurve::Linear, 1, 2, 8);
            let mut plugin = DelayPlugin::new(1, 8);
            let (output, actions) = run(&mut handler, &mut plugin, &input, 2, |_| 8, |b| b < 3 || b == 4 || b == 5);
            assert_delayed(&output, &input, 8, &format!("ramp {ramp}"));
            assert_eq!(actions[3], BypassAction::Resume, "ramp {ramp}");
            assert_eq!(actions[4], block_4, "ramp {ramp}");
            assert_eq!(actions[5], BypassAction::Passthrough, "ramp {ramp}");
            assert_eq!(actions[6], BypassAction::Resume, "ramp {ramp}");
            assert_eq!(handler.state(), BypassState::Active, "ramp {ramp}");
        }
    }

    #[test]
    fn test_back_to_active_mid_ramp_continues_the_ramp_without_a_warmup() {
        // A silent plugin with a latency of 2, so the output is the dry
        // gain. Bypassed in blocks 1 and 2 (half of the ramp of 8), then
        // back: the DSP ran all along, so no Resume and no hold; the ramp
        // goes back from where it was.
        let input = vec![1.0f32; 24];
        let mut handler = BypassHandler::new(8, CrossfadeCurve::Linear, 1, 2, 2);
        let mut plugin = DelayPlugin::silent(1, 2);
        let (output, actions) = run(&mut handler, &mut plugin, &input, 2, |_| 2, |b| b == 1 || b == 2);
        assert_eq!(output[2..11], [0.0, 0.125, 0.25, 0.375, 0.5, 0.375, 0.25, 0.125, 0.0]);
        assert!(!actions.contains(&BypassAction::Resume), "{actions:?}");
        assert_eq!(handler.state(), BypassState::Active);
    }

    #[test]
    fn test_latency_change_while_bypassed_moves_the_dry_signal() {
        // Bypassed with a latency of 2, then 4: from the change on the
        // output is the input 4 samples later.
        let input = rising(24);
        let mut handler = BypassHandler::new(0, CrossfadeCurve::Linear, 1, 4, 4);
        let mut plugin = DelayPlugin::new(1, 0);
        let (output, _) = run(&mut handler, &mut plugin, &input, 4, |b| if b < 3 { 2 } else { 4 }, |_| true);
        assert_delayed(&output[..12], &input, 2, "latency 2");
        assert_eq!(output[12..], input[8..20]);
    }

    #[test]
    fn test_resume_without_latency_or_ramp_goes_straight_to_active() {
        let mut handler = BypassHandler::new(0, CrossfadeCurve::Linear, 1, 4, 0);
        let mut plugin = DelayPlugin::new(1, 0);
        let input = rising(12);
        let (output, actions) = run(&mut handler, &mut plugin, &input, 4, |_| 0, |b| b == 0);
        assert_eq!(output, input);
        assert_eq!(actions, [BypassAction::Passthrough, BypassAction::Resume, BypassAction::Process]);
        assert_eq!(handler.state(), BypassState::Active);
    }

    #[test]
    fn test_reset_clears_the_dry_signal_and_ends_the_transition() {
        // Bypassed with a latency of 2, then reset: the next block starts
        // with silence for the latency.
        let mut handler = BypassHandler::new(4, CrossfadeCurve::Linear, 1, 4, 2);
        let mut plugin = DelayPlugin::new(1, 2);
        let input = rising(16);
        run(&mut handler, &mut plugin, &input, 4, |_| 2, |_| true);
        handler.reset();
        assert_eq!(handler.state(), BypassState::Bypassed);
        let mut samples = [20.0f32, 21.0, 22.0, 23.0];
        let mut buffer = Buffer::new([&mut samples[..]], std::iter::empty(), 1, 4);
        assert_eq!(handler.begin(true, 2, &mut buffer), BypassAction::Passthrough);
        assert_eq!(samples, [0.0, 0.0, 20.0, 21.0]);

        // Reset during a ramp to active ends it there.
        let mut samples = [0.0f32; 4];
        let mut buffer = Buffer::new([&mut samples[..]], std::iter::empty(), 1, 4);
        assert_eq!(handler.begin(false, 2, &mut buffer), BypassAction::Resume);
        handler.reset();
        assert_eq!(handler.state(), BypassState::Active);
    }

    #[test]
    fn test_bypassed_mono_to_stereo_delays_the_input_channel_only() {
        // Bypass at once (no ramp) with a latency of 2: channel 0 gets the
        // input 2 samples later, also across blocks; channel 1 has no input
        // and stays silent.
        let mut handler = BypassHandler::new(0, CrossfadeCurve::Linear, 1, 4, 2);
        let mut left = [1.0f32, 2.0, 3.0, 4.0];
        let mut right = [0.0f32; 4];
        let mut buffer = Buffer::new([&mut left[..], &mut right[..]], std::iter::empty(), 1, 4);
        assert_eq!(handler.begin(true, 2, &mut buffer), BypassAction::Passthrough);
        assert_eq!(left, [0.0, 0.0, 1.0, 2.0]);
        assert_eq!(right, [0.0; 4]);

        let mut left = [5.0f32, 6.0, 7.0, 8.0];
        let mut right = [0.0f32; 4];
        let mut buffer = Buffer::new([&mut left[..], &mut right[..]], std::iter::empty(), 1, 4);
        assert_eq!(handler.begin(true, 2, &mut buffer), BypassAction::Passthrough);
        assert_eq!(left, [3.0, 4.0, 5.0, 6.0]);
        assert_eq!(right, [0.0; 4]);
    }

    #[test]
    fn test_linear_and_s_curve_keep_the_level_of_related_signals() {
        // Crossfading a signal with itself: the gains add up to 1 for the
        // curves meant for a bypass.
        for curve in [CrossfadeCurve::Linear, CrossfadeCurve::SCurve] {
            for step in 0..=10 {
                let (wet, dry): (f64, f64) = curve.gains(f64::from(step) / 10.0);
                assert!((wet + dry - 1.0).abs() < 1e-12, "{curve:?} at {step}");
            }
        }
        // Equal power raises a related signal by 3 dB in the middle.
        let (wet, dry): (f64, f64) = CrossfadeCurve::EqualPower.gains(0.5);
        assert!((wet + dry - std::f64::consts::SQRT_2).abs() < 1e-12);
    }
}
