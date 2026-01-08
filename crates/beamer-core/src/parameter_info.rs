//! Parameter metadata types.
//!
//! This module provides types for describing parameter metadata:
//! - [`ParameterInfo`] - Complete parameter description (name, range, flags, etc.)
//! - [`ParameterFlags`] - Behavioral flags (automation, bypass, list, etc.)

use crate::parameter_groups::{GroupId, ROOT_GROUP_ID};
use crate::types::{ParameterId, ParameterValue};

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
    /// Parameter group ID. ROOT_GROUP_ID (0) for ungrouped parameters.
    pub group_id: GroupId,
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
            group_id: ROOT_GROUP_ID,
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
            group_id: ROOT_GROUP_ID,
        }
    }

    /// Set the group ID (parameter group).
    pub const fn with_group(mut self, group_id: GroupId) -> Self {
        self.group_id = group_id;
        self
    }
}
