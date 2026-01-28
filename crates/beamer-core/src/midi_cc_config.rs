//! Static MIDI CC configuration for VST3 IMidiMapping support.
//!
//! This module provides [`MidiCcConfig`], a compile-time configuration type that
//! specifies which MIDI controllers a plugin wants to receive. `MidiCcConfig`
//! is pure configuration - the framework handles runtime state internally via
//! [`MidiCcState`](crate::MidiCcState).
//!
//! # Usage
//!
//! ```ignore
//! use beamer_core::{Descriptor, MidiCcConfig};
//!
//! impl Descriptor for MySynth {
//!     fn midi_cc_config(&self) -> Option<MidiCcConfig> {
//!         // Use a preset for common configurations
//!         Some(MidiCcConfig::SYNTH_BASIC)
//!
//!         // Or build a custom configuration
//!         // Some(MidiCcConfig::new()
//!         //     .with_pitch_bend()
//!         //     .with_mod_wheel()
//!         //     .with_cc(7).with_cc(11).with_cc(64))
//!     }
//!
//!     // No midi_cc field needed!
//!     // No manual moving in prepare/unprepare!
//! }
//! ```
//!
//! # The VST3 MIDI Problem (Why This Exists)
//!
//! VST3 doesn't have a standard way to receive MIDI CC messages. Most DAWs don't
//! send `kLegacyMIDICCOutEvent` for input - instead, they expect plugins to use
//! the `IMidiMapping` interface, which maps MIDI controllers to parameters.
//!
//! This module implements the "hidden parameter trick":
//!
//! 1. Plugin returns `MidiCcConfig` from `midi_cc_config()` - pure configuration
//! 2. Framework creates hidden VST3 parameters for each enabled controller
//! 3. Framework implements `IMidiMapping` to map MIDI CCs to these parameters
//! 4. When the DAW sends MIDI CC, it becomes a parameter change
//! 5. Framework converts parameter changes back to `MidiEvent` for `process_midi()`
//! 6. Plugin can also read current values directly via `ProcessContext::midi_cc()`
//!
//! This allows your synth/effect to receive pitch bend, mod wheel, etc. in
//! DAWs that don't support raw MIDI CC input to VST3 plugins.
//!
//! # Const Fn Builder
//!
//! All builder methods are `const fn`, allowing compile-time configuration:
//!
//! ```ignore
//! const MY_MIDI_CONFIG: MidiCcConfig = MidiCcConfig::new()
//!     .with_pitch_bend()
//!     .with_mod_wheel();
//! ```
//!
//! # Presets
//!
//! Common configurations are available as constants:
//!
//! - [`MidiCcConfig::SYNTH_BASIC`] - Pitch bend, mod wheel, volume, expression, sustain
//! - [`MidiCcConfig::SYNTH_FULL`] - Basic + aftertouch, pan, breath controller
//! - [`MidiCcConfig::EFFECT_BASIC`] - Mod wheel, expression (for modulated effects)

// =============================================================================
// Constants
// =============================================================================

/// Maximum supported controller number (pitch bend = 129)
pub const MAX_CC_CONTROLLER: usize = 130;

/// Extended MIDI controller numbers (from VST3 SDK ivstmidicontrollers.h)
pub mod controller {
    /// Channel pressure / aftertouch (VST3 kAfterTouch)
    pub const AFTERTOUCH: u8 = 128;
    /// Pitch bend wheel (VST3 kPitchBend)
    pub const PITCH_BEND: u8 = 129;
}

// =============================================================================
// MidiCcConfig
// =============================================================================

/// Static configuration for MIDI CC emulation.
///
/// This is a pure configuration type with no runtime state. Plugin authors
/// return this from `Plugin::midi_cc_config()` to specify which MIDI controllers
/// they want to receive. The framework creates and manages runtime state
/// internally.
///
/// All builder methods are `const fn` for compile-time configuration.
///
/// # Example
///
/// ```ignore
/// fn midi_cc_config(&self) -> Option<MidiCcConfig> {
///     Some(MidiCcConfig::new()
///         .with_pitch_bend()
///         .with_mod_wheel()
///         .with_ccs(&[7, 10, 11, 64]))
/// }
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MidiCcConfig {
    /// Enabled controller flags
    enabled: [bool; MAX_CC_CONTROLLER],
}

impl MidiCcConfig {
    // =========================================================================
    // Presets (const, ready to use)
    // =========================================================================

