//! State beyond parameters: `State` fields of a model are saved and loaded
//! with the parameters and handed over to the audio thread.

#![cfg(feature = "derive")]

use beamer::prelude::*;
use beamer::serde::{Deserialize, Serialize};

/// A value type of a plugin that has no serde dependency of its own.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(crate = "beamer::serde")]
struct Sample {
    path: String,
    root_note: u8,
}

#[beamer::group]
struct LayerGroup {
    #[parameter(id = "level", name = "Level", default = 1.0, range = 0.0..=1.0)]
    level: FloatParameter,

    #[state(id = "sample")]
    sample: State<Sample>,
}

#[beamer::model]
struct SamplerModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    #[state(id = "name")]
    name: State<String>,

    #[nested(group = "Layer A")]
    layer: LayerGroup,

    /// Not saved: what the audio thread plays, decoded from the sample.
    decoded: Handover<Vec<f32>>,
}

/// A struct that has state and no parameters of its own.
#[beamer::model]
struct EditorModel {
    #[state(id = "zoom")]
    zoom: State<u32>,
}

/// The same nested struct in two fields.
#[beamer::model]
struct TwoLayerModel {
    #[nested(group = "Layer A")]
    first: LayerGroup,

    #[nested(group = "Layer B")]
    second: LayerGroup,
}

fn kick() -> Sample {
    Sample {
        path: String::from("kick.wav"),
        root_note: 36,
    }
}

#[test]
fn test_state_fields_start_with_their_defaults() {
    let model = SamplerModel::default();

    assert_eq!(*model.name.get(), "");
    assert_eq!(*model.layer.sample.get(), Sample::default());
    assert!(model.decoded.get().is_empty());
}

#[test]
fn test_state_fields_are_saved_after_the_parameters() {
    let model = SamplerModel::default();
    model.gain.set_normalized(0.25);
    model.name.set(String::from("a"));

    // Written out in full, so that a change of the format fails here.
    let expected: Vec<u8> = [
        // Magic, version 1, 103 bytes of payload.
        &[0x42, 0x4D, 0x52, 0x53][..],
        &[1, 0, 0, 0],
        &[103, 0, 0, 0],
        // The parameter of the root.
        &[1],
        &[4, 0],
        b"gain",
        &[8, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0xD0, 0x3F],
        // Its state: kind 2, JSON.
        &[2],
        &[4, 0],
        b"name",
        &[3, 0, 0, 0],
        b"\"a\"",
        // The nested struct, under the name of its field.
        &[1],
        &[11, 0],
        b"layer/level",
        &[8, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0xF0, 0x3F],
        &[2],
        &[12, 0],
        b"layer/sample",
        &[25, 0, 0, 0],
        br#"{"path":"","root_note":0}"#,
    ]
    .concat();

    assert_eq!(model.save_state(), expected);
}

#[test]
fn test_state_round_trip_restores_parameters_and_state() {
    let saved = SamplerModel::default();
    saved.gain.set_normalized(0.25);
    saved.name.set(String::from("My Kit"));
    saved.layer.level.set_normalized(0.5);
    saved.layer.sample.set(kick());
    saved.decoded.set(vec![0.5; 8]);
    let bytes = saved.save_state();

    let loaded = SamplerModel::default();
    loaded.load_state(&bytes).expect("state loads");

    assert_eq!(loaded.gain.get_normalized(), 0.25);
    assert_eq!(*loaded.name.get(), "My Kit");
    assert_eq!(loaded.layer.level.get_normalized(), 0.5);
    assert_eq!(*loaded.layer.sample.get(), kick());
    // A field without an attribute is not part of the state.
    assert!(loaded.decoded.get().is_empty());

    // What was loaded is saved to the same bytes.
    assert_eq!(loaded.save_state(), bytes);
}

#[test]
fn test_loaded_state_reaches_the_audio_thread() {
    let saved = SamplerModel::default();
    saved.layer.sample.set(kick());
    let bytes = saved.save_state();

    let model = SamplerModel::default();
    // As a processor would in prepare().
    let mut sample = model.layer.sample.reader();
    assert_eq!(sample.changed(), None);

    model.load_state(&bytes).expect("state loads");
    assert_eq!(sample.changed(), Some(&kick()));
    assert_eq!(sample.changed(), None);
}

#[test]
fn test_state_without_an_entry_goes_back_to_its_default() {
    // A state that was saved before the plugin had state fields.
    let saved = SamplerModel::default();
    saved.gain.set_normalized(0.25);
    let bytes = saved.save_state();
    let name_entry: &[u8] = &[2, 4, 0, b'n', b'a', b'm', b'e', 2, 0, 0, 0, b'"', b'"'];
    let position = bytes
        .windows(name_entry.len())
        .position(|window| window == name_entry)
        .expect("the entry of the name is in the state");
    let mut without_name = bytes.clone();
    without_name.drain(position..position + name_entry.len());
    let payload = (without_name.len() - 12) as u32;
    without_name[8..12].copy_from_slice(&payload.to_le_bytes());

    let model = SamplerModel::default();
    model.name.set(String::from("before"));
    model.load_state(&without_name).expect("state loads");

    assert_eq!(model.gain.get_normalized(), 0.25);
    assert_eq!(*model.name.get(), "");
}

