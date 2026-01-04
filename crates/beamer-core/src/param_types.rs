//! New parameter types with encapsulated atomic storage.
//!
//! This module provides high-level parameter types that encapsulate atomic storage,
//! range mapping, and value formatting. Each type implements the [`ParamRef`] trait
//! for type-erased runtime access.
//!
//! # Example
//!
//! ```ignore
//! use beamer_core::param_types::{FloatParam, IntParam, BoolParam, Params, ParamRef};
//!
//! struct MyParams {
//!     gain: FloatParam,
//!     attack: FloatParam,
//! }
//!
//! impl MyParams {
//!     fn new() -> Self {
//!         Self {
//!             // IDs are set via with_id() for manual usage, or by #[derive(Params)] macro
//!             gain: FloatParam::db("Gain", 0.0, -60.0..=12.0).with_id(0),
//!             attack: FloatParam::ms("Attack", 10.0, 0.1..=100.0).with_id(1),
//!         }
//!     }
//! }
//! ```

use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use crate::param_format::Formatter;
use crate::param_range::{LinearMapper, LogMapper, RangeMapper};
use crate::params::{ParamFlags, ParamInfo};
use crate::smoothing::{Smoother, SmoothingStyle};
use crate::types::{ParamId, ParamValue};

// =============================================================================
// ParamRef Trait - Type-erased parameter access
// =============================================================================

/// Trait for type-erased parameter access at runtime.
///
/// This allows iteration over heterogeneous parameter collections
/// and runtime lookup without knowing the concrete parameter type.
///
/// All implementations must be thread-safe (`Send + Sync`) for
/// concurrent access from audio, UI, and host threads.
pub trait ParamRef: Send + Sync {
    /// Get the parameter's unique ID.
    fn id(&self) -> ParamId;

    /// Get the parameter's display name.
    fn name(&self) -> &'static str;

    /// Get the parameter's short name for constrained UIs.
    fn short_name(&self) -> &'static str;

    /// Get the parameter's unit string (e.g., "dB", "Hz", "ms").
    fn units(&self) -> &'static str;

    /// Get the parameter flags.
    fn flags(&self) -> &ParamFlags;

    /// Get the default normalized value.
    fn default_normalized(&self) -> ParamValue;

    /// Get the step count (0 = continuous, 1 = toggle, >1 = discrete).
    fn step_count(&self) -> i32;

    /// Get the current normalized value (0.0-1.0).
    ///
    /// This is lock-free and safe to call from the audio thread.
    fn get_normalized(&self) -> ParamValue;

    /// Set the normalized value (0.0-1.0).
    ///
    /// This is lock-free and safe to call from any thread.
    /// Values are clamped to [0.0, 1.0].
    fn set_normalized(&self, value: ParamValue);

    /// Get the current plain value in natural units.
    fn get_plain(&self) -> ParamValue;

    /// Set the plain value in natural units.
    fn set_plain(&self, value: ParamValue);

    /// Format the current value for display.
    fn display(&self) -> String {
        self.display_normalized(self.get_normalized())
    }

    /// Format a normalized value for display.
    fn display_normalized(&self, normalized: ParamValue) -> String;

    /// Parse a display string to a normalized value.
    ///
    /// Returns `None` if parsing fails.
    fn parse(&self, s: &str) -> Option<ParamValue>;

    /// Convert a normalized value to a plain value.
    fn normalized_to_plain(&self, normalized: ParamValue) -> ParamValue;

    /// Convert a plain value to a normalized value.
    fn plain_to_normalized(&self, plain: ParamValue) -> ParamValue;

    /// Get the full ParamInfo for this parameter.
    ///
    /// This is used by the `#[derive(Params)]` macro to generate the
    /// `Parameters::info()` implementation.
    fn info(&self) -> &ParamInfo;
}

// =============================================================================
// Params Trait - Parameter collection
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
/// use beamer_core::param_types::{FloatParam, Params, ParamRef};
///
/// struct MyParams {
///     gain: FloatParam,
/// }
///
/// impl Params for MyParams {
///     fn count(&self) -> usize { 1 }
///
///     fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParamRef> + '_> {
///         Box::new(std::iter::once(&self.gain as &dyn ParamRef))
///     }
///
///     fn by_id(&self, id: u32) -> Option<&dyn ParamRef> {
///         match id {
///             0 => Some(&self.gain),
///             _ => None,
///         }
///     }
/// }
/// ```
pub trait Params: Send + Sync + crate::params::Units {
    /// Returns the total number of parameters.
    fn count(&self) -> usize;

