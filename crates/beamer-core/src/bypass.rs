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
//! # Example
//!
//! ```ignore
//! use beamer_core::{BypassHandler, BypassAction, CrossfadeCurve, Buffer};
//!
//! struct MyPlugin {
//!     bypass_handler: BypassHandler,
//!     gain: f32,
//! }
//!
//! impl Processor for MyPlugin {
//!     fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
//!         let is_bypassed = self.parameters.bypass.get();
//!
//!         match self.bypass_handler.begin(is_bypassed) {
//!             BypassAction::Passthrough => {
//!                 // Fully bypassed - just copy input to output
//!                 buffer.copy_to_output();
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
    /// Transitioning from bypassed to active (crossfading to wet).
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
    /// Plugin is fully bypassed. Copy input to output (or call `buffer.copy_to_output()`).
    /// No DSP processing needed.
    Passthrough,

    /// Plugin is fully active. Run your DSP normally.
    /// No crossfade needed.
    Process,

    /// Plugin is transitioning. Run your DSP, then call `bypass_handler.finish(buffer)`
    /// to apply the crossfade.
    ProcessAndCrossfade,
}

// =============================================================================
// CrossfadeCurve
// =============================================================================

/// Crossfade curve shape for bypass transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CrossfadeCurve {
    /// Linear crossfade. Simple but may have slight loudness dip at center.
    /// gain = position (0.0 to 1.0)
    #[default]
    Linear,

    /// Equal-power crossfade. Maintains constant loudness during transition.
    /// gain = sin(position * PI/2) for fade-in, cos(position * PI/2) for fade-out
    EqualPower,

    /// S-curve crossfade. Faster start/end, smoother middle.
    /// gain = 3x^2 - 2x^3 (smoothstep)
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
                let smooth = t * t * (3.0 - 2.0 * t); // smoothstep
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
/// match self.bypass_handler.begin(is_bypassed) {
///     BypassAction::Passthrough => buffer.copy_to_output(),
///     BypassAction::Process => self.process_dsp(buffer),
///     BypassAction::ProcessAndCrossfade => {
///         self.process_dsp(buffer);
///         self.bypass_handler.finish(buffer);
///     }
/// }
/// ```
///
/// # Sample Type Flexibility
///
/// BypassHandler is not generic over sample type. The `finish()` method
/// is generic, so a single BypassHandler instance can process both
/// `Buffer<f32>` and `Buffer<f64>` buffers.
///
/// # Real-Time Safety
///
/// This struct performs no heap allocations and is safe to use in
/// audio processing callbacks.
pub struct BypassHandler {
    /// Current bypass state
    state: BypassState,
    /// Current position in ramp (0 = start, ramp_samples = end)
    ramp_position: u32,
    /// Total ramp length in samples
    ramp_samples: u32,
    /// Crossfade curve to use
    curve: CrossfadeCurve,
}

