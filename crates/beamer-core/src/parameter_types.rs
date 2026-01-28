//! High-level parameter types with encapsulated atomic storage.
//!
//! This module provides the recommended way to define plugin parameters. It includes
//! parameter types ([`FloatParameter`], [`IntParameter`], [`BoolParameter`], [`EnumParameter`]) that
//! encapsulate atomic storage, range mapping, and value formatting.
//!
//! # The `Parameters` Trait (Recommended)
//!
//! The [`Parameters`] trait is the preferred way to define parameters. Use `#[derive(Parameters)]`
//! for automatic implementation:
//!
//! ```ignore
//! use beamer::prelude::*;
//!
//! #[derive(Parameters)]
//! pub struct MyParameters {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     pub gain: FloatParameter,
//!
//!     #[parameter(id = "attack", name = "Attack", default = 10.0, range = 0.1..=100.0, kind = "ms")]
//!     pub attack: FloatParameter,
//! }
//! ```
//!
//! The derive macro generates implementations for both `Parameters` and
//! [`ParameterStore`](crate::parameter_store::ParameterStore) traits. See [`crate::parameter_store`]
//! for details on the relationship between these traits.
//!
//! # Parameter Types
//!
//! - [`FloatParameter`] - Continuous float values with range mapping and smoothing
//! - [`IntParameter`] - Discrete integer values
//! - [`BoolParameter`] - Toggle/boolean values
//! - [`EnumParameter`] - Discrete enum choices (use with `#[derive(EnumParameter)]`)

use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use crate::parameter_format::Formatter;
use crate::parameter_groups::{GroupId, GroupInfo, ParameterGroups, ROOT_GROUP_ID};
use crate::parameter_info::{ParameterFlags, ParameterInfo, ParameterUnit};
use crate::parameter_range::{LinearMapper, LogMapper, LogOffsetMapper, PowerMapper, RangeMapper};
use crate::smoothing::{Smoother, SmoothingStyle};
use crate::types::{ParameterId, ParameterValue};

// =============================================================================
// ParameterRef Trait - Type-erased parameter access
// =============================================================================

/// Trait for type-erased parameter access at runtime.
///
/// This allows iteration over heterogeneous parameter collections
/// and runtime lookup without knowing the concrete parameter type.
///
/// All implementations must be thread-safe (`Send + Sync`) for
/// concurrent access from audio, UI, and host threads.
pub trait ParameterRef: Send + Sync {
    /// Get the parameter's unique ID.
    fn id(&self) -> ParameterId;

    /// Get the parameter's display name.
    fn name(&self) -> &'static str;

    /// Get the parameter's short name for constrained UIs.
    fn short_name(&self) -> &'static str;

    /// Get the parameter's unit string (e.g., "dB", "Hz", "ms").
    fn units(&self) -> &'static str;

    /// Get the parameter flags.
    fn flags(&self) -> &ParameterFlags;

    /// Get the default normalized value.
    fn default_normalized(&self) -> ParameterValue;

    /// Get the step count (0 = continuous, 1 = toggle, >1 = discrete).
    fn step_count(&self) -> i32;

    /// Get the current normalized value (0.0-1.0).
    ///
    /// This is lock-free and safe to call from the audio thread.
    fn get_normalized(&self) -> ParameterValue;

    /// Set the normalized value (0.0-1.0).
    ///
    /// This is lock-free and safe to call from any thread.
    /// Values are clamped to [0.0, 1.0].
    fn set_normalized(&self, value: ParameterValue);

    /// Get the current plain value in natural units.
    fn get_plain(&self) -> ParameterValue;

    /// Set the plain value in natural units.
    fn set_plain(&self, value: ParameterValue);

    /// Format the current value for display.
    fn display(&self) -> String {
        self.display_normalized(self.get_normalized())
    }

    /// Format a normalized value for display.
    fn display_normalized(&self, normalized: ParameterValue) -> String;

    /// Parse a display string to a normalized value.
    ///
    /// Returns `None` if parsing fails.
    fn parse(&self, s: &str) -> Option<ParameterValue>;

    /// Convert a normalized value to a plain value.
    fn normalized_to_plain(&self, normalized: ParameterValue) -> ParameterValue;

    /// Convert a plain value to a normalized value.
    fn plain_to_normalized(&self, plain: ParameterValue) -> ParameterValue;

    /// Get the full ParameterInfo for this parameter.
    ///
    /// This is used by the `#[derive(Parameters)]` macro to generate the
    /// `ParameterStore::info()` implementation.
    fn info(&self) -> &ParameterInfo;
}

// =============================================================================
// Parameters Trait - Parameter collection
// =============================================================================

/// Trait for parameter collections.
///
/// Implement this trait to declare your plugin's parameters. This trait
/// provides both type-erased iteration (for VST3 integration) and
/// automatic state serialization.
///
/// # Example
///
/// ```ignore
/// use beamer_core::parameter_types::{FloatParameter, Parameters, ParameterRef};
///
/// struct MyParameters {
///     gain: FloatParameter,
/// }
///
/// impl Parameters for MyParameters {
///     fn count(&self) -> usize { 1 }
///
///     fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
///         Box::new(std::iter::once(&self.gain as &dyn ParameterRef))
///     }
///
///     fn by_id(&self, id: u32) -> Option<&dyn ParameterRef> {
///         match id {
///             0 => Some(&self.gain),
///             _ => None,
///         }
///     }
/// }
/// ```
pub trait Parameters: Send + Sync + ParameterGroups {
    /// Returns the total number of parameters.
    fn count(&self) -> usize;

    /// Iterate over all parameters (type-erased).
    fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_>;

    /// Get a parameter by its ID.
    fn by_id(&self, id: ParameterId) -> Option<&dyn ParameterRef>;

    /// Get a mutable reference to a parameter by its ID.
    ///
    /// Note: This returns `&dyn ParameterRef` (not `&mut`) because atomic
    /// parameters can be modified through shared references.
    fn by_id_mut(&mut self, id: ParameterId) -> Option<&dyn ParameterRef> {
        self.by_id(id)
    }

    /// Set group ID for all direct parameters in this collection.
    ///
    /// Called by parent structs when initializing nested parameter groups.
    /// The default implementation does nothing (for flat parameter structs).
    fn set_all_group_ids(&mut self, _group_id: GroupId) {
        // Default: no-op (macro generates override for parameter-containing structs)
    }

    // =========================================================================
    // Nested Group Discovery (for recursive group ID assignment)
    // =========================================================================

    /// Number of direct nested parameter groups in this struct.
    ///
    /// Default is 0 (no nested groups). The `#[derive(Parameters)]` macro
    /// generates an override for structs with `#[nested]` fields.
    fn nested_count(&self) -> usize {
        0
    }

    /// Get information about a nested group by index.
    ///
    /// Returns the group name and a reference to the nested Parameters.
    /// Default returns None (no nested groups).
    fn nested_group(&self, _index: usize) -> Option<(&'static str, &dyn Parameters)> {
        None
    }

    /// Get mutable access to a nested group by index.
    ///
    /// Returns the group name and a mutable reference to the nested Parameters.
    /// Default returns None (no nested groups).
    fn nested_group_mut(&mut self, _index: usize) -> Option<(&'static str, &mut dyn Parameters)> {
        None
    }

    /// Recursively assign group IDs to all nested groups.
    ///
    /// This method traverses the nested group hierarchy and assigns
    /// sequential group IDs, properly setting parent relationships for
    /// deeply nested groups.
    ///
    /// # Arguments
    ///
    /// * `start_id` - The first group ID to assign (typically 1, since 0 is root)
    /// * `parent_id` - The parent group ID for this level's nested groups
    ///
    /// # Returns
    ///
    /// The next available group ID after all assignments.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Called by set_group_ids() on the top-level struct:
    /// let next_id = self.assign_group_ids(1, 0);
    /// ```
    fn assign_group_ids(&mut self, start_id: i32, _parent_id: i32) -> i32 {
        let mut next_id = start_id;

        for i in 0..self.nested_count() {
            if let Some((_, nested)) = self.nested_group_mut(i) {
                let group_id = next_id;
                next_id += 1;

                // Set group ID on all direct parameters in this nested group
                nested.set_all_group_ids(group_id);

                // Recurse into this nested group's nested groups
                // The current group_id becomes the parent for nested groups
                next_id = nested.assign_group_ids(next_id, group_id);
            }
        }

        next_id
    }

    /// Collect all group infos from nested groups recursively.
    ///
    /// This is used by the `ParameterGroups` trait implementation to build the
    /// complete list of groups for the DAW.
    ///
    /// # Arguments
    ///
    /// * `groups` - Vector to append GroupInfo entries to
    /// * `start_id` - The first group ID for this level
    /// * `parent_id` - The parent group ID for this level's groups
    ///
    /// # Returns
    ///
    /// The next available group ID after all groups are collected.
    fn collect_groups(
        &self,
        groups: &mut Vec<GroupInfo>,
        start_id: i32,
        parent_id: i32,
    ) -> i32 {
        let mut next_id = start_id;

        for i in 0..self.nested_count() {
            if let Some((name, nested)) = self.nested_group(i) {
                let group_id = next_id;
                next_id += 1;

                groups.push(GroupInfo::new(group_id, name, parent_id));

                // Recurse into nested groups
                next_id = nested.collect_groups(groups, next_id, group_id);
            }
        }

        next_id
    }

    // =========================================================================
    // State Serialization (with path support for nested groups)
    // =========================================================================

