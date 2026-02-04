//! Runtime state for MIDI CC emulation.
//!
//! This module provides [`MidiCcState`], which holds the current values of
//! MIDI controllers. Unlike `MidiCcConfig` (pure configuration), `MidiCcState`
//! contains runtime state with atomic values for thread-safe access.
//!
//! **This type is framework-internal.** Plugin authors don't need to create
//! or manage `MidiCcState` - the VST3 wrapper handles it automatically.
//! Plugins can read current CC values via [`ProcessContext::midi_cc()`].

use std::sync::atomic::{AtomicU64, Ordering};

use crate::midi_cc_config::{controller, MidiCcConfig, MAX_CC_CONTROLLER};
use crate::parameter_groups::{GroupInfo, ParameterGroups, ROOT_GROUP_ID};
use crate::parameter_info::{ParameterFlags, ParameterInfo, ParameterUnit};
use crate::parameter_store::ParameterStore;
use crate::types::{ParameterId, ParameterValue};

// =============================================================================
// Constants
// =============================================================================

/// Base parameter ID for MIDI CC emulation parameters.
///
/// Uses a high value to avoid collision with user-defined parameters.
/// The controller number (0-129) is added to get the final parameter ID.
///
/// - CC 0-127: Standard MIDI CCs
/// - CC 128: Channel aftertouch (kAfterTouch)
/// - CC 129: Pitch bend (kPitchBend)
pub const MIDI_CC_PARAM_BASE: u32 = 0x10000000; // 268435456

// =============================================================================
// MidiCcState
// =============================================================================

/// Runtime state for MIDI CC emulation.
///
/// Created by the framework from a [`MidiCcConfig`]. Holds current controller
/// values as atomic floats for thread-safe access from both host and audio threads.
///
/// Plugin authors can read values via [`ProcessContext::midi_cc()`]:
///
/// ```ignore
/// fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
///     if let Some(cc) = context.midi_cc() {
///         let pitch_bend = cc.pitch_bend();  // -1.0 to 1.0
///         let mod_wheel = cc.mod_wheel();    // 0.0 to 1.0
///         let volume = cc.cc(7);             // 0.0 to 1.0
///     }
/// }
/// ```
pub struct MidiCcState {
    /// Enabled controller flags (copied from config)
    enabled: [bool; MAX_CC_CONTROLLER],
    /// Current values (normalized 0.0-1.0, stored as f64 bits)
    values: [AtomicU64; MAX_CC_CONTROLLER],
    /// Pre-computed parameter info for enabled controllers
    parameter_infos: Vec<CcParameterInfo>,
    /// Total enabled controller count
    enabled_count: usize,
}

/// Internal storage for parameter info
struct CcParameterInfo {
    controller: u8,
    info: ParameterInfo,
}

impl MidiCcState {
    /// Create state from configuration.
    ///
    /// This is called by the framework when initializing the VST3 wrapper.
    pub fn from_config(config: &MidiCcConfig) -> Self {
        // Initialize all atomic values to 0.0 (or 0.5 for pitch bend center)
        let values = std::array::from_fn(|i| {
            let default: f64 = if i == controller::PITCH_BEND as usize {
                0.5 // Pitch bend center
            } else {
                0.0
            };
            AtomicU64::new(default.to_bits())
        });

        // Copy enabled flags and build parameter infos
        let enabled = *config.enabled_flags();
        let mut parameter_infos = Vec::new();
        let mut enabled_count = 0;

        for (i, &is_enabled) in enabled.iter().enumerate() {
            if is_enabled {
                enabled_count += 1;
                let controller = i as u8;
                let info = Self::create_parameter_info(controller);
                parameter_infos.push(CcParameterInfo { controller, info });
            }
        }

        Self {
            enabled,
            values,
            parameter_infos,
            enabled_count,
        }
    }

    // =========================================================================
    // Value Access (for plugins via ProcessContext)
    // =========================================================================

    /// Get the current pitch bend value (-1.0 to 1.0).
    ///
    /// Returns 0.0 if pitch bend is not enabled.
    #[inline]
    pub fn pitch_bend(&self) -> f32 {
        if self.enabled[controller::PITCH_BEND as usize] {
            let normalized = self.get_normalized_internal(controller::PITCH_BEND);
            (normalized * 2.0 - 1.0) as f32
        } else {
            0.0
        }
    }

    /// Get the current aftertouch value (0.0 to 1.0).
    ///
    /// Returns 0.0 if aftertouch is not enabled.
    #[inline]
    pub fn aftertouch(&self) -> f32 {
        if self.enabled[controller::AFTERTOUCH as usize] {
            self.get_normalized_internal(controller::AFTERTOUCH) as f32
        } else {
            0.0
        }
    }

    /// Get the current mod wheel value (CC 1, 0.0 to 1.0).
    ///
    /// Returns 0.0 if mod wheel is not enabled.
    #[inline]
    pub fn mod_wheel(&self) -> f32 {
        self.cc(1)
    }

