//! Keeping an open web UI in step with its plugin.
//!
//! Each open editor has an [`EditorSync`]. The format wrapper creates it
//! when the editor opens and drops it when the editor closes. At every
//! frame of the editor (about 60 times a second, on the main thread) the
//! wrapper asks it for a script that brings the web UI up to date: the
//! parameters and `State` fields that changed since the last frame and the
//! events the plugin emitted. The wrapper runs the script in the WebView.
//!
//! The scripts call the JavaScript runtime of the WebView
//! (`window.__BEAMER__`). The format wrappers build them here, so that
//! they send the same messages.
//!
//! A `State` field is known by its path, as in a saved state. To tell
//! whether the web UI has its latest value, the sync reads the generation
//! of the field first and the value second: the value it sends is then at
//! least as new as the generation it notes. A change in between is sent
//! again in the next frame with the same value, which the runtime ignores,
//! and is never lost.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use crate::model::SharedModel;
use crate::parameter_store::{params_to_init_json, ParameterStore};
use crate::state_format::SavedState;
use crate::types::ParameterId;
use crate::webview_events::WebViewEventQueue;

/// The link between one open editor and the plugin it shows.
///
/// For the main thread of the editor. Holds the model and the event queue
/// itself, so that it keeps working when the editor outlives the plugin
/// instance.
pub struct EditorSync {
    /// The model of the plugin: its parameters and `State` fields.
    model: Arc<dyn SharedModel>,
    /// The plugin's queue of events for the web UI, if it has one.
    events: Option<WebViewEventQueue>,
    /// The id of each parameter, in the order of `ParameterStore::info`,
    /// with the bits of the normalized value the web UI knows. `None` until
    /// it was sent. The ids are taken once: listing the parameters by index
    /// is slow, and they stay the same for the life of the plugin.
    last_values: Vec<(ParameterId, Option<u64>)>,
    /// The generation of the value of each `State` field the web UI got
    /// last, by path.
    state_generations: HashMap<String, u64>,
    /// The `state_generation()` of the model at the last look at the
    /// `State` fields. `None` before the first.
    state_sum: Option<u64>,
    /// Whether a large `State` value was reported already.
    reported_large: bool,
}

/// A `State` value above this size is reported once: it is serialized and
/// sent again with every change.
const LARGE_STATE: usize = 1 << 20;

impl EditorSync {
    /// Start keeping an editor in step. Events the plugin emits from now
    /// on reach it.
    pub fn open(model: Arc<dyn SharedModel>, events: Option<WebViewEventQueue>) -> Self {
        if let Some(events) = &events {
            events.editor_opened();
        }
        // Nothing was sent yet, so the first frame sends every parameter.
        let parameters = model.parameter_store();
        let last_values = (0..parameters.count())
            .filter_map(|index| parameters.info(index))
            .map(|info| (info.id, None))
            .collect();
        Self {
            model,
            events,
            last_values,
            state_generations: HashMap::new(),
            state_sum: None,
            reported_large: false,
        }
    }

    /// The parameters of the plugin, as the host sees them.
    pub fn parameter_store(&self) -> &dyn ParameterStore {
        self.model.parameter_store()
    }

    /// The script that hands a freshly loaded page its parameters, the
    /// values of the `State` fields and the name of the plugin format
    /// (`"vst3"`, `"auv2"`, `"auv3"`).
    pub fn init_script(&mut self, plugin_format: &str) -> String {
        let parameters = params_to_init_json(self.model.parameter_store());
        let format = json_string(plugin_format);

        // Every field, whether or not it changed: the page knows nothing.
        self.state_sum = Some(self.model.state_generation());
        let mut states = String::new();
        self.for_each_new_state(true, &mut |path, json| {
            if !states.is_empty() {
                states.push(',');
            }
            push_entry(&mut states, path, json);
        });

        format!(
            "window.__BEAMER__._onInit({parameters},\
             {{\"pluginFormat\":{format},\"state\":{{{states}}}}})"
        )
    }