    /// Serialize parameters with a path prefix for nested group support.
    ///
    /// This is called by macro-generated `save_state` to handle hierarchical
    /// parameter structures. Each nested group adds its name to the path.
    ///
    /// # Format
    ///
    /// Each entry: `[path_len: u8][path: utf8 bytes][value: f64]`
    ///
    /// Path examples:
    /// - `"gain"` - top-level parameter
    /// - `"filter/cutoff"` - parameter in nested "Filter" group
    /// - `"osc1/filter/resonance"` - deeply nested parameter
    ///
    /// # Arguments
    ///
    /// * `data` - Buffer to append serialized data to
    /// * `prefix` - Current path prefix (empty for root level)
    fn save_state_prefixed(&self, data: &mut Vec<u8>, prefix: &str) {
        // Default implementation for flat parameter structs (no nesting)
        // The macro generates an override for structs with nested groups
        for parameter in self.iter() {
            // For default impl, use numeric ID as string
            let id_str = parameter.id().to_string();
            let path = if prefix.is_empty() {
                id_str
            } else {
                format!("{}/{}", prefix, id_str)
            };

            let path_bytes = path.as_bytes();
            data.push(path_bytes.len() as u8);
            data.extend_from_slice(path_bytes);
            data.extend_from_slice(&parameter.get_normalized().to_le_bytes());
        }
    }

    /// Serialize all parameters to bytes.
    ///
    /// Format: `[path_len: u8, path: utf8, value: f64]*`
    ///
    /// Parameters in nested groups use path-based IDs like "filter/cutoff"
    /// to avoid collisions when the same nested struct is reused.
    fn save_state(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.count() * 20);
        self.save_state_prefixed(&mut data, "");
        data
    }

    /// Load a single parameter by its path.
    ///
    /// This is called during state restoration to route each (path, value) pair
    /// to the correct parameter. The path may include group prefixes like
    /// "filter/cutoff" for nested parameters.
    ///
    /// Returns `true` if the parameter was found and set, `false` otherwise.
    ///
    /// The default implementation handles flat parameter structs by matching
    /// the path against numeric IDs. The macro generates an override for
    /// structs with nested groups that routes based on path segments.
    fn load_state_path(&mut self, path: &str, value: f64) -> bool {
        // Default implementation for flat structs (no nesting)
        // Try to parse as numeric ID
        if let Ok(id) = path.parse::<u32>() {
            if let Some(parameter) = self.by_id_mut(id) {
                parameter.set_normalized(value.clamp(0.0, 1.0));
                return true;
            }
        }
        false
    }

    /// Restore parameters from bytes.
    ///
    /// Format: `[path_len: u8, path: utf8, value: f64]*`
    /// Unknown parameter paths are silently ignored for forward compatibility.
    fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.is_empty() {
            return Ok(());
        }

        let mut cursor = 0;
        while cursor < data.len() {
            // Read path length
            let path_len = data[cursor] as usize;
            cursor += 1;

            if cursor + path_len + 8 > data.len() {
                break; // Incomplete data
            }

            // Read path string
            let path = match std::str::from_utf8(&data[cursor..cursor + path_len]) {
                Ok(s) => s,
                Err(_) => {
                    cursor += path_len + 8;
                    continue; // Skip invalid UTF-8
                }
            };
            cursor += path_len;

            // Read value
            let value_bytes: [u8; 8] = data[cursor..cursor + 8]
                .try_into()
                .map_err(|_| "Invalid state data")?;
            let value = f64::from_le_bytes(value_bytes);
            cursor += 8;

            // Try to set parameter by path
            // Default implementation uses numeric ID parsing
            if let Ok(id) = path.parse::<u32>() {
                if let Some(parameter) = self.by_id_mut(id) {
                    parameter.set_normalized(value.clamp(0.0, 1.0));
                }
            }
        }

        Ok(())
    }

    // =========================================================================
    // Smoothing Support
    // =========================================================================

    /// Set sample rate for all smoothers in this parameter collection.
    ///
    /// Call this from `Processor::setup()` to initialize smoothers
    /// with the correct sample rate.
    ///
    /// **Oversampling:** If your plugin uses oversampling, pass the actual
    /// processing rate: `sample_rate * oversampling_factor`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// impl Processor for MyPlugin {
    ///     fn setup(&mut self, sample_rate: f64, _max_buffer_size: usize) {
    ///         self.parameters.set_sample_rate(sample_rate);
    ///     }
    /// }
    /// ```
    fn set_sample_rate(&mut self, _sample_rate: f64) {
        // Default no-op. The #[derive(Parameters)] macro generates an override
        // that calls set_sample_rate on each parameter field.
    }

    /// Reset all smoothers to their current values (no ramp).
    ///
    /// Called automatically by the framework after loading state to avoid
    /// ramps to loaded values. You typically don't need to call this directly.
    fn reset_smoothing(&mut self) {
        // Default no-op. The #[derive(Parameters)] macro generates an override
        // that calls reset_smoothing on each parameter field.
    }
}

// =============================================================================
// FloatParameter - Float parameter with atomic storage
// =============================================================================

/// Float parameter with atomic storage and automatic formatting.
///
/// # Specialized Constructors
///
/// - [`FloatParameter::new`]: Generic float parameter
/// - [`FloatParameter::db`]: Decibel parameter with dB formatting
/// - [`FloatParameter::hz`]: Frequency parameter with logarithmic mapping
/// - [`FloatParameter::ms`]: Milliseconds parameter
/// - [`FloatParameter::seconds`]: Seconds parameter
/// - [`FloatParameter::percent`]: Percentage parameter (0-100%)
/// - [`FloatParameter::pan`]: Pan parameter (L-C-R)
/// - [`FloatParameter::ratio`]: Compressor ratio parameter
///
/// # Example
///
/// ```ignore
/// // Create parameter - ID is set separately via with_id() or #[derive(Parameters)]
/// let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0).with_id(0);
/// let freq = FloatParameter::hz("Frequency", 440.0, 20.0..=20000.0).with_id(1);
///
/// // Read/write plain values
/// let current_gain = gain.get(); // Returns linear value
/// freq.set(1000.0); // Set to 1000 Hz
///
/// // For DSP: get linear amplitude
/// let amplitude = gain.as_linear();
/// ```
pub struct FloatParameter {
    /// Parameter metadata (id, name, units, flags, etc.)
    info: ParameterInfo,
    /// Atomic storage for normalized value (0.0-1.0)
    value: AtomicU64,
    /// Range mapper for normalized ↔ plain value conversion
    range: Box<dyn RangeMapper>,
    /// Formatter for display string conversion
    formatter: Formatter,
    /// Optional smoother for avoiding zipper noise
    smoother: Option<Smoother>,
    /// Whether this parameter stores dB values (for as_linear() optimization)
    is_db: bool,
    /// Optional step size for discrete stepping. None = continuous.
    step_size: Option<f64>,
}

impl FloatParameter {
    /// Create a generic float parameter with linear mapping.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value in plain units
    /// * `range` - Valid range in plain units (inclusive)
    pub fn new(name: &'static str, default: f64, range: RangeInclusive<f64>) -> Self {
        let mapper = LinearMapper::new(range);
        let default_normalized = mapper.normalize(default);

        Self {
            info: ParameterInfo {
                id: 0, // Set via with_id() or macro
                name,
                short_name: name,
                units: "",
                unit: ParameterUnit::Generic,
                default_normalized,
                step_count: 0,
                flags: ParameterFlags::default(),
                group_id: ROOT_GROUP_ID,
            },
            value: AtomicU64::new(default_normalized.to_bits()),
            range: Box::new(mapper),
            formatter: Formatter::Float { precision: 2 },
            smoother: None,
            is_db: false,
            step_size: None,
        }
    }

    /// Create a decibel parameter.
    ///
    /// The parameter value is stored in **dB** internally. Use [`as_linear`](Self::as_linear)
    /// to get the linear amplitude for DSP processing.
    ///
    /// - [`get`](Self::get) returns the dB value (for display, host automation)
    /// - [`as_linear`](Self::as_linear) returns linear amplitude (for DSP)
    /// - [`normalized_to_plain`](ParameterRef::normalized_to_plain) returns dB (matches `units`)
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_db` - Default value in dB
    /// * `range_db` - Valid range in dB (inclusive)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0).with_id(0);
    ///
    /// // For DSP: use as_linear() to get amplitude multiplier
    /// let amplitude = gain.as_linear(); // 0 dB → 1.0, -6 dB → ~0.5
    ///
    /// // For display/automation: get() returns dB value
    /// let db_value = gain.get(); // Returns -6.0 for -6 dB
    /// ```
    pub fn db(name: &'static str, default_db: f64, range_db: RangeInclusive<f64>) -> Self {
        // Store dB values directly (not linear) so normalized_to_plain returns dB
        // Use as_linear() in DSP code to get linear amplitude
        let min_db = *range_db.start();
        let mapper = LinearMapper::new(range_db);
        let default_normalized = mapper.normalize(default_db);
        let formatter = Formatter::DecibelDirect { precision: 1, min_db };

        Self {
            info: ParameterInfo {
                id: 0,
                name,
                short_name: name,
                units: formatter.unit(),
                unit: ParameterUnit::Decibels,
                default_normalized,
                step_count: 0,
                flags: ParameterFlags::default(),
                group_id: ROOT_GROUP_ID,
            },
            value: AtomicU64::new(default_normalized.to_bits()),
            range: Box::new(mapper),
            formatter,
            smoother: None,
            is_db: true,
            step_size: None,
        }
    }

