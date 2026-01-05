//! MIDI CC parameter emulation for VST3 compatibility.
//!
//! VST3 doesn't send MIDI CC, pitch bend, or aftertouch directly to plugins.
//! Instead, most DAWs convert these to parameter changes via the `IMidiMapping`
//! interface. This module provides `MidiCcParams` which creates hidden parameters
//! for MIDI controllers and converts parameter changes back to MIDI events.
//!
//! # Usage
//!
//! ```ignore
//! use beamer_core::{Plugin, MidiCcParams};
//!
//! struct MyPlugin {
//!     params: MyParams,
//!     midi_cc_params: MidiCcParams,
//! }
//!
//! impl Plugin for MyPlugin {
//!     fn midi_cc_params(&self) -> Option<&MidiCcParams> {
//!         Some(&self.midi_cc_params)
//!     }
//!
//!     fn create() -> Self {
//!         Self {
//!             params: MyParams::default(),
//!             midi_cc_params: MidiCcParams::new()
//!                 .with_pitch_bend()
//!                 .with_mod_wheel()
//!                 .with_ccs(&[7, 10, 11, 64]),  // Volume, Pan, Expression, Sustain
//!         }
//!     }
//! }
//! ```
//!
//! # How It Works
//!
//! 1. Plugin configures `MidiCcParams` with desired controllers
//! 2. VST3 wrapper exposes hidden parameters for each enabled controller
//! 3. DAW queries `IMidiMapping` and gets parameter IDs for MIDI controllers
//! 4. When DAW sends parameter changes, wrapper converts them to `MidiEvent`
//! 5. Plugin receives MIDI events normally via `process_midi()`

use std::sync::atomic::{AtomicU64, Ordering};

use crate::params::{ParamFlags, ParamInfo, Parameters, Units, ROOT_UNIT_ID};
use crate::types::{ParamId, ParamValue};

// =============================================================================
// Constants
// =============================================================================

/// Base parameter ID for MIDI CC emulation parameters.
///
/// Uses a high value to avoid collision with user-defined parameters.
/// The controller number (0-129) is added to get the final param ID.
///
/// - CC 0-127: Standard MIDI CCs
/// - CC 128: Channel aftertouch (kAfterTouch)
/// - CC 129: Pitch bend (kPitchBend)
pub const MIDI_CC_PARAM_BASE: u32 = 0x10000000; // 268435456

/// Maximum supported controller number (pitch bend = 129)
const MAX_CONTROLLER: usize = 130;

/// Extended MIDI controller numbers (from VST3 SDK ivstmidicontrollers.h)
pub mod controller {
    /// Channel pressure / aftertouch
    pub const AFTERTOUCH: u8 = 128;
    /// Pitch bend wheel
    pub const PITCH_BEND: u8 = 129;
}

// =============================================================================
// MidiCcParams
// =============================================================================

/// Hidden parameters for MIDI CC emulation.
///
/// Create with the builder pattern to specify which controllers to emulate.
/// The VST3 wrapper will expose hidden parameters and convert parameter
/// changes back to MIDI events.
///
/// # Example
///
/// ```ignore
/// let cc_params = MidiCcParams::new()
///     .with_pitch_bend()
///     .with_aftertouch()
///     .with_mod_wheel()
///     .with_ccs(&[7, 10, 11, 64]);  // Volume, Pan, Expression, Sustain
/// ```
pub struct MidiCcParams {
    /// Enabled controller flags (bitset for 0-127, plus special flags)
    enabled: [bool; MAX_CONTROLLER],
    /// Current values (normalized 0.0-1.0, stored as f64 bits)
    values: [AtomicU64; MAX_CONTROLLER],
    /// Pre-computed parameter info for enabled controllers
    param_infos: Vec<CcParamInfo>,
    /// Total enabled controller count
    enabled_count: usize,
}

/// Internal storage for parameter info
struct CcParamInfo {
    controller: u8,
    info: ParamInfo,
}

impl MidiCcParams {
    /// Create a new `MidiCcParams` with no controllers enabled.
    ///
    /// Use builder methods to enable specific controllers.
    pub fn new() -> Self {
        // Initialize all atomic values to 0.0 (or 0.5 for pitch bend center)
        let values = std::array::from_fn(|_| AtomicU64::new(0));

        Self {
            enabled: [false; MAX_CONTROLLER],
            values,
            param_infos: Vec::new(),
            enabled_count: 0,
        }
    }

    // =========================================================================
    // Builder Methods
    // =========================================================================

    /// Enable pitch bend emulation (controller 129).
    ///
    /// Pitch bend uses normalized 0.0-1.0 where 0.5 is center.
    /// The framework converts this to -1.0 to 1.0 for `MidiEvent::pitch_bend`.
    pub fn with_pitch_bend(mut self) -> Self {
        self.enable_controller(controller::PITCH_BEND);
        // Set default to 0.5 (center position)
        self.values[controller::PITCH_BEND as usize]
            .store(0.5f64.to_bits(), Ordering::Relaxed);
        self
    }

