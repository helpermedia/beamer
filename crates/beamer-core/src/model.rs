//! The model of a plugin: everything it knows, shared by every thread.
//!
//! The model is the struct of a plugin that holds what every thread
//! shares: the parameters, the `State` fields that are saved with them, the
//! `WebViewEventQueue` of the web UI and plain values such as a
//! `Handover`. It lives in one `Arc` for the life of the instance.
//!
//! [`Parameters`] covers the parameters alone. [`Saved`] adds the `State`
//! fields and saves and loads the parameters and `State` fields together.
//! [`WebViewEvents`] tells where the event queue of the web UI is.
//! [`SharedModel`] is the model as the format wrappers and the editor use
//! it, behind a `dyn`.

use std::any::Any;

use crate::parameter_store::ParameterStore;
use crate::parameter_types::Parameters;
use crate::state_format::{join_path, SavedState, StateEntries, StateWriter};
use crate::webview_events::WebViewEventQueue;

/// A struct whose fields are saved with the plugin: its parameters and
/// `State` fields, nested groups included, and saving and loading them.
///
/// `#[beamer::model]` and `#[beamer::group]` generate this for a model and
/// each of its nested groups. A hand-written model or group adds an empty
/// `impl Saved for MyModel {}`: the defaults save and load its parameters
/// and report no `State` fields.
pub trait Saved: Parameters {
    /// A number that changes whenever a `State` field of the plugin is set:
    /// the sum of their generations, nested groups included.
    ///
    /// A format wrapper compares it with the value it saw last to tell the
    /// host that the plugin state changed. Parameters are not part of it:
    /// the host learns about their changes through the edits.
    /// `#[beamer::model]` and `#[beamer::group]` generate it; the default is for structs
    /// without `State` fields.
    fn state_generation(&self) -> u64 {
        0
    }

    /// Call `visit` for every `State` field with its path: the `id` of its
    /// attribute, under the field names of the nested groups it sits in,
    /// as in a saved state (`"zoom"`, `"layer/sample"`).
    ///
    /// A web UI reaches the `State` fields through it. Not for the audio
    /// thread: the path of a field in a nested group is built as a
    /// `String`. `#[beamer::model]` and `#[beamer::group]` generate it; the default is for
    /// structs without `State` fields.
    fn for_each_state(&self, _visit: &mut dyn FnMut(&str, &dyn SavedState)) {}

    // =========================================================================
    // State (see `state_format` for the bytes)
    // =========================================================================

    /// Add the entries of this struct to a state that is being saved.
    ///
    /// `#[beamer::model]` and `#[beamer::group]` generate this: every field under its id,
    /// with the entries of a nested group under the name of its field.
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
    /// by `#[beamer::model]` and `#[beamer::group]` as well. Every field asks for its entry
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

/// Where the queue of events for the web UI is: a [`WebViewEventQueue`]
/// field of this struct, or else the first one of a nested group in field
/// order.
///
/// `#[beamer::model]` and `#[beamer::group]` generate this for a model and
/// each of its nested groups. A hand-written model or group adds an empty
/// `impl WebViewEvents for MyModel {}`, or returns its queue.
pub trait WebViewEvents {
    /// The plugin's queue of events for the web UI.
    ///
    /// The format wrappers deliver what is emitted into it while an editor
    /// is open. `#[beamer::model]` and `#[beamer::group]` find the field by its type; the
    /// default is for structs without one.
    fn webview_events(&self) -> Option<&WebViewEventQueue> {
        None
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

    /// See [`Saved::state_generation`].
    fn state_generation(&self) -> u64;

    /// See [`Saved::for_each_state`].
    fn for_each_state(&self, visit: &mut dyn FnMut(&str, &dyn SavedState));

    /// See [`Saved::save_state`].
    fn save_state(&self) -> Vec<u8>;

    /// See [`Saved::load_state`].
    fn load_state(&self, data: &[u8]) -> Result<(), String>;

    /// The model as `Any`, to get back at its concrete type.
    fn as_any(&self) -> &dyn Any;
}

impl<T: Saved + 'static> SharedModel for T {
    fn parameter_store(&self) -> &dyn ParameterStore {
        self
    }

    fn parameters(&self) -> &dyn Parameters {
        self
    }

    fn state_generation(&self) -> u64 {
        Saved::state_generation(self)
    }

    fn for_each_state(&self, visit: &mut dyn FnMut(&str, &dyn SavedState)) {
        Saved::for_each_state(self, visit);
    }

    fn save_state(&self) -> Vec<u8> {
        Saved::save_state(self)
    }

