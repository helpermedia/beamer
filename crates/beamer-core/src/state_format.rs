//! The bytes a plugin state is saved to.
//!
//! The state of a plugin is the state of its model: the values
//! of the parameters and the [`State`](crate::State) fields. This module
//! is the one place that knows how those are laid out in bytes. A model
//! only says which entries it has ([`ModelFields::save_entries`]) and takes
//! its values back from a set of entries ([`ModelFields::load_entries`]).
//!
//! # Format, version 1
//!
//! All numbers are little-endian.
//!
//! | Field | Size | Value |
//! |-------|------|-------|
//! | magic | 4 | `42 4D 52 53` |
//! | version | u32 | 1 |
//! | payload length | u32 | number of bytes that follow |
//! | entries | payload length | see below |
//!
//! Each entry:
//!
//! | Field | Size | Value |
//! |-------|------|-------|
//! | kind | u8 | 1: parameter, 2: state |
//! | path length | u16 | number of bytes of the path |
//! | path | path length | UTF-8, segments separated by `/` |
//! | value length | u32 | number of bytes of the value |
//! | value | value length | kind 1: the normalized value as f64. kind 2: JSON |
//!
//! The path of a field is its id. A field of a nested group has
//! the name of the nested field in front: `filter/cutoff`.
//!
//! # Loading
//!
//! - Empty data changes nothing.
//! - Data with another magic or version, with a length that does not fit
//!   the data or with a path that is not UTF-8 is refused as a whole and
//!   changes nothing.
//! - Every field takes its value from its entry. A field without a usable
//!   entry goes back to its default (a `State` to the value it was created
//!   with), so what a load results in depends on the data alone and not on
//!   what the plugin held before.
//! - An entry is not usable when its kind does not fit the field, when a
//!   parameter value is not eight bytes or not a finite number, or when the
//!   JSON of a state does not fit its type. Such an entry is logged.
//! - An entry whose path no field asks for is skipped and logged. When a
//!   path occurs more than once, the last entry counts.
//!
//! A load is always a whole state: it cannot be used to change some fields
//! and leave the others.
//!
//! [`ModelFields::save_entries`]: crate::ModelFields::save_entries
//! [`ModelFields::load_entries`]: crate::ModelFields::load_entries

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use crate::parameter_types::ParameterRef;

/// The first four bytes of a saved state.
pub const MAGIC: [u8; 4] = [0x42, 0x4D, 0x52, 0x53];

/// The version of the format this module writes.
pub const VERSION: u32 = 1;

/// Entry kind of a parameter value.
const KIND_PARAMETER: u8 = 1;

/// Entry kind of a `State` value.
const KIND_STATE: u8 = 2;

/// Size of the magic, the version and the payload length.
const HEADER_SIZE: usize = 12;

/// Join a path prefix and the id of a field.
pub fn join_path(prefix: &str, id: &str) -> String {
    if prefix.is_empty() {
        id.to_string()
    } else {
        format!("{prefix}/{id}")
    }
}

/// A field that is saved as JSON: a [`State`](crate::State).
///
/// The format knows such a field only through this trait.
pub trait SavedState {
    /// The current value as JSON.
    fn save_json(&self) -> Result<Vec<u8>, String>;

    /// Set the value from JSON. Leaves the value when the JSON does not fit.
    fn load_json(&self, json: &[u8]) -> Result<(), String>;

    /// Set the value back to its default.
    fn reset_to_default(&self);

    /// How often the value was set. It changes with every change.
    fn generation(&self) -> u64;
}

/// Size of an entry without its path and its value: the kind and the two
/// lengths.
const ENTRY_OVERHEAD: usize = 7;

/// Collects the entries of a state that is being saved.
pub struct StateWriter {
    data: Vec<u8>,
    /// The paths written so far, to notice two fields with the same path.
    paths: HashSet<String>,
}

impl StateWriter {
    /// Start a state.
    pub fn new() -> Self {
        let mut data = Vec::with_capacity(256);
        data.extend_from_slice(&MAGIC);
        data.extend_from_slice(&VERSION.to_le_bytes());
        // The payload length is filled in by `finish`.
        data.extend_from_slice(&0u32.to_le_bytes());
        Self {
            data,
            paths: HashSet::new(),
        }
    }

