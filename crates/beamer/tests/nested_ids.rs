//! The ids of parameters in nested groups.
//!
//! A parameter is known by its path: the `id` of its attribute behind the
//! field names of the nested groups it sits in. The id that hosts see is
//! the hash of that path.

#![cfg(feature = "derive")]

use std::collections::HashSet;

use beamer::core::parameter_store::ParameterStore;
use beamer::core::parameter_types::check_unique_ids;
use beamer::core::types::fnv1a_hash;
use beamer::core::types::ParameterId;
use beamer::prelude::*;

#[beamer::group]
struct EnvelopeGroup {
    #[parameter(id = "attack", name = "Attack", default = 10.0, range = 0.1..=1000.0, kind = "ms")]
    attack: FloatParameter,
}

#[beamer::group]
struct LayerGroup {
    #[parameter(id = "level", name = "Level", default = 1.0, range = 0.0..=1.0)]
    level: FloatParameter,

    #[parameter(id = "voices", name = "Voices", default = 4, range = 1..=8)]
    voices: IntParameter,

    #[nested(group = "Envelope")]
    envelope: EnvelopeGroup,
}

/// The same nested group in two fields, next to a parameter of the root
/// that has the id of one in the nested group.
#[beamer::model]
struct TwoLayerModel {
    #[parameter(id = "level", name = "Level", default = 0.5, range = 0.0..=1.0)]
    level: FloatParameter,

    #[nested(group = "Layer A")]
    first: LayerGroup,

    #[nested(group = "Layer B")]
    second: LayerGroup,
}

/// Field names that are keywords.
#[beamer::model]
struct KeywordModel {
    #[parameter(id = "loop", name = "Loop", default = false)]
    r#loop: BoolParameter,

    #[nested(group = "Type")]
    r#type: EnvelopeGroup,
}

fn paths(parameters: &dyn Parameters) -> Vec<String> {
    parameters
        .iter()
        .map(|parameter| parameter.info().string_id.to_string())
        .collect()
}

#[test]
fn test_string_id_is_the_path_of_the_parameter() {
    let model = TwoLayerModel::default();

    assert_eq!(
        paths(&model),
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
    let model = TwoLayerModel::default();

    for parameter in model.iter() {
        assert_eq!(parameter.id(), fnv1a_hash(&parameter.info().string_id));
    }

    // A parameter of the root keeps the hash of its id alone.
    assert_eq!(model.level.id(), fnv1a_hash("level"));
    assert_eq!(model.second.envelope.attack.id(), fnv1a_hash("second/envelope/attack"));
}

#[test]
fn test_nested_struct_in_two_fields_has_distinct_host_ids() {
    let model = TwoLayerModel::default();

    let ids: HashSet<ParameterId> = model.iter().map(|parameter| parameter.id()).collect();
    assert_eq!(ids.len(), Parameters::count(&model));
    assert!(check_unique_ids(&model).is_ok());
}

#[test]
fn test_a_nested_struct_on_its_own_has_its_own_paths() {
    let layer = LayerGroup::default();

    assert_eq!(paths(&layer), ["level", "voices", "envelope/attack"]);
    assert_eq!(layer.envelope.attack.id(), fnv1a_hash("envelope/attack"));
}

#[test]
fn test_lookup_by_id_reaches_the_right_field() {
    let model = TwoLayerModel::default();

    let id = model.second.level.id();
    ParameterStore::set_normalized(&model, id, 0.25);

    assert_eq!(model.second.level.get_normalized(), 0.25);
    assert_eq!(model.first.level.get_normalized(), 1.0);
    assert_eq!(model.level.get_normalized(), 0.5);
    assert_eq!(ParameterStore::get_normalized(&model, id), 0.25);

    let found = model.by_id(model.first.envelope.attack.id()).expect("found");
    assert_eq!(found.info().string_id, "first/envelope/attack");

    assert!(model.by_id(fnv1a_hash("voices")).is_none());
}

#[test]
fn test_info_by_index_reports_the_ids() {
    let model = TwoLayerModel::default();

    let listed: Vec<ParameterId> = (0..ParameterStore::count(&model))
        .map(|index| ParameterStore::info(&model, index).expect("in range").id)
        .collect();
    let iterated: Vec<ParameterId> = model.iter().map(|parameter| parameter.id()).collect();
    assert_eq!(listed, iterated);
}

#[test]
fn test_saved_state_uses_the_same_paths() {
    let model = TwoLayerModel::default();
    let saved = model.save_state();

    for path in paths(&model) {
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
    let model = TwoLayerModel::default();
    let json = beamer::core::params_to_init_json(&model);
    let dump: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    let string_ids: Vec<&str> = dump
        .as_array()
        .expect("array")
        .iter()
        .map(|entry| entry["stringId"].as_str().expect("a string"))
        .collect();
    assert_eq!(string_ids, paths(&model));
}

#[test]
fn test_reset_parameters_reaches_nested_groups() {
    let model = TwoLayerModel::default();
    model.level.set(0.9);
    model.first.voices.set(8);
    model.second.envelope.attack.set(500.0);

    model.reset_parameters();
    assert_eq!(model.level.get(), 0.5);
    assert_eq!(model.first.voices.get(), 4);
    assert!((model.second.envelope.attack.get() - 10.0).abs() < 1e-9);
}

#[test]
fn test_field_names_that_are_keywords() {
    let model = KeywordModel::default();

    assert_eq!(paths(&model), ["loop", "type/attack"]);
    assert_eq!(KeywordModel::GROUP_TYPE, 1);
}

/// A group whose `Default` is written by hand, because its parameter has
/// no declarative attributes.
#[beamer::group(no_default)]
struct HandBuiltGroup {
    #[parameter(id = "drive")]
    drive: FloatParameter,
}

impl Default for HandBuiltGroup {
    fn default() -> Self {
        let mut group = Self {
            drive: FloatParameter::new("Drive", 0.0, 0.0..=1.0),
        };
        group.assign_ids("");
        group
    }
}

#[beamer::model]
struct HandBuiltModel {
    #[nested(group = "Stage")]
    stage: HandBuiltGroup,
}

#[test]
fn test_hand_written_default_assigns_its_ids() {
    let group = HandBuiltGroup::default();
    assert_eq!(paths(&group), ["drive"]);
    assert_eq!(group.drive.id(), fnv1a_hash("drive"));

    // Nested, the model that holds it gives it its place.
    let holder = HandBuiltModel::default();
    assert_eq!(paths(&holder), ["stage/drive"]);
    assert_eq!(holder.stage.drive.id(), fnv1a_hash("stage/drive"));
}

/// A model that holds a hand-written `Parameters` implementation.
#[beamer::model]
struct ClashingModel {
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
    let holder = ClashingModel::default();

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

impl Saved for ClashingParameters {}

impl WebViewEvents for ClashingParameters {}

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

/// A generic model with a flat group and a nested group.
#[beamer::model]
struct GenericModel<T: Send + Sync + 'static> {
    #[parameter(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
    cutoff: FloatParameter,

    #[nested(group = "Envelope")]
    envelope: EnvelopeGroup,

    /// What the type parameter stands for, not a parameter.
    marker: std::marker::PhantomData<T>,
}

#[test]
fn test_generic_struct_with_groups() {
    let model = GenericModel::<f32>::default();

    assert_eq!(GenericModel::<f32>::PARAM_GROUP_FILTER, 1);
    assert_eq!(GenericModel::<f32>::GROUP_ENVELOPE, 2);
    assert_eq!(model.envelope.attack.info().string_id, "envelope/attack");
    assert_eq!(ParameterStore::count(&model), 2);
}