impl BypassHandler {
    /// Create a new bypass handler.
    ///
    /// # Arguments
    /// * `ramp_samples` - Number of samples for crossfade (0 = instant bypass)
    /// * `curve` - Crossfade curve shape
    pub fn new(ramp_samples: u32, curve: CrossfadeCurve) -> Self {
        Self {
            state: BypassState::Active,
            ramp_position: 0,
            ramp_samples,
            curve,
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

    /// Set the ramp length. Takes effect on next state transition.
    pub fn set_ramp_samples(&mut self, samples: u32) {
        self.ramp_samples = samples;
    }

    /// Set the crossfade curve. Takes effect on next state transition.
    pub fn set_curve(&mut self, curve: CrossfadeCurve) {
        self.curve = curve;
    }

    /// Begin bypass processing for this buffer.
    ///
    /// Call this at the start of your `process()` method. It updates the internal
    /// state and returns what action you should take.
    ///
    /// # Arguments
    /// * `bypassed` - Current bypass parameter state (true = bypassed)
    ///
    /// # Returns
    /// A [`BypassAction`] telling you what to do:
    /// - `Passthrough`: Just copy input to output, no DSP needed
    /// - `Process`: Run your DSP normally
    /// - `ProcessAndCrossfade`: Run your DSP, then call `finish()`
    ///
    /// # Example
    ///
    /// ```ignore
    /// match self.bypass_handler.begin(is_bypassed) {
    ///     BypassAction::Passthrough => buffer.copy_to_output(),
    ///     BypassAction::Process => self.process_dsp(buffer),
    ///     BypassAction::ProcessAndCrossfade => {
    ///         self.process_dsp(buffer);
    ///         self.bypass_handler.finish(buffer);
    ///     }
    /// }
    /// ```
    pub fn begin(&mut self, bypassed: bool) -> BypassAction {
        self.set_bypass(bypassed);

        match self.state {
            BypassState::Bypassed => BypassAction::Passthrough,
            BypassState::Active => BypassAction::Process,
            BypassState::RampingToBypassed | BypassState::RampingToActive => {
                BypassAction::ProcessAndCrossfade
            }
        }
    }

    /// Finish bypass processing by applying the crossfade.
    ///
    /// Call this AFTER your DSP processing when `begin()` returned
    /// `BypassAction::ProcessAndCrossfade`.
    ///
    /// This blends the wet signal (in output buffer) with the dry signal
    /// (in input buffer) according to the current ramp position.
    ///
    /// # Arguments
    /// * `buffer` - The buffer containing processed (wet) output and original (dry) input
    pub fn finish<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        let num_samples = buffer.num_samples();
        self.apply_crossfade(buffer, num_samples);
    }

    /// Update bypass target state (internal).
    fn set_bypass(&mut self, bypassed: bool) {
        // Handle instant bypass (zero ramp) - snap directly to final state
        if self.ramp_samples == 0 {
            let target = if bypassed {
                BypassState::Bypassed
            } else {
                BypassState::Active
            };
            if self.state != target {
                self.state = target;
                self.ramp_position = 0;
            }
            return;
        }

        match (self.state, bypassed) {
            // Start ramping to bypassed
            (BypassState::Active, true) => {
                self.state = BypassState::RampingToBypassed;
                self.ramp_position = 0;
            }
            // Reverse: was ramping to bypass, now going back to active
            (BypassState::RampingToBypassed, false) => {
                self.state = BypassState::RampingToActive;
                // Keep current ramp_position for smooth reversal
            }
            // Start ramping to active
            (BypassState::Bypassed, false) => {
                self.state = BypassState::RampingToActive;
                self.ramp_position = self.ramp_samples;
            }
            // Reverse: was ramping to active, now going back to bypass
            (BypassState::RampingToActive, true) => {
                self.state = BypassState::RampingToBypassed;
                // Keep current ramp_position for smooth reversal
            }
            // Already in correct stable state, or continuing ramp
            _ => {}
        }
    }

    fn apply_crossfade<S: Sample>(&mut self, buffer: &mut Buffer<S>, num_samples: usize) {
        // Guard: instant bypass when ramp_samples is 0
        if self.ramp_samples == 0 {
            return;
        }

        // Guard: no channels to process
        let num_channels = buffer.num_input_channels().min(buffer.num_output_channels());
        if num_channels == 0 {
            return;
        }

        let ramp_samples_f = self.ramp_samples as f64;
        let ramping_to_bypass = self.state == BypassState::RampingToBypassed;

        // Process sample by sample
        for sample_idx in 0..num_samples {
            // Calculate normalized position (0.0 = wet, 1.0 = dry)
            let t = (self.ramp_position as f64) / ramp_samples_f;
            let (wet_gain, dry_gain): (S, S) = self.curve.gains(t);

            // Apply crossfade to all channels for this sample
            for ch in 0..num_channels {
                let dry = buffer.input(ch)[sample_idx];
                let wet = buffer.output(ch)[sample_idx];
                buffer.output(ch)[sample_idx] = wet * wet_gain + dry * dry_gain;
            }

            // Advance ramp position (once per sample)
            if ramping_to_bypass {
                self.ramp_position = (self.ramp_position + 1).min(self.ramp_samples);
            } else {
                self.ramp_position = self.ramp_position.saturating_sub(1);
            }
        }

        // Check if ramp complete
        if ramping_to_bypass && self.ramp_position >= self.ramp_samples {
            self.state = BypassState::Bypassed;
        } else if !ramping_to_bypass && self.ramp_position == 0 {
            self.state = BypassState::Active;
        }
    }
}

impl Default for BypassHandler {
    /// Create a bypass handler with default settings (64 samples, linear curve).
    fn default() -> Self {
        Self::new(64, CrossfadeCurve::Linear)
    }
}