    /// Iterate over all parameters (type-erased).
    fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParamRef> + '_>;

    /// Get a parameter by its ID.
    fn by_id(&self, id: ParamId) -> Option<&dyn ParamRef>;

    /// Get a mutable reference to a parameter by its ID.
    ///
    /// Note: This returns `&dyn ParamRef` (not `&mut`) because atomic
    /// parameters can be modified through shared references.
    fn by_id_mut(&mut self, id: ParamId) -> Option<&dyn ParamRef> {
        self.by_id(id)
    }

    /// Set unit ID for all direct parameters in this collection.
    ///
    /// Called by parent structs when initializing nested parameter groups.
    /// The default implementation does nothing (for flat parameter structs).
    fn set_all_unit_ids(&mut self, _unit_id: crate::params::UnitId) {
        // Default: no-op (macro generates override for param-containing structs)
    }

    // =========================================================================
    // Nested Group Discovery (for recursive unit ID assignment)
    // =========================================================================

    /// Number of direct nested parameter groups in this struct.
    ///
    /// Default is 0 (no nested groups). The `#[derive(Params)]` macro
    /// generates an override for structs with `#[nested]` fields.
    fn nested_count(&self) -> usize {
        0
    }

    /// Get information about a nested group by index.
    ///
    /// Returns the group name and a reference to the nested Params.
    /// Default returns None (no nested groups).
    fn nested_group(&self, _index: usize) -> Option<(&'static str, &dyn Params)> {
        None
    }

    /// Get mutable access to a nested group by index.
    ///
    /// Returns the group name and a mutable reference to the nested Params.
    /// Default returns None (no nested groups).
    fn nested_group_mut(&mut self, _index: usize) -> Option<(&'static str, &mut dyn Params)> {
        None
    }

    /// Recursively assign unit IDs to all nested groups.
    ///
    /// This method traverses the nested group hierarchy and assigns
    /// sequential unit IDs, properly setting parent relationships for
    /// deeply nested groups.
    ///
    /// # Arguments
    ///
    /// * `start_id` - The first unit ID to assign (typically 1, since 0 is root)
    /// * `parent_id` - The parent unit ID for this level's nested groups
    ///
    /// # Returns
    ///
    /// The next available unit ID after all assignments.
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Called by set_unit_ids() on the top-level struct:
    /// let next_id = self.assign_unit_ids(1, 0);
    /// ```
    fn assign_unit_ids(&mut self, start_id: i32, _parent_id: i32) -> i32 {
        let mut next_id = start_id;

        for i in 0..self.nested_count() {
            if let Some((_, nested)) = self.nested_group_mut(i) {
                let unit_id = next_id;
                next_id += 1;

                // Set unit ID on all direct params in this nested group
                nested.set_all_unit_ids(unit_id);

                // Recurse into this nested group's nested groups
                // The current unit_id becomes the parent for nested groups
                next_id = nested.assign_unit_ids(next_id, unit_id);
            }
        }

        next_id
    }