    fn load_state(&self, data: &[u8]) -> Result<(), String> {
        Saved::load_state(self, data)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Make a struct a model by forwarding the model traits to one of its
/// fields, for test models in the format wrappers, which cannot use
/// `#[beamer::model]`.
///
/// `__forward_model!(MidiModel, parameters)` implements `ParameterGroups`,
/// `Parameters`, `Saved` and `WebViewEvents` for `MidiModel` through
/// its field `parameters`. The rest of the struct (a `Mutex`, a counter) is
/// what the test looks at.
#[doc(hidden)]
#[macro_export]
macro_rules! __forward_model {
    ($model:ty, $field:ident) => {
        impl $crate::parameter_groups::ParameterGroups for $model {
            fn group_count(&self) -> usize {
                $crate::parameter_groups::ParameterGroups::group_count(&self.$field)
            }

            fn group_info(&self, index: usize) -> Option<$crate::parameter_groups::GroupInfo> {
                $crate::parameter_groups::ParameterGroups::group_info(&self.$field, index)
            }

            fn find_group_by_name(&self, name: &str) -> Option<$crate::parameter_groups::GroupId> {
                $crate::parameter_groups::ParameterGroups::find_group_by_name(&self.$field, name)
            }
        }

        impl $crate::parameter_types::Parameters for $model {
            fn count(&self) -> usize {
                $crate::parameter_types::Parameters::count(&self.$field)
            }

            fn iter(&self) -> Box<dyn Iterator<Item = &dyn $crate::parameter_types::ParameterRef> + '_> {
                $crate::parameter_types::Parameters::iter(&self.$field)
            }

            fn by_id(&self, id: $crate::types::ParameterId) -> Option<&dyn $crate::parameter_types::ParameterRef> {
                $crate::parameter_types::Parameters::by_id(&self.$field, id)
            }

            fn assign_ids(&mut self, prefix: &str) {
                $crate::parameter_types::Parameters::assign_ids(&mut self.$field, prefix);
            }

            fn set_all_group_ids(&mut self, group_id: $crate::parameter_groups::GroupId) {
                $crate::parameter_types::Parameters::set_all_group_ids(&mut self.$field, group_id);
            }

            fn nested_count(&self) -> usize {
                $crate::parameter_types::Parameters::nested_count(&self.$field)
            }

            fn nested_group(
                &self,
                index: usize,
            ) -> Option<(&'static str, &dyn $crate::parameter_types::Parameters)> {
                $crate::parameter_types::Parameters::nested_group(&self.$field, index)
            }

            fn nested_group_mut(
                &mut self,
                index: usize,
            ) -> Option<(&'static str, &mut dyn $crate::parameter_types::Parameters)> {
                $crate::parameter_types::Parameters::nested_group_mut(&mut self.$field, index)
            }

            fn assign_group_ids(&mut self, start_id: i32, parent_id: i32) -> i32 {
                $crate::parameter_types::Parameters::assign_group_ids(&mut self.$field, start_id, parent_id)
            }

            fn collect_groups(
                &self,
                groups: &mut Vec<$crate::parameter_groups::GroupInfo>,
                start_id: i32,
                parent_id: i32,
            ) -> i32 {
                $crate::parameter_types::Parameters::collect_groups(&self.$field, groups, start_id, parent_id)
            }

            fn set_sample_rate(&self, sample_rate: f64) {
                $crate::parameter_types::Parameters::set_sample_rate(&self.$field, sample_rate);
            }

            fn reset_smoothing(&self) {
                $crate::parameter_types::Parameters::reset_smoothing(&self.$field);
            }
        }

        impl $crate::model::Saved for $model {
            fn state_generation(&self) -> u64 {
                $crate::model::Saved::state_generation(&self.$field)
            }

            fn for_each_state(
                &self,
                visit: &mut dyn FnMut(&str, &dyn $crate::state_format::SavedState),
            ) {
                $crate::model::Saved::for_each_state(&self.$field, visit);
            }

            fn save_entries(&self, writer: &mut $crate::state_format::StateWriter, prefix: &str) {
                $crate::model::Saved::save_entries(&self.$field, writer, prefix);
            }

            fn load_entries(&self, entries: &$crate::state_format::StateEntries<'_>, prefix: &str) {
                $crate::model::Saved::load_entries(&self.$field, entries, prefix);
            }

            fn save_state(&self) -> Vec<u8> {
                $crate::model::Saved::save_state(&self.$field)
            }

            fn load_state(&self, data: &[u8]) -> Result<(), String> {
                $crate::model::Saved::load_state(&self.$field, data)
            }
        }

        impl $crate::model::WebViewEvents for $model {
            fn webview_events(&self) -> Option<&$crate::webview_events::WebViewEventQueue> {
                $crate::model::WebViewEvents::webview_events(&self.$field)
            }
        }
    };
}
