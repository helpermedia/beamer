//! The ids of parameters in nested parameter structs.
//!
//! A parameter is known by its path: the `id` of its attribute behind the
//! field names of the nested structs it sits in. The id that hosts see is
//! the hash of that path.

#![cfg(feature = "derive")]

use std::collections::HashSet;

use beamer::core::parameter_info::ParameterInfo;
use beamer::core::parameter_store::ParameterStore;
use beamer::core::parameter_types::check_unique_ids;
use beamer::core::preset::{fnv1a_hash, FactoryPresets, PresetInfo, PresetValue};
use beamer::core::types::ParameterId;
use beamer::prelude::*;

#[derive(Parameters)]
struct EnvelopeParameters {
    #[parameter(id = "attack", name = "Attack", default = 10.0, range = 0.1..=1000.0, kind = "ms")]
    attack: FloatParameter,
}

#[derive(Parameters)]
struct LayerParameters {
    #[parameter(id = "level", name = "Level", default = 1.0, range = 0.0..=1.0)]
    level: FloatParameter,

    #[parameter(id = "voices", name = "Voices", default = 4, range = 1..=8)]
    voices: IntParameter,

    #[nested(group = "Envelope")]
    envelope: EnvelopeParameters,
}

/// The same nested struct in two fields, next to a parameter of the root
/// that has the id of one in the nested struct.
#[derive(Parameters)]
struct TwoLayerParameters {
    #[parameter(id = "level", name = "Level", default = 0.5, range = 0.0..=1.0)]
    level: FloatParameter,

    #[nested(group = "Layer A")]
    first: LayerParameters,

    #[nested(group = "Layer B")]
    second: LayerParameters,
}

/// Field names that are keywords.
#[derive(Parameters)]
struct KeywordParameters {
    #[parameter(id = "loop", name = "Loop", default = false)]
    r#loop: BoolParameter,

    #[nested(group = "Type")]
    r#type: EnvelopeParameters,
}

fn paths(parameters: &dyn Parameters) -> Vec<String> {
    parameters
        .iter()
        .map(|parameter| parameter.info().string_id.to_string())
        .collect()
}

#[test]
fn test_string_id_is_the_path_of_the_parameter() {
    let parameters = TwoLayerParameters::default();

    assert_eq!(
        paths(&parameters),
        [
            "level",
            "first/level",
            "first/voices",
            "first/envelope/attack",
            "second/level",
            "second/voices",
            "second/envelope/attack",
        ]
    );
}

#[test]
fn test_host_id_is_the_hash_of_the_path() {
    let parameters = TwoLayerParameters::default();

    for parameter in parameters.iter() {
        assert_eq!(parameter.id(), fnv1a_hash(&parameter.info().string_id));
    }

    // A parameter of the root keeps the hash of its id alone.
    assert_eq!(parameters.level.id(), fnv1a_hash("level"));
    assert_eq!(parameters.second.envelope.attack.id(), fnv1a_hash("second/envelope/attack"));
}

#[test]
fn test_nested_struct_in_two_fields_has_distinct_host_ids() {
    let parameters = TwoLayerParameters::default();

    let ids: HashSet<ParameterId> = parameters.iter().map(|parameter| parameter.id()).collect();
    assert_eq!(ids.len(), Parameters::count(&parameters));
    assert!(check_unique_ids(&parameters).is_ok());
}

#[test]
fn test_a_nested_struct_on_its_own_has_its_own_paths() {
    let layer = LayerParameters::default();

    assert_eq!(paths(&layer), ["level", "voices", "envelope/attack"]);
    assert_eq!(layer.envelope.attack.id(), fnv1a_hash("envelope/attack"));
}

#[test]
fn test_lookup_by_id_reaches_the_right_field() {
    let parameters = TwoLayerParameters::default();

    let id = parameters.second.level.id();
    ParameterStore::set_normalized(&parameters, id, 0.25);

    assert_eq!(parameters.second.level.get_normalized(), 0.25);
    assert_eq!(parameters.first.level.get_normalized(), 1.0);
    assert_eq!(parameters.level.get_normalized(), 0.5);
    assert_eq!(ParameterStore::get_normalized(&parameters, id), 0.25);

    let found = parameters.by_id(parameters.first.envelope.attack.id()).expect("found");
    assert_eq!(found.info().string_id, "first/envelope/attack");

    assert!(parameters.by_id(fnv1a_hash("voices")).is_none());
}

