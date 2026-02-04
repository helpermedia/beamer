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

// SAFETY: NoPresets contains only PhantomData<P> which is always Send + Sync.
unsafe impl<P> Send for NoPresets<P> {}
// SAFETY: NoPresets contains only PhantomData<P> which is always Send + Sync.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameter_groups::{GroupInfo, ParameterGroups};
    use crate::parameter_info::{ParameterFlags, ParameterInfo, ParameterUnit};
    use crate::parameter_types::{ParameterRef, Parameters};
    use std::sync::atomic::{AtomicU64, Ordering};

    // =========================================================================
    // Mock Parameter for testing
    // =========================================================================

    /// A minimal mock parameter for testing preset application.
    struct MockParameter {
        id: ParameterId,
        name: &'static str,
        value: AtomicU64,
        info: ParameterInfo,
    }

    impl MockParameter {
        fn new(id: ParameterId, name: &'static str) -> Self {
            Self {
                id,
                name,
                value: AtomicU64::new(0.0f64.to_bits()),
                info: ParameterInfo {
                    id,
                    name,
                    short_name: name,
                    units: "",
                    unit: ParameterUnit::Generic,
                    step_count: 0,
                    default_normalized: 0.0,
                    flags: ParameterFlags::default(),
                    group_id: 0,
                },
            }
        }

        fn get_value(&self) -> f64 {
            f64::from_bits(self.value.load(Ordering::Relaxed))
        }
    }

    impl ParameterRef for MockParameter {
        fn id(&self) -> ParameterId {
            self.id
        }

        fn name(&self) -> &'static str {
            self.name
        }

        fn short_name(&self) -> &'static str {
            self.name
        }

        fn units(&self) -> &'static str {
            ""
        }

        fn flags(&self) -> &ParameterFlags {
            &self.info.flags
        }

        fn default_normalized(&self) -> f64 {
            0.0
        }

        fn step_count(&self) -> i32 {
            0
        }

        fn get_normalized(&self) -> f64 {
            self.get_value()
        }

        fn set_normalized(&self, value: f64) {
            self.value.store(value.to_bits(), Ordering::Relaxed);
        }

        fn get_plain(&self) -> f64 {
            // For simplicity, plain = normalized in mock
            self.get_normalized()
        }

        fn set_plain(&self, value: f64) {
            self.set_normalized(value);
        }

        fn display_normalized(&self, normalized: f64) -> String {
            format!("{:.2}", normalized)
        }

        fn parse(&self, s: &str) -> Option<f64> {
            s.parse().ok()
        }

        fn normalized_to_plain(&self, normalized: f64) -> f64 {
            // 1:1 mapping for simplicity
            normalized
        }

        fn plain_to_normalized(&self, plain: f64) -> f64 {
            // 1:1 mapping for simplicity
            plain
        }

        fn info(&self) -> &ParameterInfo {
            &self.info
        }
    }

    // =========================================================================
    // Mock Parameters Collection
    // =========================================================================

    /// A simple parameter collection with two parameters for testing.
    struct MockParameters {
        gain: MockParameter,
        mix: MockParameter,
    }

    impl MockParameters {
        fn new() -> Self {
            Self {
                gain: MockParameter::new(fnv1a_hash("gain"), "Gain"),
                mix: MockParameter::new(fnv1a_hash("mix"), "Mix"),
            }
        }
    }

    impl ParameterGroups for MockParameters {
        fn group_count(&self) -> usize {
            1
        }

        fn group_info(&self, index: usize) -> Option<GroupInfo> {
            if index == 0 {
                Some(GroupInfo::root())
            } else {
                None
            }
        }
    }

    impl Parameters for MockParameters {
        fn count(&self) -> usize {
            2
        }

        fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
            Box::new(
                [&self.gain as &dyn ParameterRef, &self.mix as &dyn ParameterRef].into_iter(),
            )
        }

        fn by_id(&self, id: ParameterId) -> Option<&dyn ParameterRef> {
            if id == self.gain.id {
                Some(&self.gain)
            } else if id == self.mix.id {
                Some(&self.mix)
            } else {
                None
            }
        }
    }

    // =========================================================================
    // Test Preset Implementation
    // =========================================================================

    /// A manual implementation of `FactoryPresets` for testing.
    struct TestPresets;

    const TEST_PRESET_VALUES_0: &[PresetValue] = &[
        PresetValue {
            id: fnv1a_hash("gain"),
            plain_value: 0.5,
        },
        PresetValue {
            id: fnv1a_hash("mix"),
            plain_value: 1.0,
        },
    ];

    const TEST_PRESET_VALUES_1: &[PresetValue] = &[PresetValue {
        id: fnv1a_hash("gain"),
        plain_value: 0.0,
    }];

    impl FactoryPresets for TestPresets {
        type Parameters = MockParameters;

        fn count() -> usize {
            2
        }

        fn info(index: usize) -> Option<PresetInfo> {
            match index {
                0 => Some(PresetInfo { name: "Full Mix" }),
                1 => Some(PresetInfo { name: "Silent" }),
                _ => None,
            }
        }

        fn values(index: usize) -> &'static [PresetValue] {
            match index {
                0 => TEST_PRESET_VALUES_0,
                1 => TEST_PRESET_VALUES_1,
                _ => &[],
            }
        }
    }

    // =========================================================================
    // NoPresets Tests
    // =========================================================================

    #[test]
    fn no_presets_count_returns_zero() {
        assert_eq!(NoPresets::<MockParameters>::count(), 0);
    }

    #[test]
    fn no_presets_info_returns_none() {
        assert!(NoPresets::<MockParameters>::info(0).is_none());
        assert!(NoPresets::<MockParameters>::info(1).is_none());
        assert!(NoPresets::<MockParameters>::info(usize::MAX).is_none());
    }

    #[test]
    fn no_presets_values_returns_empty_slice() {
        assert!(NoPresets::<MockParameters>::values(0).is_empty());
        assert!(NoPresets::<MockParameters>::values(1).is_empty());
    }

    #[test]
    fn no_presets_apply_returns_false() {
        let params = MockParameters::new();
        assert!(!NoPresets::<MockParameters>::apply(0, &params));
        assert!(!NoPresets::<MockParameters>::apply(1, &params));
        assert!(!NoPresets::<MockParameters>::apply(usize::MAX, &params));
    }

    // =========================================================================
    // PresetInfo Tests
    // =========================================================================

    #[test]
    fn preset_info_can_be_created_with_name() {
        let info = PresetInfo { name: "My Preset" };
        assert_eq!(info.name, "My Preset");
    }

    #[test]
    fn preset_info_supports_empty_name() {
        let info = PresetInfo { name: "" };
        assert_eq!(info.name, "");
    }

    #[test]
    fn preset_info_is_copy() {
        let info = PresetInfo { name: "Test" };
        let info2 = info; // Copy
        assert_eq!(info.name, info2.name);
    }

    #[test]
    fn preset_info_is_clone() {
        let info = PresetInfo { name: "Test" };
        // Use Clone::clone explicitly to test Clone trait, not Copy
        let info2 = Clone::clone(&info);
        assert_eq!(info.name, info2.name);
    }

    // =========================================================================
    // PresetValue Tests
    // =========================================================================

    #[test]
    fn preset_value_stores_id_and_plain_value() {
        let value = PresetValue {
            id: 12345,
            plain_value: 0.75,
        };
        assert_eq!(value.id, 12345);
        assert!((value.plain_value - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn preset_value_is_copy() {
        let value = PresetValue {
            id: 100,
            plain_value: 0.5,
        };
        let value2 = value; // Copy
        assert_eq!(value.id, value2.id);
        assert!((value.plain_value - value2.plain_value).abs() < f64::EPSILON);
    }

    // =========================================================================
    // FactoryPresets Trait Tests (via TestPresets)
    // =========================================================================

    #[test]
    fn test_presets_count() {
        assert_eq!(TestPresets::count(), 2);
    }

    #[test]
    fn test_presets_info_valid_index() {
        let info0 = TestPresets::info(0);
        assert!(info0.is_some());
        assert_eq!(info0.unwrap().name, "Full Mix");

        let info1 = TestPresets::info(1);
        assert!(info1.is_some());
        assert_eq!(info1.unwrap().name, "Silent");
    }

    #[test]
    fn test_presets_info_invalid_index() {
        assert!(TestPresets::info(2).is_none());
        assert!(TestPresets::info(usize::MAX).is_none());
    }

    #[test]
    fn test_presets_values_valid_index() {
        let values0 = TestPresets::values(0);
        assert_eq!(values0.len(), 2);
        assert_eq!(values0[0].id, fnv1a_hash("gain"));
        assert!((values0[0].plain_value - 0.5).abs() < f64::EPSILON);
        assert_eq!(values0[1].id, fnv1a_hash("mix"));
        assert!((values0[1].plain_value - 1.0).abs() < f64::EPSILON);

        let values1 = TestPresets::values(1);
        assert_eq!(values1.len(), 1);
        assert_eq!(values1[0].id, fnv1a_hash("gain"));
        assert!((values1[0].plain_value - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_presets_values_invalid_index() {
        assert!(TestPresets::values(2).is_empty());
        assert!(TestPresets::values(usize::MAX).is_empty());
    }

    #[test]
    fn test_presets_apply_sets_parameters() {
        let params = MockParameters::new();

        // Initial values should be 0.0
        assert!((params.gain.get_value() - 0.0).abs() < f64::EPSILON);
        assert!((params.mix.get_value() - 0.0).abs() < f64::EPSILON);

        // Apply preset 0 (Full Mix: gain=0.5, mix=1.0)
        let result = TestPresets::apply(0, &params);
        assert!(result);
        assert!((params.gain.get_value() - 0.5).abs() < f64::EPSILON);
        assert!((params.mix.get_value() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_presets_apply_sparse_preset() {
        let params = MockParameters::new();

        // Set initial values
        params.gain.set_normalized(0.75);
        params.mix.set_normalized(0.5);

        // Apply preset 1 (Silent: only gain=0.0)
        // Mix should remain at 0.5 (sparse preset)
        let result = TestPresets::apply(1, &params);
        assert!(result);
        assert!((params.gain.get_value() - 0.0).abs() < f64::EPSILON);
        assert!((params.mix.get_value() - 0.5).abs() < f64::EPSILON); // Unchanged
    }

    #[test]
    fn test_presets_apply_invalid_index_returns_false() {
        let params = MockParameters::new();

        // Set initial values
        params.gain.set_normalized(0.75);
        params.mix.set_normalized(0.5);

        // Apply invalid preset
        let result = TestPresets::apply(2, &params);
        assert!(!result);

        // Values should remain unchanged
        assert!((params.gain.get_value() - 0.75).abs() < f64::EPSILON);
        assert!((params.mix.get_value() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_presets_apply_with_unknown_parameter_id() {
        // Test that preset values with unknown parameter IDs are silently ignored
        struct PresetsWithUnknownParam;

        const UNKNOWN_PARAM_VALUES: &[PresetValue] = &[
            PresetValue {
                id: fnv1a_hash("gain"),
                plain_value: 0.5,
            },
            PresetValue {
                id: fnv1a_hash("unknown"),
                plain_value: 0.9,
            },
        ];

        impl FactoryPresets for PresetsWithUnknownParam {
            type Parameters = MockParameters;

            fn count() -> usize {
                1
            }

            fn info(index: usize) -> Option<PresetInfo> {
                if index == 0 {
                    Some(PresetInfo { name: "Test" })
                } else {
                    None
                }
            }

            fn values(index: usize) -> &'static [PresetValue] {
                if index == 0 {
                    UNKNOWN_PARAM_VALUES
                } else {
                    &[]
                }
            }
        }

        let params = MockParameters::new();

        // Apply preset with unknown parameter ID - should succeed and apply known params
        let result = PresetsWithUnknownParam::apply(0, &params);
        assert!(result);
        assert!((params.gain.get_value() - 0.5).abs() < f64::EPSILON);
    }

    // =========================================================================
    // fnv1a_hash Tests
    // =========================================================================

    #[test]
    fn fnv1a_hash_produces_consistent_values() {
        // Same input should produce same hash
        assert_eq!(fnv1a_hash("gain"), fnv1a_hash("gain"));
        assert_eq!(fnv1a_hash("mix"), fnv1a_hash("mix"));
    }

    #[test]
    fn fnv1a_hash_different_inputs_produce_different_hashes() {
        // Different inputs should (usually) produce different hashes
        assert_ne!(fnv1a_hash("gain"), fnv1a_hash("mix"));
        assert_ne!(fnv1a_hash("a"), fnv1a_hash("b"));
    }

    #[test]
    fn fnv1a_hash_empty_string() {
        // Empty string should produce a consistent hash (the offset basis)
        let hash = fnv1a_hash("");
        assert_eq!(hash, 2166136261); // FNV offset basis
    }

    #[test]
    fn fnv1a_hash_is_const() {
        // Verify hash can be used in const context
        const GAIN_HASH: u32 = fnv1a_hash("gain");
        assert_eq!(GAIN_HASH, fnv1a_hash("gain"));
    }
}