    /// Basic synth configuration: pitch bend, mod wheel, volume, expression, sustain.
    ///
    /// Includes:
    /// - Pitch bend (±1.0)
    /// - CC 1: Mod wheel
    /// - CC 7: Volume
    /// - CC 11: Expression
    /// - CC 64: Sustain pedal
    pub const SYNTH_BASIC: Self = Self::new()
        .with_pitch_bend()
        .with_mod_wheel()
        .with_cc(7)   // Volume
        .with_cc(11)  // Expression
        .with_cc(64); // Sustain

    /// Full synth configuration: basic + aftertouch, pan, breath controller.
    ///
    /// Includes everything in [`SYNTH_BASIC`](Self::SYNTH_BASIC) plus:
    /// - Channel aftertouch
    /// - CC 2: Breath controller
    /// - CC 10: Pan
    pub const SYNTH_FULL: Self = Self::new()
        .with_pitch_bend()
        .with_aftertouch()
        .with_mod_wheel()
        .with_cc(2)   // Breath
        .with_cc(7)   // Volume
        .with_cc(10)  // Pan
        .with_cc(11)  // Expression
        .with_cc(64); // Sustain

    /// Basic effect configuration: mod wheel and expression for modulated effects.
    ///
    /// Includes:
    /// - CC 1: Mod wheel
    /// - CC 11: Expression
    pub const EFFECT_BASIC: Self = Self::new()
        .with_mod_wheel()
        .with_cc(11); // Expression

    // =========================================================================
    // Constructor
    // =========================================================================

    /// Create a new `MidiCcConfig` with no controllers enabled.
    ///
    /// Use builder methods to enable specific controllers, or use a preset
    /// like [`SYNTH_BASIC`](Self::SYNTH_BASIC).
    #[inline]
    pub const fn new() -> Self {
        Self {
            enabled: [false; MAX_CC_CONTROLLER],
        }
    }

    // =========================================================================
    // Builder Methods (all const fn)
    // =========================================================================

    /// Enable pitch bend emulation (controller 129).
    ///
    /// Pitch bend uses normalized 0.0-1.0 where 0.5 is center.
    /// The framework converts this to -1.0 to 1.0 range.
    #[inline]
    pub const fn with_pitch_bend(mut self) -> Self {
        self.enabled[controller::PITCH_BEND as usize] = true;
        self
    }

    /// Enable channel aftertouch emulation (controller 128).
    #[inline]
    pub const fn with_aftertouch(mut self) -> Self {
        self.enabled[controller::AFTERTOUCH as usize] = true;
        self
    }

    /// Enable mod wheel emulation (CC 1).
    #[inline]
    pub const fn with_mod_wheel(mut self) -> Self {
        self.enabled[1] = true;
        self
    }

    /// Enable a specific MIDI CC (0-127).
    ///
    /// # Panics
    ///
    /// Panics if `cc >= 128`. In const context, this is a compile-time error.
    /// This catches typos like `.with_cc(130)` (meant 30).
    #[inline]
    pub const fn with_cc(mut self, cc: u8) -> Self {
        assert!(cc < 128, "CC number must be 0-127");
        self.enabled[cc as usize] = true;
        self
    }

    /// Enable multiple MIDI CCs.
    ///
    /// # Panics
    ///
    /// Panics if any CC is >= 128.
    ///
    /// **Note:** Unlike other builder methods, this is not `const fn` because
    /// slices cannot be used in const context. For compile-time configuration,
    /// use multiple `with_cc()` calls or a preset like [`SYNTH_BASIC`](Self::SYNTH_BASIC).
    #[inline]
    pub fn with_ccs(mut self, ccs: &[u8]) -> Self {
        for &cc in ccs {
            assert!(cc < 128, "CC number must be 0-127, got {}", cc);
            self.enabled[cc as usize] = true;
        }
        self
    }

    /// Enable all standard MIDI CCs (0-127).
    ///
    /// **Warning**: This creates 128 hidden parameters. Use sparingly.
    #[inline]
    pub const fn with_all_ccs(mut self) -> Self {
        let mut cc = 0;
        while cc < 128 {
            self.enabled[cc] = true;
            cc += 1;
        }
        self
    }

    // =========================================================================
    // Query Methods
    // =========================================================================

    /// Check if a controller is enabled.
    #[inline]
    pub const fn is_enabled(&self, controller: u8) -> bool {
        let idx = controller as usize;
        if idx < MAX_CC_CONTROLLER {
            self.enabled[idx]
        } else {
            false
        }
    }

    /// Check if pitch bend is enabled.
    #[inline]
    pub const fn has_pitch_bend(&self) -> bool {
        self.enabled[controller::PITCH_BEND as usize]
    }

    /// Check if aftertouch is enabled.
    #[inline]
    pub const fn has_aftertouch(&self) -> bool {
        self.enabled[controller::AFTERTOUCH as usize]
    }

