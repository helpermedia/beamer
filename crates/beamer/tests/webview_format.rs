//! The `format` of each parameter kind in the parameter dump a WebView gets.

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

#[derive(Parameters)]
struct TestParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,

    #[parameter(id = "voices", name = "Voices", default = 4, range = 1..=8)]
    voices: IntParameter,

    #[parameter(id = "enabled", name = "Enabled", default = true)]
    enabled: BoolParameter,

    #[parameter(id = "mode", name = "Mode")]
    mode: EnumParameter<Mode>,
}

#[test]
fn test_init_dump_reports_the_format_of_each_parameter_kind() {
    let parameters = TestParameters::default();
    let json = beamer::core::params_to_init_json(&parameters);
    let dump: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    let format_of = |string_id: &str| {
        dump.as_array()
            .expect("array")
            .iter()
            .find(|entry| entry["stringId"] == string_id)
            .unwrap_or_else(|| panic!("no entry for {string_id}"))["format"]
            .as_str()
            .expect("format is a string")
            .to_owned()
    };

    assert_eq!(format_of("gain"), "db");
    assert_eq!(format_of("voices"), "float");
    assert_eq!(format_of("enabled"), "boolean");
    assert_eq!(format_of("mode"), "enum");
}
