//! Low-level parameter system for VST3 host communication.
//!
//! This module provides the [`Vst3Parameters`] trait for direct VST3 host communication.
//! It exposes the raw normalized value interface that VST3 expects.
//!
//! # Choosing Between `Parameters` and `Vst3Parameters`
//!
//! Beamer provides two parameter traits that work together:
//!
//! - **[`Parameters`](crate::parameter_types::Parameters)** (recommended): High-level trait with
//!   type-erased iteration, automatic state serialization, and support for parameter
//!   types like `FloatParameter`, `IntParameter`, and `BoolParameter`. Use `#[derive(Parameters)]`
//!   for automatic implementation.
//!
//! - **[`Vst3Parameters`]**: Low-level trait for direct VST3 host communication. Provides
//!   raw access to normalized values and parameter metadata. Useful when you need
//!   fine-grained control over parameter handling or are building custom parameter
//!   systems.
//!
//! For most plugins, use `#[derive(Parameters)]` which automatically implements both traits.
//! The `Parameters` trait builds on top of `Vst3Parameters` to provide a more ergonomic API.
//!
//! # Thread Safety
//!
//! The [`Vst3Parameters`] trait requires `Send + Sync` because parameters may be
//! accessed from multiple threads:
//! - Audio thread: reads parameter values during processing
//! - UI thread: displays and modifies parameter values
//! - Host thread: automation playback and recording
//!
//! Use atomic types (e.g., `AtomicU64` with `to_bits`/`from_bits`) for lock-free access.

use crate::types::{ParameterId, ParameterValue};

// =============================================================================
// VST3 Unit System (Parameter Grouping)
// =============================================================================

/// VST3 Unit ID type.
///
/// Units are used to organize parameters into hierarchical groups in the DAW UI.
/// Each unit has a unique ID and can have a parent unit.
pub type UnitId = i32;

/// Root unit ID constant (parameters with no group).
///
/// The root unit (ID 0) always exists and contains ungrouped parameters.
pub const ROOT_UNIT_ID: UnitId = 0;

/// Information about a parameter group (VST3 Unit).
///
/// Units form a tree structure via parent_id references:
/// - Root unit (id=0, parent=0) always exists implicitly
/// - Top-level groups have parent_id=0
/// - Nested groups reference their parent's unit_id
#[derive(Debug, Clone)]
pub struct UnitInfo {
    /// Unique unit identifier.
    pub id: UnitId,
    /// Display name shown in DAW (e.g., "Filter", "Amp Envelope").
    pub name: &'static str,
    /// Parent unit ID (ROOT_UNIT_ID for top-level groups).
    pub parent_id: UnitId,
}

impl UnitInfo {
    /// Create a new unit info.
    pub const fn new(id: UnitId, name: &'static str, parent_id: UnitId) -> Self {
        Self { id, name, parent_id }
    }

    /// Create the root unit.
    pub const fn root() -> Self {
        Self {
            id: ROOT_UNIT_ID,
            name: "",
            parent_id: ROOT_UNIT_ID,
        }
    }
}

/// Trait for querying VST3 unit hierarchy.
///
/// Implemented automatically by `#[derive(Parameters)]` when nested groups are present.
/// Provides information about parameter groups for DAW display.
///
/// Unit IDs are assigned dynamically at runtime to support deeply nested groups
/// where the same nested struct type can appear in multiple contexts with
/// different parent units.
pub trait Units {
    /// Total number of units (including root).
    ///
    /// Returns 1 if there are no groups (just the root unit).
    /// For nested groups, this returns 1 + total nested groups (including deeply nested).
    fn unit_count(&self) -> usize {
        1 // Default: only root unit
    }

    /// Get unit info by index.
    ///
    /// Index 0 always returns the root unit.
    /// Returns `UnitInfo` by value to support dynamic construction for nested groups.
    fn unit_info(&self, index: usize) -> Option<UnitInfo> {
        if index == 0 {
            Some(UnitInfo::root())
        } else {
            None
        }
    }

    /// Find unit ID by name (linear search).
    fn find_unit_by_name(&self, name: &str) -> Option<UnitId> {
        for i in 0..self.unit_count() {
            if let Some(info) = self.unit_info(i) {
                if info.name == name {
                    return Some(info.id);
                }
            }
        }
        None
    }
}