    /// Check if mod wheel (CC 1) is enabled.
    #[inline]
    pub const fn has_mod_wheel(&self) -> bool {
        self.enabled[1]
    }

    /// Get the number of enabled controllers.
    #[inline]
    pub const fn enabled_count(&self) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < MAX_CC_CONTROLLER {
            if self.enabled[i] {
                count += 1;
            }
            i += 1;
        }
        count
    }

    /// Get the enabled flags array (for framework use).
    #[inline]
    pub const fn enabled_flags(&self) -> &[bool; MAX_CC_CONTROLLER] {
        &self.enabled
    }
}

impl Default for MidiCcConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for MidiCcConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let enabled: Vec<u8> = self
            .enabled
            .iter()
            .enumerate()
            .filter_map(|(i, &e)| if e { Some(i as u8) } else { None })
            .collect();
        f.debug_struct("MidiCcConfig")
            .field("enabled_controllers", &enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_const_builder() {
        // Const builder using only const fn methods
        const CONFIG: MidiCcConfig = MidiCcConfig::new()
            .with_pitch_bend()
            .with_mod_wheel()
            .with_cc(7)
            .with_cc(64);

        assert!(CONFIG.has_pitch_bend());
        assert!(CONFIG.has_mod_wheel());
        assert!(CONFIG.is_enabled(7));
        assert!(CONFIG.is_enabled(64));
        assert!(!CONFIG.has_aftertouch());
        assert!(!CONFIG.is_enabled(10));
        assert_eq!(CONFIG.enabled_count(), 4);
    }

    #[test]
    fn test_with_ccs_runtime() {
        // with_ccs is not const fn, works at runtime
        let config = MidiCcConfig::new()
            .with_pitch_bend()
            .with_ccs(&[7, 10, 11, 64]);

        assert!(config.has_pitch_bend());
        assert!(config.is_enabled(7));
        assert!(config.is_enabled(10));
        assert!(config.is_enabled(11));
        assert!(config.is_enabled(64));
        assert_eq!(config.enabled_count(), 5);
    }

    #[test]
    fn test_default() {
        let config = MidiCcConfig::default();
        assert_eq!(config.enabled_count(), 0);
        assert!(!config.has_pitch_bend());
    }

    #[test]
    fn test_with_all_ccs() {
        let config = MidiCcConfig::new().with_all_ccs();
        assert_eq!(config.enabled_count(), 128);
        assert!(config.is_enabled(0));
        assert!(config.is_enabled(127));
        assert!(!config.has_pitch_bend());
        assert!(!config.has_aftertouch());
    }

    #[test]
    fn test_synth_basic_preset() {
        // SYNTH_BASIC is const, can be used at compile time
        const CONFIG: MidiCcConfig = MidiCcConfig::SYNTH_BASIC;

        assert!(CONFIG.has_pitch_bend());
        assert!(CONFIG.has_mod_wheel());
        assert!(CONFIG.is_enabled(7));  // Volume
        assert!(CONFIG.is_enabled(11)); // Expression
        assert!(CONFIG.is_enabled(64)); // Sustain
        assert!(!CONFIG.has_aftertouch());
        assert_eq!(CONFIG.enabled_count(), 5);
    }

    #[test]
    fn test_synth_full_preset() {
        const CONFIG: MidiCcConfig = MidiCcConfig::SYNTH_FULL;

        assert!(CONFIG.has_pitch_bend());
        assert!(CONFIG.has_aftertouch());
        assert!(CONFIG.has_mod_wheel());
        assert!(CONFIG.is_enabled(2));  // Breath
        assert!(CONFIG.is_enabled(7));  // Volume
        assert!(CONFIG.is_enabled(10)); // Pan
        assert!(CONFIG.is_enabled(11)); // Expression
        assert!(CONFIG.is_enabled(64)); // Sustain
        assert_eq!(CONFIG.enabled_count(), 8);
    }

    #[test]
    fn test_effect_basic_preset() {
        const CONFIG: MidiCcConfig = MidiCcConfig::EFFECT_BASIC;

        assert!(CONFIG.has_mod_wheel());
        assert!(CONFIG.is_enabled(11)); // Expression
        assert!(!CONFIG.has_pitch_bend());
        assert_eq!(CONFIG.enabled_count(), 2);
    }

    #[test]
    #[should_panic(expected = "CC number must be 0-127")]
    fn test_invalid_cc_panics() {
        let _ = MidiCcConfig::new().with_cc(130);
    }

    #[test]
    #[should_panic(expected = "CC number must be 0-127")]
    fn test_invalid_ccs_panics() {
        let _ = MidiCcConfig::new().with_ccs(&[7, 130, 64]);
    }
}
