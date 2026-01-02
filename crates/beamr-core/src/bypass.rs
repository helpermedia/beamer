//! Bypass handling with smooth crossfading.
//!
//! This module provides utilities for implementing soft bypass in audio plugins.
//! Soft bypass crossfades between wet (processed) and dry (passthrough) signals
//! to avoid clicks when bypass is toggled.
//!
//! # Overview
//!
//! - [`BypassState`] - Current bypass state (Active, Bypassed, or transitioning)
//! - [`CrossfadeCurve`] - Crossfade curve shape (Linear, EqualPower, SCurve)
//! - [`BypassHandler`] - Main utility for handling bypass with automatic crossfading
//!
//! # Example
//!
//! ```ignore
//! use beamr_core::{BypassHandler, CrossfadeCurve, Buffer, AudioProcessor, AuxiliaryBuffers, ProcessContext};
//!
//! struct MyPlugin {
//!     bypass_handler: BypassHandler,
//!     // ...
//! }
//!
//! impl AudioProcessor for MyPlugin {
//!     fn setup(&mut self, sample_rate: f64, _max_buffer_size: usize) {
//!         // 10ms crossfade with equal-power curve
//!         let ramp_samples = (sample_rate * 0.01) as u32;
//!         self.bypass_handler = BypassHandler::new(ramp_samples, CrossfadeCurve::EqualPower);
//!     }
//!
//!     fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, ctx: &ProcessContext) {
//!         let is_bypassed = self.params.bypass_value() > 0.5;
//!
//!         self.bypass_handler.process(buffer, is_bypassed, |buf| {
//!             // Your DSP code here - only called when processing is needed
//!             self.apply_reverb(buf, aux);
//!         });
//!     }
//!
//!     fn bypass_ramp_samples(&self) -> u32 {
//!         self.bypass_handler.ramp_samples()
//!     }
//! }
//! ```

use crate::buffer::Buffer;

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

    /// S-curve crossfade. Attempt at faster start/end, smoother middle.
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
    /// Tuple of (wet_gain, dry_gain)
    #[inline]
    pub fn gains(&self, t: f32) -> (f32, f32) {
        match self {
            CrossfadeCurve::Linear => (1.0 - t, t),
            CrossfadeCurve::EqualPower => {
                let angle = t * std::f32::consts::FRAC_PI_2;
                (angle.cos(), angle.sin())
            }
            CrossfadeCurve::SCurve => {
                let smooth = t * t * (3.0 - 2.0 * t); // smoothstep
                (1.0 - smooth, smooth)
            }
        }
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
/// # Real-Time Safety
///
/// This struct performs no heap allocations and is safe to use in
/// audio processing callbacks.
///
/// # Example
///
/// ```ignore
/// use beamr_core::{BypassHandler, CrossfadeCurve, Buffer};
///
/// struct MyPlugin {
///     bypass_handler: BypassHandler,
///     // ...
/// }
///
/// impl AudioProcessor for MyPlugin {
///     fn setup(&mut self, sample_rate: f64, _max_buffer_size: usize) {
///         // 10ms crossfade with equal-power curve
///         let ramp_samples = (sample_rate * 0.01) as u32;
///         self.bypass_handler = BypassHandler::new(ramp_samples, CrossfadeCurve::EqualPower);
///     }
///
///     fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, ctx: &ProcessContext) {
///         let is_bypassed = self.params.bypass_value() > 0.5;
///
///         self.bypass_handler.process(buffer, is_bypassed, |buf| {
///             // Your DSP code here - only called when processing is needed
///             self.apply_reverb(buf, aux);
///         });
///     }
///
///     fn bypass_ramp_samples(&self) -> u32 {
///         self.bypass_handler.ramp_samples()
///     }
/// }
/// ```
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

    /// Update bypass target state.
    ///
    /// Call this at the start of each process() with the current bypass parameter value.
    /// State transitions happen automatically.
    pub fn set_bypass(&mut self, bypassed: bool) {
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

    /// Process audio with bypass handling.
    ///
    /// This is the main method plugins should call. It handles:
    /// - Passthrough when fully bypassed
    /// - Normal processing when fully active
    /// - Crossfading during transitions
    ///
    /// # Arguments
    /// * `buffer` - Audio buffer to process
    /// * `bypassed` - Current bypass parameter state (true = bypassed)
    /// * `process_fn` - Closure that performs DSP (only called when needed)
    ///
    /// # Behavior
    ///
    /// | State | process_fn called? | Output |
    /// |-------|-------------------|--------|
    /// | Active | Yes | Wet signal |
    /// | RampingToBypassed | Yes | Crossfade wet→dry |
    /// | Bypassed | No | Dry passthrough |
    /// | RampingToActive | Yes | Crossfade dry→wet |
    pub fn process<F>(&mut self, buffer: &mut Buffer, bypassed: bool, process_fn: F)
    where
        F: FnOnce(&mut Buffer),
    {
        // Update target state
        self.set_bypass(bypassed);

        let num_samples = buffer.num_samples();

        match self.state {
            BypassState::Bypassed => {
                // Fully bypassed: just copy input to output
                buffer.copy_to_output();
            }

            BypassState::Active => {
                // Fully active: just run DSP
                process_fn(buffer);
            }

            BypassState::RampingToBypassed | BypassState::RampingToActive => {
                // Need to crossfade - run DSP first to get wet signal
                process_fn(buffer);

                // Apply per-sample crossfade
                self.apply_crossfade(buffer, num_samples);
            }
        }
    }

    fn apply_crossfade(&mut self, buffer: &mut Buffer, num_samples: usize) {
        // Guard: instant bypass when ramp_samples is 0
        if self.ramp_samples == 0 {
            return;
        }

        // Guard: no channels to process
        let num_channels = buffer.num_input_channels().min(buffer.num_output_channels());
        if num_channels == 0 {
            return;
        }

        let ramp_samples_f = self.ramp_samples as f32;
        let ramping_to_bypass = self.state == BypassState::RampingToBypassed;

        // Process sample by sample
        for sample_idx in 0..num_samples {
            // Calculate normalized position (0.0 = wet, 1.0 = dry)
            let t = (self.ramp_position as f32) / ramp_samples_f;
            let (wet_gain, dry_gain) = self.curve.gains(t);

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