    /// Create a dB parameter with power curve mapping for more resolution at maximum.
    ///
    /// Uses a power curve (exponent = 2.0) to provide more resolution near 0 dB
    /// and less resolution at the minimum. Ideal for threshold parameters where
    /// precision near 0 dB is important.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_db` - Default value in dB
    /// * `range_db` - Valid range in dB (inclusive)
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Threshold parameter: -60 to 0 dB with more resolution near 0 dB
    /// let threshold = FloatParameter::db_log("Threshold", -20.0, -60.0..=0.0);
    /// ```
    pub fn db_log(name: &'static str, default_db: f64, range_db: RangeInclusive<f64>) -> Self {
        let min_db = *range_db.start();
        let mapper = PowerMapper::new(range_db, 2.0);
        let default_normalized = mapper.normalize(default_db);
        let formatter = Formatter::DecibelDirect { precision: 1, min_db };

        Self {
            info: ParameterInfo {
                id: 0,
                name,
                short_name: name,
                units: formatter.unit(),
                unit: ParameterUnit::Decibels,
                default_normalized,
                step_count: 0,
                flags: ParameterFlags::default(),
                group_id: ROOT_GROUP_ID,
            },
            value: AtomicU64::new(default_normalized.to_bits()),
            range: Box::new(mapper),
            formatter,
            smoother: None,
            is_db: true,
            step_size: None,
        }
    }

    /// Create a dB parameter with true logarithmic mapping (using offset).
    ///
    /// Uses logarithmic mapping for ranges that include negative values by
    /// offsetting to positive space. Provides geometric mean at midpoint.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_db` - Default value in dB
    /// * `range_db` - Valid range in dB (inclusive)
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Threshold parameter with true logarithmic behavior
    /// let threshold = FloatParameter::db_log_offset("Threshold", -20.0, -60.0..=0.0);
    /// ```
    pub fn db_log_offset(
        name: &'static str,
        default_db: f64,
        range_db: RangeInclusive<f64>,
    ) -> Self {
        let min_db = *range_db.start();
        let mapper = LogOffsetMapper::new(range_db);
        let default_normalized = mapper.normalize(default_db);
        let formatter = Formatter::DecibelDirect { precision: 1, min_db };

        Self {
            info: ParameterInfo {
                id: 0,
                name,
                short_name: name,
                units: formatter.unit(),
                unit: ParameterUnit::Decibels,
                default_normalized,
                step_count: 0,
                flags: ParameterFlags::default(),
                group_id: ROOT_GROUP_ID,
            },
            value: AtomicU64::new(default_normalized.to_bits()),
            range: Box::new(mapper),
            formatter,
            smoother: None,
            is_db: true,
            step_size: None,
        }
    }

    /// Create a frequency parameter with logarithmic mapping.
    ///
    /// Logarithmic mapping provides a perceptually uniform distribution
    /// across the frequency range.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_hz` - Default value in Hz
    /// * `range_hz` - Valid range in Hz (inclusive, must be positive)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let freq = FloatParameter::hz("Frequency", 440.0, 20.0..=20000.0).with_id(0);
    /// ```
    pub fn hz(name: &'static str, default_hz: f64, range_hz: RangeInclusive<f64>) -> Self {
        let mapper = LogMapper::new(range_hz.clone());
        let default_normalized = mapper.normalize(default_hz);
        let formatter = Formatter::Frequency;

        Self {
            info: ParameterInfo {
                id: 0,
                name,
                short_name: name,
                units: formatter.unit(),
                unit: ParameterUnit::Hertz,
                default_normalized,
                step_count: 0,
                flags: ParameterFlags::default(),
                group_id: ROOT_GROUP_ID,
            },
            value: AtomicU64::new(default_normalized.to_bits()),
            range: Box::new(mapper),
            formatter,
            smoother: None,
            is_db: false,
            step_size: None,
        }
    }

    /// Create a milliseconds parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_ms` - Default value in milliseconds
    /// * `range_ms` - Valid range in milliseconds (inclusive)
    pub fn ms(name: &'static str, default_ms: f64, range_ms: RangeInclusive<f64>) -> Self {
        let mut parameter = Self::new(name, default_ms, range_ms);
        let formatter = Formatter::Milliseconds { precision: 1 };
        parameter.info.units = formatter.unit();
        parameter.info.unit = ParameterUnit::Milliseconds;
        parameter.formatter = formatter;
        parameter
    }

    /// Create a seconds parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_s` - Default value in seconds
    /// * `range_s` - Valid range in seconds (inclusive)
    pub fn seconds(name: &'static str, default_s: f64, range_s: RangeInclusive<f64>) -> Self {
        let mut parameter = Self::new(name, default_s, range_s);
        let formatter = Formatter::Seconds { precision: 2 };
        parameter.info.units = formatter.unit();
        parameter.info.unit = ParameterUnit::Seconds;
        parameter.formatter = formatter;
        parameter
    }

    /// Create a percentage parameter.
    ///
    /// The value is stored as 0.0-1.0 internally but displayed as 0%-100%.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_pct` - Default value as 0.0-1.0 (not 0-100)
    pub fn percent(name: &'static str, default_pct: f64) -> Self {
        let mut parameter = Self::new(name, default_pct, 0.0..=1.0);
        let formatter = Formatter::Percent { precision: 0 };
        parameter.info.units = formatter.unit();
        parameter.info.unit = ParameterUnit::Percent;
        parameter.formatter = formatter;
        parameter
    }

    /// Create a pan parameter.
    ///
    /// Range is -1.0 (full left) to +1.0 (full right), with 0.0 being center.
    /// Display: "L50", "C", "R50"
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value (-1.0 to +1.0, typically 0.0)
    pub fn pan(name: &'static str, default: f64) -> Self {
        let mut parameter = Self::new(name, default, -1.0..=1.0);
        parameter.info.unit = ParameterUnit::Pan;
        parameter.formatter = Formatter::Pan;
        parameter
    }

