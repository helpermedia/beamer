//! Plugin state beyond parameters.
//!
//! A [`State`] holds a value that is saved and restored with the plugin
//! state, next to the parameters: the path of a loaded sample, the layout of
//! an editor, a table the user edited. It lives in the parameter struct,
//! which every thread of a plugin instance shares.
//!
//! A `State` is a [`Handover`] that is also saved: host and UI threads set
//! it as a whole, and the audio thread reads it through a
//! [`HandoverReader`] without waiting.
//!
//! ```ignore
//! // On a host or UI thread, for example in a WebView handler:
//! parameters.sample_path.set(String::from("kick.wav"));
//!
//! // In prepare():
//! SamplerProcessor { sample_path: self.parameters.sample_path.reader(), /* ... */ }
//!
//! // In process():
//! if let Some(path) = self.sample_path.changed() { /* ... */ }
//! ```
//!
//! # What can be saved
//!
//! The value is saved as JSON through serde, so `T` can be any type that
//! derives `Serialize` and `Deserialize`. A plugin without a serde
//! dependency of its own derives through the re-export:
//! `#[serde(crate = "beamer::serde")]`.
//!
//! A few things JSON cannot hold, and a value that contains them does not
//! come back from a saved state:
//!
//! - Floats that are not finite. NaN and the infinities are written as
//!   `null` and refused when read as a float. Finite floats come back
//!   exactly.
//! - Maps whose keys are not strings or integers.
//!
//! A `HashMap` is saved in a different order each time, so two saves of the
//! same value differ in their bytes. Use a `BTreeMap` where that matters.

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::handover::{Handover, HandoverReader};

/// A value that is saved with the plugin state and handed over to the audio
/// thread.
///
/// See the [module documentation](self).
pub struct State<T> {
    value: Handover<T>,
    /// The value this state was created with, which `reset()` goes back to.
    initial: Arc<T>,
}

impl<T> State<T>
where
    T: Serialize + DeserializeOwned + Send + Sync,
{
    /// Create a state that holds `value`.
    ///
    /// This is also the value the state goes back to with
    /// [`reset()`](Self::reset), for example when a saved plugin state has
    /// no value for it.
    pub fn new(value: T) -> Self {
        let initial = Arc::new(value);
        let value = Handover::new_shared(Arc::clone(&initial));
        Self { value, initial }
    }

    /// Get the current value.
    ///
    /// For host and UI threads. The audio thread reads through a
    /// [`reader()`](Self::reader).
    pub fn get(&self) -> Arc<T> {
        self.value.get()
    }

    /// Replace the value.
    ///
    /// For host and UI threads, such as a WebView handler. Every reader
    /// gets the new value with its next call.
    pub fn set(&self, value: impl Into<Arc<T>>) {
        self.value.set(value);
    }

    /// Create a reader for the audio thread.
    ///
    /// Call this in `prepare()` and keep the reader in the processor.
    pub fn reader(&self) -> HandoverReader<T> {
        self.value.reader()
    }

    /// How often the value was set, by the plugin or by a loaded plugin
    /// state.
    ///
    /// Takes no lock. A UI can compare this with the number it saw last to
    /// find out whether the value changed behind its back.
    pub fn generation(&self) -> u64 {
        self.value.generation()
    }

    /// Set the value back to the one this state was created with.
    pub fn reset(&self) {
        self.set(Arc::clone(&self.initial));
    }

    /// The current value as it is saved: JSON.
    pub fn save(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&*self.get()).map_err(|error| error.to_string())
    }

    /// Set the value from saved JSON.
    ///
    /// Leaves the value as it is when the bytes cannot be read as a `T`.
    pub fn load(&self, bytes: &[u8]) -> Result<(), String> {
        let value: T = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
        self.set(value);
        Ok(())
    }
}

impl<T> Default for State<T>
where
    T: Serialize + DeserializeOwned + Default + Send + Sync,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
    struct Sample {
        path: String,
        root_note: u8,
        gain: f64,
        loop_points: Option<(u32, u32)>,
    }

    fn sample() -> Sample {
        Sample {
            path: String::from("kick \"909\".wav"),
            root_note: 60,
            gain: 0.1 + 0.2,
            loop_points: Some((128, 4096)),
        }
    }

    #[test]
    fn test_set_get_and_reset() {
        let state = State::<Sample>::default();
        assert_eq!(*state.get(), Sample::default());

        state.set(sample());
        assert_eq!(*state.get(), sample());
        assert_eq!(state.generation(), 1);

        state.reset();
        assert_eq!(*state.get(), Sample::default());
        assert_eq!(state.generation(), 2);
    }

    #[test]
    fn test_reset_goes_back_to_the_value_of_new() {
        let state = State::new(String::from("init.wav"));
        state.set(String::from("kick.wav"));

        state.reset();
        assert_eq!(*state.get(), "init.wav");
    }

    #[test]
    fn test_floats_that_are_not_finite_do_not_come_back() {
        let state = State::new(vec![1.0f64, f64::NAN]);
        let saved = state.save().expect("it is written, as null");
        assert_eq!(saved, b"[1.0,null]");

        let loaded = State::<Vec<f64>>::default();
        assert!(loaded.load(&saved).is_err());
    }

    #[test]
    fn test_reader_follows_the_state() {
        let state = State::new(String::from("first"));
        let mut reader = state.reader();
        assert_eq!(reader.current(), "first");

        state.set(String::from("second"));
        assert_eq!(reader.changed().map(String::as_str), Some("second"));
        assert_eq!(reader.changed(), None);
    }

    #[test]
    fn test_saved_state_loads_to_the_same_value() {
        let state = State::new(sample());
        let saved = state.save().expect("the sample can be saved");

        let loaded = State::<Sample>::default();
        loaded.load(&saved).expect("the saved bytes can be loaded");
        assert_eq!(*loaded.get(), sample());

        // Saving what was loaded gives the same bytes again.
        assert_eq!(loaded.save().expect("it can be saved again"), saved);
    }

    #[test]
    fn test_saved_bytes_are_json() {
        let state = State::new(vec![1u8, 2, 3]);
        assert_eq!(state.save().unwrap(), b"[1,2,3]");

        let state = State::new(String::from("a"));
        assert_eq!(state.save().unwrap(), b"\"a\"");
    }

    #[test]
    fn test_loading_reaches_a_reader() {
        let state = State::<Vec<u8>>::default();
        let mut reader = state.reader();

        state.load(b"[4,5]").expect("valid JSON for the type");
        assert_eq!(reader.changed().map(Vec::as_slice), Some(&[4u8, 5][..]));
    }

    #[test]
    fn test_bytes_that_do_not_fit_leave_the_value() {
        let state = State::new(sample());
        let generation = state.generation();

        assert!(state.load(b"not json").is_err());
        assert!(state.load(b"[1,2,3]").is_err());
        assert!(state.load(b"").is_err());
        assert_eq!(*state.get(), sample());
        assert_eq!(state.generation(), generation);
    }
}