    fn entry(&mut self, kind: u8, path: &str, value: &[u8]) {
        let (Ok(path_length), Ok(value_length)) =
            (u16::try_from(path.len()), u32::try_from(value.len()))
        else {
            log::error!("State entry '{path}' is too large to be saved.");
            return;
        };
        // The payload length in the header is a u32. An entry that does not
        // fit any more is left out, so that the header stays right and the
        // rest of the state can be loaded.
        let payload = self.data.len() - HEADER_SIZE + ENTRY_OVERHEAD + path.len() + value.len();
        if u32::try_from(payload).is_err() {
            log::error!("State entry '{path}' does not fit into the state any more.");
            return;
        }
        // Two fields with the same path would both load the value of the
        // second one.
        if !self.paths.insert(path.to_string()) {
            log::warn!("Two fields are saved under the state path '{path}'.");
        }
        self.data.push(kind);
        self.data.extend_from_slice(&path_length.to_le_bytes());
        self.data.extend_from_slice(path.as_bytes());
        self.data.extend_from_slice(&value_length.to_le_bytes());
        self.data.extend_from_slice(value);
    }

    /// Add the value of a parameter.
    pub fn parameter(&mut self, path: &str, parameter: &dyn ParameterRef) {
        self.entry(
            KIND_PARAMETER,
            path,
            &parameter.get_normalized().to_le_bytes(),
        );
    }

    /// Add the value of a `State` field.
    pub fn state(&mut self, path: &str, state: &dyn SavedState) {
        match state.save_json() {
            Ok(json) => self.entry(KIND_STATE, path, &json),
            Err(error) => log::error!("State '{path}' cannot be saved: {error}"),
        }
    }

    /// End the state and return its bytes.
    pub fn finish(mut self) -> Vec<u8> {
        // `entry` keeps the payload within what the header can describe.
        let payload = (self.data.len() - HEADER_SIZE) as u32;
        self.data[8..HEADER_SIZE].copy_from_slice(&payload.to_le_bytes());
        self.data
    }
}

impl Default for StateWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// One entry of a saved state.
struct Entry<'a> {
    kind: u8,
    value: &'a [u8],
    /// Whether a field asked for this entry.
    used: Cell<bool>,
}

/// The entries of a saved state, for the fields to take their values from.
pub struct StateEntries<'a> {
    entries: HashMap<&'a str, Entry<'a>>,
}

impl<'a> StateEntries<'a> {
    /// Read the entries of a saved state.
    ///
    /// Checks the whole data before anything is applied. Returns `Ok(None)`
    /// for empty data, which stands for "no state".
    pub fn parse(data: &'a [u8]) -> Result<Option<Self>, String> {
        if data.is_empty() {
            return Ok(None);
        }
        if data.len() < HEADER_SIZE || data[..4] != MAGIC {
            return Err("not a state of this plugin format".to_string());
        }
        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != VERSION {
            return Err(format!(
                "state format version {version} is not supported (this is version {VERSION})"
            ));
        }
        let payload_length = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let Some(mut rest) = data[HEADER_SIZE..].get(..payload_length) else {
            return Err("the state is cut off".to_string());
        };

        let mut entries = HashMap::new();
        while !rest.is_empty() {
            let cut_off = || "a state entry is cut off".to_string();

            let (&kind, after) = rest.split_first().ok_or_else(cut_off)?;
            let (length, after) = after.split_first_chunk::<2>().ok_or_else(cut_off)?;
            let path_length = usize::from(u16::from_le_bytes(*length));
            let (path, after) = after.split_at_checked(path_length).ok_or_else(cut_off)?;
            let (length, after) = after.split_first_chunk::<4>().ok_or_else(cut_off)?;
            let value_length = u32::from_le_bytes(*length) as usize;
            let (value, after) = after.split_at_checked(value_length).ok_or_else(cut_off)?;
            rest = after;

            // The writer only writes UTF-8, so anything else is damage,
            // like an entry that is cut off.
            let path = std::str::from_utf8(path)
                .map_err(|_| "a state entry has a path that is not UTF-8".to_string())?;
            // A later entry with the same path replaces an earlier one.
            entries.insert(
                path,
                Entry {
                    kind,
                    value,
                    used: Cell::new(false),
                },
            );
        }

        Ok(Some(Self { entries }))
    }

