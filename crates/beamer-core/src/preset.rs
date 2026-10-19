//! Factory presets of Beamer plugins.
//!
//! A factory preset is a name and a function that sets parameters of the
//! model. A plugin lists its presets in [`Model::PRESETS`](crate::Model::PRESETS):
//!
//! ```ignore
//! #[beamer::export]
//! #[beamer::model]
//! pub struct GainModel {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     pub gain: FloatParameter,
//! }
//!
//! impl Model for GainModel {
//!     type Setup = ();
//!     type Processor = GainProcessor;
//!     const PRESETS: &'static [Preset<Self>] = &[
//!         Preset::new("Unity", |m| m.gain.set(0.0)),
//!         Preset::new("Quiet", |m| m.gain.set(-12.0)),
//!         Preset::new("Boost", |m| m.gain.set(6.0)),
//!     ];
//!     // ...
//! }
//! ```
//!
//! Hosts list the presets in this order and select one by its number. The
//! AU wrappers also apply the preset of a MIDI program change.
//!
//! # Sparse Presets
//!
//! A preset sets only the parameters it names. The others keep their
//! current values (they are NOT reset to defaults). A preset that should
//! sound the same whatever was loaded before starts with
//! [`Parameters::reset_parameters`](crate::Parameters::reset_parameters).

/// A factory preset: the name hosts show and the function that sets the
/// parameters of the model `M`.
///
/// A host may apply a preset from one of its own threads or from the audio
/// thread (a program change) while audio runs. The function may therefore
/// only set parameters (their values are atomic) and read immutable data:
/// no allocation, no locks and no [`State`](crate::State) fields.
pub struct Preset<M: 'static> {
    name: &'static str,
    apply: fn(&M),
}

impl<M> Preset<M> {
    /// A preset with the name `name` that `apply` sets on the model.
    ///
    /// A closure that captures nothing can be the function, so the presets
    /// fit in a `const`.
    pub const fn new(name: &'static str, apply: fn(&M)) -> Self {
        Self { name, apply }
    }

    /// The name hosts show in their preset browser.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Set the parameters of this preset on `model`.
    pub fn apply(&self, model: &M) {
        (self.apply)(model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parameter_groups::{GroupInfo, ParameterGroups};
    use crate::parameter_info::{ParameterFlags, ParameterInfo, ParameterUnit};
    use crate::parameter_types::{ParameterRef, Parameters};
    use crate::types::{fnv1a_hash, ParameterId};
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
                    string_id: std::borrow::Cow::Borrowed(""),
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
    // Test Presets
    // =========================================================================

    /// Two presets, the second sparse.
    const PRESETS: &[Preset<MockParameters>] = &[
        Preset::new("Full Mix", |p| {
            p.gain.set_normalized(0.5);
            p.mix.set_normalized(1.0);
        }),
        Preset::new("Silent", |p| p.gain.set_normalized(0.0)),
    ];

    #[test]
    fn test_name_of_a_preset() {
        assert_eq!(PRESETS[0].name(), "Full Mix");
        assert_eq!(PRESETS[1].name(), "Silent");
    }

    #[test]
    fn test_apply_sets_the_parameters_of_the_preset() {
        let params = MockParameters::new();
        PRESETS[0].apply(&params);
        assert_eq!(params.gain.get_value(), 0.5);
        assert_eq!(params.mix.get_value(), 1.0);
    }

    #[test]
    fn test_sparse_preset_keeps_the_other_parameters() {
        let params = MockParameters::new();
        params.gain.set_normalized(0.75);
        params.mix.set_normalized(0.5);

        PRESETS[1].apply(&params);
        assert_eq!(params.gain.get_value(), 0.0);
        assert_eq!(params.mix.get_value(), 0.5);
    }

    #[test]
    fn test_reset_parameters_sets_the_defaults() {
        // MockParameter has the default 0.0.
        let params = MockParameters::new();
        params.gain.set_normalized(0.75);
        params.mix.set_normalized(0.5);

        params.reset_parameters();
        assert_eq!(params.gain.get_value(), 0.0);
        assert_eq!(params.mix.get_value(), 0.0);
    }
}