#[test]
fn test_info_by_index_reports_the_ids() {
    let parameters = TwoLayerParameters::default();

    let listed: Vec<ParameterId> = (0..ParameterStore::count(&parameters))
        .map(|index| ParameterStore::info(&parameters, index).expect("in range").id)
        .collect();
    let iterated: Vec<ParameterId> = parameters.iter().map(|parameter| parameter.id()).collect();
    assert_eq!(listed, iterated);
}

#[test]
fn test_saved_state_uses_the_same_paths() {
    let parameters = TwoLayerParameters::default();
    let saved = parameters.save_state();

    for path in paths(&parameters) {
        // An entry holds the length of its path and then the path.
        let mut needle = (path.len() as u16).to_le_bytes().to_vec();
        needle.extend_from_slice(path.as_bytes());
        assert!(
            saved.windows(needle.len()).any(|window| window == needle),
            "no entry for {path}"
        );
    }
}

#[test]
fn test_web_ui_gets_the_paths() {
    let parameters = TwoLayerParameters::default();
    let json = beamer::core::params_to_init_json(&parameters);
    let dump: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    let string_ids: Vec<&str> = dump
        .as_array()
        .expect("array")
        .iter()
        .map(|entry| entry["stringId"].as_str().expect("a string"))
        .collect();
    assert_eq!(string_ids, paths(&parameters));
}

/// What `Presets.toml` turns into: the key of a value, hashed.
struct LayerPresets;

impl FactoryPresets for LayerPresets {
    type Parameters = TwoLayerParameters;

    fn count() -> usize {
        1
    }

    fn info(index: usize) -> Option<PresetInfo> {
        (index == 0).then_some(PresetInfo { name: "Quiet B" })
    }

    fn values(_index: usize) -> &'static [PresetValue] {
        const VALUES: &[PresetValue] = &[
            PresetValue {
                id: fnv1a_hash("second/level"),
                plain_value: 0.1,
            },
            PresetValue {
                id: fnv1a_hash("first/voices"),
                plain_value: 8.0,
            },
        ];
        VALUES
    }
}

#[test]
fn test_preset_names_a_nested_parameter_by_its_path() {
    let parameters = TwoLayerParameters::default();
    assert!(LayerPresets::apply(0, &parameters));

    assert!((parameters.second.level.get() - 0.1).abs() < 1e-9);
    assert_eq!(parameters.first.voices.get(), 8);
    assert_eq!(parameters.first.level.get(), 1.0);
    assert_eq!(parameters.second.voices.get(), 4);
}

#[test]
fn test_field_names_that_are_keywords() {
    let parameters = KeywordParameters::default();

    assert_eq!(paths(&parameters), ["loop", "type/attack"]);
    assert_eq!(KeywordParameters::GROUP_TYPE, 1);
}

/// A derived struct whose `Default` is written by hand, because its
/// parameter has no declarative attributes.
#[derive(Parameters)]
struct HandBuiltParameters {
    #[parameter(id = "drive")]
    drive: FloatParameter,
}

impl Default for HandBuiltParameters {
    fn default() -> Self {
        let mut parameters = Self {
            drive: FloatParameter::new("Drive", 0.0, 0.0..=1.0),
        };
        parameters.assign_ids("");
        parameters
    }
}

#[derive(Parameters)]
struct HandBuiltHolder {
    #[nested(group = "Stage")]
    stage: HandBuiltParameters,
}

#[test]
fn test_hand_written_default_assigns_its_ids() {
    let parameters = HandBuiltParameters::default();
    assert_eq!(paths(&parameters), ["drive"]);
    assert_eq!(parameters.drive.id(), fnv1a_hash("drive"));

    // Nested, the struct that holds it gives it its place.
    let holder = HandBuiltHolder::default();
    assert_eq!(paths(&holder), ["stage/drive"]);
    assert_eq!(holder.stage.drive.id(), fnv1a_hash("stage/drive"));
}