    /// Create a ratio parameter for compressors.
    ///
    /// Display: "4.0:1", "∞:1"
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default ratio value
    /// * `range` - Valid ratio range (inclusive)
    pub fn ratio(name: &'static str, default: f64, range: RangeInclusive<f64>) -> Self {
        let mut parameter = Self::new(name, default, range);
        parameter.info.unit = ParameterUnit::Ratio;
        parameter.formatter = Formatter::Ratio { precision: 1 };
        parameter
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// This is typically called by the `#[derive(Parameters)]` macro to assign
    /// the FNV-1a hash of the string ID. For manual usage, you can pass
    /// any unique u32 value.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0).with_id(0x050c5d1f);
    /// ```
    pub fn with_id(mut self, id: ParameterId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the group ID (parameter group) for this parameter.
    ///
    /// Used by the `#[derive(Parameters)]` macro to assign parameters to groups.
    pub fn with_group(mut self, group_id: GroupId) -> Self {
        self.info.group_id = group_id;
        self
    }

    /// Set the group ID in-place (for runtime assignment by parent structs).
    pub fn set_group_id(&mut self, group_id: GroupId) {
        self.info.group_id = group_id;
    }

    /// Make the parameter read-only (display only, not automatable).
    pub fn readonly(mut self) -> Self {
        self.info.flags.is_readonly = true;
        self.info.flags.can_automate = false;
        self
    }

    /// Disable automation for this parameter.
    pub fn non_automatable(mut self) -> Self {
        self.info.flags.can_automate = false;
        self
    }

    /// Set the unit type hint for AU hosts.
    ///
    /// This is typically set automatically by the constructor (e.g., `db()` sets `Decibels`),
    /// but can be overridden if needed.
    pub fn with_unit(mut self, unit: ParameterUnit) -> Self {
        self.info.unit = unit;
        self
    }

    /// Set the step size for discrete stepping.
    ///
    /// When set, values are snapped to the nearest multiple of `step_size`
    /// within the parameter's range. The `step_count` is automatically calculated
    /// as `((max - min) / step_size).round()` for host UI integration.
    ///
    /// # Panics
    ///
    /// Panics if `step_size <= 0.0`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Volume control that snaps to 0.5 dB increments
    /// let volume = FloatParameter::db("Volume", 0.0, -60.0..=12.0)
    ///     .with_step_size(0.5);
    ///
    /// volume.set(-5.3);
    /// assert_eq!(volume.get(), -5.5); // Snapped to nearest 0.5
    /// ```
    ///
    /// # Format-specific behavior
    ///
    /// - **VST3**: Fully supported. The `step_count` is communicated to the host,
    ///   which uses it for UI granularity and automation.
    /// - **AUv3**: Values are snapped correctly. However, the AUv3 API has no step
    ///   property, so hosts may display their own UI granularity based on the
    ///   parameter range.
    /// - **AUv2**: Values are snapped correctly. The AUv2 `AudioUnitParameterInfo`
    ///   structure has no step size field (format limitation), so hosts display
    ///   raw floating-point precision in their native UI.
    pub fn with_step_size(mut self, step_size: f64) -> Self {
        assert!(
            step_size > 0.0,
            "step_size must be positive, got {}",
            step_size
        );

        let (min, max) = self.range.range();
        let range_size = max - min;

        // Calculate step_count: number of intervals (not values)
        // step_count = 0 means continuous, step_count = N means N+1 discrete values
        let step_count = if step_size >= range_size {
            // Step size larger than range: treat as 2 values (min, max)
            1
        } else {
            (range_size / step_size).round() as i32
        };

        self.step_size = Some(step_size);
        self.info.step_count = step_count;
        self
    }

    /// Get the step size, if configured.
    pub fn step_size(&self) -> Option<f64> {
        self.step_size
    }

    /// Get the step count for host UI integration.
    ///
    /// Returns 0 for continuous parameters, or N for parameters with N+1 discrete values.
    pub fn step_count(&self) -> i32 {
        self.info.step_count
    }

    /// Set the display precision for this parameter.
    ///
    /// This updates the precision of the current formatter. For formatters that
    /// don't support precision (e.g., `Pan`, `Boolean`), this has no effect.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // High-precision gain for mastering plugins
    /// let gain = FloatParameter::db("Output", 0.0, -12.0..=12.0)
    ///     .with_precision(2);  // Shows "-0.50 dB" instead of "-0.5 dB"
    ///
    /// // Frequency with custom precision
    /// let freq = FloatParameter::hz("Cutoff", 1000.0, 20.0..=20000.0)
    ///     .with_precision(0);  // Note: Frequency formatter uses auto-scaling
    /// ```
    pub fn with_precision(mut self, precision: usize) -> Self {
        self.formatter = self.formatter.with_precision(precision);
        self
    }

    /// Replace the formatter for this parameter.
    ///
    /// This allows complete customization of how the parameter value is displayed
    /// and parsed. The unit string is automatically updated to match the new
    /// formatter.
    ///
    /// **Note:** For dB parameters created with [`db()`](Self::db), changing the
    /// formatter does not change the underlying value storage or the behavior of
    /// [`as_linear()`](Self::as_linear). The `is_db` flag remains set based on
    /// how the parameter was constructed.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Use a ratio formatter for a parameter
    /// let ratio = FloatParameter::new("Ratio", 4.0, 1.0..=20.0, LinearMapper::new(1.0..=20.0))
    ///     .with_formatter(Formatter::Ratio { precision: 1 });  // Shows "4.0:1"
    ///
    /// // Override precision and use a different dB formatter
    /// let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
    ///     .with_formatter(Formatter::DecibelDirect { precision: 2, min_db: -60.0 });
    /// ```
    pub fn with_formatter(mut self, formatter: Formatter) -> Self {
        self.info.units = formatter.unit();
        self.formatter = formatter;
        self
    }

    /// Get the current formatter.
    pub fn formatter(&self) -> &Formatter {
        &self.formatter
    }

    /// Get the parameter metadata.
    pub fn info(&self) -> &ParameterInfo {
        &self.info
    }

    /// Get mutable access to the parameter metadata.
    ///
    /// Used for runtime modification of parameter properties like group_id.
    pub fn info_mut(&mut self) -> &mut ParameterInfo {
        &mut self.info
    }

    // === Value access ===

    /// Get the current plain value in natural units.
    #[inline]
    pub fn get(&self) -> f64 {
        let normalized = f64::from_bits(self.value.load(Ordering::Relaxed));
        self.range.denormalize(normalized)
    }

    /// Set the plain value in natural units.
    ///
    /// If a step size is configured, the value is snapped to the nearest step.
    #[inline]
    pub fn set(&self, value: f64) {
        let snapped = match self.step_size {
            Some(step) => {
                let (min, max) = self.range.range();
                snap_to_step(value, step, min, max)
            }
            None => value,
        };
        let normalized = self.range.normalize(snapped);
        self.value.store(normalized.to_bits(), Ordering::Relaxed);
    }

    /// Get the value as linear amplitude.
    ///
    /// For dB parameters, this converts from dB to linear amplitude.
    /// For other parameters, this is equivalent to `get()`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0);
    ///
    /// // get() returns dB value for display
    /// assert_eq!(gain.get(), 0.0); // 0 dB
    ///
    /// // as_linear() returns linear amplitude for DSP
    /// assert!((gain.as_linear() - 1.0).abs() < 0.001); // ~1.0 linear
    /// ```
    #[inline]
    pub fn as_linear(&self) -> f64 {
        let plain = self.get();
        if self.is_db {
            db_to_linear(plain)
        } else {
            plain
        }
    }

    // === Smoothing methods ===

    /// Add smoothing to this parameter.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
    ///     .with_smoother(SmoothingStyle::Exponential(5.0));  // 5ms
    /// ```
    pub fn with_smoother(mut self, style: SmoothingStyle) -> Self {
        let current = self.get();
        let mut smoother = Smoother::new(style);
        smoother.reset(current);
        self.smoother = Some(smoother);
        self
    }

    /// Set sample rate for smoothing.
    ///
    /// Call this from `Processor::setup()`. If using oversampling,
    /// pass `sample_rate * oversampling_factor`.
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        let current_value = self.get();
        if let Some(ref mut smoother) = self.smoother {
            smoother.set_sample_rate(sample_rate);
            smoother.set_target(current_value);
        }
    }

    /// Get the current smoothed value without advancing.
    ///
    /// If no smoother is configured, returns the raw value.
    #[inline]
    pub fn smoothed(&self) -> f64 {
        match &self.smoother {
            Some(s) => s.current(),
            None => self.get(),
        }
    }

    /// Get the current smoothed value as f32.
    #[inline]
    pub fn smoothed_f32(&self) -> f32 {
        self.smoothed() as f32
    }

    /// Advance the smoother by one sample and return the smoothed value.
    ///
    /// Call once per sample in the audio loop. Requires `&mut self`.
    ///
    /// If no smoother is configured, returns the raw value.
    #[inline]
    pub fn tick_smoothed(&mut self) -> f64 {
        let current_value = self.get();
        match &mut self.smoother {
            Some(s) => {
                // Update target from atomic value (in case host changed it)
                s.set_target(current_value);
                s.tick()
            }
            None => current_value,
        }
    }

    /// Advance the smoother by one sample and return the smoothed value as f32.
    #[inline]
    pub fn tick_smoothed_f32(&mut self) -> f32 {
        self.tick_smoothed() as f32
    }

    /// Skip smoothing forward by n samples.
    ///
    /// Use for block processing when per-sample smoothing isn't needed.
    pub fn skip_smoothing(&mut self, samples: usize) {
        let current_value = self.get();
        if let Some(ref mut smoother) = self.smoother {
            smoother.set_target(current_value);
            smoother.skip(samples);
        }
    }

    /// Fill buffer with smoothed values (f64).
    pub fn fill_smoothed(&mut self, buffer: &mut [f64]) {
        let current_value = self.get();
        match &mut self.smoother {
            Some(s) => {
                s.set_target(current_value);
                s.fill(buffer);
            }
            None => {
                buffer.fill(current_value);
            }
        }
    }

    /// Fill buffer with smoothed values (f32).
    pub fn fill_smoothed_f32(&mut self, buffer: &mut [f32]) {
        let current_value = self.get();
        match &mut self.smoother {
            Some(s) => {
                s.set_target(current_value);
                s.fill_f32(buffer);
            }
            None => {
                buffer.fill(current_value as f32);
            }
        }
    }

    /// Check if parameter is currently smoothing.
    pub fn is_smoothing(&self) -> bool {
        self.smoother
            .as_ref()
            .map(|s| s.is_smoothing())
            .unwrap_or(false)
    }

    /// Reset smoother to current value (no ramp).
    ///
    /// Use when loading state to avoid ramps to loaded values.
    pub fn reset_smoothing(&mut self) {
        let current_value = self.get();
        if let Some(ref mut smoother) = self.smoother {
            smoother.reset(current_value);
        }
    }
}

impl ParameterRef for FloatParameter {
    fn id(&self) -> ParameterId {
        self.info.id
    }

    fn name(&self) -> &'static str {
        self.info.name
    }

    fn short_name(&self) -> &'static str {
        self.info.short_name
    }

    fn units(&self) -> &'static str {
        self.info.units
    }

    fn flags(&self) -> &ParameterFlags {
        &self.info.flags
    }

    fn default_normalized(&self) -> ParameterValue {
        self.info.default_normalized
    }

    fn step_count(&self) -> i32 {
        self.info.step_count
    }

    fn get_normalized(&self) -> ParameterValue {
        f64::from_bits(self.value.load(Ordering::Relaxed))
    }

    fn set_normalized(&self, value: ParameterValue) {
        self.value
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    fn get_plain(&self) -> ParameterValue {
        self.get()
    }

    fn set_plain(&self, value: ParameterValue) {
        self.set(value);
    }

    fn display_normalized(&self, normalized: ParameterValue) -> String {
        let plain = self.range.denormalize(normalized);
        self.formatter.text(plain)
    }

    fn parse(&self, s: &str) -> Option<ParameterValue> {
        let plain = self.formatter.parse(s)?;
        Some(self.range.normalize(plain))
    }

    fn normalized_to_plain(&self, normalized: ParameterValue) -> ParameterValue {
        self.range.denormalize(normalized)
    }

    fn plain_to_normalized(&self, plain: ParameterValue) -> ParameterValue {
        self.range.normalize(plain)
    }

    fn info(&self) -> &ParameterInfo {
        &self.info
    }
}

// FloatParameter is automatically Send + Sync because:
// - AtomicU64 is Send + Sync
// - Box<dyn RangeMapper> is Send + Sync (RangeMapper: Send + Sync)
// - All other fields (&'static str, f64, Formatter, ParameterFlags) are Send + Sync
// No unsafe impl needed - the compiler verifies this automatically.

// =============================================================================
// IntParameter - Integer parameter with atomic storage
// =============================================================================

/// Integer parameter with atomic storage.
///
/// # Specialized Constructors
///
/// - [`IntParameter::new`]: Generic integer parameter
/// - [`IntParameter::semitones`]: Semitones parameter for pitch shifting
///
/// # Example
///
/// ```ignore
/// let octave = IntParameter::semitones("Octave", 0, -24..=24).with_id(0);
/// println!("Current: {} semitones", octave.get());
/// ```
pub struct IntParameter {
    /// Parameter metadata (id, name, units, flags, etc.)
    info: ParameterInfo,
    /// Atomic storage for the integer value
    value: AtomicI64,
    /// Minimum value
    min: i64,
    /// Maximum value
    max: i64,
    /// Formatter for display string conversion
    formatter: Formatter,
}