    /// Get the current value of a MIDI CC (0.0 to 1.0).
    ///
    /// Returns 0.0 if the CC is not enabled.
    #[inline]
    pub fn cc(&self, cc: u8) -> f32 {
        if (cc as usize) < MAX_CC_CONTROLLER && self.enabled[cc as usize] {
            self.get_normalized_internal(cc) as f32
        } else {
            0.0
        }
    }

    // =========================================================================
    // Query Methods
    // =========================================================================

    /// Check if a controller is enabled.
    #[inline]
    pub fn has_controller(&self, controller: u8) -> bool {
        if (controller as usize) < MAX_CC_CONTROLLER {
            self.enabled[controller as usize]
        } else {
            false
        }
    }

    /// Check if pitch bend is enabled.
    #[inline]
    pub fn has_pitch_bend(&self) -> bool {
        self.enabled[controller::PITCH_BEND as usize]
    }

    /// Check if aftertouch is enabled.
    #[inline]
    pub fn has_aftertouch(&self) -> bool {
        self.enabled[controller::AFTERTOUCH as usize]
    }

    /// Get the number of enabled controllers.
    #[inline]
    pub fn enabled_count(&self) -> usize {
        self.enabled_count
    }

    /// Iterate over enabled controller numbers.
    pub fn enabled_controllers(&self) -> impl Iterator<Item = u8> + '_ {
        self.parameter_infos.iter().map(|info| info.controller)
    }

    // =========================================================================
    // Parameter ID Helpers
    // =========================================================================

    /// Get parameter ID for a controller.
    #[inline]
    pub const fn parameter_id(controller: u8) -> u32 {
        MIDI_CC_PARAM_BASE + controller as u32
    }

    /// Check if a parameter ID belongs to MIDI CC emulation.
    #[inline]
    pub const fn is_midi_cc_parameter(parameter_id: u32) -> bool {
        parameter_id >= MIDI_CC_PARAM_BASE && parameter_id < MIDI_CC_PARAM_BASE + MAX_CC_CONTROLLER as u32
    }

    /// Extract controller number from a MIDI CC parameter ID.
    ///
    /// Returns `None` if the parameter_id is not a MIDI CC parameter.
    #[inline]
    pub const fn parameter_id_to_controller(parameter_id: u32) -> Option<u8> {
        if Self::is_midi_cc_parameter(parameter_id) {
            Some((parameter_id - MIDI_CC_PARAM_BASE) as u8)
        } else {
            None
        }
    }

    // =========================================================================
    // Internal Methods
    // =========================================================================

    fn create_parameter_info(controller: u8) -> ParameterInfo {
        let id = Self::parameter_id(controller);

        // Determine name based on controller
        let (name, short_name): (&'static str, &'static str) = match controller {
            controller::PITCH_BEND => ("Pitch Bend", "PB"),
            controller::AFTERTOUCH => ("Aftertouch", "AT"),
            1 => ("Mod Wheel", "MW"),
            2 => ("Breath Controller", "BC"),
            7 => ("Volume", "Vol"),
            10 => ("Pan", "Pan"),
            11 => ("Expression", "Exp"),
            64 => ("Sustain Pedal", "Sus"),
            _ => ("MIDI CC", "CC"),
        };

        let default = if controller == controller::PITCH_BEND {
            0.5
        } else {
            0.0
        };

        ParameterInfo {
            id,
            name,
            short_name,
            units: "",
            unit: ParameterUnit::Generic,
            default_normalized: default,
            step_count: 0,
            flags: ParameterFlags {
                can_automate: true,
                is_readonly: false,
                is_bypass: false,
                is_list: false,
                is_hidden: true, // Hidden from DAW parameter list
            },
            group_id: ROOT_GROUP_ID,
        }
    }

    #[inline]
    fn get_normalized_internal(&self, controller: u8) -> f64 {
        let idx = controller as usize;
        if idx < MAX_CC_CONTROLLER {
            f64::from_bits(self.values[idx].load(Ordering::Relaxed))
        } else {
            0.0
        }
    }

    fn set_normalized_internal(&self, controller: u8, value: f64) {
        let idx = controller as usize;
        if idx < MAX_CC_CONTROLLER {
            self.values[idx].store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }
}

// SAFETY: AtomicU64 is Send + Sync, and all other fields are either primitive or Vec.
unsafe impl Send for MidiCcState {}
// SAFETY: AtomicU64 is Send + Sync, and all other fields are either primitive or Vec.
unsafe impl Sync for MidiCcState {}

impl core::fmt::Debug for MidiCcState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let enabled: Vec<u8> = self
            .enabled
            .iter()
            .enumerate()
            .filter_map(|(i, &e)| if e { Some(i as u8) } else { None })
            .collect();
        f.debug_struct("MidiCcState")
            .field("enabled_controllers", &enabled)
            .field("enabled_count", &self.enabled_count)
            .finish()
    }
}

// =============================================================================
// Parameters Trait Implementation (for VST3 integration)
// =============================================================================

