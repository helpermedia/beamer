//! Factory preset support for Beamer plugins.
//!
//! This module provides types for defining factory presets - collections of
//! predefined parameter values that users can select from their DAW's preset browser.
//!
//! # Design
//!
//! Presets are opt-in via the [`FactoryPresets`] trait. Plugins that don't need
//! presets use the default [`NoPresets`] marker type, which provides an empty
//! preset list.
//!
//! # Sparse Presets
//!
//! Presets can be "sparse" - they only specify values for some parameters.
//! When a sparse preset is applied, unspecified parameters keep their current
//! values (they are NOT reset to defaults).
//!
//! # Example
//!
//! ```ignore
//! use beamer_macros::Presets;
//!
//! #[derive(Presets)]
//! #[preset(parameters = GainParameters)]
//! pub enum GainPresets {
//!     #[preset(name = "Unity", values(gain = 0.0))]
//!     Unity,
//!
//!     #[preset(name = "Quiet", values(gain = -12.0))]
//!     Quiet,
//!
//!     #[preset(name = "Boost", values(gain = 6.0))]
//!     Boost,
//! }
//! ```

use std::marker::PhantomData;

use crate::parameter_types::Parameters;
use crate::types::ParameterId;

/// Information about a single preset.
#[derive(Debug, Clone, Copy)]
pub struct PresetInfo {
    /// Display name shown in the DAW's preset browser.
    pub name: &'static str,
}

/// A single parameter value within a preset.
///
/// The parameter is identified by its hash ID (computed at compile time
/// by the derive macro from the string ID).
#[derive(Debug, Clone, Copy)]
pub struct PresetValue {
    /// Parameter hash ID (FNV-1a hash of the string ID).
    pub id: ParameterId,
    /// Plain value in natural units (e.g., dB, Hz, ms).
    /// Converted to normalized at apply time using the parameter's range.
    pub plain_value: f64,
}

/// Trait for factory preset collections.
///
/// This trait is typically implemented via the `#[derive(Presets)]` macro.
/// The trait provides methods to enumerate presets and apply them to a
/// parameter collection.
///
/// # Type Parameter
///
/// The `Parameters` associated type must match the plugin's parameter struct.
/// This ensures type safety when applying presets.
pub trait FactoryPresets: Send + Sync + 'static {
    /// The parameter struct type this preset collection applies to.
    type Parameters: Parameters;

    /// Returns the total number of factory presets.
    fn count() -> usize;

    /// Returns information about a preset at the given index.
    ///
    /// Returns `None` if `index >= count()`.
    fn info(index: usize) -> Option<PresetInfo>;

    /// Returns the parameter values for a preset at the given index.
    ///
    /// Returns an empty slice if `index >= count()`.
    /// Each value contains a parameter hash ID and its plain value.
    fn values(index: usize) -> &'static [PresetValue];

    /// Applies a preset to the given parameters.
    ///
    /// Only the parameters specified in the preset are modified.
    /// Other parameters keep their current values. Plain values are
    /// converted to normalized using each parameter's range.
    ///
    /// Returns `true` if the preset was applied successfully,
    /// `false` if the index was out of range.
    fn apply(index: usize, parameters: &Self::Parameters) -> bool {
        if index >= Self::count() {
            return false;
        }

        let values = Self::values(index);
        for value in values {
            if let Some(param) = parameters.by_id(value.id) {
                let normalized = param.plain_to_normalized(value.plain_value);
                param.set_normalized(normalized);
            }
        }

        true
    }
}

/// Default implementation for plugins without factory presets.
///
/// This type provides an empty preset list and is used as the default
/// when a plugin doesn't define any presets.
pub struct NoPresets<P>(PhantomData<P>);

impl<P: Parameters + 'static> FactoryPresets for NoPresets<P> {
    type Parameters = P;

    fn count() -> usize {
        0
    }

    fn info(_index: usize) -> Option<PresetInfo> {
        None
    }

    fn values(_index: usize) -> &'static [PresetValue] {
        &[]
    }
}

// Ensure NoPresets is Send + Sync
unsafe impl<P> Send for NoPresets<P> {}
unsafe impl<P> Sync for NoPresets<P> {}

/// Compute the FNV-1a hash of a string at compile time.
///
/// This is the same hash algorithm used by the Parameters derive macro
/// for parameter IDs. Use this to compute preset parameter IDs.
pub const fn fnv1a_hash(s: &str) -> u32 {
    const FNV_OFFSET_BASIS: u32 = 2166136261;
    const FNV_PRIME: u32 = 16777619;

    let bytes = s.as_bytes();
    let mut hash = FNV_OFFSET_BASIS;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}
