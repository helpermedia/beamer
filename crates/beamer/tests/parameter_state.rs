//! The state of a plugin is the state of its parameters: the bytes they are
//! saved to and loaded from.

#![cfg(feature = "derive")]

use beamer::prelude::*;

#[derive(Parameters)]
struct FilterParameters {
    #[parameter(id = "cutoff", name = "Cutoff", default = 0.0, range = 0.0..=1.0)]
    cutoff: FloatParameter,
}

#[derive(Parameters)]
struct TestParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    #[parameter(id = "enabled", name = "Enabled", default = false)]
    enabled: BoolParameter,

    #[nested(group = "Filter")]
    filter: FilterParameters,
}

/// One state entry: the length of the path, the path and the normalized value.
fn entry(path: &str, normalized: f64) -> Vec<u8> {
    let mut bytes = vec![path.len() as u8];
    bytes.extend_from_slice(path.as_bytes());
    bytes.extend_from_slice(&normalized.to_le_bytes());
    bytes
}

#[test]
fn test_state_has_the_documented_bytes() {
    let parameters = TestParameters::default();
    parameters.gain.set_normalized(0.25);
    parameters.enabled.set_normalized(1.0);
    parameters.filter.cutoff.set_normalized(0.5);

    // Written out in full, so that a change of the format fails here.
    let expected: Vec<u8> = [
        &[4u8][..],
        b"gain",
        &[0, 0, 0, 0, 0, 0, 0xD0, 0x3F],
        &[7],
        b"enabled",
        &[0, 0, 0, 0, 0, 0, 0xF0, 0x3F],
        &[13],
        b"Filter/cutoff",
        &[0, 0, 0, 0, 0, 0, 0xE0, 0x3F],
    ]
    .concat();

    assert_eq!(parameters.save_state(), expected);
}

#[test]
fn test_state_round_trip_restores_every_value() {
    let saved = TestParameters::default();
    saved.gain.set_normalized(0.25);
    saved.enabled.set_normalized(1.0);
    saved.filter.cutoff.set_normalized(0.5);

    let loaded = TestParameters::default();
    loaded.load_state(&saved.save_state()).expect("state loads");

    assert_eq!(loaded.gain.get_normalized(), 0.25);
    assert_eq!(loaded.enabled.get_normalized(), 1.0);
    assert_eq!(loaded.filter.cutoff.get_normalized(), 0.5);
}

#[test]
fn test_load_ignores_unknown_paths_and_keeps_the_rest() {
    let state = [
        entry("removed", 0.75),
        entry("gain", 0.25),
        entry("Removed Group/cutoff", 0.75),
        entry("Filter/removed", 0.75),
        entry("Filter/cutoff", 0.5),
    ]
    .concat();

    let parameters = TestParameters::default();
    parameters.load_state(&state).expect("state loads");

    assert_eq!(parameters.gain.get_normalized(), 0.25);
    assert_eq!(parameters.enabled.get_normalized(), 0.0);
    assert_eq!(parameters.filter.cutoff.get_normalized(), 0.5);
}

#[test]
fn test_load_clamps_values_and_stops_at_a_cut_off_entry() {
    let mut state = entry("gain", 1.5);
    // The last entry misses its value.
    state.extend_from_slice(&[13]);
    state.extend_from_slice(b"Filter/cutoff");

    let parameters = TestParameters::default();
    parameters.load_state(&state).expect("state loads");

    assert_eq!(parameters.gain.get_normalized(), 1.0);
    assert_eq!(parameters.filter.cutoff.get_normalized(), 0.0);
}