impl ParameterStore for MidiCcState {
    fn count(&self) -> usize {
        self.enabled_count
    }

    fn info(&self, index: usize) -> Option<&ParameterInfo> {
        self.parameter_infos.get(index).map(|i| &i.info)
    }

    fn get_normalized(&self, id: ParameterId) -> ParameterValue {
        if let Some(controller) = Self::parameter_id_to_controller(id) {
            self.get_normalized_internal(controller)
        } else {
            0.0
        }
    }

    fn set_normalized(&self, id: ParameterId, value: ParameterValue) {
        if let Some(controller) = Self::parameter_id_to_controller(id) {
            self.set_normalized_internal(controller, value);
        }
    }

    fn normalized_to_string(&self, id: ParameterId, normalized: ParameterValue) -> String {
        if let Some(controller) = Self::parameter_id_to_controller(id) {
            if controller == controller::PITCH_BEND {
                // Display pitch bend as bipolar semitones (assuming ±2 semitones default)
                // Center (0.5 normalized) = 0 st, min (0.0) = -2 st, max (1.0) = +2 st
                let semitones = (normalized * 2.0 - 1.0) * 2.0;
                return format!("{:+.1} st", semitones);
            }
        }
        format!("{:.0}", normalized * 127.0)
    }

    fn string_to_normalized(&self, _id: ParameterId, string: &str) -> Option<ParameterValue> {
        // Try parsing as 0-127
        if let Ok(v) = string.parse::<f64>() {
            return Some((v / 127.0).clamp(0.0, 1.0));
        }
        // Try parsing as percentage
        if let Some(v) = string.strip_suffix('%') {
            if let Ok(v) = v.trim().parse::<f64>() {
                return Some((v / 100.0).clamp(0.0, 1.0));
            }
        }
        None
    }

    fn normalized_to_plain(&self, _id: ParameterId, normalized: ParameterValue) -> ParameterValue {
        normalized * 127.0
    }

    fn plain_to_normalized(&self, _id: ParameterId, plain: ParameterValue) -> ParameterValue {
        (plain / 127.0).clamp(0.0, 1.0)
    }
}

// =============================================================================
// ParameterGroups Trait Implementation (no parameter grouping for hidden parameters)
// =============================================================================

impl ParameterGroups for MidiCcState {
    fn group_count(&self) -> usize {
        1 // Only root group
    }

    fn group_info(&self, index: usize) -> Option<GroupInfo> {
        if index == 0 {
            Some(GroupInfo::root())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_config() {
        let config = MidiCcConfig::new()
            .with_pitch_bend()
            .with_mod_wheel()
            .with_ccs(&[7, 64]);

        let state = MidiCcState::from_config(&config);

        assert!(state.has_pitch_bend());
        assert!(state.has_controller(1)); // mod wheel
        assert!(state.has_controller(7)); // volume
        assert!(state.has_controller(64)); // sustain
        assert!(!state.has_aftertouch());
        assert_eq!(state.enabled_count(), 4);
    }

    #[test]
    fn test_pitch_bend_default() {
        let config = MidiCcConfig::new().with_pitch_bend();
        let state = MidiCcState::from_config(&config);

        // Pitch bend default: 0.5 normalized (center), which maps to 0.0 bipolar
        assert!((state.pitch_bend() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_set_and_get() {
        let config = MidiCcConfig::new().with_pitch_bend().with_mod_wheel();
        let state = MidiCcState::from_config(&config);

        // Set pitch bend to max (1.0 normalized = +1.0 bipolar)
        let pb_id = MidiCcState::parameter_id(controller::PITCH_BEND);
        state.set_normalized(pb_id, 1.0);
        assert!((state.pitch_bend() - 1.0).abs() < 0.01);

        // Set pitch bend to min (0.0 normalized = -1.0 bipolar)
        state.set_normalized(pb_id, 0.0);
        assert!((state.pitch_bend() - (-1.0)).abs() < 0.01);

        // Set mod wheel
        let mw_id = MidiCcState::parameter_id(1);
        state.set_normalized(mw_id, 0.75);
        assert!((state.mod_wheel() - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_parameter_id_helpers() {
        assert_eq!(MidiCcState::parameter_id(1), MIDI_CC_PARAM_BASE + 1);
        assert_eq!(MidiCcState::parameter_id(129), MIDI_CC_PARAM_BASE + 129);

        assert!(MidiCcState::is_midi_cc_parameter(MIDI_CC_PARAM_BASE));
        assert!(MidiCcState::is_midi_cc_parameter(MIDI_CC_PARAM_BASE + 129));
        assert!(!MidiCcState::is_midi_cc_parameter(0));
        assert!(!MidiCcState::is_midi_cc_parameter(MIDI_CC_PARAM_BASE + 200));

        assert_eq!(
            MidiCcState::parameter_id_to_controller(MIDI_CC_PARAM_BASE + 1),
            Some(1)
        );
        assert_eq!(MidiCcState::parameter_id_to_controller(100), None);
    }
}