    /// The script of one frame: the parameters that changed since the last
    /// frame, then the events that wait. `None` when nothing changed.
    pub fn frame_script(&mut self) -> Option<String> {
        let mut script = String::new();
        let parameters = self.model.parameter_store();

        let mut any = false;
        for (id, last) in self.last_values.iter_mut() {
            let id = *id;
            let value = parameters.get_normalized(id);
            // Bits, so that a value that stays NaN is not sent again.
            if *last == Some(value.to_bits()) {
                continue;
            }
            *last = Some(value.to_bits());
            script.push_str(if any { "," } else { "window.__BEAMER__._onParams({" });
            any = true;
            write_parameter(&mut script, parameters, id, value);
        }
        if any {
            script.push_str("});");
        }

        // The sum of the generations changes with every set of a field, so
        // a frame without a change looks at no field.
        let sum = self.model.state_generation();
        if self.state_sum != Some(sum) {
            self.state_sum = Some(sum);
            let mut states = String::new();
            self.for_each_new_state(false, &mut |path, json| {
                let separator = if states.is_empty() { "window.__BEAMER__._onState({" } else { "," };
                states.push_str(separator);
                push_entry(&mut states, path, json);
            });
            if !states.is_empty() {
                states.push_str("});");
                script.push_str(&states);
            }
        }

        if let Some(events) = self.events.as_ref().and_then(WebViewEventQueue::take_script) {
            script.push_str(&events);
        }

        (!script.is_empty()).then_some(script)
    }

    /// The script that tells the web UI the value of a parameter it set,
    /// as it is in effect now.
    ///
    /// The wrapper calls it after it stored the value and told the host,
    /// which can change the value (an Audio Unit host rounds it to 32
    /// bits). The value counts as sent, so the next frame does not send
    /// it again.
    pub fn parameter_script(&mut self, id: ParameterId) -> String {
        let parameters = self.model.parameter_store();
        let value = parameters.get_normalized(id);
        if let Some((_, last)) = self.last_values.iter_mut().find(|(known, _)| *known == id) {
            *last = Some(value.to_bits());
        }

        let mut script = String::from("window.__BEAMER__._onParams({");
        write_parameter(&mut script, parameters, id, value);
        script.push_str("});");
        script
    }
}

impl EditorSync {
    /// Set a `State` field to a value the web UI sent, and answer with the
    /// value in effect.
    ///
    /// A value that does not fit the type of the field leaves the field as
    /// it is and is logged; the answer then corrects the web UI. Returns
    /// `None` for a path that names no field.
    pub fn state_script(&mut self, path: &str, value: &serde_json::Value) -> Option<String> {
        let json = serde_json::to_vec(value).ok()?;
        let mut answer = None;
        let generations = &mut self.state_generations;
        self.model.for_each_state(&mut |field, state| {
            if field != path {
                return;
            }
            if let Err(error) = state.load_json(&json) {
                log::warn!(
                    "The web UI set State field {path:?} to a value that does not fit: {error}"
                );
            }
            if let Some(current) = snapshot(state, path) {
                generations.insert(path.to_string(), current.0);
                let mut script = String::from("window.__BEAMER__._onState({");
                push_entry(&mut script, path, &current.1);
                script.push_str("});");
                answer = Some(script);
            }
        });
        if answer.is_none() {
            log::warn!(
                "The web UI set State field {path:?}, which the plugin does not have."
            );
        }
        answer
    }

    /// Call `send` with the path and JSON of each `State` field whose value
    /// the web UI does not have yet, or of every field with `all`, and note
    /// them as sent.
    fn for_each_new_state(&mut self, all: bool, send: &mut dyn FnMut(&str, &[u8])) {
        let generations = &mut self.state_generations;
        let reported_large = &mut self.reported_large;
        self.model.for_each_state(&mut |path, state| {
            if !all && generations.get(path) == Some(&state.generation()) {
                return;
            }
            let Some((generation, json)) = snapshot(state, path) else {
                return;
            };
            if json.len() > LARGE_STATE && !*reported_large {
                *reported_large = true;
                log::warn!(
                    "State field {path:?} is {} bytes as JSON. \
                     The web UI gets it again with every change.",
                    json.len()
                );
            }
            generations.insert(path.to_string(), generation);
            send(path, &json);
        });
    }
}

/// The generation of a `State` field and its value as JSON, read in that
/// order (see the module documentation). `None`, logged, when the value
/// cannot be written as JSON.
fn snapshot(state: &dyn SavedState, path: &str) -> Option<(u64, Vec<u8>)> {
    let generation = state.generation();
    match state.save_json() {
        Ok(json) => Some((generation, json)),
        Err(error) => {
            log::warn!("State field {path:?} cannot be sent to the web UI: {error}");
            None
        }
    }
}