#[test]
fn test_state_entry_that_does_not_fit_gives_the_default_and_keeps_the_rest() {
    let saved = SamplerModel::default();
    saved.gain.set_normalized(0.25);
    saved.name.set(String::from("kit"));
    let mut bytes = saved.save_state();

    // Turn the JSON string of the name into something that is not one.
    let json = bytes
        .windows(5)
        .position(|window| window == b"\"kit\"")
        .expect("the name is in the state");
    bytes[json] = b'[';

    let model = SamplerModel::default();
    model.name.set(String::from("before"));
    model.load_state(&bytes).expect("state loads");

    assert_eq!(*model.name.get(), "");
    assert_eq!(model.gain.get_normalized(), 0.25);
}

#[test]
fn test_state_fields_are_not_parameters() {
    let model = SamplerModel::default();

    // The gain and the level of the nested layer.
    assert_eq!(Parameters::count(&model), 2);
    assert_eq!(model.iter().count(), 2);
}

#[test]
fn test_struct_with_state_and_no_parameters() {
    let saved = EditorModel::default();
    assert_eq!(Parameters::count(&saved), 0);
    assert_eq!(saved.iter().count(), 0);
    saved.zoom.set(150);

    let loaded = EditorModel::default();
    loaded.load_state(&saved.save_state()).expect("state loads");
    assert_eq!(*loaded.zoom.get(), 150);
}

#[test]
fn test_nested_struct_in_two_fields_keeps_two_states() {
    let saved = TwoLayerModel::default();
    saved.first.sample.set(kick());
    saved.second.level.set_normalized(0.25);

    let loaded = TwoLayerModel::default();
    loaded.load_state(&saved.save_state()).expect("state loads");

    assert_eq!(*loaded.first.sample.get(), kick());
    assert_eq!(*loaded.second.sample.get(), Sample::default());
    assert_eq!(loaded.first.level.get_normalized(), 1.0);
    assert_eq!(loaded.second.level.get_normalized(), 0.25);
}

#[test]
fn test_loading_a_state_counts_as_a_change_for_a_ui() {
    let saved = SamplerModel::default();
    saved.name.set(String::from("My Kit"));
    let bytes = saved.save_state();

    let model = SamplerModel::default();
    let before = model.name.generation();
    model.load_state(&bytes).expect("state loads");

    // A UI that polls the generation notices that it has to read again.
    assert!(model.name.generation() > before);
}

#[test]
fn test_state_generation_changes_with_state_fields_only() {
    let model = SamplerModel::default();
    let start = model.state_generation();

    // Parameters and fields that are not saved do not count.
    model.gain.set_normalized(0.5);
    model.decoded.set(vec![0.0; 4]);
    assert_eq!(model.state_generation(), start);

    model.name.set(String::from("a"));
    let after_name = model.state_generation();
    assert_ne!(after_name, start);

    // A State field of a nested struct counts too.
    model.layer.sample.set(kick());
    assert_ne!(model.state_generation(), after_name);
}

/// The paths and JSON values that `for_each_state` visits, through the
/// model as the wrappers see it.
fn visited(model: &dyn beamer::core::SharedModel) -> Vec<(String, String)> {
    let mut seen = Vec::new();
    model.for_each_state(&mut |path, state| {
        let json = state.save_json().expect("state saves");
        seen.push((path.to_string(), String::from_utf8(json).expect("JSON is UTF-8")));
    });
    seen
}

#[test]
fn test_state_fields_are_visited_by_their_paths() {
    let model = SamplerModel::default();
    model.name.set(String::from("My Kit"));
    model.layer.sample.set(kick());

    // The paths of the saved state: the id, behind the field of a nested
    // struct. The fields that are not saved are not visited.
    assert_eq!(
        visited(&model),
        vec![
            (String::from("name"), String::from("\"My Kit\"")),
            (
                String::from("layer/sample"),
                String::from("{\"path\":\"kick.wav\",\"root_note\":36}")
            ),
        ]
    );
}

#[test]
fn test_nested_struct_in_two_fields_is_visited_twice() {
    let model = TwoLayerModel::default();
    let paths: Vec<String> = visited(&model).into_iter().map(|(path, _)| path).collect();
    assert_eq!(paths, ["first/sample", "second/sample"]);
}

/// A keyword as the name of a nested field, three levels deep.
#[beamer::model]
struct KitModel {
    #[nested(group = "Type")]
    r#type: SamplerModel,
}

#[test]
fn test_deeply_nested_state_fields_have_the_paths_of_the_saved_state() {
    let model = KitModel::default();
    let paths: Vec<String> = visited(&model).into_iter().map(|(path, _)| path).collect();
    assert_eq!(paths, ["type/name", "type/layer/sample"]);

    // Each path is the path of an entry of the saved state.
    let saved = model.save_state();
    for path in &paths {
        let mut needle = (path.len() as u16).to_le_bytes().to_vec();
        needle.extend_from_slice(path.as_bytes());
        assert!(
            saved.windows(needle.len()).any(|window| window == needle),
            "{path} is not in the saved state"
        );
    }
}

#[test]
fn test_a_visited_state_field_can_be_set() {
    let model = EditorModel::default();
    model.for_each_state(&mut |path, state| {
        assert_eq!(path, "zoom");
        state.load_json(b"150").expect("JSON fits");
    });
    assert_eq!(*model.zoom.get(), 150);
}

#[test]
fn test_struct_without_state_fields_visits_nothing() {
    #[beamer::model]
    struct GainModel {
        #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
        gain: FloatParameter,
    }

    assert!(visited(&GainModel::default()).is_empty());
}
