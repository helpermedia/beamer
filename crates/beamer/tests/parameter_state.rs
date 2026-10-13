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

/// One parameter entry: kind, path and the normalized value.
fn entry(path: &str, normalized: f64) -> Vec<u8> {
    let mut bytes = vec![1u8];
    bytes.extend_from_slice(&(path.len() as u16).to_le_bytes());
    bytes.extend_from_slice(path.as_bytes());
    bytes.extend_from_slice(&8u32.to_le_bytes());
    bytes.extend_from_slice(&normalized.to_le_bytes());
    bytes
}

/// A state of format version 1 with the given entries.
fn state(entries: &[Vec<u8>]) -> Vec<u8> {
    let payload = entries.concat();
    let mut bytes = vec![0x42, 0x4D, 0x52, 0x53, 1, 0, 0, 0];
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

fn values(parameters: &TestParameters) -> [f64; 3] {
    [
        parameters.gain.get_normalized(),
        parameters.enabled.get_normalized(),
        parameters.filter.cutoff.get_normalized(),
    ]
}

#[test]
fn test_state_has_the_documented_bytes() {
    let parameters = TestParameters::default();
    parameters.gain.set_normalized(0.25);
    parameters.enabled.set_normalized(1.0);
    parameters.filter.cutoff.set_normalized(0.5);

    // Written out in full, so that a change of the format fails here.
    let expected: Vec<u8> = [
        // Magic, version 1, 69 bytes of payload.
        &[0x42, 0x4D, 0x52, 0x53][..],
        &[1, 0, 0, 0],
        &[69, 0, 0, 0],
        // Kind 1 (parameter), path, eight bytes of f64.
        &[1],
        &[4, 0],
        b"gain",
        &[8, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0xD0, 0x3F],
        &[1],
        &[7, 0],
        b"enabled",
        &[8, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0xF0, 0x3F],
        // A nested struct is saved under the name of its field, not under
        // the display name of its group.
        &[1],
        &[13, 0],
        b"filter/cutoff",
        &[8, 0, 0, 0],
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

    assert_eq!(values(&loaded), [0.25, 1.0, 0.5]);
}

#[test]
fn test_load_skips_unknown_paths_and_keeps_the_rest() {
    let state = state(&[
        entry("removed", 0.75),
        entry("gain", 0.25),
        entry("removed_group/cutoff", 0.75),
        entry("filter/removed", 0.75),
        entry("filter/cutoff", 0.5),
    ]);

    let parameters = TestParameters::default();
    parameters.load_state(&state).expect("state loads");

    assert_eq!(values(&parameters), [0.25, 0.0, 0.5]);
}

#[test]
fn test_load_depends_on_the_state_alone() {
    // Only one of the three parameters is in the state.
    let state = state(&[entry("filter/cutoff", 0.5)]);

    let parameters = TestParameters::default();
    parameters.gain.set_normalized(0.9);
    parameters.enabled.set_normalized(1.0);
    parameters.load_state(&state).expect("state loads");

    // The others are back at their defaults, whatever they were before.
    assert_eq!(values(&parameters), [0.0, 0.0, 0.5]);
}

#[test]
fn test_load_clamps_values() {
    let state = state(&[entry("gain", 1.5), entry("filter/cutoff", -1.0)]);

    let parameters = TestParameters::default();
    parameters.load_state(&state).expect("state loads");

    assert_eq!(values(&parameters), [1.0, 0.0, 0.0]);
}

#[test]
fn test_empty_data_changes_nothing() {
    let parameters = TestParameters::default();
    parameters.gain.set_normalized(0.9);

    parameters.load_state(&[]).expect("no state is not an error");
    assert_eq!(values(&parameters), [0.9, 0.0, 0.0]);
}

#[test]
fn test_data_that_is_not_a_state_is_refused_and_changes_nothing() {
    let parameters = TestParameters::default();
    parameters.gain.set_normalized(0.9);
    let valid = state(&[entry("gain", 0.25), entry("filter/cutoff", 0.5)]);

    // Cut off in the middle of the second entry.
    assert!(parameters.load_state(&valid[..valid.len() - 4]).is_err());
    // Another version.
    let mut newer = valid.clone();
    newer[4] = 2;
    assert!(parameters.load_state(&newer).is_err());
    // Something else entirely.
    assert!(parameters.load_state(b"not a state at all").is_err());

    assert_eq!(values(&parameters), [0.9, 0.0, 0.0]);
}