/// A struct that holds a hand-written `Parameters` implementation.
#[derive(Parameters)]
struct ClashingHolder {
    #[nested(group = "Hand")]
    hand: ClashingParameters,
}

impl Default for ClashingParameters {
    fn default() -> Self {
        Self {
            first: FloatParameter::new("First", 0.0, 0.0..=1.0)
                .with_id(7)
                .with_string_id("first"),
            second: FloatParameter::new("Second", 0.0, 0.0..=1.0).with_id(8),
        }
    }
}

#[test]
fn test_hand_written_implementation_keeps_its_ids_when_nested() {
    let holder = ClashingHolder::default();

    assert_eq!(holder.hand.first.id(), 7);
    assert_eq!(holder.hand.second.id(), 8);
    assert_eq!(paths(&holder), ["first", ""]);
    assert!(holder.by_id(8).is_some());
}

#[test]
fn test_parameter_without_a_string_id_is_reported_by_its_name() {
    let parameters = ClashingParameters {
        first: FloatParameter::new("First", 0.0, 0.0..=1.0),
        second: FloatParameter::new("Second", 0.0, 0.0..=1.0),
    };

    let error = check_unique_ids(&parameters).expect_err("both ids are 0");
    assert!(error.contains("\"First\" (no string id)"), "{error}");
    assert!(error.contains("\"Second\" (no string id)"), "{error}");
    assert!(error.contains("assign_ids"), "{error}");
}

/// Two parameters that were given the same id by hand.
struct ClashingParameters {
    first: FloatParameter,
    second: FloatParameter,
}

impl ParameterGroups for ClashingParameters {}

impl Parameters for ClashingParameters {
    fn count(&self) -> usize {
        2
    }

    fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
        Box::new([&self.first as &dyn ParameterRef, &self.second].into_iter())
    }

    fn by_id(&self, id: ParameterId) -> Option<&dyn ParameterRef> {
        self.iter().find(|parameter| parameter.id() == id)
    }
}

impl ParameterStore for ClashingParameters {
    fn count(&self) -> usize {
        2
    }

    fn info(&self, index: usize) -> Option<&ParameterInfo> {
        self.iter().nth(index).map(|parameter| parameter.info())
    }

    fn get_normalized(&self, id: ParameterId) -> f64 {
        self.by_id(id).map_or(0.0, |parameter| parameter.get_normalized())
    }

    fn set_normalized(&self, id: ParameterId, value: f64) {
        if let Some(parameter) = self.by_id(id) {
            parameter.set_normalized(value);
        }
    }

    fn normalized_to_string(&self, id: ParameterId, normalized: f64) -> String {
        self.by_id(id)
            .map_or_else(String::new, |parameter| parameter.display_normalized(normalized))
    }

    fn string_to_normalized(&self, id: ParameterId, string: &str) -> Option<f64> {
        self.by_id(id)?.parse(string)
    }

    fn normalized_to_plain(&self, id: ParameterId, normalized: f64) -> f64 {
        self.by_id(id).map_or(0.0, |parameter| parameter.normalized_to_plain(normalized))
    }

    fn plain_to_normalized(&self, id: ParameterId, plain: f64) -> f64 {
        self.by_id(id).map_or(0.0, |parameter| parameter.plain_to_normalized(plain))
    }
}

#[test]
fn test_two_parameters_with_one_host_id_are_reported() {
    let parameters = ClashingParameters {
        first: FloatParameter::new("First", 0.0, 0.0..=1.0)
            .with_id(7)
            .with_string_id("first"),
        second: FloatParameter::new("Second", 0.0, 0.0..=1.0)
            .with_id(7)
            .with_string_id("second"),
    };

    let error = check_unique_ids(&parameters).expect_err("the ids clash");
    assert!(error.contains("\"first\""), "{error}");
    assert!(error.contains("\"second\""), "{error}");
    assert!(error.contains("0x00000007"), "{error}");
}
