//! Low-level parameter store for host communication.
//!
//! This module provides the [`ParameterStore`] trait for direct host communication.
//! It exposes the raw normalized value interface that hosts expect.
//!
//! # Choosing Between `Parameters` and `ParameterStore`
//!
//! Beamer provides two parameter traits that work together:
//!
//! - **[`Parameters`](crate::parameter_types::Parameters)** (recommended): High-level trait with
//!   type-erased iteration, automatic state serialization and support for parameter
//!   types like `FloatParameter`, `IntParameter` and `BoolParameter`. Use `#[derive(Parameters)]`
//!   for automatic implementation.
//!
//! - **[`ParameterStore`]**: Low-level trait for direct host communication. Provides
//!   raw access to normalized values and parameter metadata. Useful when you need
//!   fine-grained control over parameter handling or are building custom parameter
//!   systems.
//!
//! For most plugins, use `#[derive(Parameters)]` which automatically implements both traits.
//! The `Parameters` trait builds on top of `ParameterStore` to provide a more ergonomic API.
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
use crate::types::{ParameterId, ParameterValue};

/// Low-level trait for plugin parameter collections (host interface).
///
/// Implement this trait to declare your plugin's parameters. The format wrappers
/// (AU, VST3) use this to communicate parameter information and values to the host.
///
/// # Example
///
/// ```ignore
/// use std::sync::atomic::{AtomicU64, Ordering};
/// use beamer_core::{ParameterStore, ParameterInfo, ParameterId, ParameterValue};
///
/// pub struct MyParameters {
///     gain: AtomicU64,
///     gain_info: ParameterInfo,
/// }
///
/// impl ParameterStore for MyParameters {
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

impl ParameterGroups for NoParameters {}

impl ParameterStore for NoParameters {
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

/// Build a JSON array of parameter info for WebView init dumps.
///
/// Returns a JSON string like `[{"id":0,"stringId":"gain",...}, ...]`.
/// Used by both VST3 and AU format wrappers to send the initial
/// parameter state to the JavaScript runtime.
pub fn params_to_init_json(store: &dyn ParameterStore) -> String {
    let entries: Vec<ParamInitEntry> = (0..store.count())
        .filter_map(|i| {
            let info = store.info(i)?;
            Some(ParamInitEntry {
                id: info.id,
                string_id: info.string_id,
                name: info.name,
                min: store.normalized_to_plain(info.id, 0.0),
                max: store.normalized_to_plain(info.id, 1.0),
                default_value: info.default_normalized,
                value: store.get_normalized(info.id),
                units: info.units,
                steps: info.step_count,
            })
        })
        .collect();
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ParamInitEntry {
    id: u32,
    string_id: &'static str,
    name: &'static str,
    min: f64,
    max: f64,
    default_value: f64,
    value: f64,
    units: &'static str,
    steps: i32,
}