    /// Find the entry of a field, if it has the kind the field expects.
    fn take(&self, path: &str, kind: u8) -> Option<&'a [u8]> {
        let entry = self.entries.get(path)?;
        entry.used.set(true);
        if entry.kind != kind {
            log::warn!(
                "State entry '{path}' has kind {} where {kind} is expected. Using the default.",
                entry.kind
            );
            return None;
        }
        Some(entry.value)
    }

    /// Give a parameter the value of its entry, or its default.
    pub fn load_parameter(&self, path: &str, parameter: &dyn ParameterRef) {
        let value = self.take(path, KIND_PARAMETER).and_then(|value| {
            let value = <[u8; 8]>::try_from(value).ok().map(f64::from_le_bytes);
            match value {
                Some(value) if value.is_finite() => Some(value.clamp(0.0, 1.0)),
                _ => {
                    log::warn!("State entry '{path}' is not a parameter value. Using the default.");
                    None
                }
            }
        });
        parameter.set_normalized(value.unwrap_or(parameter.info().default_normalized));
    }

    /// Give a `State` field the value of its entry, or its default.
    pub fn load_state(&self, path: &str, state: &dyn SavedState) {
        let loaded = match self.take(path, KIND_STATE) {
            Some(json) => match state.load_json(json) {
                Ok(()) => true,
                Err(error) => {
                    log::warn!("State entry '{path}' cannot be read: {error}. Using the default.");
                    false
                }
            },
            None => false,
        };
        if !loaded {
            state.reset_to_default();
        }
    }

    /// Log the entries that no field asked for.
    pub fn log_unused(&self) {
        for (path, entry) in &self.entries {
            if !entry.used.get() {
                log::warn!("Skipped state entry '{path}': the plugin has no such field.");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FloatParameter, ModelFields, Parameters, State};

    fn parameter(default: f64) -> FloatParameter {
        FloatParameter::new("Test", default, 0.0..=1.0)
    }

    /// The bytes of one entry.
    fn entry(kind: u8, path: &str, value: &[u8]) -> Vec<u8> {
        let mut bytes = vec![kind];
        bytes.extend_from_slice(&(path.len() as u16).to_le_bytes());
        bytes.extend_from_slice(path.as_bytes());
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(value);
        bytes
    }

    /// A state with the given entries.
    fn state(entries: &[Vec<u8>]) -> Vec<u8> {
        let payload = entries.concat();
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
        bytes
    }

    fn parse(data: &[u8]) -> StateEntries<'_> {
        StateEntries::parse(data)
            .expect("the state is valid")
            .expect("the state is not empty")
    }

    #[test]
    fn test_writer_produces_the_documented_bytes() {
        let gain = parameter(0.0);
        gain.set_normalized(0.25);
        let name = State::new(String::from("a"));

        let mut writer = StateWriter::new();
        writer.parameter("gain", &gain);
        writer.state("filter/name", &name);

        // Written out in full, so that a change of the format fails here.
        let expected: Vec<u8> = [
            // Magic, version 1, 40 bytes of payload.
            &[0x42, 0x4D, 0x52, 0x53][..],
            &[1, 0, 0, 0],
            &[40, 0, 0, 0],
            // A parameter: kind, path, eight bytes of f64.
            &[1],
            &[4, 0],
            b"gain",
            &[8, 0, 0, 0],
            &[0, 0, 0, 0, 0, 0, 0xD0, 0x3F],
            // A state: kind, path, JSON.
            &[2],
            &[11, 0],
            b"filter/name",
            &[3, 0, 0, 0],
            b"\"a\"",
        ]
        .concat();
        assert_eq!(writer.finish(), expected);
    }

    #[test]
    fn test_empty_writer_is_a_header() {
        assert_eq!(
            StateWriter::new().finish(),
            [0x42, 0x4D, 0x52, 0x53, 1, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn test_entries_give_fields_their_values() {
        let data = state(&[
            entry(KIND_PARAMETER, "gain", &0.25f64.to_le_bytes()),
            entry(KIND_STATE, "name", b"\"kick\""),
        ]);
        let entries = parse(&data);

        let gain = parameter(0.5);
        let name = State::new(String::from("before"));
        entries.load_parameter("gain", &gain);
        entries.load_state("name", &name);

        assert_eq!(gain.get_normalized(), 0.25);
        assert_eq!(*name.get(), "kick");
    }

    #[test]
    fn test_field_without_an_entry_gets_its_default() {
        let data = state(&[]);
        let entries = parse(&data);

        let gain = parameter(0.5);
        gain.set_normalized(0.9);
        let name = State::new(String::from("initial"));
        name.set(String::from("before"));
        entries.load_parameter("gain", &gain);
        entries.load_state("name", &name);

        assert_eq!(gain.get_normalized(), 0.5);
        assert_eq!(*name.get(), "initial");
    }

    #[test]
    fn test_entry_that_does_not_fit_gives_the_default() {
        let data = state(&[
            // A state where a parameter is expected and the other way round.
            entry(KIND_STATE, "gain", b"0.25"),
            entry(KIND_PARAMETER, "name", &0.25f64.to_le_bytes()),
            // Not eight bytes, not finite.
            entry(KIND_PARAMETER, "short", &[0, 0, 0xD0, 0x3F]),
            entry(KIND_PARAMETER, "nan", &f64::NAN.to_le_bytes()),
            entry(KIND_PARAMETER, "infinite", &f64::INFINITY.to_le_bytes()),
            // JSON of another type.
            entry(KIND_STATE, "number", b"\"text\""),
            // A kind of a later version.
            entry(3, "future", &[1, 2, 3]),
        ]);
        let entries = parse(&data);

        for path in ["gain", "short", "nan", "infinite", "future"] {
            let field = parameter(0.5);
            field.set_normalized(0.9);
            entries.load_parameter(path, &field);
            assert_eq!(field.get_normalized(), 0.5, "{path}");
        }
        let name = State::new(String::from("initial"));
        name.set(String::from("before"));
        entries.load_state("name", &name);
        assert_eq!(*name.get(), "initial");
        let number = State::new(7u32);
        number.set(8);
        entries.load_state("number", &number);
        assert_eq!(*number.get(), 7);
    }

    #[test]
    fn test_parameter_values_are_clamped() {
        let data = state(&[
            entry(KIND_PARAMETER, "high", &1.5f64.to_le_bytes()),
            entry(KIND_PARAMETER, "low", &(-0.5f64).to_le_bytes()),
        ]);
        let entries = parse(&data);

        let high = parameter(0.5);
        let low = parameter(0.5);
        entries.load_parameter("high", &high);
        entries.load_parameter("low", &low);
        assert_eq!(high.get_normalized(), 1.0);
        assert_eq!(low.get_normalized(), 0.0);
    }

    #[test]
    fn test_last_entry_of_a_path_counts() {
        let data = state(&[
            entry(KIND_PARAMETER, "gain", &0.25f64.to_le_bytes()),
            entry(KIND_PARAMETER, "gain", &0.75f64.to_le_bytes()),
        ]);
        let gain = parameter(0.0);
        parse(&data).load_parameter("gain", &gain);
        assert_eq!(gain.get_normalized(), 0.75);
    }

    #[test]
    fn test_empty_data_is_no_state() {
        assert!(StateEntries::parse(&[]).expect("empty data is valid").is_none());
    }

    #[test]
    fn test_data_that_is_not_a_state_is_refused() {
        let valid = state(&[entry(KIND_PARAMETER, "gain", &0.25f64.to_le_bytes())]);
        assert!(StateEntries::parse(&valid).is_ok());

        // Another magic.
        let mut other = valid.clone();
        other[0] = b'X';
        assert!(StateEntries::parse(&other).is_err());

        // Another version.
        let mut newer = valid.clone();
        newer[4] = 2;
        assert!(StateEntries::parse(&newer).is_err());

        // Shorter than a header.
        assert!(StateEntries::parse(&valid[..7]).is_err());

        // Cut off anywhere after the header: the payload is not all there.
        for length in HEADER_SIZE..valid.len() {
            assert!(StateEntries::parse(&valid[..length]).is_err(), "{length}");
        }

        // A payload length that cuts an entry in two.
        for payload in 1..(valid.len() - HEADER_SIZE) {
            let mut cut = valid.clone();
            cut[8] = payload as u8;
            assert!(StateEntries::parse(&cut).is_err(), "{payload}");
        }
    }

    #[test]
    fn test_bytes_after_the_payload_are_ignored() {
        let mut data = state(&[entry(KIND_PARAMETER, "gain", &0.25f64.to_le_bytes())]);
        data.extend_from_slice(&[0xFF; 5]);

        let gain = parameter(0.0);
        parse(&data).load_parameter("gain", &gain);
        assert_eq!(gain.get_normalized(), 0.25);
    }

    #[test]
    fn test_empty_payload_resets_every_field() {
        // A header with no entries, and bytes after it that do not count.
        let mut data = state(&[]);
        data.extend_from_slice(&[0xFF; 3]);
        let entries = parse(&data);

        let gain = parameter(0.5);
        gain.set_normalized(0.9);
        entries.load_parameter("gain", &gain);
        assert_eq!(gain.get_normalized(), 0.5);
    }

    #[test]
    fn test_path_that_is_not_utf8_refuses_the_state() {
        let mut broken = entry(KIND_PARAMETER, "gain", &0.25f64.to_le_bytes());
        // The first byte of the path.
        broken[3] = 0xFF;
        let data = state(&[
            entry(KIND_PARAMETER, "other", &0.25f64.to_le_bytes()),
            broken,
        ]);
        assert!(StateEntries::parse(&data).is_err());
    }

    #[test]
    fn test_entry_that_is_too_large_is_left_out() {
        let gain = parameter(0.0);
        gain.set_normalized(0.25);
        let long_path = "x".repeat(usize::from(u16::MAX) + 1);

        let mut writer = StateWriter::new();
        writer.parameter(&long_path, &gain);
        writer.parameter("gain", &gain);
        let data = writer.finish();

        // The state is whole without the entry.
        assert_eq!(data, state(&[entry(KIND_PARAMETER, "gain", &0.25f64.to_le_bytes())]));
    }

    /// A parameter struct that is written by hand, with a parameter and a
    /// state.
    struct Manual {
        gain: FloatParameter,
        name: State<String>,
        /// Whether the entries have paths of their own or the default ones.
        named: bool,
    }

    impl Manual {
        fn new(named: bool) -> Self {
            Self {
                gain: parameter(0.5).with_id(7),
                name: State::new(String::from("initial")),
                named,
            }
        }
    }

    impl crate::ParameterGroups for Manual {}

    impl crate::Parameters for Manual {
        fn count(&self) -> usize {
            1
        }

        fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
            Box::new(std::iter::once(&self.gain as &dyn ParameterRef))
        }

        fn by_id(&self, id: crate::ParameterId) -> Option<&dyn ParameterRef> {
            (id == 7).then_some(&self.gain as &dyn ParameterRef)
        }
    }

    impl crate::ModelFields for Manual {
        fn save_entries(&self, writer: &mut StateWriter, prefix: &str) {
            if self.named {
                writer.parameter(&join_path(prefix, "gain"), &self.gain);
                writer.state(&join_path(prefix, "name"), &self.name);
            } else {
                for parameter in self.iter() {
                    writer.parameter(&join_path(prefix, &parameter.id().to_string()), parameter);
                }
            }
        }

        fn load_entries(&self, entries: &StateEntries<'_>, prefix: &str) {
            if self.named {
                entries.load_parameter(&join_path(prefix, "gain"), &self.gain);
                entries.load_state(&join_path(prefix, "name"), &self.name);
            } else {
                for parameter in self.iter() {
                    entries.load_parameter(&join_path(prefix, &parameter.id().to_string()), parameter);
                }
            }
        }
    }

    #[test]
    fn test_parameters_and_state_round_trip_through_a_parameter_struct() {
        let saved = Manual::new(true);
        saved.gain.set_normalized(0.25);
        saved.name.set(String::from("kick"));
        let data = saved.save_state();

        let loaded = Manual::new(true);
        loaded.load_state(&data).expect("the state loads");
        assert_eq!(loaded.gain.get_normalized(), 0.25);
        assert_eq!(*loaded.name.get(), "kick");
        assert_eq!(loaded.save_state(), data);

        // Data that is not a state changes nothing.
        assert!(loaded.load_state(&data[..data.len() - 1]).is_err());
        assert_eq!(loaded.gain.get_normalized(), 0.25);
        assert_eq!(*loaded.name.get(), "kick");
    }

    #[test]
    fn test_struct_without_names_is_saved_under_numeric_ids() {
        let saved = Manual::new(false);
        saved.gain.set_normalized(0.25);
        let data = saved.save_state();
        assert_eq!(data, state(&[entry(KIND_PARAMETER, "7", &0.25f64.to_le_bytes())]));

        let loaded = Manual::new(false);
        loaded.load_state(&data).expect("the state loads");
        assert_eq!(loaded.gain.get_normalized(), 0.25);
    }

    #[test]
    fn test_load_does_not_set_a_state_that_is_at_its_default_already() {
        let data = state(&[]);
        let entries = parse(&data);

        let name = State::new(String::from("initial"));
        let mut reader = name.reader();
        entries.load_state("name", &name);

        // Nothing changed for the audio thread.
        assert_eq!(name.generation(), 0);
        assert_eq!(reader.changed(), None);
    }

    #[test]
    fn test_join_path() {
        assert_eq!(join_path("", "gain"), "gain");
        assert_eq!(join_path("filter", "cutoff"), "filter/cutoff");
        assert_eq!(join_path("osc/filter", "cutoff"), "osc/filter/cutoff");
    }
}