impl IntParameter {
    /// Create a generic integer parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value
    /// * `range` - Valid range (inclusive)
    pub fn new(name: &'static str, default: i64, range: RangeInclusive<i64>) -> Self {
        let min = *range.start();
        let max = *range.end();
        // Use i128 to avoid overflow for extreme ranges like i64::MIN..=i64::MAX
        let range_size = (max as i128) - (min as i128);
        let default_offset = (default as i128) - (min as i128);
        let default_normalized = if range_size == 0 {
            0.5
        } else {
            ((default_offset as f64) / (range_size as f64)).clamp(0.0, 1.0)
        };

        // Cap step_count at i32::MAX for very large ranges
        let step_count = if range_size > i32::MAX as i128 {
            i32::MAX
        } else {
            range_size as i32
        };

        Self {
            info: ParameterInfo {
                id: 0,
                name,
                short_name: name,
                units: "",
                unit: ParameterUnit::Generic,
                default_normalized,
                step_count,
                flags: ParameterFlags::default(),
                group_id: ROOT_GROUP_ID,
            },
            value: AtomicI64::new(default.clamp(min, max)),
            min,
            max,
            formatter: Formatter::Float { precision: 0 },
        }
    }

    /// Create a semitones parameter for pitch shifting.
    ///
    /// Format: "+12", "-7", "0" (unit "st" via `units()`)
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value in semitones
    /// * `range` - Valid range in semitones (inclusive)
    pub fn semitones(name: &'static str, default: i64, range: RangeInclusive<i64>) -> Self {
        let mut parameter = Self::new(name, default, range);
        let formatter = Formatter::Semitones;
        parameter.info.units = formatter.unit();
        parameter.info.unit = ParameterUnit::RelativeSemiTones;
        parameter.formatter = formatter;
        parameter
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// This is typically called by the `#[derive(Parameters)]` macro to assign
    /// the FNV-1a hash of the string ID.
    pub fn with_id(mut self, id: ParameterId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the group ID (parameter group) for this parameter.
    ///
    /// Used by the `#[derive(Parameters)]` macro to assign parameters to groups.
    pub fn with_group(mut self, group_id: GroupId) -> Self {
        self.info.group_id = group_id;
        self
    }

    /// Set the group ID in-place (for runtime assignment by parent structs).
    pub fn set_group_id(&mut self, group_id: GroupId) {
        self.info.group_id = group_id;
    }

    /// Make the parameter read-only.
    pub fn readonly(mut self) -> Self {
        self.info.flags.is_readonly = true;
        self.info.flags.can_automate = false;
        self
    }

    /// Disable automation for this parameter.
    pub fn non_automatable(mut self) -> Self {
        self.info.flags.can_automate = false;
        self
    }

    /// Set the unit type hint for AU hosts.
    ///
    /// This is typically set automatically by the constructor (e.g., `semitones()` sets
    /// `RelativeSemiTones`), but can be overridden if needed.
    pub fn with_unit(mut self, unit: ParameterUnit) -> Self {
        self.info.unit = unit;
        self
    }

    /// Get the parameter metadata.
    pub fn info(&self) -> &ParameterInfo {
        &self.info
    }

    /// Get mutable access to the parameter metadata.
    ///
    /// Used for runtime modification of parameter properties like group_id.
    pub fn info_mut(&mut self) -> &mut ParameterInfo {
        &mut self.info
    }

    /// Set the display precision for this parameter.
    ///
    /// This updates the precision of the current formatter. For formatters that
    /// don't support precision (e.g., `Semitones`), this has no effect.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // IntParameter uses Float { precision: 0 } by default.
    /// // Use with_formatter() to change to a formatter that supports precision,
    /// // then with_precision() to adjust it:
    /// let value = IntParameter::new("Value", 50, 0..=100)
    ///     .with_formatter(Formatter::Percent { precision: 1 })
    ///     .with_precision(0);  // Shows "50%" instead of "50.0%"
    /// ```
    pub fn with_precision(mut self, precision: usize) -> Self {
        self.formatter = self.formatter.with_precision(precision);
        self
    }

    /// Replace the formatter for this parameter.
    ///
    /// This allows complete customization of how the parameter value is displayed
    /// and parsed. The unit string is automatically updated to match the new
    /// formatter.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let transpose = IntParameter::new("Transpose", 0, -24..=24)
    ///     .with_formatter(Formatter::Semitones);  // Shows "+12", "-7", "0"
    /// ```
    pub fn with_formatter(mut self, formatter: Formatter) -> Self {
        self.info.units = formatter.unit();
        self.formatter = formatter;
        self
    }

    /// Get the current formatter.
    pub fn formatter(&self) -> &Formatter {
        &self.formatter
    }

    // === Value access ===

    /// Get the current integer value.
    #[inline]
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Set the integer value.
    #[inline]
    pub fn set(&self, value: i64) {
        self.value
            .store(value.clamp(self.min, self.max), Ordering::Relaxed);
    }

    // === Smoothing compatibility (no-ops for IntParameter) ===

    /// No-op for compatibility with the `#[derive(Parameters)]` macro.
    ///
    /// Integer parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn set_sample_rate(&mut self, _sample_rate: f64) {
        // No-op: IntParameter doesn't support smoothing
    }

    /// No-op for compatibility with the `#[derive(Parameters)]` macro.
    ///
    /// Integer parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn reset_smoothing(&mut self) {
        // No-op: IntParameter doesn't support smoothing
    }
}

impl ParameterRef for IntParameter {
    fn id(&self) -> ParameterId {
        self.info.id
    }

    fn name(&self) -> &'static str {
        self.info.name
    }

    fn short_name(&self) -> &'static str {
        self.info.short_name
    }

    fn units(&self) -> &'static str {
        self.info.units
    }

    fn flags(&self) -> &ParameterFlags {
        &self.info.flags
    }

    fn default_normalized(&self) -> ParameterValue {
        self.info.default_normalized
    }

    fn step_count(&self) -> i32 {
        self.info.step_count
    }

    fn get_normalized(&self) -> ParameterValue {
        self.plain_to_normalized(self.get() as f64)
    }

    fn set_normalized(&self, value: ParameterValue) {
        let plain = self.normalized_to_plain(value).round() as i64;
        self.set(plain);
    }

    fn get_plain(&self) -> ParameterValue {
        self.get() as f64
    }

    fn set_plain(&self, value: ParameterValue) {
        self.set(value.round() as i64);
    }

    fn display_normalized(&self, normalized: ParameterValue) -> String {
        let plain = self.normalized_to_plain(normalized).round();
        self.formatter.text(plain)
    }

    fn parse(&self, s: &str) -> Option<ParameterValue> {
        let plain = self.formatter.parse(s)?;
        Some(self.plain_to_normalized(plain))
    }

    fn normalized_to_plain(&self, normalized: ParameterValue) -> ParameterValue {
        let normalized = normalized.clamp(0.0, 1.0);
        (self.min as f64) + normalized * ((self.max - self.min) as f64)
    }

    fn plain_to_normalized(&self, plain: ParameterValue) -> ParameterValue {
        if self.max == self.min {
            return 0.5;
        }
        ((plain - self.min as f64) / (self.max - self.min) as f64).clamp(0.0, 1.0)
    }

    fn info(&self) -> &ParameterInfo {
        &self.info
    }
}

// =============================================================================
// BoolParameter - Boolean parameter
// =============================================================================

/// Boolean parameter (toggle).
///
/// # Specialized Constructors
///
/// - [`BoolParameter::new`]: Generic boolean parameter
/// - [`BoolParameter::bypass`]: Bypass parameter with VST3 flags
///
/// # Example
///
/// ```ignore
/// let enabled = BoolParameter::new("Enabled", true).with_id(0);
/// let bypass = BoolParameter::bypass().with_id(1);
///
/// if enabled.get() && !bypass.get() {
///     // Process audio
/// }
/// ```
pub struct BoolParameter {
    /// Parameter metadata (id, name, units, flags, etc.)
    info: ParameterInfo,
    /// Atomic storage for the boolean value
    value: AtomicBool,
    /// Formatter for display string conversion
    formatter: Formatter,
}

impl BoolParameter {
    /// Create a generic boolean parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value
    pub fn new(name: &'static str, default: bool) -> Self {
        Self {
            info: ParameterInfo {
                id: 0,
                name,
                short_name: name,
                units: "",
                unit: ParameterUnit::Boolean,
                default_normalized: if default { 1.0 } else { 0.0 },
                step_count: 1, // Toggle
                flags: ParameterFlags::default(),
                group_id: ROOT_GROUP_ID,
            },
            value: AtomicBool::new(default),
            formatter: Formatter::Boolean,
        }
    }

