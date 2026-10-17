//! The model of a plugin: everything it knows, shared by every thread.
//!
//! The model is the parameter struct of a plugin with all its fields: the
//! parameters, the `State` fields that are saved with them, the
//! `WebViewEvents` queue of the web UI and plain values such as a
//! `Handover`. It lives in one `Arc` for the life of the instance.
//!
//! [`Parameters`] covers the parameters alone. [`ModelFields`] adds what
//! the model holds beyond them and saves and loads all of it.
//! [`SharedModel`] is the model as the format wrappers and the editor use
//! it, behind a `dyn`.

use std::any::Any;

use crate::parameter_store::ParameterStore;
use crate::parameter_types::Parameters;
use crate::state_format::{join_path, SavedState, StateEntries, StateWriter};
use crate::webview_events::WebViewEvents;

/// What a model holds beyond its parameters, and saving and loading all of
/// it.
///
/// `#[derive(Parameters)]` generates this for a struct and each of its
/// nested structs. A hand-written [`Parameters`] implementation adds an
/// empty `impl ModelFields for MyParameters {}`: the defaults save and load
/// its parameters and report no `State` fields and no event queue.
pub trait ModelFields: Parameters {
    /// The plugin's queue of events for the web UI: a [`WebViewEvents`]
    /// field of this struct, or else the first one of a nested struct in
    /// field order.
    ///
    /// The format wrappers deliver what is emitted into it while an editor
    /// is open. `#[derive(Parameters)]` finds the field by its type; the
    /// default is for structs without one.
    fn webview_events(&self) -> Option<&WebViewEvents> {
        None
    }

    /// A number that changes whenever a `State` field of the plugin is set:
    /// the sum of their generations, nested structs included.
    ///
    /// A format wrapper compares it with the value it saw last to tell the
    /// host that the plugin state changed. Parameters are not part of it:
    /// the host learns about their changes through the edits.
    /// `#[derive(Parameters)]` generates it; the default is for structs
    /// without `State` fields.
    fn state_generation(&self) -> u64 {
        0
    }

    /// Call `visit` for every `State` field with its path: the `id` of its
    /// attribute, under the field names of the nested structs it sits in,
    /// as in a saved state (`"zoom"`, `"layer/sample"`).
    ///
    /// A web UI reaches the `State` fields through it. Not for the audio
    /// thread: the path of a field in a nested struct is built as a
    /// `String`. `#[derive(Parameters)]` generates it; the default is for
    /// structs without `State` fields.
    fn for_each_state(&self, _visit: &mut dyn FnMut(&str, &dyn SavedState)) {}

    // =========================================================================
    // State (see `state_format` for the bytes)
    // =========================================================================

    /// Add the entries of this struct to a state that is being saved.
    ///
    /// `#[derive(Parameters)]` generates this: every field under its id,
    /// with the entries of a nested struct under the name of its field.
    /// `prefix` is the path of this struct, empty for the root.
    ///
    /// A host may save the state from one of its own threads while audio
    /// runs, so this may only read parameter values (they are atomic),
    /// `State` fields and immutable data.
    ///
    /// The default is for hand-written flat implementations: every
    /// parameter under its numeric id.
    fn save_entries(&self, writer: &mut StateWriter, prefix: &str) {
        for parameter in self.iter() {
            let path = join_path(prefix, &parameter.id().to_string());
            writer.parameter(&path, parameter);
        }
    }

    /// Take the values of this struct from the entries of a saved state.
    ///
    /// The counterpart of [`save_entries`](Self::save_entries), generated
    /// by `#[derive(Parameters)]` as well. Every field asks for its entry
    /// and goes back to its default when there is none it can use.
    ///
    /// Takes `&self`: a host may load a state from one of its own threads
    /// while audio runs. This may only write parameter values (they are
    /// atomic) and `State` fields. The framework resets the smoothers on
    /// the audio thread afterwards.
    fn load_entries(&self, entries: &StateEntries<'_>, prefix: &str) {
        for parameter in self.iter() {
            let path = join_path(prefix, &parameter.id().to_string());
            entries.load_parameter(&path, parameter);
        }
    }

    /// Save the state of the plugin: all parameters and `State` fields.
    ///
    /// See [`state_format`](crate::state_format) for the bytes.
    fn save_state(&self) -> Vec<u8> {
        let mut writer = StateWriter::new();
        self.save_entries(&mut writer, "");
        writer.finish()
    }

    /// Load a state that [`save_state`](Self::save_state) produced.
    ///
    /// Empty data changes nothing. Data that is not a state of this format
    /// is refused and changes nothing. Otherwise every field gets the value
    /// of its entry or its default: a load is always a whole state and
    /// cannot be used to change some fields only.
    fn load_state(&self, data: &[u8]) -> Result<(), String> {
        let Some(entries) = StateEntries::parse(data)? else {
            return Ok(());
        };
        self.load_entries(&entries, "");
        entries.log_unused();
        Ok(())
    }
}

/// The model of a plugin as the format wrappers and the editor use it.
///
/// Everything behind this trait works through a shared reference. The
/// parameter values are atomic, `State` fields and the event queue are
/// shared and the rest is immutable. That is what lets a wrapper answer the
/// host and keep an editor in step while the audio thread renders.
///
/// Implemented for every model, so a wrapper turns its `Arc` of the model
/// into an `Arc<dyn SharedModel>`.
pub trait SharedModel: Send + Sync {
    /// The values and metadata of the parameters, as the host sees them.
    fn parameter_store(&self) -> &dyn ParameterStore;

    /// The parameters: their ids and their groups.
    fn parameters(&self) -> &dyn Parameters;

    /// See [`ModelFields::state_generation`].
    fn state_generation(&self) -> u64;

    /// See [`ModelFields::for_each_state`].
    fn for_each_state(&self, visit: &mut dyn FnMut(&str, &dyn SavedState));

    /// See [`ModelFields::save_state`].
    fn save_state(&self) -> Vec<u8>;

    /// See [`ModelFields::load_state`].
    fn load_state(&self, data: &[u8]) -> Result<(), String>;

    /// The model as `Any`, to get back at its concrete type.
    fn as_any(&self) -> &dyn Any;
}

impl<T: ModelFields + ParameterStore + 'static> SharedModel for T {
    fn parameter_store(&self) -> &dyn ParameterStore {
        self
    }

    fn parameters(&self) -> &dyn Parameters {
        self
    }

    fn state_generation(&self) -> u64 {
        ModelFields::state_generation(self)
    }

    fn for_each_state(&self, visit: &mut dyn FnMut(&str, &dyn SavedState)) {
        ModelFields::for_each_state(self, visit);
    }

    fn save_state(&self) -> Vec<u8> {
        ModelFields::save_state(self)
    }

    fn load_state(&self, data: &[u8]) -> Result<(), String> {
        ModelFields::load_state(self, data)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