/// Flags controlling parameter behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParameterFlags {
    /// Parameter can be automated by the host.
    pub can_automate: bool,
    /// Parameter is read-only (display only).
    pub is_readonly: bool,
    /// Parameter is the bypass switch.
    pub is_bypass: bool,
    /// Parameter should be displayed as a dropdown list (for enums).
    /// When true, host shows text labels from getParameterStringByValue().
    pub is_list: bool,
    /// Parameter is hidden from the DAW's parameter list.
    /// Used for internal parameters like MIDI CC emulation.
    pub is_hidden: bool,
}

impl Default for ParameterFlags {
    fn default() -> Self {
        Self {
            can_automate: true,
            is_readonly: false,
            is_bypass: false,
            is_list: false,
            is_hidden: false,
        }
    }
}

/// Metadata describing a single parameter.
#[derive(Debug, Clone)]
pub struct ParameterInfo {
    /// Unique parameter identifier.
    pub id: ParameterId,
    /// Full parameter name (e.g., "Master Volume").
    pub name: &'static str,
    /// Short parameter name for constrained UIs (e.g., "Vol").
    pub short_name: &'static str,
    /// Unit label (e.g., "dB", "%", "Hz").
    pub units: &'static str,
    /// Default value in normalized form (0.0 to 1.0).
    pub default_normalized: ParameterValue,
    /// Number of discrete steps. 0 = continuous, 1 = toggle, >1 = discrete.
    pub step_count: i32,
    /// Behavioral flags.
    pub flags: ParameterFlags,
    /// VST3 Unit ID (parameter group). ROOT_UNIT_ID (0) for ungrouped parameters.
    pub unit_id: UnitId,
}

impl ParameterInfo {
    /// Create a new continuous parameter with default flags.
    pub const fn new(id: ParameterId, name: &'static str) -> Self {
        Self {
            id,
            name,
            short_name: name,
            units: "",
            default_normalized: 0.5,
            step_count: 0,
            flags: ParameterFlags {
                can_automate: true,
                is_readonly: false,
                is_bypass: false,
                is_list: false,
                is_hidden: false,
            },
            unit_id: ROOT_UNIT_ID,
        }
    }

    /// Set the short name.
    pub const fn with_short_name(mut self, short_name: &'static str) -> Self {
        self.short_name = short_name;
        self
    }

    /// Set the unit label.
    pub const fn with_units(mut self, units: &'static str) -> Self {
        self.units = units;
        self
    }

    /// Set the default normalized value.
    pub const fn with_default(mut self, default: ParameterValue) -> Self {
        self.default_normalized = default;
        self
    }

    /// Set the step count (0 = continuous).
    pub const fn with_steps(mut self, steps: i32) -> Self {
        self.step_count = steps;
        self
    }

    /// Set parameter flags.
    pub const fn with_flags(mut self, flags: ParameterFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Create a bypass toggle parameter with standard configuration.
    ///
    /// This creates a parameter pre-configured as a bypass switch:
    /// - Toggle (step_count = 1)
    /// - Automatable
    /// - Marked with `is_bypass = true` flag
    /// - Default value = 0.0 (not bypassed)
    ///
    /// # Example
    ///
    /// ```ignore
    /// const PARAM_BYPASS: u32 = 0;
    ///
    /// struct MyParameters {
    ///     bypass: AtomicU64,
    ///     bypass_info: ParameterInfo,
    /// }
    ///
    /// impl MyParameters {
    ///     fn new() -> Self {
    ///         Self {
    ///             bypass: AtomicU64::new(0.0f64.to_bits()),
    ///             bypass_info: ParameterInfo::bypass(PARAM_BYPASS),
    ///         }
    ///     }
    /// }
    /// ```
    pub const fn bypass(id: ParameterId) -> Self {
        Self {
            id,
            name: "Bypass",
            short_name: "Byp",
            units: "",
            default_normalized: 0.0,
            step_count: 1,
            flags: ParameterFlags {
                can_automate: true,
                is_readonly: false,
                is_bypass: true,
                is_list: false,
                is_hidden: false,
            },
            unit_id: ROOT_UNIT_ID,
        }
    }

    /// Set the unit ID (parameter group).
    pub const fn with_unit(mut self, unit_id: UnitId) -> Self {
        self.unit_id = unit_id;
        self
    }
}

/// Low-level trait for plugin parameter collections (VST3 interface).
///
/// Implement this trait to declare your plugin's parameters. The VST3 wrapper
/// will use this to communicate parameter information and values to the host.
///
/// # Example
///
/// ```ignore
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use beamer_core::{Vst3Parameters, ParameterInfo, ParameterId, ParameterValue};
///
/// pub struct MyParameters {
///     gain: AtomicU64,
///     gain_info: ParameterInfo,
/// }
///
/// impl Vst3Parameters for MyParameters {
///     fn count(&self) -> usize { 1 }
///
///     fn info(&self, index: usize) -> Option<&ParameterInfo> {
///         match index {
///             0 => Some(&self.gain_info),
///             _ => None,
///         }
///     }
///
///     fn get_normalized(&self, id: ParameterId) -> ParameterValue {
///         match id {
///             0 => f64::from_bits(self.gain.load(Ordering::Relaxed)),
///             _ => 0.0,
///         }
///     }
///
///     fn set_normalized(&self, id: ParameterId, value: ParameterValue) {
///         match id {
///             0 => self.gain.store(value.to_bits(), Ordering::Relaxed),
///             _ => {}
///         }
///     }
///
///     // ... implement other methods
/// }
/// ```
pub trait Vst3Parameters: Send + Sync {
    /// Returns the number of parameters.
    fn count(&self) -> usize;