    /// Create a bypass parameter with proper VST3 flags.
    ///
    /// This creates a parameter pre-configured as a bypass switch:
    /// - Name: "Bypass"
    /// - Short name: "Byp"
    /// - Default: false (not bypassed)
    /// - Marked with `is_bypass = true` flag for VST3
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    pub fn bypass() -> Self {
        Self {
            info: ParameterInfo {
                id: 0,
                name: "Bypass",
                short_name: "Byp",
                units: "",
                unit: ParameterUnit::Boolean,
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
            },
            value: AtomicBool::new(false),
            formatter: Formatter::Boolean,
        }
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// This is typically called by the `#[derive(Parameters)]` macro to assign
    /// the FNV-1a hash of the string ID.
    pub fn with_id(mut self, id: ParameterId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the group ID (parameter group) for this parameter.
    ///
    /// Used by the `#[derive(Parameters)]` macro to assign parameters to groups.
    pub fn with_group(mut self, group_id: GroupId) -> Self {
        self.info.group_id = group_id;
        self
    }

    /// Set the group ID in-place (for runtime assignment by parent structs).
    pub fn set_group_id(&mut self, group_id: GroupId) {
        self.info.group_id = group_id;
    }

    /// Make the parameter read-only.
    pub fn readonly(mut self) -> Self {
        self.info.flags.is_readonly = true;
        self.info.flags.can_automate = false;
        self
    }

    /// Disable automation for this parameter.
    pub fn non_automatable(mut self) -> Self {
        self.info.flags.can_automate = false;
        self
    }

    /// Set the unit type hint for AU hosts.
    ///
    /// BoolParameter defaults to `Boolean` which renders as a checkbox.
    /// This can be overridden if needed.
    pub fn with_unit(mut self, unit: ParameterUnit) -> Self {
        self.info.unit = unit;
        self
    }

    /// Get the parameter metadata.
    pub fn info(&self) -> &ParameterInfo {
        &self.info
    }

    /// Get mutable access to the parameter metadata.
    ///
    /// Used for runtime modification of parameter properties like group_id.
    pub fn info_mut(&mut self) -> &mut ParameterInfo {
        &mut self.info
    }

    // === Value access ===

    /// Get the current boolean value.
    #[inline]
    pub fn get(&self) -> bool {
        self.value.load(Ordering::Relaxed)
    }

    /// Set the boolean value.
    #[inline]
    pub fn set(&self, value: bool) {
        self.value.store(value, Ordering::Relaxed);
    }

    // === Smoothing compatibility (no-ops for BoolParameter) ===

    /// No-op for compatibility with the `#[derive(Parameters)]` macro.
    ///
    /// Boolean parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn set_sample_rate(&mut self, _sample_rate: f64) {
        // No-op: BoolParameter doesn't support smoothing
    }

    /// No-op for compatibility with the `#[derive(Parameters)]` macro.
    ///
    /// Boolean parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn reset_smoothing(&mut self) {
        // No-op: BoolParameter doesn't support smoothing
    }
}

impl ParameterRef for BoolParameter {
    fn id(&self) -> ParameterId {
        self.info.id
    }

    fn name(&self) -> &'static str {
        self.info.name
    }

    fn short_name(&self) -> &'static str {
        self.info.short_name
    }

    fn units(&self) -> &'static str {
        self.info.units
    }

    fn flags(&self) -> &ParameterFlags {
        &self.info.flags
    }

    fn default_normalized(&self) -> ParameterValue {
        self.info.default_normalized
    }

    fn step_count(&self) -> i32 {
        self.info.step_count
    }

    fn get_normalized(&self) -> ParameterValue {
        if self.get() {
            1.0
        } else {
            0.0
        }
    }

    fn set_normalized(&self, value: ParameterValue) {
        self.set(value > 0.5);
    }

    fn get_plain(&self) -> ParameterValue {
        self.get_normalized()
    }

    fn set_plain(&self, value: ParameterValue) {
        self.set_normalized(value);
    }

    fn display_normalized(&self, normalized: ParameterValue) -> String {
        self.formatter.text(normalized)
    }

    fn parse(&self, s: &str) -> Option<ParameterValue> {
        self.formatter.parse(s)
    }

    fn normalized_to_plain(&self, normalized: ParameterValue) -> ParameterValue {
        normalized
    }

    fn plain_to_normalized(&self, plain: ParameterValue) -> ParameterValue {
        plain
    }

    fn info(&self) -> &ParameterInfo {
        &self.info
    }
}

// =============================================================================
// EnumParameterValue Trait - For enums used as parameter values
// =============================================================================

/// Trait for enums that can be used as parameter values.
///
/// This trait is implemented by `#[derive(EnumParameter)]` and provides the
/// interface for converting between enum variants and indices.
///
/// # Example
///
/// ```ignore
/// use beamer::EnumParameter;
///
/// #[derive(Copy, Clone, PartialEq, EnumParameter)]
/// pub enum FilterType {
///     #[name = "Low Pass"]
///     LowPass,
///     #[default]
///     #[name = "High Pass"]
///     HighPass,
///     BandPass,  // Uses "BandPass" as display name
/// }
/// ```
pub trait EnumParameterValue: Copy + Clone + PartialEq + Send + Sync + 'static {
    /// Number of variants in the enum.
    const COUNT: usize;

    /// Index of the default variant (from `#[default]` or first variant).
    const DEFAULT_INDEX: usize;

    /// Convert variant index (0-based) to enum value.
    fn from_index(index: usize) -> Option<Self>;

    /// Convert enum value to variant index.
    fn to_index(self) -> usize;

    /// Get the default enum value (from `#[default]` or first variant).
    fn default_value() -> Self;

    /// Get display name for a variant index.
    fn name(index: usize) -> &'static str;

    /// Get all variant names in order.
    fn names() -> &'static [&'static str];
}

// =============================================================================
// EnumParameter - Enum parameter with atomic storage
// =============================================================================

/// Enum parameter for discrete choices (filter types, waveforms, etc.).
///
/// # Example
///
/// ```ignore
/// use beamer::prelude::*;
/// use beamer::EnumParameter;
///
/// #[derive(Copy, Clone, PartialEq, EnumParameter)]
/// pub enum FilterType {
///     #[name = "Low Pass"]
///     LowPass,
///     #[default]
///     #[name = "High Pass"]
///     HighPass,
/// }
///
/// #[derive(Parameters)]
/// pub struct FilterParameters {
///     #[parameter(id = "filter_type")]
///     pub filter_type: EnumParameter<FilterType>,
/// }
///
/// impl Default for FilterParameters {
///     fn default() -> Self {
///         Self {
///             // Uses HighPass as default (from #[default] attribute)
///             filter_type: EnumParameter::new("Filter Type"),
///         }
///     }
/// }
///
/// // In DSP code:
/// fn process(&self) {
///     match self.parameters.filter_type.get() {
///         FilterType::LowPass => { /* ... */ }
///         FilterType::HighPass => { /* ... */ }
///     }
/// }
/// ```
pub struct EnumParameter<E: EnumParameterValue> {
    /// Parameter metadata (id, name, units, flags, etc.)
    info: ParameterInfo,
    /// Atomic storage for the variant index
    value: std::sync::atomic::AtomicUsize,
    /// Phantom data for the enum type
    _marker: std::marker::PhantomData<E>,
}

impl<E: EnumParameterValue> EnumParameter<E> {
    /// Create a new enum parameter using the trait's default value.
    ///
    /// The default value is determined by the `#[default]` attribute on the enum,
    /// or the first variant if no default is specified.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Parameters)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    ///
    /// # Example
    ///
    /// ```ignore
    /// let filter_type = EnumParameter::new("Filter Type")
    ///     .with_id(hash);
    /// ```
    pub fn new(name: &'static str) -> Self {
        Self::with_value(name, E::default_value())
    }

    /// Create a new enum parameter with an explicit default value.
    ///
    /// Use this when you want to override the `#[default]` attribute.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default enum value
    ///
    /// # Example
    ///
    /// ```ignore
    /// let filter_type = EnumParameter::with_value("Filter Type", FilterType::LowPass)
    ///     .with_id(hash);
    /// ```
    pub fn with_value(name: &'static str, default: E) -> Self {
        let default_index = default.to_index();
        let default_normalized = index_to_normalized(default_index, E::COUNT);

        Self {
            info: ParameterInfo {
                id: 0,
                name,
                short_name: name,
                units: "",
                unit: ParameterUnit::Indexed,
                default_normalized,
                step_count: (E::COUNT.saturating_sub(1)) as i32,
                // EnumParameter is always a list (dropdown), even with only 2 choices
                flags: ParameterFlags {
                    is_list: true,
                    ..ParameterFlags::default()
                },
                group_id: ROOT_GROUP_ID,
            },
            value: std::sync::atomic::AtomicUsize::new(default_index),
            _marker: std::marker::PhantomData,
        }
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// This is typically called by the `#[derive(Parameters)]` macro to assign
    /// the FNV-1a hash of the string ID.
    pub fn with_id(mut self, id: ParameterId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the group ID (parameter group) for this parameter.
    ///
    /// Used by the `#[derive(Parameters)]` macro to assign parameters to groups.
    pub fn with_group(mut self, group_id: GroupId) -> Self {
        self.info.group_id = group_id;
        self
    }

    /// Set the group ID in-place (for runtime assignment by parent structs).
    pub fn set_group_id(&mut self, group_id: GroupId) {
        self.info.group_id = group_id;
    }

    /// Make the parameter read-only.
    pub fn readonly(mut self) -> Self {
        self.info.flags.is_readonly = true;
        self.info.flags.can_automate = false;
        self
    }

    /// Disable automation for this parameter.
    pub fn non_automatable(mut self) -> Self {
        self.info.flags.can_automate = false;
        self
    }

    /// Set the unit type hint for AU hosts.
    ///
    /// EnumParameter defaults to `Indexed` which renders as a dropdown.
    /// This can be overridden if needed.
    pub fn with_unit(mut self, unit: ParameterUnit) -> Self {
        self.info.unit = unit;
        self
    }

    /// Get the parameter metadata.
    pub fn info(&self) -> &ParameterInfo {
        &self.info
    }

    /// Get mutable access to the parameter metadata.
    ///
    /// Used for runtime modification of parameter properties like group_id.
    pub fn info_mut(&mut self) -> &mut ParameterInfo {
        &mut self.info
    }

    // === Value access ===

    /// Get the current enum value.
    ///
    /// If the stored index is invalid (e.g., due to corrupted state),
    /// returns the first variant as a fallback.
    #[inline]
    pub fn get(&self) -> E {
        let index = self.value.load(Ordering::Relaxed);
        // Defensive: if index is somehow out of bounds, fall back to first variant
        E::from_index(index).unwrap_or_else(|| {
            E::from_index(0).expect("enum must have at least one variant")
        })
    }

    /// Set the enum value.
    #[inline]
    pub fn set(&self, value: E) {
        self.value.store(value.to_index(), Ordering::Relaxed);
    }

    // === Smoothing compatibility (no-ops for EnumParameter) ===

    /// No-op for compatibility with the `#[derive(Parameters)]` macro.
    ///
    /// Enum parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn set_sample_rate(&mut self, _sample_rate: f64) {
        // No-op: EnumParameter doesn't support smoothing
    }

    /// No-op for compatibility with the `#[derive(Parameters)]` macro.
    ///
    /// Enum parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn reset_smoothing(&mut self) {
        // No-op: EnumParameter doesn't support smoothing
    }
}

