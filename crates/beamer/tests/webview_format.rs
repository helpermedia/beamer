//! The `format` of each parameter kind in the parameter dump a WebView gets
//! and the texts of the values of a discrete parameter.

#![cfg(feature = "derive")]

use beamer::prelude::*;

#[derive(Copy, Clone, PartialEq, EnumParameter)]
enum Mode {
    #[default]
    #[name = "Clean"]
    Clean,
    #[name = "Warm"]
    Warm,
}

#[beamer::model]
struct TestModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,

    #[parameter(id = "voices", name = "Voices", default = 4, range = 1..=8)]
    voices: IntParameter,

    #[parameter(id = "enabled", name = "Enabled", default = true)]
    enabled: BoolParameter,

    #[parameter(id = "mode", name = "Mode")]
    mode: EnumParameter<Mode>,

    #[parameter(id = "note", name = "Note", default = 60, range = 0..=127)]
    note: IntParameter,

    #[parameter(id = "offset", name = "Offset", default = 0, range = 0..=128)]
    offset: IntParameter,
}

/// The entry of a parameter in the init dump.
fn entry(dump: &serde_json::Value, string_id: &str) -> serde_json::Value {
    dump.as_array()
        .expect("array")
        .iter()
        .find(|entry| entry["stringId"] == string_id)
        .unwrap_or_else(|| panic!("no entry for {string_id}"))
        .clone()
}

/// The texts of the values of a parameter in the init dump.
fn value_texts(dump: &serde_json::Value, string_id: &str) -> Vec<String> {
    entry(dump, string_id)["valueTexts"]
        .as_array()
        .expect("valueTexts is an array")
        .iter()
        .map(|text| text.as_str().expect("a text").to_owned())
        .collect()
}

#[test]
fn test_init_dump_reports_the_format_of_each_parameter_kind() {
    let model = TestModel::default();
    let json = beamer::core::params_to_init_json(&model);
    let dump: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    let format_of = |string_id: &str| {
        entry(&dump, string_id)["format"]
            .as_str()
            .expect("format is a string")
            .to_owned()
    };

    assert_eq!(format_of("gain"), "db");
    assert_eq!(format_of("voices"), "float");
    assert_eq!(format_of("enabled"), "boolean");
    assert_eq!(format_of("mode"), "enum");
}

#[test]
fn test_init_dump_lists_the_texts_of_the_values_of_a_discrete_parameter() {
    let model = TestModel::default();
    let json = beamer::core::params_to_init_json(&model);
    let dump: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    assert_eq!(value_texts(&dump, "mode"), ["Clean", "Warm"]);
    assert_eq!(value_texts(&dump, "enabled"), ["Off", "On"]);
    assert_eq!(value_texts(&dump, "voices"), ["1", "2", "3", "4", "5", "6", "7", "8"]);

    // 128 values are listed, 129 are not.
    let notes = value_texts(&dump, "note");
    assert_eq!(notes.len(), 128);
    assert_eq!((notes[0].as_str(), notes[127].as_str()), ("0", "127"));
    assert!(value_texts(&dump, "offset").is_empty());

    // A continuous parameter has none.
    assert!(value_texts(&dump, "gain").is_empty());
}
