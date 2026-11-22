//! High-level parameter types with encapsulated atomic storage.
//!
//! This module provides the recommended way to define plugin parameters. It includes
//! parameter types ([`FloatParameter`], [`IntParameter`], [`BoolParameter`], [`EnumParameter`]) that
//! encapsulate atomic storage, range mapping and value formatting.
//!
//! # The `Parameters` Trait (Recommended)
//!
//! The [`Parameters`] trait is the preferred way to define parameters. Use `#[beamer::model]`
//! for automatic implementation:
//!
//! ```ignore
//! use beamer::prelude::*;
//!
//! #[beamer::model]
//! pub struct MyModel {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     pub gain: FloatParameter,
//!
//!     #[parameter(id = "attack", name = "Attack", default = 10.0, range = 0.1..=100.0, kind = "ms")]
//!     pub attack: FloatParameter,
//! }
//! ```
//!
//! The attribute generates implementations of `Parameters` and
//! [`Saved`](crate::model::Saved), along with the traits of the web UI.
//! Every `Parameters` is a
//! [`ParameterStore`](crate::parameter_store::ParameterStore) as well. See
//! [`crate::parameter_store`] for details on the relationship between
//! these traits.
//!
//! # Parameter Types
//!
//! - [`FloatParameter`] - Continuous float values with range mapping and smoothing
//! - [`IntParameter`] - Discrete integer values
//! - [`BoolParameter`] - Toggle/boolean values
//! - [`EnumParameter`] - Discrete enum choices (use with `#[derive(EnumParameter)]`)

use std::borrow::Cow;
use std::ops::RangeInclusive;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use crate::parameter_format::{step_decimals, Formatter, MiddleC, MAX_STEP_DECIMALS};
use crate::parameter_groups::{GroupId, GroupInfo, ParameterGroups, ROOT_GROUP_ID};
use crate::parameter_info::{ParameterFlags, ParameterInfo, ParameterUnit};
use crate::parameter_range::{LinearMapper, LogMapper, LogOffsetMapper, PowerMapper, RangeMapper};
use crate::smoothing::{AtomicSmoother, SmoothingStyle};
use crate::state_format::join_path;
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
/// concurrent access from audio, UI and host threads.
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

    /// The normalized value the parameter stores for `normalized`: what
    /// [`get_normalized`](Self::get_normalized) returns after
    /// [`set_normalized`](Self::set_normalized) with it, such as the value
    /// of the nearest step of a discrete parameter or of a float parameter
    /// with a step.
    ///
    /// The default clamps the value, as a continuous parameter stores it.
    fn stored_normalized(&self, normalized: ParameterValue) -> ParameterValue {
        normalized.clamp(0.0, 1.0)
    }

    /// Whether the parameter goes from one step to the next without a
    /// smoother, as an int, a bool or an enum parameter does.
    ///
    /// A host ramp then changes such a parameter where the ramp reaches the
    /// next step. A float parameter with a step stores its steps too, but is
    /// not discrete: its smoother moves between them. The default is
    /// `false`, for a continuous parameter.
    fn is_discrete(&self) -> bool {
        false
    }

    /// The step of a float parameter with one, in plain units. `None` for
    /// one without it or for a discrete parameter, whose steps are its
    /// choices.
    fn step_size(&self) -> Option<f64> {
        None
    }

    /// The normalized value `steps` steps from `normalized` (up for a
    /// positive count, down for a negative one), counted from the step
    /// nearest `normalized` and kept within the range: the next choice of a
    /// discrete parameter, the next step of a float parameter with one, as
    /// a control's arrow key moves it. `None` for a continuous parameter.
    ///
    /// The default counts the [`step_count`](Self::step_count) steps spread
    /// evenly over the normalized range, as a discrete parameter has them.
    fn step_from(&self, normalized: ParameterValue, steps: i32) -> Option<ParameterValue> {
        step_evenly(normalized, steps, self.step_count())
    }

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

    /// Get the formatter kind string (e.g. "db", "pan", "hz").
    ///
    /// The WebView reports it as the `format` of a parameter. Besides the
    /// kinds of [`Formatter::kind`], enum parameters report "enum".
    fn formatter_kind(&self) -> &'static str {
        "float"
    }

    /// Get the full ParameterInfo for this parameter.
    ///
    /// The [`ParameterStore`](crate::parameter_store::ParameterStore) of
    /// every [`Parameters`] answers `info()` and `info_by_id()` with it.
    fn info(&self) -> &ParameterInfo;
}

// =============================================================================
// Parameters Trait - Parameter collection
// =============================================================================

/// The path of a parameter: its `id` behind the path of its struct.
///
/// Used by the code that `#[beamer::model]` and `#[beamer::group]` generate. The path of a
/// parameter in the root struct is its `id`, which needs no allocation.
pub fn parameter_path(prefix: &str, id: &'static str) -> Cow<'static, str> {
    if prefix.is_empty() {
        Cow::Borrowed(id)
    } else {
        Cow::Owned(join_path(prefix, id))
    }
}

/// Check that no two parameters share the id that hosts see. The same
/// goes for groups.
///
/// That id is a 32-bit hash of the parameter's path, so two different
/// paths can hash to the same value. It is unlikely. When it happens a
/// host would treat the two parameters as one. The same error shows when
/// ids were never set, for example by a hand-written `Default` that does
/// not call [`Parameters::assign_ids`]. A group's id is a 31-bit hash of
/// its path (see [`group_id`](crate::parameter_groups::group_id)). It
/// must be above 0, the root group's id. Every parameter must be in the
/// root or in one of the listed groups.
///
/// The format wrappers call this when a plugin instance is created. They
/// refuse the instance and report the error, which names the parameters
/// or groups. A test of the plugin that calls this shows it earlier.
pub fn check_unique_ids(parameters: &dyn Parameters) -> Result<(), String> {
    let mut seen: Vec<(ParameterId, &dyn ParameterRef)> = parameters
        .iter()
        .map(|parameter| (parameter.id(), parameter))
        .collect();
    seen.sort_unstable_by_key(|(id, _)| *id);

    // A parameter by its path, or by its name when it has no string id.
    let label = |parameter: &dyn ParameterRef| {
        let info = parameter.info();
        if info.string_id.is_empty() {
            format!("\"{}\" (no string id)", info.name)
        } else {
            format!("\"{}\"", info.string_id)
        }
    };

    for pair in seen.windows(2) {
        if pair[0].0 == pair[1].0 {
            let (first, second) = (pair[0].1, pair[1].1);
            // Without string ids, the ids were most likely never set.
            let unset = first.info().string_id.is_empty() && second.info().string_id.is_empty();
            let advice = if unset {
                "set their ids, which a hand-written `Default` does with `assign_ids(\"\")`"
            } else {
                "give one of them another id"
            };
            return Err(format!(
                "parameters {} and {} both have the host id 0x{:08x}; {}",
                label(first),
                label(second),
                pair[0].0,
                advice
            ));
        }
    }

    // The format wrappers take the first group for the root.
    if let Some(first) = parameters.group_info(0).filter(|group| group.id != ROOT_GROUP_ID) {
        return Err(format!(
            "the first group is \"{}\" with the id {}; `group_info(0)` must return the root group (id 0)",
            first.name, first.id
        ));
    }

    // The groups by their ids, a nested group also by its field path when
    // its id and name match a nested field.
    let mut nested = Vec::new();
    nested_group_paths(parameters, "", &mut nested);
    let mut groups: Vec<(GroupId, String)> = (1..parameters.group_count())
        .filter_map(|index| parameters.group_info(index))
        .map(|group| {
            let field = nested.iter().position(|(path, name)| {
                *name == group.name && crate::parameter_groups::group_id(path) == group.id
            });
            match field {
                Some(position) => {
                    let (path, _) = nested.remove(position);
                    (group.id, format!("\"{}\" (field {})", group.name, path))
                }
                None => (group.id, format!("\"{}\"", group.name)),
            }
        })
        .collect();
    groups.sort_unstable_by_key(|(id, _)| *id);

    // A flat group gets another id by another name, a nested group by
    // another field name.
    let how = "a flat group by its name, a nested group by its field";
    if let Some((id, label)) = groups.iter().find(|(id, _)| *id <= ROOT_GROUP_ID) {
        return Err(format!(
            "group {} has the id {}, but group ids are above 0, the root group's; \
             rename it ({}), or give a hand-written group an id above 0",
            label, id, how
        ));
    }
    for pair in groups.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(format!(
                "groups {} and {} both have the id 0x{:08x}; rename one of them ({})",
                pair[0].1, pair[1].1, pair[0].0, how
            ));
        }
    }

    // A parameter in a group that is not listed: its group ids were never
    // set, or it was given a number by hand.
    for parameter in parameters.iter() {
        let group = parameter.info().group_id;
        if group != ROOT_GROUP_ID && groups.binary_search_by_key(&group, |(id, _)| *id).is_err() {
            return Err(format!(
                "parameter {} is in the group 0x{:08x}, which the plugin does not list; \
                 a hand-written `Default` sets the group ids with `set_group_ids()`, \
                 `with_group` takes `group_id(path)`",
                label(parameter),
                group
            ));
        }
    }
    Ok(())
}