impl<E: EnumParameterValue> ParameterRef for EnumParameter<E> {
    fn id(&self) -> ParameterId {
        self.info.id
    }

    fn name(&self) -> &'static str {
        self.info.name
    }

    fn short_name(&self) -> &'static str {
        self.info.short_name
    }

    fn units(&self) -> &'static str {
        self.info.units
    }

    fn flags(&self) -> &ParameterFlags {
        &self.info.flags
    }

    fn default_normalized(&self) -> ParameterValue {
        self.info.default_normalized
    }

    fn step_count(&self) -> i32 {
        self.info.step_count
    }

    fn get_normalized(&self) -> ParameterValue {
        let index = self.value.load(Ordering::Relaxed);
        index_to_normalized(index, E::COUNT)
    }

    fn set_normalized(&self, value: ParameterValue) {
        let index = normalized_to_index(value, E::COUNT);
        self.value.store(index, Ordering::Relaxed);
    }

    fn get_plain(&self) -> ParameterValue {
        self.value.load(Ordering::Relaxed) as f64
    }

    fn set_plain(&self, value: ParameterValue) {
        let index = (value.round() as usize).min(E::COUNT.saturating_sub(1));
        self.value.store(index, Ordering::Relaxed);
    }

    fn display_normalized(&self, normalized: ParameterValue) -> String {
        let index = normalized_to_index(normalized, E::COUNT);
        E::name(index).to_string()
    }

    fn parse(&self, s: &str) -> Option<ParameterValue> {
        // Try to match variant name (case-insensitive)
        let s_lower = s.to_lowercase();
        for (i, name) in E::names().iter().enumerate() {
            if name.to_lowercase() == s_lower {
                return Some(self.plain_to_normalized(i as f64));
            }
        }
        // Also try parsing as index
        s.parse::<usize>()
            .ok()
            .filter(|&i| i < E::COUNT)
            .map(|i| self.plain_to_normalized(i as f64))
    }

    fn normalized_to_plain(&self, normalized: ParameterValue) -> ParameterValue {
        normalized_to_index(normalized, E::COUNT) as f64
    }

    fn plain_to_normalized(&self, plain: ParameterValue) -> ParameterValue {
        index_to_normalized(plain.round() as usize, E::COUNT)
    }

    fn info(&self) -> &ParameterInfo {
        &self.info
    }
}

// EnumParameter<E> is Send + Sync because:
// - AtomicUsize is Send + Sync
// - PhantomData<E> is Send + Sync when E: Send + Sync (required by EnumParameterValue trait bounds)
// - ParameterInfo is Send + Sync
// No unsafe impl needed - the compiler verifies this automatically.

// =============================================================================
// Helper functions
// =============================================================================

// --- Enum normalization helpers ---

/// Convert an enum variant index to a normalized value [0.0, 1.0].
///
/// For enums with N variants, index 0 maps to 0.0 and index N-1 maps to 1.0.
/// Single-variant enums always return 0.0.
#[inline]
fn index_to_normalized(index: usize, count: usize) -> f64 {
    if count <= 1 {
        0.0
    } else {
        index as f64 / (count - 1) as f64
    }
}

/// Convert a normalized value [0.0, 1.0] to an enum variant index.
///
/// The result is clamped to [0, count-1]. Rounds to nearest index.
#[inline]
fn normalized_to_index(normalized: f64, count: usize) -> usize {
    if count <= 1 {
        0
    } else {
        ((normalized * (count - 1) as f64).round() as usize).min(count - 1)
    }
}

// --- Other helpers ---

/// Convert decibels to linear amplitude.
#[inline]
fn db_to_linear(db: f64) -> f64 {
    if db <= -100.0 {
        0.0
    } else {
        10.0_f64.powf(db / 20.0)
    }
}

