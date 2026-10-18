//! Low-level parameter store for host communication.
//!
//! This module provides the [`ParameterStore`] trait for direct host communication.
//! It exposes the raw normalized value interface that hosts expect.
//!
//! # How the Traits Work Together
//!
//! A model and each nested group in it implement three traits:
//!
//! - **[`Parameters`]**: the parameters themselves, with type-erased
//!   iteration, ids, groups and smoothing, for parameter types like
//!   `FloatParameter`, `IntParameter` and `BoolParameter`.
//!
//! - **[`Saved`](crate::model::Saved)**: the `State` fields beside the
//!   parameters, and saving and loading the parameters and `State` fields
//!   together.
//!
//! - **[`WebViewEvents`](crate::model::WebViewEvents)**: where the event
//!   queue of the web UI is.
//!
//! For most plugins, use `#[beamer::model]` (and `#[beamer::group]` for
//! nested groups), which implements all three.
//!
//! Every [`Parameters`] is also a [`ParameterStore`]: the parameters as the
//! host sees them, with raw access to normalized values and parameter
//! metadata. A store without parameter objects, such as the MIDI CC
//! parameters of the VST3 wrapper, implements [`ParameterStore`] directly.
//!
//! # Thread Safety
//!
//! The [`ParameterStore`] trait requires `Send + Sync` because parameters may be
//! accessed from multiple threads:
//! - Audio thread: reads parameter values during processing
//! - UI thread: displays and modifies parameter values
//! - Host thread: automation playback and recording
//!
//! Use atomic types (e.g., `AtomicU64` with `to_bits`/`from_bits`) for lock-free access.

use crate::parameter_groups::ParameterGroups;
use crate::parameter_info::ParameterInfo;
use crate::parameter_types::Parameters;
use crate::types::{ParameterId, ParameterValue};

/// Low-level trait for plugin parameter collections (host interface).
///
/// The format wrappers (AU, VST3) use this to communicate parameter
/// information and values to the host. Every [`Parameters`] implements it
/// through its parameter objects. Implement it directly only for a store
/// that keeps values without parameter objects.
///
/// # Example
///
/// ```ignore
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use beamer_core::{ParameterStore, ParameterInfo, ParameterId, ParameterValue};
///
/// pub struct MyStore {
///     gain: AtomicU64,
///     gain_info: ParameterInfo,
/// }
///
/// impl ParameterStore for MyStore {
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
pub trait ParameterStore: Send + Sync {
    /// Returns the number of parameters.
    fn count(&self) -> usize;

    /// Returns parameter info by index (0 to count-1).
    ///
    /// Returns `None` if index is out of bounds. For listing the parameters
    /// on host and editor threads: the implementation of every
    /// [`Parameters`] walks its parameters and allocates. Use
    /// [`info_by_id`](Self::info_by_id) to look one up.
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

    /// Get the formatter kind string for a parameter (e.g. "db", "pan", "hz").
    ///
    /// Used by the webview IPC protocol so JavaScript can implement custom
    /// formatting. Returns "float" by default.
    fn formatter_kind(&self, _id: ParameterId) -> &'static str {
        "float"
    }

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

/// Every parameter collection is a parameter store: the host view looks the
/// parameters up through [`Parameters`].
impl<T: Parameters> ParameterStore for T {
    fn count(&self) -> usize {
        Parameters::count(self)
    }

    // Builds the iterator, so it allocates: for the host and editor threads,
    // which list the parameters by index.
    fn info(&self, index: usize) -> Option<&ParameterInfo> {
        self.iter().nth(index).map(|parameter| parameter.info())
    }

    fn get_normalized(&self, id: ParameterId) -> ParameterValue {
        self.by_id(id).map_or(0.0, |parameter| parameter.get_normalized())
    }

    fn set_normalized(&self, id: ParameterId, value: ParameterValue) {
        if let Some(parameter) = self.by_id(id) {
            parameter.set_normalized(value);
        }
    }

    fn normalized_to_string(&self, id: ParameterId, normalized: ParameterValue) -> String {
        self.by_id(id)
            .map(|parameter| parameter.display_normalized(normalized))
            .unwrap_or_default()
    }

    fn string_to_normalized(&self, id: ParameterId, string: &str) -> Option<ParameterValue> {
        self.by_id(id).and_then(|parameter| parameter.parse(string))
    }

    fn normalized_to_plain(&self, id: ParameterId, normalized: ParameterValue) -> ParameterValue {
        self.by_id(id)
            .map_or(0.0, |parameter| parameter.normalized_to_plain(normalized))
    }

    fn plain_to_normalized(&self, id: ParameterId, plain: ParameterValue) -> ParameterValue {
        self.by_id(id)
            .map_or(0.0, |parameter| parameter.plain_to_normalized(plain))
    }

    fn formatter_kind(&self, id: ParameterId) -> &'static str {
        self.by_id(id)
            .map_or("float", |parameter| parameter.formatter_kind())
    }

    // The AU render thread converts the values of parameter events through
    // this, so it must not allocate: by_id compares ids without one.
    fn info_by_id(&self, id: ParameterId) -> Option<&ParameterInfo> {
        self.by_id(id).map(|parameter| parameter.info())
    }
}

/// Empty parameter collection for plugins with no parameters.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoParameters;

impl ParameterGroups for NoParameters {}

impl Parameters for NoParameters {
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

impl crate::model::Saved for NoParameters {}

impl crate::model::WebViewEvents for NoParameters {}

/// Build a JSON array of parameter info for WebView init dumps.
///
/// Returns a JSON string like `[{"id":0,"stringId":"gain",...}, ...]`.
/// Used by both VST3 and AU format wrappers to send the initial
/// parameter state to the JavaScript runtime.
pub fn params_to_init_json(store: &dyn ParameterStore) -> String {
    let entries: Vec<ParamInitEntry> = (0..store.count())
        .filter_map(|i| {
            let info = store.info(i)?;
            let normalized = store.get_normalized(info.id);
            Some(ParamInitEntry {
                id: info.id,
                string_id: &info.string_id,
                name: info.name,
                min: store.normalized_to_plain(info.id, 0.0),
                max: store.normalized_to_plain(info.id, 1.0),
                default_value: info.default_normalized,
                value: normalized,
                plain_value: store.normalized_to_plain(info.id, normalized),
                display_text: store.normalized_to_string(info.id, normalized),
                format: store.formatter_kind(info.id),
                units: info.units,
                steps: info.step_count,
            })
        })
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ParamInitEntry<'a> {
    id: u32,
    string_id: &'a str,
    name: &'static str,
    min: f64,
    max: f64,
    default_value: f64,
    value: f64,
    plain_value: f64,
    display_text: String,
    format: &'static str,
    units: &'static str,
    steps: i32,
}