/// The paths and names of the nested groups below `parameters`.
fn nested_group_paths(parameters: &dyn Parameters, prefix: &str, groups: &mut Vec<(String, &'static str)>) {
    for index in 0..parameters.nested_count() {
        if let Some((field, name, nested)) = parameters.nested_group(index) {
            let path = join_path(prefix, field);
            nested_group_paths(nested, &path, groups);
            groups.push((path, name));
        }
    }
}

/// The id of the plugin's bypass parameter (the one with the `bypass`
/// flag), if it has one. Iterating the parameters allocates: call it while
/// preparing, not on the audio thread.
pub fn bypass_parameter(parameters: &dyn Parameters) -> Option<ParameterId> {
    parameters.iter().find(|p| p.flags().is_bypass).map(|p| p.id())
}

/// Whether the bypass parameter `id` is on, none without one. Realtime-safe.
pub fn bypass_on(parameters: &dyn Parameters, id: Option<ParameterId>) -> Option<bool> {
    id.and_then(|id| parameters.by_id(id)).map(|p| p.get_normalized() >= 0.5)
}

/// Trait for parameter collections.
///
/// Implement this trait to declare your plugin's parameters. It covers the
/// parameters alone: type-erased iteration, ids, groups and smoothing.
/// Saving and loading and `State` fields are in
/// [`Saved`](crate::model::Saved), which a hand-written implementation adds
/// as an empty `impl`. A model also has the traits of its web UI.
///
/// # Example
///
/// ```ignore
/// use beamer_core::parameter_types::{FloatParameter, Parameters, ParameterRef};
///
/// struct MyModel {
///     gain: FloatParameter,
/// }
///
/// impl Parameters for MyModel {
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

    /// Give every parameter of this struct its ids.
    ///
    /// The string id of a parameter is its path: the `id` of its attribute
    /// behind `prefix`, which is the path of this struct in the struct that
    /// holds it (empty for the root). A parameter `attack` in the nested
    /// field `osc1` gets `osc1/attack`. The id that hosts see is the hash
    /// of that path, so the same nested group can sit in several fields.
    ///
    /// `#[beamer::model]` and `#[beamer::group]` generate this and call it in the `Default`
    /// it generates. A hand-written `Default` (with `no_default`) calls
    /// `assign_ids("")` itself.
    ///
    /// The default does nothing, for hand-written implementations that set
    /// their ids themselves. Such a struct keeps its ids wherever it is
    /// nested, so it can be nested in one field only.
    fn assign_ids(&mut self, _prefix: &str) {}

    /// Set group ID for all direct parameters in this collection.
    ///
    /// Called by parent structs when initializing nested parameter groups.
    /// The default implementation does nothing (for structs without nested groups).
    fn set_all_group_ids(&mut self, _group_id: GroupId) {
        // Default: no-op (macro generates override for parameter-containing structs)
    }

    /// Name notes with `middle_c` in every parameter that shows a MIDI note
    /// number as its name (`kind = "note_number"`), also in nested groups.
    ///
    /// The format wrappers call it once when they create the model, with
    /// `middle_c` from `Config.toml`. `#[beamer::model]` and
    /// `#[beamer::group]` generate it; the default does nothing, for
    /// hand-written implementations, whose parameters keep the formatter
    /// they were given.
    fn set_middle_c(&mut self, _middle_c: MiddleC) {}

    // =========================================================================
    // Nested Group Discovery (for recursive group ID assignment)
    // =========================================================================

    /// Number of direct nested parameter groups in this struct.
    ///
    /// Default is 0 (no nested groups). `#[beamer::model]` and `#[beamer::group]`
    /// generates an override for structs with `#[nested]` fields.
    fn nested_count(&self) -> usize {
        0
    }

    /// Get information about a nested group by index.
    ///
    /// Returns the field name (its part of the paths of the parameters and
    /// groups inside it), the group name and the nested Parameters.
    /// Default returns None (no nested groups).
    fn nested_group(&self, _index: usize) -> Option<(&'static str, &'static str, &dyn Parameters)> {
        None
    }

    /// Get mutable access to a nested group by index.
    ///
    /// Returns the field name, the group name and a mutable reference to
    /// the nested Parameters.
    /// Default returns None (no nested groups).
    fn nested_group_mut(&mut self, _index: usize) -> Option<(&'static str, &'static str, &mut dyn Parameters)> {
        None
    }

    /// Recursively give the parameters of all nested groups their group ids.
    ///
    /// A nested group's id is the [`group_id`](crate::parameter_groups::group_id)
    /// of its path: its field name behind `prefix`, the path of this struct
    /// (empty for the root).
    ///
    /// # Example
    ///
    /// ```ignore
    /// // Called by set_group_ids() on the top-level struct:
    /// self.assign_group_ids("");
    /// ```
    fn assign_group_ids(&mut self, prefix: &str) {
        for i in 0..self.nested_count() {
            if let Some((field, _, nested)) = self.nested_group_mut(i) {
                let path = join_path(prefix, field);
                let group_id = crate::parameter_groups::group_id(&path);

                // Set group ID on all direct parameters in this nested group.
                nested.set_all_group_ids(group_id);

                // Recurse into the groups nested in it.
                nested.assign_group_ids(&path);
            }
        }
    }

    /// Collect all group infos from nested groups recursively.
    ///
    /// This is used by the `ParameterGroups` trait implementation to build the
    /// complete list of groups for the DAW.
    ///
    /// # Arguments
    ///
    /// * `groups` - Vector to append GroupInfo entries to
    /// * `prefix` - The path of this struct, empty for the root
    /// * `parent_id` - The group id of this struct, the parent of its nested groups
    fn collect_groups(&self, groups: &mut Vec<GroupInfo>, prefix: &str, parent_id: GroupId) {
        for i in 0..self.nested_count() {
            if let Some((field, name, nested)) = self.nested_group(i) {
                let path = join_path(prefix, field);
                let group_id = crate::parameter_groups::group_id(&path);

                groups.push(GroupInfo::new(group_id, name, parent_id));

                // Recurse into nested groups.
                nested.collect_groups(groups, &path, group_id);
            }
        }
    }

    // =========================================================================
    // Smoothing Support
    // =========================================================================

    /// Set sample rate for all smoothers in this parameter collection.
    ///
    /// The framework calls this with the host's sample rate before
    /// `Model::prepare()`, so a plugin that processes at the host's
    /// rate has nothing to do.
    ///
    /// **Oversampling:** If your plugin uses oversampling, call this in
    /// `prepare()` with the actual processing rate:
    /// `sample_rate * oversampling_factor`.
    ///
    /// Call this while no audio is processed (`prepare()` is such a
    /// moment): the smoothers belong to the audio thread.
    fn set_sample_rate(&self, _sample_rate: f64) {
        // Default no-op. #[beamer::model] and #[beamer::group] generate an override
        // that calls set_sample_rate on each float parameter and nested field.
    }

    /// Reset all smoothers to their current values (no ramp).
    ///
    /// Called automatically by the framework after loading state to avoid
    /// ramps to loaded values. You typically don't need to call this directly.
    /// It belongs to the audio thread, like every method that advances a
    /// smoother.
    fn reset_smoothing(&self) {
        // Default no-op. #[beamer::model] and #[beamer::group] generate an override
        // that calls reset_smoothing on each float parameter and nested field.
    }

    // =========================================================================
    // Defaults
    // =========================================================================

    /// Set every parameter, also in nested groups, to its default value.
    ///
    /// A factory preset that should sound the same whatever was loaded
    /// before starts with this. `#[beamer::model]` and `#[beamer::group]`
    /// generate an override that does not allocate, so it is safe on the
    /// audio thread; the default goes through [`iter`](Self::iter), which
    /// allocates.
    fn reset_parameters(&self) {
        for parameter in self.iter() {
            parameter.set_normalized(parameter.default_normalized());
        }
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
/// - [`FloatParameter::percent`]: Percentage parameter (usually 0-100%)
/// - [`FloatParameter::pan`]: Pan parameter (L-C-R)
/// - [`FloatParameter::ratio`]: Compressor ratio parameter
///
/// # Example
///
/// ```ignore
/// // Create parameter - ID is set separately via with_id() or #[beamer::model]
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
    smoother: Option<AtomicSmoother>,
    /// Whether this parameter stores dB values (for as_linear() optimization)
    is_db: bool,
    /// Optional step size for discrete stepping. None = continuous.
    step_size: Option<f64>,
    /// The decimals of the values on the steps, to which a snapped value
    /// is rounded. Unused without a step.
    step_decimals: usize,
}

impl FloatParameter {
    /// Create a generic float parameter with linear mapping.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
                string_id: Cow::Borrowed(""),
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
            step_decimals: 0,
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
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
                string_id: Cow::Borrowed(""),
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
            step_decimals: 0,
        }
    }

    /// Create a dB parameter with power curve mapping for more resolution at maximum.
    ///
    /// Uses a power curve (exponent = 2.0) to provide more resolution near 0 dB
    /// and less resolution at the minimum. Ideal for threshold parameters where
    /// precision near 0 dB is important.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
                string_id: Cow::Borrowed(""),
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
            step_decimals: 0,
        }
    }

    /// Create a dB parameter with true logarithmic mapping (using offset).
    ///
    /// Uses logarithmic mapping for ranges that include negative values by
    /// offsetting to positive space. Provides geometric mean at midpoint.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
                string_id: Cow::Borrowed(""),
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
            step_decimals: 0,
        }
    }

    /// Create a frequency parameter with logarithmic mapping.
    ///
    /// Logarithmic mapping provides a perceptually uniform distribution
    /// across the frequency range.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
        let formatter = Formatter::Frequency { precision: None };

        Self {
            info: ParameterInfo {
                id: 0,
                string_id: Cow::Borrowed(""),
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
            step_decimals: 0,
        }
    }

    /// Create a milliseconds parameter.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
    /// The value is a fraction, displayed as a percentage: 0.0-1.0 shows as
    /// 0%-100%. A range of 0.0..=0.95 ends at 95%.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default_pct` - Default value as a fraction (0.5, not 50)
    /// * `range_pct` - Valid range as fractions (inclusive), usually 0.0..=1.0
    pub fn percent(name: &'static str, default_pct: f64, range_pct: RangeInclusive<f64>) -> Self {
        let mut parameter = Self::new(name, default_pct, range_pct);
        let formatter = Formatter::Percent { precision: 0 };
        parameter.info.units = formatter.unit();
        parameter.info.unit = ParameterUnit::Percent;
        parameter.formatter = formatter;
        parameter
    }

    /// Create a pan parameter.
    ///
    /// -1.0 is full left, +1.0 full right and 0.0 center.
    /// Display: "L50", "C", "R50"
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value, typically 0.0
    /// * `range` - Valid range (inclusive), usually -1.0..=1.0
    pub fn pan(name: &'static str, default: f64, range: RangeInclusive<f64>) -> Self {
        let mut parameter = Self::new(name, default, range);
        parameter.info.unit = ParameterUnit::Pan;
        parameter.formatter = Formatter::Pan;
        parameter
    }

    /// Create a ratio parameter for compressors.
    ///
    /// Display: "4.0:1", "∞:1"
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
    /// For parameters that are set up by hand: any u32 that is unique in
    /// the plugin. `#[beamer::model]` and `#[beamer::group]` set the id themselves, through
    /// [`set_path`](Self::set_path).
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

    /// Set the string identifier, for parameters that are set up by hand.
    pub fn with_string_id(mut self, string_id: &'static str) -> Self {
        self.info.string_id = Cow::Borrowed(string_id);
        self
    }

    /// Set both ids from the path of the parameter in its model.
    ///
    /// See [`ParameterInfo::set_path`]. Called by `#[beamer::model]` and `#[beamer::group]`.
    pub fn set_path(&mut self, path: Cow<'static, str>) {
        self.info.set_path(path);
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the group ID (parameter group) for this parameter.
    ///
    /// Used by `#[beamer::model]` and `#[beamer::group]` to assign parameters to groups.
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
    /// from the range's minimum, within the range (the maximum counts as a
    /// step, also when it lies between two): when set and wherever a
    /// plain value is read (`get()`, the display text, the target a
    /// smoother moves towards), so a value reads back exactly (440.0, not
    /// 439.99999999999983 after the round trip through a log mapping). Any
    /// normalized value set is stored as that of the nearest step too, so a
    /// change of less than half a step changes nothing. The default snaps
    /// as well. The display shows the decimals of the steps: a step of 0.1
    /// shows "442.5", a step of 1.0 shows "442". A later `with_precision()`
    /// or `with_formatter()` overrides that.
    ///
    /// Hosts see the parameter as continuous (a step count of 0): a host
    /// spreads a parameter's steps evenly over the normalized range, which
    /// a mapping that is not linear (`hz`, `db_log`) does not, so the host's
    /// steps would skip or double some of the parameter's. A continuous
    /// host control reaches every step, as each position snaps to the
    /// nearest one.
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
    pub fn with_step_size(mut self, step_size: f64) -> Self {
        assert!(
            step_size > 0.0,
            "step_size must be positive, got {}",
            step_size
        );

        let (min, max) = self.range.range();
        self.step_size = Some(step_size);
        // The values on the steps are min + k * step_size: they have the
        // decimals of the step or of the minimum, whichever has more.
        self.step_decimals = step_decimals(step_size).max(step_decimals(min));
        // The maximum is a value too, which `snap_to_step` returns as it is:
        // the display shows its decimals as well, unless it has no end of
        // them (a maximum of 2 pi).
        let max_decimals = step_decimals(max);
        let shown_decimals = if max_decimals < MAX_STEP_DECIMALS {
            self.step_decimals.max(max_decimals)
        } else {
            self.step_decimals
        };
        self.formatter = self
            .formatter
            .with_step(10f64.powi(-(shown_decimals as i32)));
        // Put the default and the value on a step, so that they read back
        // exactly.
        let default = self.plain(self.info.default_normalized);
        self.info.default_normalized = self.range.normalize(default);
        let value = self.plain(f64::from_bits(self.value.load(Ordering::Relaxed)));
        self.value
            .store(self.range.normalize(value).to_bits(), Ordering::Relaxed);
        self.reset_smoothing();
        self
    }

    /// Get the step size, if configured.
    pub fn step_size(&self) -> Option<f64> {
        self.step_size
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
    /// // Frequency in whole Hz: with a precision it shows "1500", not "1.50k".
    /// let freq = FloatParameter::hz("Cutoff", 1000.0, 20.0..=20000.0)
    ///     .with_precision(0);
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
    ///
    /// If a step size is configured, the value is on a step.
    #[inline]
    pub fn get(&self) -> f64 {
        self.plain(f64::from_bits(self.value.load(Ordering::Relaxed)))
    }

    /// Set the plain value in natural units.
    ///
    /// If a step size is configured, the value is snapped to the nearest step.
    #[inline]
    pub fn set(&self, value: f64) {
        let normalized = self.range.normalize(self.snapped(value));
        self.value.store(normalized.to_bits(), Ordering::Relaxed);
    }

    /// The plain value of `normalized`, on the nearest step when a step
    /// size is configured.
    #[inline]
    fn plain(&self, normalized: f64) -> f64 {
        self.snapped(self.range.denormalize(normalized))
    }

    /// The plain value `value` on the nearest step when a step size is
    /// configured, else `value` itself.
    #[inline]
    fn snapped(&self, value: f64) -> f64 {
        match self.step_size {
            Some(step) => {
                let (min, max) = self.range.range();
                snap_to_step(value, step, self.step_decimals, min, max)
            }
            None => value,
        }
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
    //
    // These take `&self`, so that the audio thread can smooth parameters
    // that it shares with the host and UI threads. The state of the
    // smoother is kept in atomics for that. Only one thread at a time may
    // advance a smoother (the audio thread): a second one would not break
    // memory safety, but the two would disturb each other's ramp.

    /// Add smoothing to this parameter.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
    ///     .with_smoother(SmoothingStyle::Exponential(5.0));  // 5ms
    /// ```
    pub fn with_smoother(mut self, style: SmoothingStyle) -> Self {
        self.smoother = match style {
            SmoothingStyle::None => None,
            _ => {
                let bits = self.value.load(Ordering::Relaxed);
                let value = self.plain(f64::from_bits(bits));
                Some(AtomicSmoother::new(style, bits, value))
            }
        };
        self
    }

    /// Set sample rate for smoothing.
    ///
    /// The framework calls this with the host's sample rate before
    /// `Model::prepare()`. If using oversampling, call it in
    /// `prepare()` with `sample_rate * oversampling_factor`.
    pub fn set_sample_rate(&self, sample_rate: f64) {
        if let Some(smoother) = &self.smoother {
            smoother.set_sample_rate(
                sample_rate,
                self.value.load(Ordering::Relaxed),
                |normalized| self.plain(normalized),
            );
        }
    }

    /// Get the current smoothed value without advancing.
    ///
    /// If no smoother is configured, returns the raw value.
    #[inline]
    pub fn smoothed(&self) -> f64 {
        match &self.smoother {
            Some(smoother) => smoother.current(),
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
    /// Call once per sample in the audio loop, on the audio thread only.
    ///
    /// If no smoother is configured, returns the raw value.
    #[inline]
    pub fn tick_smoothed(&self) -> f64 {
        match &self.smoother {
            // The smoother follows the atomic value (in case the host
            // changed it).
            Some(smoother) => smoother.tick(self.value.load(Ordering::Relaxed), |normalized| {
                self.plain(normalized)
            }),
            None => self.get(),
        }
    }

    /// Advance the smoother by one sample and return the smoothed value as f32.
    #[inline]
    pub fn tick_smoothed_f32(&self) -> f32 {
        self.tick_smoothed() as f32
    }

    /// Skip smoothing forward by n samples.
    ///
    /// Use for block processing when per-sample smoothing isn't needed.
    pub fn skip_smoothing(&self, samples: usize) {
        if let Some(smoother) = &self.smoother {
            smoother.skip(
                self.value.load(Ordering::Relaxed),
                |normalized| self.plain(normalized),
                samples,
            );
        }
    }

    /// Fill buffer with smoothed values (f64).
    pub fn fill_smoothed(&self, buffer: &mut [f64]) {
        match &self.smoother {
            Some(smoother) => smoother.fill(
                self.value.load(Ordering::Relaxed),
                |normalized| self.plain(normalized),
                buffer.len(),
                |index, value| buffer[index] = value,
            ),
            None => {
                buffer.fill(self.get());
            }
        }
    }

    /// Fill buffer with smoothed values (f32).
    pub fn fill_smoothed_f32(&self, buffer: &mut [f32]) {
        match &self.smoother {
            Some(smoother) => smoother.fill(
                self.value.load(Ordering::Relaxed),
                |normalized| self.plain(normalized),
                buffer.len(),
                |index, value| buffer[index] = value as f32,
            ),
            None => {
                buffer.fill(self.get() as f32);
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
    pub fn reset_smoothing(&self) {
        if let Some(smoother) = &self.smoother {
            let bits = self.value.load(Ordering::Relaxed);
            smoother.reset(bits, self.plain(f64::from_bits(bits)));
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
            .store(self.stored_normalized(value).to_bits(), Ordering::Relaxed);
    }

    // With a step the normalized value of the nearest step, so that a
    // control that moves by a part of the range (an arrow key) starts from
    // a step each time and reaches the next one.
    fn stored_normalized(&self, normalized: ParameterValue) -> ParameterValue {
        let clamped = normalized.clamp(0.0, 1.0);
        match self.step_size {
            Some(_) => self.range.normalize(self.plain(clamped)),
            None => clamped,
        }
    }

    fn step_size(&self) -> Option<f64> {
        self.step_size
    }

    // A whole step in plain units from the nearest step, whatever the
    // mapping: on a log mapping the steps lie closer together in normalized
    // values at the top than at the bottom.
    fn step_from(&self, normalized: ParameterValue, steps: i32) -> Option<ParameterValue> {
        let step = self.step_size?;
        let (min, max) = self.range.range();
        let mut from = self.plain(normalized.clamp(0.0, 1.0));
        let mut steps = steps;
        // From a maximum off the steps the first step down is the highest
        // step below it, which can be nearer than a whole step.
        if steps < 0 && from == max {
            let below = self.snapped(min + ((max - min) / step).floor() * step);
            if below < max {
                from = below;
                steps += 1;
            }
        }
        Some(self.range.normalize(self.snapped(from + f64::from(steps) * step)))
    }

    fn get_plain(&self) -> ParameterValue {
        self.get()
    }

    fn set_plain(&self, value: ParameterValue) {
        self.set(value);
    }

    fn display_normalized(&self, normalized: ParameterValue) -> String {
        self.formatter.text(self.plain(normalized))
    }

    fn parse(&self, s: &str) -> Option<ParameterValue> {
        let plain = self.formatter.parse(s)?;
        Some(self.range.normalize(self.snapped(plain)))
    }

    fn normalized_to_plain(&self, normalized: ParameterValue) -> ParameterValue {
        self.plain(normalized)
    }

    fn plain_to_normalized(&self, plain: ParameterValue) -> ParameterValue {
        self.range.normalize(self.snapped(plain))
    }

    fn formatter_kind(&self) -> &'static str {
        self.formatter.kind()
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
/// - [`IntParameter::note_number`]: MIDI note number shown as a note name
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
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
                string_id: Cow::Borrowed(""),
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
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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

    /// Create a parameter for a MIDI note number, shown as its name.
    ///
    /// Format: "C3" for 60, "F#1", "C-2" (no unit), or one octave higher
    /// with middle C as C4 (see [`MiddleC`]). A typed note name or number is
    /// read back. The plugin's `middle_c` in `Config.toml` reaches the
    /// parameter through [`Parameters::set_middle_c`]; until then it names
    /// middle C C3.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default MIDI note number
    /// * `range` - Valid range of MIDI note numbers (inclusive), usually 0 to 127
    pub fn note_number(name: &'static str, default: i64, range: RangeInclusive<i64>) -> Self {
        let mut parameter = Self::new(name, default, range);
        let formatter = Formatter::NoteNumber { middle_c: MiddleC::default() };
        parameter.info.units = formatter.unit();
        parameter.info.unit = ParameterUnit::MidiNoteNumber;
        parameter.formatter = formatter;
        parameter
    }

    // === Builder methods ===

    /// Set the parameter ID.
    ///
    /// For parameters that are set up by hand: any u32 that is unique in
    /// the plugin. `#[beamer::model]` and `#[beamer::group]` set the id themselves, through
    /// [`set_path`](Self::set_path).
    pub fn with_id(mut self, id: ParameterId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the string identifier, for parameters that are set up by hand.
    pub fn with_string_id(mut self, string_id: &'static str) -> Self {
        self.info.string_id = Cow::Borrowed(string_id);
        self
    }

    /// Set both ids from the path of the parameter in its model.
    ///
    /// See [`ParameterInfo::set_path`]. Called by `#[beamer::model]` and `#[beamer::group]`.
    pub fn set_path(&mut self, path: Cow<'static, str>) {
        self.info.set_path(path);
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the group ID (parameter group) for this parameter.
    ///
    /// Used by `#[beamer::model]` and `#[beamer::group]` to assign parameters to groups.
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

    /// Name notes with `middle_c`, when the parameter shows a MIDI note
    /// number as its name. Other formatters stay as they are.
    pub fn set_middle_c(&mut self, middle_c: MiddleC) {
        if let Formatter::NoteNumber { .. } = self.formatter {
            self.formatter = Formatter::NoteNumber { middle_c };
        }
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

    fn stored_normalized(&self, normalized: ParameterValue) -> ParameterValue {
        self.plain_to_normalized(self.normalized_to_plain(normalized).round())
    }

    fn is_discrete(&self) -> bool {
        true
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

    fn formatter_kind(&self) -> &'static str {
        self.formatter.kind()
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
    /// or by `#[beamer::model]` and `#[beamer::group]`.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name
    /// * `default` - Default value
    pub fn new(name: &'static str, default: bool) -> Self {
        Self {
            info: ParameterInfo {
                id: 0,
                string_id: Cow::Borrowed(""),
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

    /// Create the plugin's bypass parameter, which a host's own bypass
    /// button sets.
    ///
    /// This creates a parameter pre-configured as a bypass switch:
    /// - Name: "Bypass"
    /// - Short name: "Byp"
    /// - Default: false (not bypassed)
    /// - Marked with `is_bypass = true`: VST3 hosts find it through the
    ///   flag, Audio Unit hosts through their bypass property
    ///
    /// The plugin keeps processing while bypassed, so it ramps (and delays)
    /// the bypass itself, for example with a
    /// [`BypassHandler`](crate::BypassHandler). It also receives MIDI: a
    /// MIDI effect or instrument passes MIDI on or ends its notes itself.
    ///
    /// The parameter ID defaults to 0 and should be set via [`with_id`](Self::with_id)
    /// or by `#[beamer::model]` and `#[beamer::group]`.
    pub fn bypass() -> Self {
        Self {
            info: ParameterInfo {
                id: 0,
                string_id: Cow::Borrowed(""),
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
    /// For parameters that are set up by hand: any u32 that is unique in
    /// the plugin. `#[beamer::model]` and `#[beamer::group]` set the id themselves, through
    /// [`set_path`](Self::set_path).
    pub fn with_id(mut self, id: ParameterId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the string identifier, for parameters that are set up by hand.
    pub fn with_string_id(mut self, string_id: &'static str) -> Self {
        self.info.string_id = Cow::Borrowed(string_id);
        self
    }

    /// Set both ids from the path of the parameter in its model.
    ///
    /// See [`ParameterInfo::set_path`]. Called by `#[beamer::model]` and `#[beamer::group]`.
    pub fn set_path(&mut self, path: Cow<'static, str>) {
        self.info.set_path(path);
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the group ID (parameter group) for this parameter.
    ///
    /// Used by `#[beamer::model]` and `#[beamer::group]` to assign parameters to groups.
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

    fn stored_normalized(&self, normalized: ParameterValue) -> ParameterValue {
        if normalized > 0.5 {
            1.0
        } else {
            0.0
        }
    }

    fn is_discrete(&self) -> bool {
        true
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

    fn formatter_kind(&self) -> &'static str {
        self.formatter.kind()
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
/// #[beamer::model]
/// pub struct FilterModel {
///     // Uses HighPass as default (from the #[default] attribute).
///     #[parameter(id = "filter_type", name = "Filter Type")]
///     pub filter_type: EnumParameter<FilterType>,
/// }
///
/// // In DSP code:
/// fn process(&self) {
///     match self.model.filter_type.get() {
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
    /// or by `#[beamer::model]` and `#[beamer::group]`.
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
                string_id: Cow::Borrowed(""),
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
    /// For parameters that are set up by hand: any u32 that is unique in
    /// the plugin. `#[beamer::model]` and `#[beamer::group]` set the id themselves, through
    /// [`set_path`](Self::set_path).
    pub fn with_id(mut self, id: ParameterId) -> Self {
        self.info.id = id;
        self
    }

    /// Set the string identifier, for parameters that are set up by hand.
    pub fn with_string_id(mut self, string_id: &'static str) -> Self {
        self.info.string_id = Cow::Borrowed(string_id);
        self
    }

    /// Set both ids from the path of the parameter in its model.
    ///
    /// See [`ParameterInfo::set_path`]. Called by `#[beamer::model]` and `#[beamer::group]`.
    pub fn set_path(&mut self, path: Cow<'static, str>) {
        self.info.set_path(path);
    }

    /// Set the short name for constrained UIs.
    pub fn with_short_name(mut self, short: &'static str) -> Self {
        self.info.short_name = short;
        self
    }

    /// Set the group ID (parameter group) for this parameter.
    ///
    /// Used by `#[beamer::model]` and `#[beamer::group]` to assign parameters to groups.
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

    fn stored_normalized(&self, normalized: ParameterValue) -> ParameterValue {
        index_to_normalized(normalized_to_index(normalized, E::COUNT), E::COUNT)
    }

    fn is_discrete(&self) -> bool {
        true
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
        let s = s.trim();

        // Try to match variant name (case-insensitive)
        let s_lower = s.to_lowercase();
        for (i, name) in E::names().iter().enumerate() {
            if name.to_lowercase() == s_lower {
                return Some(self.plain_to_normalized(i as f64));
            }
        }
        // Also try parsing as index, also written as a float ("1.0") as a
        // host may type it.
        s.parse::<f64>()
            .ok()
            .filter(|index| index.fract() == 0.0 && *index >= 0.0 && *index < E::COUNT as f64)
            .map(|index| self.plain_to_normalized(index))
    }

    fn normalized_to_plain(&self, normalized: ParameterValue) -> ParameterValue {
        normalized_to_index(normalized, E::COUNT) as f64
    }

    fn plain_to_normalized(&self, plain: ParameterValue) -> ParameterValue {
        index_to_normalized(plain.round() as usize, E::COUNT)
    }

    fn formatter_kind(&self) -> &'static str {
        "enum"
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

/// The normalized value `steps` steps from `normalized` among `count`
/// steps spread evenly over the normalized range, as a discrete parameter
/// has them. `None` without steps.
pub(crate) fn step_evenly(normalized: ParameterValue, steps: i32, count: i32) -> Option<ParameterValue> {
    if count <= 0 {
        return None;
    }
    let count = f64::from(count);
    let index = ((normalized.clamp(0.0, 1.0) * count).round() + f64::from(steps)).clamp(0.0, count);
    Some(index / count)
}

/// Snap a value to the nearest step within a range, rounded to the
/// `decimals` of the values on the steps. Not rounded at
/// [`MAX_STEP_DECIMALS`]: the steps are then no decimals (a step of 1/3).
#[inline]
fn snap_to_step(value: f64, step_size: f64, decimals: usize, min: f64, max: f64) -> f64 {
    // Calculate the number of steps from min
    let steps_from_min = ((value - min) / step_size).round();
    // Calculate snapped value
    let snapped = min + steps_from_min * step_size;
    // The nearest f64 to the step's decimal value: three steps of 0.1 are
    // 0.3, not 0.30000000000000004.
    let snapped = if decimals < MAX_STEP_DECIMALS {
        let scale = 10f64.powi(decimals as i32);
        (snapped * scale).round() / scale
    } else {
        snapped
    };
    // The maximum counts as a step, also when it lies between two, so that
    // a value clamped to it snaps to it again when read.
    if snapped >= max || (max - value).abs() < (snapped - value).abs() {
        return max;
    }
    snapped.max(min)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_size_snapping() {
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(0.5);

        // Test snapping to nearest step
        parameter.set(2.3);
        assert!((parameter.get() - 2.5).abs() < 1e-10);

        parameter.set(2.2);
        assert!((parameter.get() - 2.0).abs() < 1e-10);

        parameter.set(2.25);
        // Round up at midpoint.
        assert!((parameter.get() - 2.5).abs() < 1e-10);
    }

    #[test]
    fn test_step_size_edge_cases() {
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(0.3);

        // Snap at boundaries
        parameter.set(-0.1);
        // Clamp to min.
        assert!((parameter.get() - 0.0).abs() < 1e-10);

        parameter.set(10.1);
        // Clamp to max.
        assert!((parameter.get() - 10.0).abs() < 1e-10);
    }

    #[test]
    fn test_a_stepped_parameter_is_continuous_to_hosts_and_reaches_every_step() {
        let reference = FloatParameter::hz("Reference", 440.0, 400.0..=480.0).with_step_size(0.1);
        assert_eq!(reference.step_count(), 0);
        // Every position of a host control lands on a step; together they
        // reach all 801 steps, none skipped.
        let mut reached = std::collections::BTreeSet::new();
        for position in 0..=8000 {
            let plain = reference.normalized_to_plain(f64::from(position) / 8000.0);
            let tenths = plain * 10.0;
            assert_eq!(tenths, tenths.round(), "{plain} is on a step");
            reached.insert(tenths as i64);
        }
        assert_eq!(reached.len(), 801);
    }

    #[test]
    fn test_step_size_with_negative_range() {
        let parameter = FloatParameter::db("Gain", 0.0, -60.0..=12.0).with_step_size(0.5);

        parameter.set(-5.3);
        assert!((parameter.get() - -5.5).abs() < 1e-10);
    }

    #[test]
    fn test_continuous_parameter_no_snapping() {
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=10.0);

        parameter.set(2.3);
        // No snapping.
        assert!((parameter.get() - 2.3).abs() < 1e-10);
        // Continuous.
        assert_eq!(parameter.step_count(), 0);
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
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=1.0).with_step_size(2.0);

        // Two values: the minimum and the maximum.
        parameter.set(0.4);
        assert_eq!(parameter.get(), 0.0);
        parameter.set(0.7);
        assert_eq!(parameter.get(), 1.0);
    }

    #[test]
    fn test_step_size_getter() {
        let parameter_with_step = FloatParameter::new("Test", 0.0, 0.0..=10.0).with_step_size(0.5);
        assert_eq!(parameter_with_step.step_size(), Some(0.5));

        let parameter_continuous = FloatParameter::new("Test", 0.0, 0.0..=10.0);
        assert_eq!(parameter_continuous.step_size(), None);
    }

    #[test]
    fn test_step_size_with_smoother() {
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=10.0)
            .with_step_size(1.0)
            .with_smoother(crate::smoothing::SmoothingStyle::Linear(10.0));

        parameter.set_sample_rate(1000.0);
        // Snaps to 5.0.
        parameter.set(5.3);

        // Target should be the snapped value
        assert!((parameter.get() - 5.0).abs() < 1e-10);
    }

    // =========================================================================
    // Smoothing through a shared reference
    // =========================================================================

    use crate::smoothing::{Smoother, SmoothingStyle};

    const SMOOTHED_STYLES: [SmoothingStyle; 3] = [
        SmoothingStyle::Linear(10.0),
        SmoothingStyle::Exponential(5.0),
        SmoothingStyle::Logarithmic(5.0),
    ];

    /// A parameter and a `Smoother` that is driven the way the parameter
    /// drives its own smoother: the target follows the value before every
    /// step.
    struct Pair {
        parameter: FloatParameter,
        reference: Smoother,
    }

    impl Pair {
        fn new(style: SmoothingStyle) -> Self {
            // The range stays positive for the logarithmic style.
            let parameter = FloatParameter::new("Test", 100.0, 20.0..=20000.0).with_smoother(style);
            let mut reference = Smoother::new(style);
            reference.reset(parameter.get());
            Self {
                parameter,
                reference,
            }
        }

        fn set_sample_rate(&mut self, sample_rate: f64) {
            self.parameter.set_sample_rate(sample_rate);
            self.reference.set_sample_rate(sample_rate);
            self.reference.set_target(self.parameter.get());
        }

        /// Advance both by `count` ticks and compare every value.
        fn tick(&mut self, count: usize) {
            for step in 0..count {
                let value = self.parameter.tick_smoothed();
                self.reference.set_target(self.parameter.get());
                let expected = self.reference.tick();
                assert_eq!(value.to_bits(), expected.to_bits(), "tick {step}");
            }
            self.assert_same_position();
        }

        fn skip(&mut self, samples: usize) {
            self.parameter.skip_smoothing(samples);
            self.reference.set_target(self.parameter.get());
            self.reference.skip(samples);
            self.assert_same_position();
        }

        fn fill(&mut self, count: usize) {
            let mut values = vec![0.0; count];
            self.parameter.fill_smoothed(&mut values);
            self.reference.set_target(self.parameter.get());
            let mut expected = vec![0.0; count];
            self.reference.fill(&mut expected);
            for (step, (value, expected)) in values.iter().zip(&expected).enumerate() {
                assert_eq!(value.to_bits(), expected.to_bits(), "fill {step}");
            }
            self.assert_same_position();
        }

        fn fill_f32(&mut self, count: usize) {
            let mut values = vec![0.0_f32; count];
            self.parameter.fill_smoothed_f32(&mut values);
            self.reference.set_target(self.parameter.get());
            let mut expected = vec![0.0_f32; count];
            self.reference.fill_f32(&mut expected);
            for (step, (value, expected)) in values.iter().zip(&expected).enumerate() {
                assert_eq!(value.to_bits(), expected.to_bits(), "fill_f32 {step}");
            }
            self.assert_same_position();
        }

        fn reset(&mut self) {
            self.parameter.reset_smoothing();
            self.reference.reset(self.parameter.get());
            self.assert_same_position();
        }

        fn assert_same_position(&self) {
            assert_eq!(
                self.parameter.smoothed().to_bits(),
                self.reference.current().to_bits()
            );
            assert_eq!(self.parameter.is_smoothing(), self.reference.is_smoothing());
        }
    }

    #[test]
    fn test_smoothing_matches_smoother_over_a_ramp() {
        for style in SMOOTHED_STYLES {
            let mut pair = Pair::new(style);
            pair.set_sample_rate(1000.0);
            pair.tick(3);

            pair.parameter.set(5000.0);
            pair.tick(4);
            assert!(pair.parameter.is_smoothing(), "{style:?}");
            assert!(pair.parameter.smoothed() > 100.0, "{style:?}");
            assert!(pair.parameter.smoothed() < 5000.0, "{style:?}");

            // Long enough for every style to arrive.
            pair.tick(400);
            assert!(!pair.parameter.is_smoothing(), "{style:?}");
            assert_eq!(pair.parameter.smoothed(), pair.parameter.get(), "{style:?}");
        }
    }

    #[test]
    fn test_smoothing_matches_smoother_when_retargeted_during_a_ramp() {
        for style in SMOOTHED_STYLES {
            let mut pair = Pair::new(style);
            pair.set_sample_rate(1000.0);
            pair.parameter.set(5000.0);
            pair.tick(4);
            pair.parameter.set(40.0);
            pair.tick(3);
            pair.parameter.set(12000.0);
            pair.tick(2);
            // Back to a value it has seen before.
            pair.parameter.set(40.0);
            pair.tick(400);
        }
    }

    #[test]
    fn test_smoothing_matches_smoother_for_a_change_below_its_threshold() {
        for style in SMOOTHED_STYLES {
            // A narrow range, so that the next normalized value is a plain
            // value the smoother takes for its target.
            let parameter = FloatParameter::new("Test", 1.0, 1.0..=2.0).with_smoother(style);
            let mut reference = Smoother::new(style);
            reference.reset(parameter.get());
            let mut pair = Pair {
                parameter,
                reference,
            };
            pair.set_sample_rate(1000.0);
            pair.parameter.set(1.5);
            pair.tick(4);

            // The stored bits change, the target does not.
            let nudged = f64::from_bits(pair.parameter.get_normalized().to_bits() + 1);
            pair.parameter.set_normalized(nudged);
            assert_ne!(nudged, 0.5);
            assert!((pair.parameter.get() - 1.5).abs() < 1e-10);
            pair.tick(3);

            // A real change starts a ramp again.
            pair.parameter.set(1.25);
            pair.tick(400);
            assert_eq!(pair.parameter.smoothed(), 1.25, "{style:?}");
        }
    }

    #[test]
    fn test_smoothing_matches_smoother_for_skip_and_fill() {
        for style in SMOOTHED_STYLES {
            let mut pair = Pair::new(style);
            pair.set_sample_rate(1000.0);
            pair.parameter.set(5000.0);
            pair.skip(3);
            pair.fill(2);
            pair.fill_f32(2);
            pair.parameter.set(40.0);
            pair.fill(5);
            pair.skip(2);
            pair.fill_f32(3);
            pair.skip(1000);
            pair.fill(0);
            pair.tick(2);
        }
    }

    #[test]
    fn test_smoothing_matches_smoother_for_reset_and_sample_rate() {
        for style in SMOOTHED_STYLES {
            let mut pair = Pair::new(style);

            // Without a sample rate.
            pair.parameter.set(5000.0);
            pair.tick(3);

            pair.set_sample_rate(1000.0);
            pair.parameter.set(40.0);
            pair.tick(3);

            pair.reset();
            assert!(!pair.parameter.is_smoothing(), "{style:?}");
            assert_eq!(pair.parameter.smoothed(), pair.parameter.get(), "{style:?}");
            pair.tick(2);

            // A new rate with a pending change of the value.
            pair.parameter.set(9000.0);
            pair.set_sample_rate(48000.0);
            pair.tick(5);
            pair.set_sample_rate(0.0);
            pair.tick(5);
        }
    }

    #[test]
    fn test_smoothing_advances_through_a_shared_reference() {
        let parameter =
            FloatParameter::new("Test", 0.0, 0.0..=10.0).with_smoother(SmoothingStyle::Linear(10.0));
        let shared: &FloatParameter = &parameter;
        shared.set_sample_rate(1000.0);

        shared.set(10.0);
        assert_eq!(shared.smoothed(), 0.0);
        assert!((shared.tick_smoothed() - 1.0).abs() < 1e-12);
        shared.skip_smoothing(4);
        assert!((shared.smoothed() - 5.0).abs() < 1e-12);
        let mut rest = [0.0; 5];
        shared.fill_smoothed(&mut rest);
        assert_eq!(rest[4], 10.0);
        assert!(!shared.is_smoothing());
    }

    #[test]
    fn test_smoothing_style_none_returns_the_value() {
        let parameter =
            FloatParameter::new("Test", 0.0, 0.0..=10.0).with_smoother(SmoothingStyle::None);
        parameter.set_sample_rate(1000.0);
        parameter.set(7.0);

        assert_eq!(parameter.smoothed(), 7.0);
        assert_eq!(parameter.tick_smoothed(), 7.0);
        assert!(!parameter.is_smoothing());
    }

    #[test]
    fn test_snap_to_step_helper() {
        // Basic snapping
        assert_eq!(snap_to_step(2.3, 0.5, 1, 0.0, 10.0), 2.5);
        assert_eq!(snap_to_step(2.2, 0.5, 1, 0.0, 10.0), 2.0);

        // Negative range
        assert_eq!(snap_to_step(-5.3, 0.5, 1, -60.0, 12.0), -5.5);

        // Clamping
        assert_eq!(snap_to_step(-1.0, 0.5, 1, 0.0, 10.0), 0.0);
        assert_eq!(snap_to_step(11.0, 0.5, 1, 0.0, 10.0), 10.0);

        // The nearest f64 to the decimal value, not the product's error.
        assert_eq!(snap_to_step(0.31, 0.1, 1, 0.0, 1.0), 0.3);
    }

    #[test]
    fn test_a_stepped_frequency_reads_back_exactly_and_shows_its_step() {
        // A log mapping's round trip alone reads 439.99999999999983.
        let reference = FloatParameter::hz("Reference", 440.0, 400.0..=480.0).with_step_size(0.1);
        assert_eq!(reference.get(), 440.0);
        assert_eq!(reference.normalized_to_plain(0.0), 400.0);
        assert_eq!(reference.normalized_to_plain(1.0), 480.0);
        assert_eq!(reference.normalized_to_plain(reference.default_normalized()), 440.0);
        assert_eq!(reference.display_normalized(reference.get_normalized()), "440.0");
        reference.set(442.5);
        assert_eq!(reference.get(), 442.5);
        assert_eq!(reference.display_normalized(reference.get_normalized()), "442.5");
        // A host's value between two steps reads as the nearest step.
        reference.set_normalized(reference.plain_to_normalized(442.53) + 1e-9);
        assert_eq!(reference.get(), 442.5);
        // Typed text snaps.
        let typed = reference.parse("442.57 Hz").unwrap();
        assert_eq!(reference.normalized_to_plain(typed), 442.6);
    }

    #[test]
    fn test_a_part_of_the_range_moves_a_log_mapped_step_to_the_next() {
        // An arrow key adds 1/800 to the normalized value: 0.09 to 0.11 Hz
        // along the log mapping of 400 to 480 Hz.
        for start in [400.0, 470.0] {
            let reference = FloatParameter::hz("Reference", 440.0, 400.0..=480.0).with_step_size(0.1);
            reference.set(start);
            for press in 1..=30 {
                reference.set_normalized(reference.get_normalized() + 1.0 / 800.0);
                let expected = ((start + 0.1 * f64::from(press)) * 10.0).round() / 10.0;
                assert_eq!(reference.get(), expected.min(480.0), "press {press} from {start}");
            }
        }
    }

    #[test]
    fn test_a_step_from_a_value_is_one_step_in_plain_units() {
        let reference = FloatParameter::hz("Reference", 440.0, 400.0..=480.0).with_step_size(0.1);
        let up = reference.step_from(reference.get_normalized(), 1).unwrap();
        assert_eq!(reference.normalized_to_plain(up), 440.1);
        let down = reference.step_from(reference.get_normalized(), -10).unwrap();
        assert_eq!(reference.normalized_to_plain(down), 439.0);
        // From between two steps it counts from the nearest one.
        let between = reference.plain_to_normalized(442.53) + 1e-9;
        assert_eq!(reference.normalized_to_plain(reference.step_from(between, 1).unwrap()), 442.6);
        // It stays within the range, also from a maximum off the steps.
        assert_eq!(reference.normalized_to_plain(reference.step_from(1.0, 1).unwrap()), 480.0);
        let ragged = FloatParameter::new("Test", 0.95, 0.0..=0.95).with_step_size(0.1);
        assert_eq!(ragged.normalized_to_plain(ragged.step_from(1.0, -1).unwrap()), 0.9);
        assert_eq!(ragged.normalized_to_plain(ragged.step_from(0.9 / 0.95, 1).unwrap()), 0.95);
        // Down from a maximum below the half step: the highest step first.
        for (max, step, expected) in [(0.93, 0.1, [0.9, 0.8]), (10.25, 1.0, [10.0, 9.0])] {
            let parameter = FloatParameter::new("Test", 0.0, 0.0..=max).with_step_size(step);
            for (steps, expected) in [-1, -2].into_iter().zip(expected) {
                let down = parameter.step_from(1.0, steps).unwrap();
                assert_eq!(parameter.normalized_to_plain(down), expected, "{steps} from {max}");
            }
        }
        // A continuous parameter has no steps.
        assert_eq!(FloatParameter::new("Gain", 0.5, 0.0..=1.0).step_from(0.5, 1), None);
        assert_eq!(FloatParameter::new("Gain", 0.5, 0.0..=1.0).step_size(), None);
    }

    #[test]
    fn test_a_step_from_a_discrete_value_is_the_next_choice() {
        let steps = IntParameter::new("Steps", 0, 0..=4);
        assert_eq!(steps.step_from(0.3, 1), Some(0.5));
        assert_eq!(steps.step_from(0.3, -1), Some(0.0));
        assert_eq!(steps.step_from(1.0, 1), Some(1.0));
        assert_eq!(BoolParameter::new("Enabled", false).step_from(0.0, 1), Some(1.0));
        assert_eq!(ParameterRef::step_size(&steps), None);
    }

    #[test]
    fn test_the_steps_start_at_the_minimum() {
        // The steps are 0.05, 0.15, 0.25 and so on: two decimals.
        let parameter = FloatParameter::new("Test", 0.15, 0.05..=0.95).with_step_size(0.1);
        assert_eq!(parameter.get(), 0.15);
        assert_eq!(parameter.display_normalized(parameter.get_normalized()), "0.15");
        parameter.set(0.31);
        assert_eq!(parameter.get(), 0.35);
    }

    #[test]
    fn test_a_maximum_off_the_steps_shows_its_own_decimals() {
        // The steps are 0.0 to 0.9, the maximum 0.95: two decimals.
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=0.95).with_step_size(0.1);
        parameter.set(0.95);
        assert_eq!(parameter.get(), 0.95);
        assert_eq!(parameter.display_normalized(parameter.get_normalized()), "0.95");
        parameter.set(0.9);
        assert_eq!(parameter.display_normalized(parameter.get_normalized()), "0.90");
    }

    #[test]
    fn test_a_maximum_without_an_end_of_decimals_leaves_the_display_alone() {
        let angle = FloatParameter::new("Angle", 1.23, 0.0..=std::f64::consts::TAU).with_step_size(0.01);
        assert_eq!(angle.get(), 1.23);
        assert_eq!(angle.display_normalized(angle.get_normalized()), "1.23");
    }

    #[test]
    fn test_a_step_below_a_millionth_keeps_its_values() {
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=0.0001).with_step_size(5e-7);
        // 24.6 steps of 5e-7 snap to 25. Before, every value read 0.
        parameter.set(0.0000123);
        assert!((parameter.get() - 0.0000125).abs() < 1e-12, "{}", parameter.get());
    }

    #[test]
    fn test_a_step_that_is_no_decimal_is_not_rounded() {
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=1.0).with_step_size(1.0 / 3.0);
        parameter.set(0.3);
        assert_eq!(parameter.get(), 1.0 / 3.0);
    }

    #[test]
    fn test_the_display_follows_the_step() {
        let attack = FloatParameter::ms("Attack", 10.0, 1.0..=100.0).with_step_size(1.0);
        assert_eq!(attack.display_normalized(attack.get_normalized()), "10");
        let mix = FloatParameter::percent("Mix", 0.5, 0.0..=1.0).with_step_size(0.005);
        mix.set(0.125);
        assert_eq!(mix.display_normalized(mix.get_normalized()), "12.5");
        // A later precision wins.
        let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
            .with_step_size(0.5)
            .with_precision(2);
        gain.set(-5.3);
        assert_eq!(gain.display_normalized(gain.get_normalized()), "-5.50");
    }

    #[test]
    fn test_a_stepped_smoother_starts_and_ends_on_a_step() {
        let reference = FloatParameter::hz("Reference", 440.0, 400.0..=480.0)
            .with_smoother(SmoothingStyle::Linear(1.0))
            .with_step_size(0.1);
        reference.set_sample_rate(1000.0);
        assert_eq!(reference.smoothed(), 440.0);
        reference.set(441.0);
        for _ in 0..10 {
            reference.tick_smoothed();
        }
        assert_eq!(reference.tick_smoothed(), 441.0);
    }

    // =========================================================================
    // FloatParameter precision and formatter tests
    // =========================================================================

    #[test]
    fn test_float_parameter_with_precision_db() {
        let parameter = FloatParameter::db("Gain", 0.0, -60.0..=12.0).with_precision(2);

        // Check precision was updated
        assert_eq!(parameter.formatter().precision(), Some(2));

        // Check display uses new precision
        parameter.set(-6.5);
        let display = parameter.display();
        assert_eq!(display, "-6.50");
    }

    #[test]
    fn test_float_parameter_with_precision_ms() {
        let parameter = FloatParameter::ms("Attack", 10.0, 0.1..=100.0).with_precision(0);

        assert_eq!(parameter.formatter().precision(), Some(0));

        parameter.set(10.5);
        let display = parameter.display();
        assert_eq!(display, "10"); // Rounded to 0 decimal places
    }

    #[test]
    fn test_float_parameter_with_precision_percent() {
        let parameter = FloatParameter::percent("Mix", 0.5, 0.0..=1.0).with_precision(1);

        assert_eq!(parameter.formatter().precision(), Some(1));

        parameter.set(0.755);
        let display = parameter.display();
        assert_eq!(display, "75.5");
    }

    #[test]
    fn test_float_parameter_with_precision_pan_no_effect() {
        let parameter = FloatParameter::pan("Pan", 0.0, -1.0..=1.0).with_precision(3);

        // Pan formatter doesn't support precision, should remain unchanged
        assert_eq!(parameter.formatter().precision(), None);
        assert!(!parameter.formatter().supports_precision());
    }

    #[test]
    fn test_float_parameter_with_formatter_replace() {
        let parameter = FloatParameter::new("Ratio", 4.0, 1.0..=20.0)
            .with_formatter(Formatter::Ratio { precision: 1 });

        // Ratio uses empty unit (suffix is ":1").
        assert_eq!(parameter.info().units, "");
        assert_eq!(parameter.formatter().precision(), Some(1));

        parameter.set(4.0);
        let display = parameter.display();
        assert_eq!(display, "4.0:1");
    }

    #[test]
    fn test_float_parameter_with_formatter_updates_units() {
        // Start with a generic float parameter (no units)
        let parameter =
            FloatParameter::new("Time", 100.0, 1.0..=1000.0).with_formatter(Formatter::Milliseconds {
                precision: 1,
            });

        // Units should be updated to match the new formatter
        assert_eq!(parameter.info().units, "ms");
    }

    #[test]
    fn test_float_parameter_with_formatter_preserves_is_db() {
        // Create a dB parameter, then change formatter
        let parameter = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
            .with_formatter(Formatter::Float { precision: 2 });

        // Even though we changed the formatter, as_linear() should still work
        // because is_db is based on how the parameter was constructed
        parameter.set(0.0);
        // 0 dB = 1.0 linear.
        assert!((parameter.as_linear() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_float_parameter_formatter_getter() {
        let parameter = FloatParameter::db("Gain", 0.0, -60.0..=12.0);

        // Check we can access the formatter
        let formatter = parameter.formatter();
        assert!(formatter.supports_precision());
        assert_eq!(formatter.unit(), "dB");
    }

    #[test]
    fn test_float_parameter_chained_precision_and_step_size() {
        let parameter = FloatParameter::db("Volume", 0.0, -60.0..=12.0)
            .with_step_size(0.5)
            .with_precision(2);

        // Both should work together.
        assert_eq!(parameter.formatter().precision(), Some(2));

        parameter.set(-5.3);
        // Snapped.
        assert!((parameter.get() - -5.5).abs() < 1e-10);
        let display = parameter.display();
        // High precision.
        assert_eq!(display, "-5.50");
    }

    // =========================================================================
    // IntParameter precision and formatter tests
    // =========================================================================

    #[test]
    fn test_int_parameter_with_precision() {
        // IntParameter uses Float { precision: 0 } by default
        let parameter = IntParameter::new("Value", 0, -100..=100).with_precision(0);

        assert_eq!(parameter.formatter().precision(), Some(0));
    }

    #[test]
    fn test_int_parameter_with_formatter() {
        let parameter =
            IntParameter::new("Transpose", 0, -24..=24).with_formatter(Formatter::Semitones);

        assert_eq!(parameter.info().units, "st");

        parameter.set(12);
        let display = parameter.display_normalized(parameter.get_normalized());
        assert_eq!(display, "+12");

        parameter.set(-7);
        let display = parameter.display_normalized(parameter.get_normalized());
        assert_eq!(display, "-7");
    }

    #[test]
    fn test_int_parameter_formatter_getter() {
        let parameter = IntParameter::semitones("Pitch", 0, -24..=24);

        let formatter = parameter.formatter();
        assert_eq!(formatter.unit(), "st");
        assert!(!formatter.supports_precision()); // Semitones doesn't have precision
    }

    // =========================================================================
    // ParameterUnit tests
    // =========================================================================

    #[test]
    fn test_float_parameter_unit_generic() {
        let parameter = FloatParameter::new("Test", 0.0, 0.0..=1.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Generic);
    }

    #[test]
    fn test_float_parameter_unit_decibels() {
        let parameter = FloatParameter::db("Gain", 0.0, -60.0..=12.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Decibels);
    }

    #[test]
    fn test_float_parameter_unit_db_log() {
        let parameter = FloatParameter::db_log("Threshold", -20.0, -60.0..=0.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Decibels);
    }

    #[test]
    fn test_float_parameter_unit_db_log_offset() {
        let parameter = FloatParameter::db_log_offset("Threshold", -20.0, -60.0..=0.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Decibels);
    }

    #[test]
    fn test_float_parameter_unit_hertz() {
        let parameter = FloatParameter::hz("Frequency", 440.0, 20.0..=20000.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Hertz);
    }

    #[test]
    fn test_float_parameter_unit_milliseconds() {
        let parameter = FloatParameter::ms("Attack", 10.0, 0.1..=100.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Milliseconds);
    }

    #[test]
    fn test_float_parameter_unit_seconds() {
        let parameter = FloatParameter::seconds("Decay", 1.0, 0.0..=10.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Seconds);
    }

    #[test]
    fn test_float_parameter_unit_percent() {
        let parameter = FloatParameter::percent("Mix", 0.5, 0.0..=1.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Percent);
    }

    #[test]
    fn test_float_parameter_unit_pan() {
        let parameter = FloatParameter::pan("Pan", 0.0, -1.0..=1.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Pan);
    }

    #[test]
    fn test_float_parameter_unit_ratio() {
        let parameter = FloatParameter::ratio("Ratio", 4.0, 1.0..=20.0);
        assert_eq!(parameter.info().unit, ParameterUnit::Ratio);
    }

    #[test]
    fn test_int_parameter_unit_generic() {
        let parameter = IntParameter::new("Value", 0, -100..=100);
        assert_eq!(parameter.info().unit, ParameterUnit::Generic);
    }

    #[test]
    fn test_int_parameter_unit_semitones() {
        let parameter = IntParameter::semitones("Transpose", 0, -24..=24);
        assert_eq!(parameter.info().unit, ParameterUnit::RelativeSemiTones);
    }

    #[test]
    fn test_bool_parameter_unit_boolean() {
        let parameter = BoolParameter::new("Enabled", true);
        assert_eq!(parameter.info().unit, ParameterUnit::Boolean);
    }

    #[test]
    fn test_bool_parameter_bypass_unit_boolean() {
        let parameter = BoolParameter::bypass();
        assert_eq!(parameter.info().unit, ParameterUnit::Boolean);
    }

    #[test]
    fn test_float_parameter_with_unit_override() {
        let parameter = FloatParameter::new("Custom", 0.5, 0.0..=1.0)
            .with_unit(ParameterUnit::Percent);
        assert_eq!(parameter.info().unit, ParameterUnit::Percent);
    }

    #[test]
    fn test_bool_parameter_formatter_kind() {
        assert_eq!(BoolParameter::new("Enabled", true).formatter_kind(), "boolean");
        assert_eq!(BoolParameter::bypass().formatter_kind(), "boolean");
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

        let parameter = EnumParameter::<TestEnum>::new("Mode");
        assert_eq!(parameter.info().unit, ParameterUnit::Indexed);
        assert_eq!(parameter.formatter_kind(), "enum");

        // A name in any case, or an index, also with spaces around it and
        // written as a float.
        let b = parameter.plain_to_normalized(1.0);
        assert_eq!(parameter.parse("b"), Some(b));
        assert_eq!(parameter.parse(" B "), Some(b));
        assert_eq!(parameter.parse("1"), Some(b));
        assert_eq!(parameter.parse(" 1 "), Some(b));
        assert_eq!(parameter.parse("1.0"), Some(b));
        for text in ["1.5", "3", "-1", "nan", "inf", "D", ""] {
            assert_eq!(parameter.parse(text), None, "\"{}\"", text);
        }

        assert_stores_as_set(&parameter);
        assert!(parameter.is_discrete());
    }

    /// Check that `stored_normalized` gives what the parameter returns after
    /// `set_normalized`, for values on, between and outside its steps.
    fn assert_stores_as_set(parameter: &dyn ParameterRef) {
        for value in [-0.5, 0.0, 0.1, 0.25, 0.3, 0.5, 0.51, 0.7, 0.75, 1.0, 1.5] {
            parameter.set_normalized(value);
            assert_eq!(parameter.stored_normalized(value), parameter.get_normalized(), "{}", value);
        }
    }

    #[test]
    fn test_stored_normalized_is_what_set_normalized_stores() {
        assert_stores_as_set(&FloatParameter::new("Gain", 0.5, 0.0..=1.0));
        assert_stores_as_set(&IntParameter::new("Steps", 0, 0..=4));
        assert_stores_as_set(&IntParameter::new("Transpose", 0, -24..=24));
        assert_stores_as_set(&BoolParameter::new("Enabled", false));
        assert_stores_as_set(&FloatParameter::new("Gain", 0.5, 0.0..=1.0).with_step_size(0.25));

        // A float with steps stores the nearest step, but goes between
        // steps with its smoother.
        let stepped = FloatParameter::new("Gain", 0.5, 0.0..=1.0).with_step_size(0.25);
        assert_eq!(stepped.stored_normalized(0.3), 0.25);
        assert!(!stepped.is_discrete());
        assert!(IntParameter::new("Steps", 0, 0..=4).is_discrete());
        assert!(BoolParameter::new("Enabled", false).is_discrete());

        // A value between two steps stores the nearest step.
        let steps = IntParameter::new("Steps", 0, 0..=4);
        assert_eq!(steps.stored_normalized(0.3), 0.25);
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