    /// Returns parameter info by index (0 to count-1).
    ///
    /// Returns `None` if index is out of bounds.
    fn info(&self, index: usize) -> Option<&ParameterInfo>;

    /// Gets the current normalized value (0.0 to 1.0) for a parameter.
    ///
    /// This must be lock-free and safe to call from the audio thread.
    fn get_normalized(&self, id: ParameterId) -> ParameterValue;

    /// Sets the normalized value (0.0 to 1.0) for a parameter.
    ///
    /// This must be lock-free and safe to call from the audio thread.
    /// Implementations should clamp the value to [0.0, 1.0].
    fn set_normalized(&self, id: ParameterId, value: ParameterValue);

    /// Converts a normalized value to a display string.
    ///
    /// Used by the host to display parameter values in automation lanes,
    /// tooltips, etc.
    fn normalized_to_string(&self, id: ParameterId, normalized: ParameterValue) -> String;

    /// Parses a display string to a normalized value.
    ///
    /// Used when the user types a value directly. Returns `None` if
    /// the string cannot be parsed.
    fn string_to_normalized(&self, id: ParameterId, string: &str) -> Option<ParameterValue>;

    /// Converts a normalized value (0.0-1.0) to a plain/real value.
    ///
    /// For example, a frequency parameter might map 0.0-1.0 to 20-20000 Hz.
    fn normalized_to_plain(&self, id: ParameterId, normalized: ParameterValue) -> ParameterValue;

    /// Converts a plain/real value to a normalized value (0.0-1.0).
    ///
    /// Inverse of `normalized_to_plain`.
    fn plain_to_normalized(&self, id: ParameterId, plain: ParameterValue) -> ParameterValue;

    /// Find parameter info by ID.
    ///
    /// Default implementation searches linearly through all parameters.
    fn info_by_id(&self, id: ParameterId) -> Option<&ParameterInfo> {
        (0..self.count()).find_map(|i| {
            let info = self.info(i)?;
            if info.id == id {
                Some(info)
            } else {
                None
            }
        })
    }
}

/// Empty parameter collection for plugins with no parameters.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoParameters;

impl Units for NoParameters {}

impl Vst3Parameters for NoParameters {
    fn count(&self) -> usize {
        0
    }

    fn info(&self, _index: usize) -> Option<&ParameterInfo> {
        None
    }

    fn get_normalized(&self, _id: ParameterId) -> ParameterValue {
        0.0
    }

    fn set_normalized(&self, _id: ParameterId, _value: ParameterValue) {}

    fn normalized_to_string(&self, _id: ParameterId, _normalized: ParameterValue) -> String {
        String::new()
    }

    fn string_to_normalized(&self, _id: ParameterId, _string: &str) -> Option<ParameterValue> {
        None
    }

    fn normalized_to_plain(&self, _id: ParameterId, normalized: ParameterValue) -> ParameterValue {
        normalized
    }

    fn plain_to_normalized(&self, _id: ParameterId, plain: ParameterValue) -> ParameterValue {
        plain
    }
}

impl crate::parameter_types::Parameters for NoParameters {
    fn count(&self) -> usize {
        0
    }

    fn iter(&self) -> Box<dyn Iterator<Item = &dyn crate::parameter_types::ParameterRef> + '_> {
        Box::new(std::iter::empty())
    }

    fn by_id(&self, _id: ParameterId) -> Option<&dyn crate::parameter_types::ParameterRef> {
        None
    }
}