    /// Enable channel aftertouch emulation (controller 128).
    pub fn with_aftertouch(mut self) -> Self {
        self.enable_controller(controller::AFTERTOUCH);
        self
    }

    /// Enable mod wheel emulation (CC 1).
    pub fn with_mod_wheel(mut self) -> Self {
        self.enable_controller(1);
        self
    }

    /// Enable a specific MIDI CC (0-127).
    pub fn with_cc(mut self, cc: u8) -> Self {
        if cc < 128 {
            self.enable_controller(cc);
        }
        self
    }

    /// Enable multiple MIDI CCs.
    pub fn with_ccs(mut self, ccs: &[u8]) -> Self {
        for &cc in ccs {
            if cc < 128 {
                self.enable_controller(cc);
            }
        }
        self
    }

    /// Enable all standard MIDI CCs (0-127).
    ///
    /// **Warning**: This creates 128 hidden parameters. Use sparingly.
    pub fn with_all_ccs(mut self) -> Self {
        for cc in 0..128 {
            self.enable_controller(cc);
        }
        self
    }

    // =========================================================================
    // Query Methods
    // =========================================================================

    /// Check if a controller is enabled.
    pub fn has_controller(&self, controller: u8) -> bool {
        if (controller as usize) < MAX_CONTROLLER {
            self.enabled[controller as usize]
        } else {
            false
        }
    }

    /// Check if pitch bend is enabled.
    pub fn has_pitch_bend(&self) -> bool {
        self.has_controller(controller::PITCH_BEND)
    }

    /// Check if aftertouch is enabled.
    pub fn has_aftertouch(&self) -> bool {
        self.has_controller(controller::AFTERTOUCH)
    }

    /// Get the current pitch bend value (-1.0 to 1.0).
    ///
    /// Returns 0.0 if pitch bend is not enabled.
    pub fn pitch_bend(&self) -> f32 {
        if self.has_pitch_bend() {
            let normalized = self.get_normalized_internal(controller::PITCH_BEND);
            (normalized * 2.0 - 1.0) as f32
        } else {
            0.0
        }
    }

    /// Get the current aftertouch value (0.0 to 1.0).
    ///
    /// Returns 0.0 if aftertouch is not enabled.
    pub fn aftertouch(&self) -> f32 {
        if self.has_aftertouch() {
            self.get_normalized_internal(controller::AFTERTOUCH) as f32
        } else {
            0.0
        }
    }

    /// Get the current value of a MIDI CC (0.0 to 1.0).
    ///
    /// Returns 0.0 if the CC is not enabled.
    pub fn cc(&self, cc: u8) -> f32 {
        if cc < 128 && self.has_controller(cc) {
            self.get_normalized_internal(cc) as f32
        } else {
            0.0
        }
    }

    /// Get the mod wheel value (CC 1, 0.0 to 1.0).
    pub fn mod_wheel(&self) -> f32 {
        self.cc(1)
    }

    /// Get the number of enabled controllers.
    pub fn enabled_count(&self) -> usize {
        self.enabled_count
    }

    /// Iterate over enabled controller numbers.
    pub fn enabled_controllers(&self) -> impl Iterator<Item = u8> + '_ {
        self.param_infos.iter().map(|info| info.controller)
    }

    // =========================================================================
    // Parameter ID Helpers
    // =========================================================================

    /// Get parameter ID for a controller.
    #[inline]
    pub const fn param_id(controller: u8) -> u32 {
        MIDI_CC_PARAM_BASE + controller as u32
    }

    /// Check if a parameter ID belongs to MIDI CC emulation.
    #[inline]
    pub const fn is_midi_cc_param(param_id: u32) -> bool {
        param_id >= MIDI_CC_PARAM_BASE && param_id < MIDI_CC_PARAM_BASE + MAX_CONTROLLER as u32
    }

    /// Extract controller number from a MIDI CC parameter ID.
    ///
    /// Returns `None` if the param_id is not a MIDI CC parameter.
    #[inline]
    pub const fn param_id_to_controller(param_id: u32) -> Option<u8> {
        if Self::is_midi_cc_param(param_id) {
            Some((param_id - MIDI_CC_PARAM_BASE) as u8)
        } else {
            None
        }
    }

    // =========================================================================
    // Internal Methods
    // =========================================================================

    fn enable_controller(&mut self, controller: u8) {
        let idx = controller as usize;
        if idx < MAX_CONTROLLER && !self.enabled[idx] {
            self.enabled[idx] = true;
            self.enabled_count += 1;

            // Create parameter info
            let info = self.create_param_info(controller);
            self.param_infos.push(CcParamInfo { controller, info });
        }
    }

    fn create_param_info(&self, controller: u8) -> ParamInfo {
        let id = Self::param_id(controller);

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

        let default = if controller == controller::PITCH_BEND { 0.5 } else { 0.0 };

        ParamInfo {
            id,
            name,
            short_name,
            units: "",
            default_normalized: default,
            step_count: 0,
            flags: ParamFlags {
                can_automate: true,
                is_readonly: false,
                is_bypass: false,
                is_list: false,
                is_hidden: true,  // Hidden from DAW parameter list
            },
            unit_id: ROOT_UNIT_ID,
        }
    }

    fn get_normalized_internal(&self, controller: u8) -> f64 {
        let idx = controller as usize;
        if idx < MAX_CONTROLLER {
            f64::from_bits(self.values[idx].load(Ordering::Relaxed))
        } else {
            0.0
        }
    }

    fn set_normalized_internal(&self, controller: u8, value: f64) {
        let idx = controller as usize;
        if idx < MAX_CONTROLLER {
            self.values[idx].store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }
}