/// Snap a value to the nearest step within a range.
#[inline]
fn snap_to_step(value: f64, step_size: f64, min: f64, max: f64) -> f64 {
    // Calculate the number of steps from min
    let steps_from_min = ((value - min) / step_size).round();
    // Calculate snapped value
    let snapped = min + steps_from_min * step_size;
    // Clamp to range (handles edge cases from rounding)
    snapped.clamp(min, max)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_size_snapping() {
        let param = FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(0.5);

        // Test snapping to nearest step
        param.set(2.3);
        assert!((param.get() - 2.5).abs() < 1e-10);

        param.set(2.2);
        assert!((param.get() - 2.0).abs() < 1e-10);

        param.set(2.25);
        assert!((param.get() - 2.5).abs() < 1e-10); // Round up at midpoint
    }

    #[test]
    fn test_step_size_edge_cases() {
        let param = FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(0.3);

        // Snap at boundaries
        param.set(-0.1);
        assert!((param.get() - 0.0).abs() < 1e-10); // Clamp to min

        param.set(10.1);
        assert!((param.get() - 10.0).abs() < 1e-10); // Clamp to max
    }

    #[test]
    fn test_step_count_calculation() {
        let param = FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(0.5);

        // 10.0 / 0.5 = 20 steps, meaning 21 discrete values
        assert_eq!(param.step_count(), 20);
    }

    #[test]
    fn test_step_size_with_negative_range() {
        let param = FloatParameter::db("Gain", 0.0, -60.0..=12.0).with_step_size(0.5);

        // Range is 72, so 144 steps
        assert_eq!(param.step_count(), 144);

        param.set(-5.3);
        assert!((param.get() - -5.5).abs() < 1e-10);
    }

    #[test]
    fn test_continuous_parameter_no_snapping() {
        let param = FloatParameter::new("Test", 0.0, 0.0..=10.0);

        param.set(2.3);
        assert!((param.get() - 2.3).abs() < 1e-10); // No snapping
        assert_eq!(param.step_count(), 0); // Continuous
    }

    #[test]
    #[should_panic(expected = "step_size must be positive")]
    fn test_step_size_zero_panics() {
        FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(0.0);
    }

    #[test]
    #[should_panic(expected = "step_size must be positive")]
    fn test_step_size_negative_panics() {
        FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(-0.5);
    }

    #[test]
    fn test_step_size_larger_than_range() {
        let param = FloatParameter::new("Test", 0.0, 0.0..=1.0).with_step_size(2.0);

        // Step larger than range = step_count of 1 (two values: min and max)
        assert_eq!(param.step_count(), 1);
    }

    #[test]
    fn test_step_size_getter() {
        let param_with_step = FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(0.5);
        assert_eq!(param_with_step.step_size(), Some(0.5));

        let param_continuous = FloatParameter::new("Test", 0.0, 0.0..=10.0);
        assert_eq!(param_continuous.step_size(), None);
    }

    #[test]
    fn test_step_size_with_smoother() {
        let mut param = FloatParameter::new("Test", 0.0, 0.0..=10.0)
            .with_step_size(1.0)
            .with_smoother(crate::smoothing::SmoothingStyle::Linear(10.0));

        param.set_sample_rate(1000.0);
        param.set(5.3); // Snaps to 5.0

        // Target should be the snapped value
        assert!((param.get() - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_snap_to_step_helper() {
        // Basic snapping
        assert!((snap_to_step(2.3, 0.5, 0.0, 10.0) - 2.5).abs() < 1e-10);
        assert!((snap_to_step(2.2, 0.5, 0.0, 10.0) - 2.0).abs() < 1e-10);

        // Negative range
        assert!((snap_to_step(-5.3, 0.5, -60.0, 12.0) - -5.5).abs() < 1e-10);

        // Clamping
        assert!((snap_to_step(-1.0, 0.5, 0.0, 10.0) - 0.0).abs() < 1e-10);
        assert!((snap_to_step(11.0, 0.5, 0.0, 10.0) - 10.0).abs() < 1e-10);
    }

    // =========================================================================
    // FloatParameter precision and formatter tests
    // =========================================================================

    #[test]
    fn test_float_parameter_with_precision_db() {
        let param = FloatParameter::db("Gain", 0.0, -60.0..=12.0).with_precision(2);

        // Check precision was updated
        assert_eq!(param.formatter().precision(), Some(2));

        // Check display uses new precision
        param.set(-6.5);
        let display = param.display();
        assert_eq!(display, "-6.50");
    }

    #[test]
    fn test_float_parameter_with_precision_ms() {
        let param = FloatParameter::ms("Attack", 10.0, 0.1..=100.0).with_precision(0);

        assert_eq!(param.formatter().precision(), Some(0));

        param.set(10.5);
        let display = param.display();
        assert_eq!(display, "10"); // Rounded to 0 decimal places
    }

    #[test]
    fn test_float_parameter_with_precision_percent() {
        let param = FloatParameter::percent("Mix", 0.5).with_precision(1);

        assert_eq!(param.formatter().precision(), Some(1));

        param.set(0.755);
        let display = param.display();
        assert_eq!(display, "75.5");
    }

    #[test]
    fn test_float_parameter_with_precision_pan_no_effect() {
        let param = FloatParameter::pan("Pan", 0.0).with_precision(3);

        // Pan formatter doesn't support precision, should remain unchanged
        assert_eq!(param.formatter().precision(), None);
        assert!(!param.formatter().supports_precision());
    }

    #[test]
    fn test_float_parameter_with_formatter_replace() {
        let param = FloatParameter::new("Ratio", 4.0, 1.0..=20.0)
            .with_formatter(Formatter::Ratio { precision: 1 });

        assert_eq!(param.info().units, ""); // Ratio uses empty unit (suffix is ":1")
        assert_eq!(param.formatter().precision(), Some(1));

        param.set(4.0);
        let display = param.display();
        assert_eq!(display, "4.0:1");
    }

    #[test]
    fn test_float_parameter_with_formatter_updates_units() {
        // Start with a generic float parameter (no units)
        let param =
            FloatParameter::new("Time", 100.0, 1.0..=1000.0).with_formatter(Formatter::Milliseconds {
                precision: 1,
            });

        // Units should be updated to match the new formatter
        assert_eq!(param.info().units, "ms");
    }

    #[test]
    fn test_float_parameter_with_formatter_preserves_is_db() {
        // Create a dB parameter, then change formatter
        let param = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
            .with_formatter(Formatter::Float { precision: 2 });

        // Even though we changed the formatter, as_linear() should still work
        // because is_db is based on how the parameter was constructed
        param.set(0.0);
        assert!((param.as_linear() - 1.0).abs() < 0.001); // 0 dB = 1.0 linear
    }

    #[test]
    fn test_float_parameter_formatter_getter() {
        let param = FloatParameter::db("Gain", 0.0, -60.0..=12.0);

        // Check we can access the formatter
        let formatter = param.formatter();
        assert!(formatter.supports_precision());
        assert_eq!(formatter.unit(), "dB");
    }

    #[test]
    fn test_float_parameter_chained_precision_and_step_size() {
        let param = FloatParameter::db("Volume", 0.0, -60.0..=12.0)
            .with_step_size(0.5)
            .with_precision(2);

        // Both should work together
        assert_eq!(param.step_count(), 144); // (12 - (-60)) / 0.5 = 144
        assert_eq!(param.formatter().precision(), Some(2));

        param.set(-5.3);
        assert!((param.get() - -5.5).abs() < 1e-10); // Snapped
        let display = param.display();
        assert_eq!(display, "-5.50"); // High precision
    }

    // =========================================================================
    // IntParameter precision and formatter tests
    // =========================================================================

    #[test]
    fn test_int_parameter_with_precision() {
        // IntParameter uses Float { precision: 0 } by default
        let param = IntParameter::new("Value", 0, -100..=100).with_precision(0);

        assert_eq!(param.formatter().precision(), Some(0));
    }

    #[test]
    fn test_int_parameter_with_formatter() {
        let param =
            IntParameter::new("Transpose", 0, -24..=24).with_formatter(Formatter::Semitones);

        assert_eq!(param.info().units, "st");

        param.set(12);
        let display = param.display_normalized(param.get_normalized());
        assert_eq!(display, "+12");

        param.set(-7);
        let display = param.display_normalized(param.get_normalized());
        assert_eq!(display, "-7");
    }

    #[test]
    fn test_int_parameter_formatter_getter() {
        let param = IntParameter::semitones("Pitch", 0, -24..=24);

        let formatter = param.formatter();
        assert_eq!(formatter.unit(), "st");
        assert!(!formatter.supports_precision()); // Semitones doesn't have precision
    }

    // =========================================================================
    // ParameterUnit tests
    // =========================================================================

    #[test]
    fn test_float_parameter_unit_generic() {
        let param = FloatParameter::new("Test", 0.0, 0.0..=1.0);
        assert_eq!(param.info().unit, ParameterUnit::Generic);
    }

    #[test]
    fn test_float_parameter_unit_decibels() {
        let param = FloatParameter::db("Gain", 0.0, -60.0..=12.0);
        assert_eq!(param.info().unit, ParameterUnit::Decibels);
    }

    #[test]
    fn test_float_parameter_unit_db_log() {
        let param = FloatParameter::db_log("Threshold", -20.0, -60.0..=0.0);
        assert_eq!(param.info().unit, ParameterUnit::Decibels);
    }

    #[test]
    fn test_float_parameter_unit_db_log_offset() {
        let param = FloatParameter::db_log_offset("Threshold", -20.0, -60.0..=0.0);
        assert_eq!(param.info().unit, ParameterUnit::Decibels);
    }

    #[test]
    fn test_float_parameter_unit_hertz() {
        let param = FloatParameter::hz("Frequency", 440.0, 20.0..=20000.0);
        assert_eq!(param.info().unit, ParameterUnit::Hertz);
    }

    #[test]
    fn test_float_parameter_unit_milliseconds() {
        let param = FloatParameter::ms("Attack", 10.0, 0.1..=100.0);
        assert_eq!(param.info().unit, ParameterUnit::Milliseconds);
    }

    #[test]
    fn test_float_parameter_unit_seconds() {
        let param = FloatParameter::seconds("Decay", 1.0, 0.0..=10.0);
        assert_eq!(param.info().unit, ParameterUnit::Seconds);
    }

    #[test]
    fn test_float_parameter_unit_percent() {
        let param = FloatParameter::percent("Mix", 0.5);
        assert_eq!(param.info().unit, ParameterUnit::Percent);
    }

    #[test]
    fn test_float_parameter_unit_pan() {
        let param = FloatParameter::pan("Pan", 0.0);
        assert_eq!(param.info().unit, ParameterUnit::Pan);
    }

    #[test]
    fn test_float_parameter_unit_ratio() {
        let param = FloatParameter::ratio("Ratio", 4.0, 1.0..=20.0);
        assert_eq!(param.info().unit, ParameterUnit::Ratio);
    }

    #[test]
    fn test_int_parameter_unit_generic() {
        let param = IntParameter::new("Value", 0, -100..=100);
        assert_eq!(param.info().unit, ParameterUnit::Generic);
    }

    #[test]
    fn test_int_parameter_unit_semitones() {
        let param = IntParameter::semitones("Transpose", 0, -24..=24);
        assert_eq!(param.info().unit, ParameterUnit::RelativeSemiTones);
    }

    #[test]
    fn test_bool_parameter_unit_boolean() {
        let param = BoolParameter::new("Enabled", true);
        assert_eq!(param.info().unit, ParameterUnit::Boolean);
    }

    #[test]
    fn test_bool_parameter_bypass_unit_boolean() {
        let param = BoolParameter::bypass();
        assert_eq!(param.info().unit, ParameterUnit::Boolean);
    }

    #[test]
    fn test_float_parameter_with_unit_override() {
        let param = FloatParameter::new("Custom", 0.5, 0.0..=1.0)
            .with_unit(ParameterUnit::Percent);
        assert_eq!(param.info().unit, ParameterUnit::Percent);
    }

    #[test]
    fn test_enum_parameter_unit_indexed() {
        #[derive(Debug, Clone, Copy, Default, PartialEq)]
        enum TestEnum {
            #[default]
            A,
            B,
            C,
        }

        impl EnumParameterValue for TestEnum {
            const COUNT: usize = 3;
            const DEFAULT_INDEX: usize = 0;

            fn from_index(index: usize) -> Option<Self> {
                match index {
                    0 => Some(Self::A),
                    1 => Some(Self::B),
                    2 => Some(Self::C),
                    _ => None,
                }
            }

            fn to_index(self) -> usize {
                match self {
                    Self::A => 0,
                    Self::B => 1,
                    Self::C => 2,
                }
            }

            fn name(index: usize) -> &'static str {
                match index {
                    0 => "A",
                    1 => "B",
                    2 => "C",
                    _ => "",
                }
            }

            fn names() -> &'static [&'static str] {
                &["A", "B", "C"]
            }

            fn default_value() -> Self {
                Self::default()
            }
        }

        let param = EnumParameter::<TestEnum>::new("Mode");
        assert_eq!(param.info().unit, ParameterUnit::Indexed);
    }

    #[test]
    fn test_parameter_unit_repr_values() {
        // Verify the repr(u32) values match Apple's AudioUnitParameterUnit enum
        assert_eq!(ParameterUnit::Generic as u32, 0);
        assert_eq!(ParameterUnit::Indexed as u32, 1);
        assert_eq!(ParameterUnit::Boolean as u32, 2);
        assert_eq!(ParameterUnit::Percent as u32, 3);
        assert_eq!(ParameterUnit::Seconds as u32, 4);
        assert_eq!(ParameterUnit::Hertz as u32, 8);
        assert_eq!(ParameterUnit::RelativeSemiTones as u32, 10);
        assert_eq!(ParameterUnit::Decibels as u32, 13);
        assert_eq!(ParameterUnit::Pan as u32, 18);
        assert_eq!(ParameterUnit::Milliseconds as u32, 24);
        assert_eq!(ParameterUnit::Ratio as u32, 25);
        assert_eq!(ParameterUnit::CustomUnit as u32, 26);
    }
}
