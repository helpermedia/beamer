//! Parameter system for audio plugins.
//!
//! This module provides traits and types for declaring and managing plugin parameters
//! in a format-agnostic way. Parameters use normalized values (0.0 to 1.0) for
//! host communication, with conversion to/from plain values handled by the plugin.
//!
//! # Thread Safety
//!
//! The [`Parameters`] trait requires `Send + Sync` because parameters may be
//! accessed from multiple threads:
//! - Audio thread: reads parameter values during processing
//! - UI thread: displays and modifies parameter values
//! - Host thread: automation playback and recording
//!
//! Use atomic types (e.g., `AtomicU64` with `to_bits`/`from_bits`) for lock-free access.

use crate::types::{ParamId, ParamValue};

/// Flags controlling parameter behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamFlags {
    /// Parameter can be automated by the host.
    pub can_automate: bool,
    /// Parameter is read-only (display only).
    pub is_readonly: bool,
    /// Parameter is the bypass switch.
    pub is_bypass: bool,
}

impl Default for ParamFlags {
    fn default() -> Self {
        Self {
            can_automate: true,
            is_readonly: false,
            is_bypass: false,
        }
    }
}

/// Metadata describing a single parameter.
#[derive(Debug, Clone)]
pub struct ParamInfo {
    /// Unique parameter identifier.
    pub id: ParamId,
    /// Full parameter name (e.g., "Master Volume").
    pub name: &'static str,
    /// Short parameter name for constrained UIs (e.g., "Vol").
    pub short_name: &'static str,
    /// Unit label (e.g., "dB", "%", "Hz").
    pub units: &'static str,
    /// Default value in normalized form (0.0 to 1.0).
    pub default_normalized: ParamValue,
    /// Number of discrete steps. 0 = continuous, 1 = toggle, >1 = discrete.
    pub step_count: i32,
    /// Behavioral flags.
    pub flags: ParamFlags,
}

impl ParamInfo {
    /// Create a new continuous parameter with default flags.
    pub const fn new(id: ParamId, name: &'static str) -> Self {
        Self {
            id,
            name,
            short_name: name,
            units: "",
            default_normalized: 0.5,
            step_count: 0,
            flags: ParamFlags {
                can_automate: true,
                is_readonly: false,
                is_bypass: false,
            },
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
    pub const fn with_default(mut self, default: ParamValue) -> Self {
        self.default_normalized = default;
        self
    }

    /// Set the step count (0 = continuous).
    pub const fn with_steps(mut self, steps: i32) -> Self {
        self.step_count = steps;
        self
    }

    /// Set parameter flags.
    pub const fn with_flags(mut self, flags: ParamFlags) -> Self {
        self.flags = flags;
        self
    }
}

/// Trait for plugin parameter collections.
///
/// Implement this trait to declare your plugin's parameters. The VST3 wrapper
/// will use this to communicate parameter information and values to the host.
///
/// # Example
///
/// ```ignore
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use beamr_core::{Parameters, ParamInfo, ParamId, ParamValue};
///
/// pub struct MyParams {
///     gain: AtomicU64,
///     gain_info: ParamInfo,
/// }
///
/// impl Parameters for MyParams {
///     fn count(&self) -> usize { 1 }
///
///     fn info(&self, index: usize) -> Option<&ParamInfo> {
///         match index {
///             0 => Some(&self.gain_info),
///             _ => None,
///         }
///     }
///
///     fn get_normalized(&self, id: ParamId) -> ParamValue {
///         match id {
///             0 => f64::from_bits(self.gain.load(Ordering::Relaxed)),
///             _ => 0.0,
///         }
///     }
///
///     fn set_normalized(&self, id: ParamId, value: ParamValue) {
///         match id {
///             0 => self.gain.store(value.to_bits(), Ordering::Relaxed),
///             _ => {}
///         }
///     }
///
///     // ... implement other methods
/// }
/// ```
pub trait Parameters: Send + Sync {
    /// Returns the number of parameters.
    fn count(&self) -> usize;

    /// Returns parameter info by index (0 to count-1).
    ///
    /// Returns `None` if index is out of bounds.
    fn info(&self, index: usize) -> Option<&ParamInfo>;

    /// Gets the current normalized value (0.0 to 1.0) for a parameter.
    ///
    /// This must be lock-free and safe to call from the audio thread.
    fn get_normalized(&self, id: ParamId) -> ParamValue;

    /// Sets the normalized value (0.0 to 1.0) for a parameter.
    ///
    /// This must be lock-free and safe to call from the audio thread.
    /// Implementations should clamp the value to [0.0, 1.0].
    fn set_normalized(&self, id: ParamId, value: ParamValue);

    /// Converts a normalized value to a display string.
    ///
    /// Used by the host to display parameter values in automation lanes,
    /// tooltips, etc.
    fn normalized_to_string(&self, id: ParamId, normalized: ParamValue) -> String;

    /// Parses a display string to a normalized value.
    ///
    /// Used when the user types a value directly. Returns `None` if
    /// the string cannot be parsed.
    fn string_to_normalized(&self, id: ParamId, string: &str) -> Option<ParamValue>;

    /// Converts a normalized value (0.0-1.0) to a plain/real value.
    ///
    /// For example, a frequency parameter might map 0.0-1.0 to 20-20000 Hz.
    fn normalized_to_plain(&self, id: ParamId, normalized: ParamValue) -> ParamValue;

    /// Converts a plain/real value to a normalized value (0.0-1.0).
    ///
    /// Inverse of `normalized_to_plain`.
    fn plain_to_normalized(&self, id: ParamId, plain: ParamValue) -> ParamValue;

    /// Find parameter info by ID.
    ///
    /// Default implementation searches linearly through all parameters.
    fn info_by_id(&self, id: ParamId) -> Option<&ParamInfo> {
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
pub struct NoParams;

impl Parameters for NoParams {
    fn count(&self) -> usize {
        0
    }

    fn info(&self, _index: usize) -> Option<&ParamInfo> {
        None
    }

    fn get_normalized(&self, _id: ParamId) -> ParamValue {
        0.0
    }

    fn set_normalized(&self, _id: ParamId, _value: ParamValue) {}

    fn normalized_to_string(&self, _id: ParamId, _normalized: ParamValue) -> String {
        String::new()
    }

    fn string_to_normalized(&self, _id: ParamId, _string: &str) -> Option<ParamValue> {
        None
    }

    fn normalized_to_plain(&self, _id: ParamId, normalized: ParamValue) -> ParamValue {
        normalized
    }

    fn plain_to_normalized(&self, _id: ParamId, plain: ParamValue) -> ParamValue {
        plain
    }
}
