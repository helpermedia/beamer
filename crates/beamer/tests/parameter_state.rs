//! The state of a plugin with parameters only: the bytes its model is
//! saved to and loaded from.

#![cfg(feature = "derive")]

use beamer::prelude::*;

#[beamer::group]
struct FilterGroup {
    #[parameter(id = "cutoff", name = "Cutoff", default = 0.0, range = 0.0..=1.0)]
    cutoff: FloatParameter,
}

#[beamer::model]
struct TestModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    #[parameter(id = "enabled", name = "Enabled", default = false)]
    enabled: BoolParameter,

    #[nested(group = "Filter")]
    filter: FilterGroup,
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

fn values(model: &TestModel) -> [f64; 3] {
    [
        model.gain.get_normalized(),
        model.enabled.get_normalized(),
        model.filter.cutoff.get_normalized(),
    ]
}

#[test]
fn test_state_has_the_documented_bytes() {
    let model = TestModel::default();
    model.gain.set_normalized(0.25);
    model.enabled.set_normalized(1.0);
    model.filter.cutoff.set_normalized(0.5);

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
        // A nested group is saved under the name of its field, not under
        // the display name of its group.
        &[1],
        &[13, 0],
        b"filter/cutoff",
        &[8, 0, 0, 0],
        &[0, 0, 0, 0, 0, 0, 0xE0, 0x3F],
    ]
    .concat();

    assert_eq!(model.save_state(), expected);
}

#[test]
fn test_state_round_trip_restores_every_value() {
    let saved = TestModel::default();
    saved.gain.set_normalized(0.25);
    saved.enabled.set_normalized(1.0);
    saved.filter.cutoff.set_normalized(0.5);

    let loaded = TestModel::default();
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

    let model = TestModel::default();
    model.load_state(&state).expect("state loads");

    assert_eq!(values(&model), [0.25, 0.0, 0.5]);
}

#[test]
fn test_load_depends_on_the_state_alone() {
    // Only one of the three parameters is in the state.
    let state = state(&[entry("filter/cutoff", 0.5)]);

    let model = TestModel::default();
    model.gain.set_normalized(0.9);
    model.enabled.set_normalized(1.0);
    model.load_state(&state).expect("state loads");

    // The others are back at their defaults, whatever they were before.
    assert_eq!(values(&model), [0.0, 0.0, 0.5]);
}

#[test]
fn test_load_clamps_values() {
    let state = state(&[entry("gain", 1.5), entry("filter/cutoff", -1.0)]);

    let model = TestModel::default();
    model.load_state(&state).expect("state loads");

    assert_eq!(values(&model), [1.0, 0.0, 0.0]);
}

#[test]
fn test_empty_data_changes_nothing() {
    let model = TestModel::default();
    model.gain.set_normalized(0.9);

    model.load_state(&[]).expect("no state is not an error");
    assert_eq!(values(&model), [0.9, 0.0, 0.0]);
}

#[test]
fn test_data_that_is_not_a_state_is_refused_and_changes_nothing() {
    let model = TestModel::default();
    model.gain.set_normalized(0.9);
    let valid = state(&[entry("gain", 0.25), entry("filter/cutoff", 0.5)]);

    // Cut off in the middle of the second entry.
    assert!(model.load_state(&valid[..valid.len() - 4]).is_err());
    // Another version.
    let mut newer = valid.clone();
    newer[4] = 2;
    assert!(model.load_state(&newer).is_err());
    // Something else entirely.
    assert!(model.load_state(b"not a state at all").is_err());

    assert_eq!(values(&model), [0.9, 0.0, 0.0]);
}