impl Default for MidiCcParams {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: AtomicU64 is Send + Sync, and all other fields are either primitive or Vec
unsafe impl Send for MidiCcParams {}
unsafe impl Sync for MidiCcParams {}

// =============================================================================
// Parameters Trait Implementation (for VST3 integration)
// =============================================================================

impl Parameters for MidiCcParams {
    fn count(&self) -> usize {
        self.enabled_count
    }

    fn info(&self, index: usize) -> Option<&ParamInfo> {
        self.param_infos.get(index).map(|i| &i.info)
    }

    fn get_normalized(&self, id: ParamId) -> ParamValue {
        if let Some(controller) = Self::param_id_to_controller(id) {
            self.get_normalized_internal(controller)
        } else {
            0.0
        }
    }

    fn set_normalized(&self, id: ParamId, value: ParamValue) {
        if let Some(controller) = Self::param_id_to_controller(id) {
            self.set_normalized_internal(controller, value);
        }
    }

    fn normalized_to_string(&self, id: ParamId, normalized: ParamValue) -> String {
        if let Some(controller) = Self::param_id_to_controller(id) {
            if controller == controller::PITCH_BEND {
                let bend = (normalized * 2.0 - 1.0) * 100.0;
                return format!("{:+.0}%", bend);
            }
        }
        format!("{:.0}", normalized * 127.0)
    }

    fn string_to_normalized(&self, _id: ParamId, string: &str) -> Option<ParamValue> {
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

    fn normalized_to_plain(&self, _id: ParamId, normalized: ParamValue) -> ParamValue {
        normalized * 127.0
    }

    fn plain_to_normalized(&self, _id: ParamId, plain: ParamValue) -> ParamValue {
        (plain / 127.0).clamp(0.0, 1.0)
    }
}

// =============================================================================
// Units Trait Implementation (no parameter grouping for hidden params)
// =============================================================================

impl Units for MidiCcParams {
    fn unit_count(&self) -> usize {
        1 // Only root unit
    }

    fn unit_info(&self, index: usize) -> Option<crate::params::UnitInfo> {
        if index == 0 {
            Some(crate::params::UnitInfo::root())
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder() {
        let params = MidiCcParams::new()
            .with_pitch_bend()
            .with_mod_wheel()
            .with_ccs(&[7, 64]);

        assert!(params.has_pitch_bend());
        assert!(!params.has_aftertouch());
        assert!(params.has_controller(1)); // mod wheel
        assert!(params.has_controller(7)); // volume
        assert!(params.has_controller(64)); // sustain
        assert!(!params.has_controller(10)); // pan not enabled
        assert_eq!(params.enabled_count(), 4);
    }

    #[test]
    fn test_param_id() {
        assert_eq!(MidiCcParams::param_id(1), MIDI_CC_PARAM_BASE + 1);
        assert_eq!(MidiCcParams::param_id(129), MIDI_CC_PARAM_BASE + 129);

        assert!(MidiCcParams::is_midi_cc_param(MIDI_CC_PARAM_BASE));
        assert!(MidiCcParams::is_midi_cc_param(MIDI_CC_PARAM_BASE + 129));
        assert!(!MidiCcParams::is_midi_cc_param(0));
        assert!(!MidiCcParams::is_midi_cc_param(MIDI_CC_PARAM_BASE + 200));

        assert_eq!(MidiCcParams::param_id_to_controller(MIDI_CC_PARAM_BASE + 1), Some(1));
        assert_eq!(MidiCcParams::param_id_to_controller(100), None);
    }

    #[test]
    fn test_values() {
        let params = MidiCcParams::new()
            .with_pitch_bend()
            .with_mod_wheel();

        // Pitch bend default: 0.5 normalized (center), which maps to 0.0 in bipolar range
        assert!((params.pitch_bend() - 0.0).abs() < 0.01);

        // Test setting values via Parameters trait
        let pb_id = MidiCcParams::param_id(controller::PITCH_BEND);
        Parameters::set_normalized(&params, pb_id, 1.0);
        assert!((params.pitch_bend() - 1.0).abs() < 0.01);

        Parameters::set_normalized(&params, pb_id, 0.0);
        assert!((params.pitch_bend() - (-1.0)).abs() < 0.01);
    }
}