    /// Collect all unit infos from nested groups recursively.
    ///
    /// This is used by the `Units` trait implementation to build the
    /// complete list of units for the DAW.
    ///
    /// # Arguments
    ///
    /// * `units` - Vector to append UnitInfo entries to
    /// * `start_id` - The first unit ID for this level
    /// * `parent_id` - The parent unit ID for this level's groups
    ///
    /// # Returns
    ///
    /// The next available unit ID after all units are collected.
    fn collect_units(
        &self,
        units: &mut Vec<crate::params::UnitInfo>,
        start_id: i32,
        parent_id: i32,
    ) -> i32 {
        let mut next_id = start_id;

        for i in 0..self.nested_count() {
            if let Some((name, nested)) = self.nested_group(i) {
                let unit_id = next_id;
                next_id += 1;

                units.push(crate::params::UnitInfo::new(unit_id, name, parent_id));

                // Recurse into nested groups
                next_id = nested.collect_units(units, next_id, unit_id);
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
        for param in self.iter() {
            // For default impl, use numeric ID as string
            let id_str = param.id().to_string();
            let path = if prefix.is_empty() {
                id_str
            } else {
                format!("{}/{}", prefix, id_str)
            };

            let path_bytes = path.as_bytes();
            data.push(path_bytes.len() as u8);
            data.extend_from_slice(path_bytes);
            data.extend_from_slice(&param.get_normalized().to_le_bytes());
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
            if let Some(param) = self.by_id_mut(id) {
                param.set_normalized(value.clamp(0.0, 1.0));
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
                if let Some(param) = self.by_id_mut(id) {
                    param.set_normalized(value.clamp(0.0, 1.0));
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
    /// Call this from `AudioProcessor::setup()` to initialize smoothers
    /// with the correct sample rate.
    ///
    /// **Oversampling:** If your plugin uses oversampling, pass the actual
    /// processing rate: `sample_rate * oversampling_factor`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// impl AudioProcessor for MyPlugin {
    ///     fn setup(&mut self, sample_rate: f64, _max_buffer_size: usize) {
    ///         self.params.set_sample_rate(sample_rate);
    ///     }
    /// }
    /// ```
    fn set_sample_rate(&mut self, _sample_rate: f64) {
        // Default no-op. The #[derive(Params)] macro generates an override
        // that calls set_sample_rate on each param field.
    }

    /// Reset all smoothers to their current values (no ramp).
    ///
    /// Called automatically by the framework after loading state to avoid
    /// ramps to loaded values. You typically don't need to call this directly.
    fn reset_smoothing(&mut self) {
        // Default no-op. The #[derive(Params)] macro generates an override
        // that calls reset_smoothing on each param field.
    }
}

// =============================================================================
// FloatParam - Float parameter with atomic storage
// =============================================================================

/// Float parameter with atomic storage and automatic formatting.
///
/// # Specialized Constructors
///
/// - [`FloatParam::new`]: Generic float parameter
/// - [`FloatParam::db`]: Decibel parameter with dB formatting
/// - [`FloatParam::hz`]: Frequency parameter with logarithmic mapping
/// - [`FloatParam::ms`]: Milliseconds parameter
/// - [`FloatParam::seconds`]: Seconds parameter
/// - [`FloatParam::percent`]: Percentage parameter (0-100%)
/// - [`FloatParam::pan`]: Pan parameter (L-C-R)
/// - [`FloatParam::ratio`]: Compressor ratio parameter
///
/// # Example
///
/// ```ignore
/// // Create parameter - ID is set separately via with_id() or #[derive(Params)]
/// let gain = FloatParam::db("Gain", 0.0, -60.0..=12.0).with_id(0);
/// let freq = FloatParam::hz("Frequency", 440.0, 20.0..=20000.0).with_id(1);
///
/// // Read/write plain values
/// let current_gain = gain.get(); // Returns linear value
/// freq.set(1000.0); // Set to 1000 Hz
///
/// // For DSP: get linear amplitude
/// let amplitude = gain.as_linear();
/// ```
pub struct FloatParam {
    /// Parameter metadata (id, name, units, flags, etc.)
    info: ParamInfo,
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
}

impl FloatParam {
    /// Create a generic float parameter with linear mapping.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
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
            info: ParamInfo {
                id: 0, // Set via with_id() or macro
                name,
                short_name: name,
                units: "",
                default_normalized,
                step_count: 0,
                flags: ParamFlags::default(),
                unit_id: crate::params::ROOT_UNIT_ID,
            },
            value: AtomicU64::new(default_normalized.to_bits()),
            range: Box::new(mapper),
            formatter: Formatter::Float { precision: 2 },
            smoother: None,
            is_db: false,
        }
    }

    /// Create a decibel parameter.
    ///
    /// The parameter value is stored in **dB** internally. Use [`as_linear`](Self::as_linear)
    /// to get the linear amplitude for DSP processing.
    ///
    /// - [`get`](Self::get) returns the dB value (for display, host automation)
    /// - [`as_linear`](Self::as_linear) returns linear amplitude (for DSP)
    /// - [`normalized_to_plain`](ParamRef::normalized_to_plain) returns dB (matches `units`)
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
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
    /// let gain = FloatParam::db("Gain", 0.0, -60.0..=12.0).with_id(0);
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

        Self {
            info: ParamInfo {
                id: 0,
                name,
                short_name: name,
                units: "dB",
                default_normalized,
                step_count: 0,
                flags: ParamFlags::default(),
                unit_id: crate::params::ROOT_UNIT_ID,
            },
            value: AtomicU64::new(default_normalized.to_bits()),
            range: Box::new(mapper),
            formatter: Formatter::DecibelDirect { precision: 1, min_db },
            smoother: None,
            is_db: true,
        }
    }

    /// Create a frequency parameter with logarithmic mapping.
    ///
    /// Logarithmic mapping provides a perceptually uniform distribution
    /// across the frequency range.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
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
    /// let freq = FloatParam::hz("Frequency", 440.0, 20.0..=20000.0).with_id(0);
    /// ```
    pub fn hz(name: &'static str, default_hz: f64, range_hz: RangeInclusive<f64>) -> Self {
        let mapper = LogMapper::new(range_hz.clone());
        let default_normalized = mapper.normalize(default_hz);

        Self {
            info: ParamInfo {
                id: 0,
                name,
                short_name: name,
                units: "Hz",
                default_normalized,
                step_count: 0,
                flags: ParamFlags::default(),
                unit_id: crate::params::ROOT_UNIT_ID,
            },
            value: AtomicU64::new(default_normalized.to_bits()),
            range: Box::new(mapper),
            formatter: Formatter::Frequency,
            smoother: None,
            is_db: false,
        }
    }

    /// Create a milliseconds parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_ms` - Default value in milliseconds
    /// * `range_ms` - Valid range in milliseconds (inclusive)
    pub fn ms(name: &'static str, default_ms: f64, range_ms: RangeInclusive<f64>) -> Self {
        let mut param = Self::new(name, default_ms, range_ms);
        param.info.units = "ms";
        param.formatter = Formatter::Milliseconds { precision: 1 };
        param
    }

    /// Create a seconds parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_s` - Default value in seconds
    /// * `range_s` - Valid range in seconds (inclusive)
    pub fn seconds(name: &'static str, default_s: f64, range_s: RangeInclusive<f64>) -> Self {
        let mut param = Self::new(name, default_s, range_s);
        param.info.units = "s";
        param.formatter = Formatter::Seconds { precision: 2 };
        param
    }

    /// Create a percentage parameter.
    ///
    /// The value is stored as 0.0-1.0 internally but displayed as 0%-100%.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_pct` - Default value as 0.0-1.0 (not 0-100)
    pub fn percent(name: &'static str, default_pct: f64) -> Self {
        let mut param = Self::new(name, default_pct, 0.0..=1.0);
        param.info.units = "%";
        param.formatter = Formatter::Percent { precision: 0 };
        param
    }

    /// Create a pan parameter.
    ///
    /// Range is -1.0 (full left) to +1.0 (full right), with 0.0 being center.
    /// Display: "L50", "C", "R50"
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value (-1.0 to +1.0, typically 0.0)
    pub fn pan(name: &'static str, default: f64) -> Self {
        let mut param = Self::new(name, default, -1.0..=1.0);
        param.formatter = Formatter::Pan;
        param
    }

    /// Create a ratio parameter for compressors.
    ///
    /// Display: "4.0:1", "∞:1"
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default ratio value
    /// * `range` - Valid ratio range (inclusive)
    pub fn ratio(name: &'static str, default: f64, range: RangeInclusive<f64>) -> Self {
        let mut param = Self::new(name, default, range);
        param.formatter = Formatter::Ratio { precision: 1 };
        param
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// This is typically called by the `#[derive(Params)]` macro to assign
    /// the FNV-1a hash of the string ID. For manual usage, you can pass
    /// any unique u32 value.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let gain = FloatParam::db("Gain", 0.0, -60.0..=12.0).with_id(0x050c5d1f);
    /// ```
    pub fn with_id(mut self, id: ParamId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the unit ID (parameter group) for this parameter.
    ///
    /// Used by the `#[derive(Params)]` macro to assign parameters to VST3 units.
    pub fn with_unit(mut self, unit_id: crate::params::UnitId) -> Self {
        self.info.unit_id = unit_id;
        self
    }

    /// Set the unit ID in-place (for runtime assignment by parent structs).
    pub fn set_unit_id(&mut self, unit_id: crate::params::UnitId) {
        self.info.unit_id = unit_id;
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

    /// Get the parameter metadata.
    pub fn info(&self) -> &ParamInfo {
        &self.info
    }

    /// Get mutable access to the parameter metadata.
    ///
    /// Used for runtime modification of parameter properties like unit_id.
    pub fn info_mut(&mut self) -> &mut ParamInfo {
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
    #[inline]
    pub fn set(&self, value: f64) {
        let normalized = self.range.normalize(value);
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
    /// let gain = FloatParam::db("Gain", 0.0, -60.0..=12.0);
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
    /// let gain = FloatParam::db("Gain", 0.0, -60.0..=12.0)
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
    /// Call this from `AudioProcessor::setup()`. If using oversampling,
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

    /// Get next smoothed value, advancing the smoother.
    ///
    /// Call once per sample in the audio loop. Requires `&mut self`.
    ///
    /// If no smoother is configured, returns the raw value.
    #[inline]
    pub fn next_smoothed(&mut self) -> f64 {
        let current_value = self.get();
        match &mut self.smoother {
            Some(s) => {
                // Update target from atomic value (in case host changed it)
                s.set_target(current_value);
                s.next()
            }
            None => current_value,
        }
    }

    /// Get next smoothed value as f32.
    #[inline]
    pub fn next_smoothed_f32(&mut self) -> f32 {
        self.next_smoothed() as f32
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

impl ParamRef for FloatParam {
    fn id(&self) -> ParamId {
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

    fn flags(&self) -> &ParamFlags {
        &self.info.flags
    }

    fn default_normalized(&self) -> ParamValue {
        self.info.default_normalized
    }

    fn step_count(&self) -> i32 {
        self.info.step_count
    }

    fn get_normalized(&self) -> ParamValue {
        f64::from_bits(self.value.load(Ordering::Relaxed))
    }

    fn set_normalized(&self, value: ParamValue) {
        self.value
            .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    fn get_plain(&self) -> ParamValue {
        self.get()
    }

    fn set_plain(&self, value: ParamValue) {
        self.set(value);
    }

    fn display_normalized(&self, normalized: ParamValue) -> String {
        let plain = self.range.denormalize(normalized);
        self.formatter.format(plain)
    }

    fn parse(&self, s: &str) -> Option<ParamValue> {
        let plain = self.formatter.parse(s)?;
        Some(self.range.normalize(plain))
    }

    fn normalized_to_plain(&self, normalized: ParamValue) -> ParamValue {
        self.range.denormalize(normalized)
    }

    fn plain_to_normalized(&self, plain: ParamValue) -> ParamValue {
        self.range.normalize(plain)
    }

    fn info(&self) -> &ParamInfo {
        &self.info
    }
}

// FloatParam is automatically Send + Sync because:
// - AtomicU64 is Send + Sync
// - Box<dyn RangeMapper> is Send + Sync (RangeMapper: Send + Sync)
// - All other fields (&'static str, f64, Formatter, ParamFlags) are Send + Sync
// No unsafe impl needed - the compiler verifies this automatically.

// =============================================================================
// IntParam - Integer parameter with atomic storage
// =============================================================================

/// Integer parameter with atomic storage.
///
/// # Specialized Constructors
///
/// - [`IntParam::new`]: Generic integer parameter
/// - [`IntParam::semitones`]: Semitones parameter for pitch shifting
///
/// # Example
///
/// ```ignore
/// let octave = IntParam::semitones("Octave", 0, -24..=24).with_id(0);
/// println!("Current: {} semitones", octave.get());
/// ```
pub struct IntParam {
    /// Parameter metadata (id, name, units, flags, etc.)
    info: ParamInfo,
    /// Atomic storage for the integer value
    value: AtomicI64,
    /// Minimum value
    min: i64,
    /// Maximum value
    max: i64,
    /// Formatter for display string conversion
    formatter: Formatter,
}

impl IntParam {
    /// Create a generic integer parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
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
            info: ParamInfo {
                id: 0,
                name,
                short_name: name,
                units: "",
                default_normalized,
                step_count,
                flags: ParamFlags::default(),
                unit_id: crate::params::ROOT_UNIT_ID,
            },
            value: AtomicI64::new(default.clamp(min, max)),
            min,
            max,
            formatter: Formatter::Float { precision: 0 },
        }
    }

    /// Create a semitones parameter for pitch shifting.
    ///
    /// Display: "+12 st", "-7 st", "0 st"
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value in semitones
    /// * `range` - Valid range in semitones (inclusive)
    pub fn semitones(name: &'static str, default: i64, range: RangeInclusive<i64>) -> Self {
        let mut param = Self::new(name, default, range);
        param.info.units = "st";
        param.formatter = Formatter::Semitones;
        param
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// This is typically called by the `#[derive(Params)]` macro to assign
    /// the FNV-1a hash of the string ID.
    pub fn with_id(mut self, id: ParamId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the unit ID (parameter group) for this parameter.
    ///
    /// Used by the `#[derive(Params)]` macro to assign parameters to VST3 units.
    pub fn with_unit(mut self, unit_id: crate::params::UnitId) -> Self {
        self.info.unit_id = unit_id;
        self
    }

    /// Set the unit ID in-place (for runtime assignment by parent structs).
    pub fn set_unit_id(&mut self, unit_id: crate::params::UnitId) {
        self.info.unit_id = unit_id;
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

    /// Get the parameter metadata.
    pub fn info(&self) -> &ParamInfo {
        &self.info
    }

    /// Get mutable access to the parameter metadata.
    ///
    /// Used for runtime modification of parameter properties like unit_id.
    pub fn info_mut(&mut self) -> &mut ParamInfo {
        &mut self.info
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

    // === Smoothing compatibility (no-ops for IntParam) ===

    /// No-op for compatibility with the `#[derive(Params)]` macro.
    ///
    /// Integer parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn set_sample_rate(&mut self, _sample_rate: f64) {
        // No-op: IntParam doesn't support smoothing
    }

    /// No-op for compatibility with the `#[derive(Params)]` macro.
    ///
    /// Integer parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn reset_smoothing(&mut self) {
        // No-op: IntParam doesn't support smoothing
    }
}

impl ParamRef for IntParam {
    fn id(&self) -> ParamId {
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

    fn flags(&self) -> &ParamFlags {
        &self.info.flags
    }

    fn default_normalized(&self) -> ParamValue {
        self.info.default_normalized
    }

    fn step_count(&self) -> i32 {
        self.info.step_count
    }

    fn get_normalized(&self) -> ParamValue {
        self.plain_to_normalized(self.get() as f64)
    }

    fn set_normalized(&self, value: ParamValue) {
        let plain = self.normalized_to_plain(value).round() as i64;
        self.set(plain);
    }

    fn get_plain(&self) -> ParamValue {
        self.get() as f64
    }

    fn set_plain(&self, value: ParamValue) {
        self.set(value.round() as i64);
    }

    fn display_normalized(&self, normalized: ParamValue) -> String {
        let plain = self.normalized_to_plain(normalized).round();
        self.formatter.format(plain)
    }

    fn parse(&self, s: &str) -> Option<ParamValue> {
        let plain = self.formatter.parse(s)?;
        Some(self.plain_to_normalized(plain))
    }

    fn normalized_to_plain(&self, normalized: ParamValue) -> ParamValue {
        let normalized = normalized.clamp(0.0, 1.0);
        (self.min as f64) + normalized * ((self.max - self.min) as f64)
    }

    fn plain_to_normalized(&self, plain: ParamValue) -> ParamValue {
        if self.max == self.min {
            return 0.5;
        }
        ((plain - self.min as f64) / (self.max - self.min) as f64).clamp(0.0, 1.0)
    }

    fn info(&self) -> &ParamInfo {
        &self.info
    }
}

// =============================================================================
// BoolParam - Boolean parameter
// =============================================================================

/// Boolean parameter (toggle).
///
/// # Specialized Constructors
///
/// - [`BoolParam::new`]: Generic boolean parameter
/// - [`BoolParam::bypass`]: Bypass parameter with VST3 flags
///
/// # Example
///
/// ```ignore
/// let enabled = BoolParam::new("Enabled", true).with_id(0);
/// let bypass = BoolParam::bypass().with_id(1);
///
/// if enabled.get() && !bypass.get() {
///     // Process audio
/// }
/// ```
pub struct BoolParam {
    /// Parameter metadata (id, name, units, flags, etc.)
    info: ParamInfo,
    /// Atomic storage for the boolean value
    value: AtomicBool,
    /// Formatter for display string conversion
    formatter: Formatter,
}

impl BoolParam {
    /// Create a generic boolean parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value
    pub fn new(name: &'static str, default: bool) -> Self {
        Self {
            info: ParamInfo {
                id: 0,
                name,
                short_name: name,
                units: "",
                default_normalized: if default { 1.0 } else { 0.0 },
                step_count: 1, // Toggle
                flags: ParamFlags::default(),
                unit_id: crate::params::ROOT_UNIT_ID,
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
    /// or the `#[derive(Params)]` macro.
    pub fn bypass() -> Self {
        Self {
            info: ParamInfo {
                id: 0,
                name: "Bypass",
                short_name: "Byp",
                units: "",
                default_normalized: 0.0,
                step_count: 1,
                flags: ParamFlags {
                    can_automate: true,
                    is_readonly: false,
                    is_bypass: true,
                    is_list: false,
                },
                unit_id: crate::params::ROOT_UNIT_ID,
            },
            value: AtomicBool::new(false),
            formatter: Formatter::Boolean,
        }
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// This is typically called by the `#[derive(Params)]` macro to assign
    /// the FNV-1a hash of the string ID.
    pub fn with_id(mut self, id: ParamId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the unit ID (parameter group) for this parameter.
    ///
    /// Used by the `#[derive(Params)]` macro to assign parameters to VST3 units.
    pub fn with_unit(mut self, unit_id: crate::params::UnitId) -> Self {
        self.info.unit_id = unit_id;
        self
    }

    /// Set the unit ID in-place (for runtime assignment by parent structs).
    pub fn set_unit_id(&mut self, unit_id: crate::params::UnitId) {
        self.info.unit_id = unit_id;
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

    /// Get the parameter metadata.
    pub fn info(&self) -> &ParamInfo {
        &self.info
    }

    /// Get mutable access to the parameter metadata.
    ///
    /// Used for runtime modification of parameter properties like unit_id.
    pub fn info_mut(&mut self) -> &mut ParamInfo {
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

    // === Smoothing compatibility (no-ops for BoolParam) ===

    /// No-op for compatibility with the `#[derive(Params)]` macro.
    ///
    /// Boolean parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn set_sample_rate(&mut self, _sample_rate: f64) {
        // No-op: BoolParam doesn't support smoothing
    }

    /// No-op for compatibility with the `#[derive(Params)]` macro.
    ///
    /// Boolean parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn reset_smoothing(&mut self) {
        // No-op: BoolParam doesn't support smoothing
    }
}

impl ParamRef for BoolParam {
    fn id(&self) -> ParamId {
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

    fn flags(&self) -> &ParamFlags {
        &self.info.flags
    }

    fn default_normalized(&self) -> ParamValue {
        self.info.default_normalized
    }

    fn step_count(&self) -> i32 {
        self.info.step_count
    }

    fn get_normalized(&self) -> ParamValue {
        if self.get() {
            1.0
        } else {
            0.0
        }
    }

    fn set_normalized(&self, value: ParamValue) {
        self.set(value > 0.5);
    }

    fn get_plain(&self) -> ParamValue {
        self.get_normalized()
    }

    fn set_plain(&self, value: ParamValue) {
        self.set_normalized(value);
    }

    fn display_normalized(&self, normalized: ParamValue) -> String {
        self.formatter.format(normalized)
    }

    fn parse(&self, s: &str) -> Option<ParamValue> {
        self.formatter.parse(s)
    }

    fn normalized_to_plain(&self, normalized: ParamValue) -> ParamValue {
        normalized
    }

    fn plain_to_normalized(&self, plain: ParamValue) -> ParamValue {
        plain
    }

    fn info(&self) -> &ParamInfo {
        &self.info
    }
}

// =============================================================================
// EnumParamValue Trait - For enums used as parameter values
// =============================================================================

/// Trait for enums that can be used as parameter values.
///
/// This trait is implemented by `#[derive(EnumParam)]` and provides the
/// interface for converting between enum variants and indices.
///
/// # Example
///
/// ```ignore
/// use beamer::EnumParam;
///
/// #[derive(Copy, Clone, PartialEq, EnumParam)]
/// pub enum FilterType {
///     #[name = "Low Pass"]
///     LowPass,
///     #[default]
///     #[name = "High Pass"]
///     HighPass,
///     BandPass,  // Uses "BandPass" as display name
/// }
/// ```
pub trait EnumParamValue: Copy + Clone + PartialEq + Send + Sync + 'static {
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
// EnumParam - Enum parameter with atomic storage
// =============================================================================

/// Enum parameter for discrete choices (filter types, waveforms, etc.).
///
/// # Example
///
/// ```ignore
/// use beamer::prelude::*;
/// use beamer::EnumParam;
///
/// #[derive(Copy, Clone, PartialEq, EnumParam)]
/// pub enum FilterType {
///     #[name = "Low Pass"]
///     LowPass,
///     #[default]
///     #[name = "High Pass"]
///     HighPass,
/// }
///
/// #[derive(Params)]
/// pub struct FilterParams {
///     #[param(id = "filter_type")]
///     pub filter_type: EnumParam<FilterType>,
/// }
///
/// impl Default for FilterParams {
///     fn default() -> Self {
///         Self {
///             // Uses HighPass as default (from #[default] attribute)
///             filter_type: EnumParam::new("Filter Type"),
///         }
///     }
/// }
///
/// // In DSP code:
/// fn process(&self) {
///     match self.params.filter_type.get() {
///         FilterType::LowPass => { /* ... */ }
///         FilterType::HighPass => { /* ... */ }
///     }
/// }
/// ```
pub struct EnumParam<E: EnumParamValue> {
    /// Parameter metadata (id, name, units, flags, etc.)
    info: ParamInfo,
    /// Atomic storage for the variant index
    value: std::sync::atomic::AtomicUsize,
    /// Phantom data for the enum type
    _marker: std::marker::PhantomData<E>,
}

impl<E: EnumParamValue> EnumParam<E> {
    /// Create a new enum parameter using the trait's default value.
    ///
    /// The default value is determined by the `#[default]` attribute on the enum,
    /// or the first variant if no default is specified.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or the `#[derive(Params)]` macro.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    ///
    /// # Example
    ///
    /// ```ignore
    /// let filter_type = EnumParam::new("Filter Type")
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
    /// let filter_type = EnumParam::with_value("Filter Type", FilterType::LowPass)
    ///     .with_id(hash);
    /// ```
    pub fn with_value(name: &'static str, default: E) -> Self {
        let default_index = default.to_index();
        let default_normalized = index_to_normalized(default_index, E::COUNT);

        Self {
            info: ParamInfo {
                id: 0,
                name,
                short_name: name,
                units: "",
                default_normalized,
                step_count: (E::COUNT.saturating_sub(1)) as i32,
                // EnumParam is always a list (dropdown), even with only 2 choices
                flags: ParamFlags {
                    is_list: true,
                    ..ParamFlags::default()
                },
                unit_id: crate::params::ROOT_UNIT_ID,
            },
            value: std::sync::atomic::AtomicUsize::new(default_index),
            _marker: std::marker::PhantomData,
        }
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// This is typically called by the `#[derive(Params)]` macro to assign
    /// the FNV-1a hash of the string ID.
    pub fn with_id(mut self, id: ParamId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the unit ID (parameter group) for this parameter.
    ///
    /// Used by the `#[derive(Params)]` macro to assign parameters to VST3 units.
    pub fn with_unit(mut self, unit_id: crate::params::UnitId) -> Self {
        self.info.unit_id = unit_id;
        self
    }

    /// Set the unit ID in-place (for runtime assignment by parent structs).
    pub fn set_unit_id(&mut self, unit_id: crate::params::UnitId) {
        self.info.unit_id = unit_id;
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

    /// Get the parameter metadata.
    pub fn info(&self) -> &ParamInfo {
        &self.info
    }

    /// Get mutable access to the parameter metadata.
    ///
    /// Used for runtime modification of parameter properties like unit_id.
    pub fn info_mut(&mut self) -> &mut ParamInfo {
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

    // === Smoothing compatibility (no-ops for EnumParam) ===

    /// No-op for compatibility with the `#[derive(Params)]` macro.
    ///
    /// Enum parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn set_sample_rate(&mut self, _sample_rate: f64) {
        // No-op: EnumParam doesn't support smoothing
    }

    /// No-op for compatibility with the `#[derive(Params)]` macro.
    ///
    /// Enum parameters don't support smoothing, so this does nothing.
    #[inline]
    pub fn reset_smoothing(&mut self) {
        // No-op: EnumParam doesn't support smoothing
    }
}

impl<E: EnumParamValue> ParamRef for EnumParam<E> {
    fn id(&self) -> ParamId {
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

    fn flags(&self) -> &ParamFlags {
        &self.info.flags
    }

    fn default_normalized(&self) -> ParamValue {
        self.info.default_normalized
    }

    fn step_count(&self) -> i32 {
        self.info.step_count
    }

    fn get_normalized(&self) -> ParamValue {
        let index = self.value.load(Ordering::Relaxed);
        index_to_normalized(index, E::COUNT)
    }

    fn set_normalized(&self, value: ParamValue) {
        let index = normalized_to_index(value, E::COUNT);
        self.value.store(index, Ordering::Relaxed);
    }

    fn get_plain(&self) -> ParamValue {
        self.value.load(Ordering::Relaxed) as f64
    }

    fn set_plain(&self, value: ParamValue) {
        let index = (value.round() as usize).min(E::COUNT.saturating_sub(1));
        self.value.store(index, Ordering::Relaxed);
    }

    fn display_normalized(&self, normalized: ParamValue) -> String {
        let index = normalized_to_index(normalized, E::COUNT);
        E::name(index).to_string()
    }

    fn parse(&self, s: &str) -> Option<ParamValue> {
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

    fn normalized_to_plain(&self, normalized: ParamValue) -> ParamValue {
        normalized_to_index(normalized, E::COUNT) as f64
    }

    fn plain_to_normalized(&self, plain: ParamValue) -> ParamValue {
        index_to_normalized(plain.round() as usize, E::COUNT)
    }

    fn info(&self) -> &ParamInfo {
        &self.info
    }
}

// EnumParam<E> is Send + Sync because:
// - AtomicUsize is Send + Sync
// - PhantomData<E> is Send + Sync when E: Send + Sync (required by EnumParamValue trait bounds)
// - ParamInfo is Send + Sync
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