/// Write `"path":value` into a JavaScript object literal.
fn push_entry(script: &mut String, path: &str, json: &[u8]) {
    script.push_str(&json_string(path));
    script.push(':');
    // serde_json writes UTF-8.
    script.push_str(std::str::from_utf8(json).unwrap_or("null"));
}

impl Drop for EditorSync {
    /// The editor is gone: events no longer wait for it.
    fn drop(&mut self) {
        if let Some(events) = &self.events {
            events.editor_closed();
        }
    }
}

/// Write the entry of one parameter of `_onParams`:
/// `id:[normalized,plain,"display text"]`.
fn write_parameter(script: &mut String, parameters: &dyn ParameterStore, id: ParameterId, value: f64) {
    let plain = parameters.normalized_to_plain(id, value);
    let text = parameters.normalized_to_string(id, value);
    let _ = write!(
        script,
        "{id}:[{},{},{}]",
        json_number(value),
        json_number(plain),
        json_string(&text)
    );
}

/// A number as JavaScript reads it. A value that is not finite becomes
/// `null`, which keeps the rest of the script valid.
fn json_number(value: f64) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| String::from("null"))
}

/// A string as a JavaScript string literal.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| String::from("\"\""))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use std::any::Any;

    use super::*;
    use crate::parameter_info::ParameterInfo;
    use crate::parameter_store::NoParameters;
    use crate::model::Saved;
    use crate::parameter_groups::ParameterGroups;
    use crate::parameter_types::{ParameterRef, Parameters};
    use crate::state::State;
    use crate::types::ParameterValue;

    /// Two parameters whose values a test sets directly. The display text
    /// of the first contains characters that a script has to escape.
    struct TwoParameters {
        infos: [ParameterInfo; 2],
        values: [AtomicU64; 2],
    }

    impl TwoParameters {
        fn new() -> Self {
            Self {
                infos: [
                    ParameterInfo::new(10, "Quote").with_string_id("quote"),
                    ParameterInfo::new(20, "Plain").with_string_id("plain"),
                ],
                values: [AtomicU64::new(0.5f64.to_bits()), AtomicU64::new(0.0f64.to_bits())],
            }
        }

        fn index(id: ParameterId) -> Option<usize> {
            match id {
                10 => Some(0),
                20 => Some(1),
                _ => None,
            }
        }
    }

    impl ParameterStore for TwoParameters {
        fn count(&self) -> usize {
            2
        }

        fn info(&self, index: usize) -> Option<&ParameterInfo> {
            self.infos.get(index)
        }

        fn get_normalized(&self, id: ParameterId) -> ParameterValue {
            Self::index(id).map_or(0.0, |i| f64::from_bits(self.values[i].load(Ordering::Relaxed)))
        }

        fn set_normalized(&self, id: ParameterId, value: ParameterValue) {
            if let Some(i) = Self::index(id) {
                self.values[i].store(value.to_bits(), Ordering::Relaxed);
            }
        }

        fn normalized_to_string(&self, id: ParameterId, normalized: ParameterValue) -> String {
            match id {
                10 => format!("say \"{normalized}\" \\ now"),
                _ => format!("{normalized}"),
            }
        }

        fn string_to_normalized(&self, _id: ParameterId, _string: &str) -> Option<ParameterValue> {
            None
        }

        fn normalized_to_plain(&self, id: ParameterId, normalized: ParameterValue) -> ParameterValue {
            match id {
                // A plain value that is not finite.
                20 => normalized / 0.0,
                _ => normalized * 2.0,
            }
        }

        fn plain_to_normalized(&self, _id: ParameterId, plain: ParameterValue) -> ParameterValue {
            plain
        }
    }

    // Values without parameter objects: a `Parameters` impl would have to
    // claim none, so this store is a `SharedModel` of its own.
    impl SharedModel for TwoParameters {
        fn parameter_store(&self) -> &dyn ParameterStore {
            self
        }

        fn parameters(&self) -> &dyn Parameters {
            &NoParameters
        }

        fn state_generation(&self) -> u64 {
            0
        }

        fn for_each_state(&self, _visit: &mut dyn FnMut(&str, &dyn SavedState)) {}

        fn save_state(&self) -> Vec<u8> {
            Vec::new()
        }

        fn load_state(&self, _data: &[u8]) -> Result<(), String> {
            Ok(())
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    fn sync() -> (Arc<TwoParameters>, EditorSync) {
        let parameters = Arc::new(TwoParameters::new());
        let sync = EditorSync::open(Arc::clone(&parameters) as Arc<dyn SharedModel>, None);
        (parameters, sync)
    }

    #[test]
    fn test_first_frame_sends_every_parameter_then_only_changes() {
        let (parameters, mut sync) = sync();

        assert_eq!(
            sync.frame_script().as_deref(),
            Some(
                "window.__BEAMER__._onParams({10:[0.5,1.0,\"say \\\"0.5\\\" \\\\ now\"],\
                 20:[0.0,null,\"0\"]});"
            )
        );
        assert_eq!(sync.frame_script(), None);

        parameters.set_normalized(20, 1.0);
        assert_eq!(
            sync.frame_script().as_deref(),
            Some("window.__BEAMER__._onParams({20:[1.0,null,\"1\"]});")
        );
        assert_eq!(sync.frame_script(), None);
    }

    #[test]
    fn test_value_the_ui_set_is_answered_and_not_sent_again() {
        let (parameters, mut sync) = sync();
        sync.frame_script();

        // The web UI sets a value, the host changes it.
        parameters.set_normalized(10, 0.25);
        parameters.set_normalized(10, 0.3);
        assert_eq!(
            sync.parameter_script(10),
            "window.__BEAMER__._onParams({10:[0.3,0.6,\"say \\\"0.3\\\" \\\\ now\"]});"
        );
        assert_eq!(sync.frame_script(), None);
    }

    #[test]
    fn test_value_that_is_not_a_number_is_sent_once() {
        let (parameters, mut sync) = sync();
        sync.frame_script();

        parameters.set_normalized(20, f64::NAN);
        assert_eq!(
            sync.frame_script().as_deref(),
            Some("window.__BEAMER__._onParams({20:[null,null,\"NaN\"]});")
        );
        assert_eq!(sync.frame_script(), None);
    }

    #[test]
    fn test_first_frame_sends_a_value_that_is_not_a_number() {
        let parameters = Arc::new(TwoParameters::new());
        parameters.set_normalized(10, f64::NAN);
        let mut sync = EditorSync::open(parameters as Arc<dyn SharedModel>, None);

        let script = sync.frame_script().expect("a first frame");
        assert!(script.contains("10:[null,null,"));
    }

    #[test]
    fn test_answer_for_an_unknown_parameter_does_not_panic() {
        let (_parameters, mut sync) = sync();
        assert_eq!(
            sync.parameter_script(99),
            "window.__BEAMER__._onParams({99:[0.0,0.0,\"0\"]});"
        );
    }

    #[test]
    fn test_frame_delivers_the_events_after_the_parameters() {
        let events = WebViewEventQueue::new();
        events.claim();
        let parameters = Arc::new(TwoParameters::new());
        let mut sync = EditorSync::open(parameters as Arc<dyn SharedModel>, Some(events.clone()));

        events.emit("tick", &1);
        let script = sync.frame_script().expect("a first frame");
        assert!(script.starts_with("window.__BEAMER__._onParams({"));
        assert!(script.ends_with("});window.__BEAMER__._onEvent(\"tick\",1);"));

        events.emit("tock", &2);
        assert_eq!(
            sync.frame_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"tock\",2);")
        );
    }

    #[test]
    fn test_events_reach_the_editor_only_while_it_is_open() {
        let events = WebViewEventQueue::new();
        events.claim();

        events.emit("before", &1);
        let mut first = EditorSync::open(Arc::new(NoParameters), Some(events.clone()));
        assert_eq!(first.frame_script(), None);

        // A second editor of the same plugin keeps the queue open.
        let second = EditorSync::open(Arc::new(NoParameters), Some(events.clone()));
        drop(second);
        events.emit("open", &2);
        assert_eq!(
            first.frame_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"open\",2);")
        );

        drop(first);
        events.emit("closed", &3);
        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_init_script_names_the_plugin_format() {
        let mut sync = EditorSync::open(Arc::new(NoParameters), None);
        assert_eq!(
            sync.init_script("vst3"),
            "window.__BEAMER__._onInit([],{\"pluginFormat\":\"vst3\",\"state\":{}})"
        );
    }

    /// Two `State` fields, one of them in a nested group, and no
    /// parameters.
    #[derive(Default)]
    struct TwoStates {
        zoom: State<u32>,
        name: State<String>,
    }

    impl ParameterGroups for TwoStates {}

    impl Parameters for TwoStates {
        fn count(&self) -> usize {
            0
        }

        fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
            Box::new(std::iter::empty())
        }

        fn by_id(&self, _id: ParameterId) -> Option<&dyn ParameterRef> {
            None
        }
    }

    impl Saved for TwoStates {
        fn state_generation(&self) -> u64 {
            self.zoom.generation().wrapping_add(self.name.generation())
        }

        fn for_each_state(&self, visit: &mut dyn FnMut(&str, &dyn SavedState)) {
            visit("zoom", &self.zoom);
            visit("layer/name", &self.name);
        }
    }

    fn state_sync() -> (Arc<TwoStates>, EditorSync) {
        let states = Arc::new(TwoStates::default());
        let sync = EditorSync::open(Arc::clone(&states) as Arc<dyn SharedModel>, None);
        (states, sync)
    }

    #[test]
    fn test_init_script_carries_the_state_values() {
        let (states, mut sync) = state_sync();
        states.name.set(String::from("say \"hi\""));

        assert_eq!(
            sync.init_script("auv3"),
            "window.__BEAMER__._onInit([],{\"pluginFormat\":\"auv3\",\
             \"state\":{\"zoom\":0,\"layer/name\":\"say \\\"hi\\\"\"}})"
        );
        // The page has every value.
        assert_eq!(sync.frame_script(), None);
    }

    #[test]
    fn test_changed_state_field_is_sent_once() {
        let (states, mut sync) = state_sync();
        sync.init_script("vst3");

        states.zoom.set(150);
        assert_eq!(
            sync.frame_script().as_deref(),
            Some("window.__BEAMER__._onState({\"zoom\":150});")
        );
        assert_eq!(sync.frame_script(), None);

        // A set to the same value is a change too: the generation moved.
        states.zoom.set(150);
        states.name.set(String::from("kit"));
        assert_eq!(
            sync.frame_script().as_deref(),
            Some("window.__BEAMER__._onState({\"zoom\":150,\"layer/name\":\"kit\"});")
        );
    }

    #[test]
    fn test_state_the_ui_set_is_answered_and_not_sent_again() {
        let (states, mut sync) = state_sync();
        sync.init_script("vst3");

        assert_eq!(
            sync.state_script("zoom", &serde_json::json!(200)).as_deref(),
            Some("window.__BEAMER__._onState({\"zoom\":200});")
        );
        assert_eq!(*states.zoom.get(), 200);
        assert_eq!(sync.frame_script(), None);
    }

    #[test]
    fn test_state_that_does_not_fit_is_corrected() {
        let (states, mut sync) = state_sync();
        sync.init_script("vst3");
        states.zoom.set(120);
        sync.frame_script();

        // The field stays as it is, and the answer tells the web UI.
        assert_eq!(
            sync.state_script("zoom", &serde_json::json!("big")).as_deref(),
            Some("window.__BEAMER__._onState({\"zoom\":120});")
        );
        assert_eq!(*states.zoom.get(), 120);
        assert_eq!(sync.state_script("nothing", &serde_json::json!(1)), None);
    }

    #[test]
    fn test_state_set_by_another_thread_after_the_ui_reaches_the_ui() {
        let (states, mut sync) = state_sync();
        sync.init_script("vst3");

        sync.state_script("zoom", &serde_json::json!(200));
        // A host thread sets the field before the next frame.
        states.zoom.set(300);
        assert_eq!(
            sync.frame_script().as_deref(),
            Some("window.__BEAMER__._onState({\"zoom\":300});")
        );
    }

    #[test]
    fn test_state_changes_before_the_page_loaded_are_in_the_init_script() {
        let (states, mut sync) = state_sync();
        // A frame before the page loaded, then a change.
        sync.frame_script();
        states.zoom.set(5);

        let init = sync.init_script("vst3");
        assert!(init.contains("\"zoom\":5"));
        assert_eq!(sync.frame_script(), None);
    }
}
