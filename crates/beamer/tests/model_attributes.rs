//! The `#[beamer::model]` and `#[beamer::group]` attributes: parameters,
//! `State` fields, the event queue and nested groups, saved and loaded
//! together.

#![cfg(feature = "derive")]

use std::sync::Mutex;

use beamer::core::ParameterUnit;
use beamer::prelude::*;

/// A nested group with a parameter and a `State` field.
#[beamer::group]
struct LayerGroup {
    #[parameter(id = "level", name = "Level", default = 1.0, range = 0.0..=1.0)]
    level: FloatParameter,

    #[state(id = "sample")]
    sample: State<String>,
}

/// A model with every kind of field.
#[beamer::model]
struct SamplerModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    #[state(id = "name")]
    name: State<String>,

    #[nested(group = "Layer")]
    layer: LayerGroup,

    /// Not saved: what the audio thread plays.
    decoded: Handover<Vec<f32>>,

    events: WebViewEventQueue,
}

/// Percent and pan parameters with and without a range of their own.
#[beamer::model]
struct MixerModel {
    #[parameter(id = "mix", name = "Mix", default = 0.5, kind = "percent")]
    mix: FloatParameter,

    #[parameter(id = "amount", name = "Amount", default = 0.0, range = 0.0..=0.95, kind = "percent")]
    amount: FloatParameter,

    #[parameter(id = "pan", name = "Pan", default = 0.0, kind = "pan")]
    pan: FloatParameter,

    #[parameter(id = "width", name = "Width", default = 0.0, range = -0.5..=0.5, kind = "pan")]
    width: FloatParameter,
}

/// A value without `Default`, kept in a plain field.
struct Device {
    channel: u8,
}

/// A model that writes its own `Default`.
#[beamer::model(no_default)]
struct DeviceModel {
    #[parameter(id = "gain", name = "Gain", default = 0.5, range = 0.0..=1.0)]
    gain: FloatParameter,

    #[nested(group = "Layer")]
    layer: LayerGroup,

    device: Mutex<Device>,
}

impl Default for DeviceModel {
    fn default() -> Self {
        let mut model = Self {
            gain: FloatParameter::new("Gain", 0.5, 0.0..=1.0),
            layer: LayerGroup::default(),
            device: Mutex::new(Device { channel: 3 }),
        };
        // What the generated `Default` does after building the value.
        model.assign_ids("");
        model.set_group_ids();
        model
    }
}

#[test]
fn test_model_saves_and_loads_its_fields_and_groups() {
    let saved = SamplerModel::default();
    saved.gain.set_normalized(0.25);
    saved.name.set(String::from("My Kit"));
    saved.layer.level.set_normalized(0.5);
    saved.layer.sample.set(String::from("kick.wav"));
    saved.decoded.set(vec![1.0; 4]);
    let data = saved.save_state();

    let loaded = SamplerModel::default();
    loaded.load_state(&data).expect("the state loads");
    assert_eq!(loaded.gain.get_normalized(), 0.25);
    assert_eq!(*loaded.name.get(), "My Kit");
    assert_eq!(loaded.layer.level.get_normalized(), 0.5);
    assert_eq!(*loaded.layer.sample.get(), "kick.wav");
    // A plain field is not saved.
    assert!(loaded.decoded.get().is_empty());
}

#[test]
fn test_model_lists_the_parameters_of_its_groups() {
    let model = SamplerModel::default();

    assert_eq!(Parameters::count(&model), 2);
    assert_eq!(model.group_count(), 2);
    let ids: Vec<String> = model.iter().map(|parameter| parameter.info().string_id.to_string()).collect();
    assert_eq!(ids, ["gain", "layer/level"]);
}

#[test]
fn test_model_finds_its_event_queue() {
    let model = SamplerModel::default();

    assert!(WebViewEvents::webview_events(&model).is_some());
    assert!(WebViewEvents::webview_events(&LayerGroup::default()).is_none());
}

#[test]
fn test_model_without_a_generated_default_uses_its_own() {
    let model = DeviceModel::default();

    assert_eq!(model.device.lock().expect("not poisoned").channel, 3);
    assert_eq!(model.gain.info().string_id, "gain");
    assert_eq!(model.layer.level.info().string_id, "layer/level");
    // The nested group has a group of its own, not the root.
    assert_eq!(model.group_count(), 2);
    assert_ne!(model.layer.level.info().group_id, model.gain.info().group_id);

    // Its parameters are saved and loaded as with a generated default.
    model.gain.set_normalized(0.75);
    let loaded = DeviceModel::default();
    loaded.load_state(&model.save_state()).expect("the state loads");
    assert_eq!(loaded.gain.get_normalized(), 0.75);
}

#[test]
fn test_percent_and_pan_take_a_given_range() {
    let model = MixerModel::default();
    let bounds = |parameter: &FloatParameter| {
        (parameter.normalized_to_plain(0.0), parameter.normalized_to_plain(1.0))
    };

    assert_eq!(bounds(&model.mix), (0.0, 1.0));
    assert_eq!(bounds(&model.amount), (0.0, 0.95));
    assert_eq!(bounds(&model.pan), (-1.0, 1.0));
    assert_eq!(bounds(&model.width), (-0.5, 0.5));

    // A given range keeps the kind's unit and display.
    assert_eq!(model.amount.info().unit, ParameterUnit::Percent);
    assert_eq!(model.amount.display_normalized(1.0), "95");
    assert_eq!(model.width.info().unit, ParameterUnit::Pan);
}

/// A nested group with a note number, as a split point.
#[beamer::group]
struct SplitGroup {
    #[parameter(id = "point", name = "Split Point", default = 60, range = 0..=127, kind = "note_number")]
    point: IntParameter,
}

/// Note numbers at the root and in a nested group.
#[beamer::model]
struct KeyboardModel {
    #[parameter(id = "root", name = "Root", default = 61, range = 0..=127, kind = "note_number")]
    root: IntParameter,

    #[parameter(id = "transpose", name = "Transpose", default = 0, range = -12..=12, kind = "semitones")]
    transpose: IntParameter,

    #[nested(group = "Split")]
    split: SplitGroup,
}

#[test]
fn test_note_numbers_follow_middle_c_also_in_nested_groups() {
    let mut model = KeyboardModel::default();
    assert_eq!(model.root.display_normalized(model.root.get_normalized()), "C#3");
    assert_eq!(model.split.point.display_normalized(model.split.point.get_normalized()), "C3");
    assert_eq!(model.root.units(), "");
    assert_eq!(model.root.formatter_kind(), "note_number");
    assert_eq!(model.root.info().unit, ParameterUnit::MidiNoteNumber);

    model.set_middle_c(MiddleC::C4);
    assert_eq!(model.root.display_normalized(model.root.get_normalized()), "C#4");
    assert_eq!(model.split.point.display_normalized(model.split.point.get_normalized()), "C4");
    assert_eq!(model.split.point.parse("C4").map(|normalized| model.split.point.normalized_to_plain(normalized)), Some(60.0));
    // Other formatters stay as they are.
    assert_eq!(model.transpose.display_normalized(model.transpose.get_normalized()), "0");
}
