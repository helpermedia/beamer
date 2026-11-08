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
//! (`window.__BEAMER__`, in `beamer_runtime.js` of this crate). Every
//! format wrapper gets them from here, so that they send the same
//! messages.
//!
//! A `State` field is known by its path, as in a saved state. To tell
//! whether the web UI has its latest value, the sync reads the generation
//! of the field first and the value second: the value it sends is then at
//! least as new as the generation it notes. A change in between is sent
//! again in the next frame with the same value (which the runtime
//! ignores) and is never lost.
//!
//! The messages of the page go the other way. The wrapper hands each one
//! to [`EditorSync::handle_message`], which reads it, sets the parameter or
//! `State` field it names or passes it to the model's handler. The wrapper
//! does what only it can do in the host through [`HostEdits`] and runs the
//! answer in the WebView.
//!
//! The sync also notes the gestures the web UI began and did not end. A
//! page that goes away during a drag or a scroll never sends the end, so
//! the wrapper ends them in the host when the editor closes or the page
//! loads again.

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use beamer_core::state_format::SavedState;
use beamer_core::{MiddleC, ParameterId, ParameterStore, SharedModel};
use serde_json::Value;

use crate::event_queue::WebViewEventQueue;
use crate::message::PageMessage;
use crate::model::WebViewDispatch;
use crate::parameter_dump::params_to_init_json;

/// The built-in invoke method that turns the text a user typed into a
/// normalized value, with the parameter's own parser. It never reaches the
/// plugin.
const PARAMETER_TEXT_TO_NORMALIZED: &str = "_beamer/paramTextToNormalized";

/// What a format wrapper does in the host when the web UI edits a
/// parameter.
///
/// The editor sync calls it on the main thread while it handles a message
/// of the page and for the gestures the page left open.
pub trait HostEdits {
    /// The web UI began a gesture on a parameter. Tell the host and return
    /// whether it was told: only a gesture the host knows about is ended
    /// for a page that goes away.
    fn begin_gesture(&mut self, id: ParameterId) -> bool;

    /// The web UI set a parameter to a normalized value, which the plugin
    /// holds already. Tell the host.
    fn set_parameter(&mut self, id: ParameterId, normalized: f64);

    /// The web UI ended a gesture on a parameter, or its page went away
    /// during one. Tell the host.
    fn end_gesture(&mut self, id: ParameterId);
}

/// The link between one open editor and the plugin it shows.
///
/// For the main thread of the editor. Holds the model, its dispatch and the
/// event queue itself, so that it keeps working when the editor outlives
/// the plugin instance.
pub struct EditorSync {
    /// The model of the plugin: its parameters and `State` fields.
    model: Arc<dyn SharedModel>,
    /// Where the web UI's invoke calls and events go: the model.
    dispatch: Arc<dyn WebViewDispatch>,
    /// The plugin's queue of events for the web UI, if it has one.
    events: Option<WebViewEventQueue>,
    /// The name of MIDI note 60 in the plugin's note names, for the page.
    middle_c: MiddleC,
    /// The id of each parameter, in the order of `ParameterStore::info`,
    /// with the bits of the normalized value the web UI knows. `None` until
    /// it was sent. The ids are taken once: listing the parameters by index
    /// is slow. They stay the same for the life of the plugin.
    last_values: Vec<(ParameterId, Option<u64>)>,
    /// The generation of the value of each `State` field the web UI got
    /// last, by path.
    state_generations: HashMap<String, u64>,
    /// The `state_generation()` of the model at the last look at the
    /// `State` fields. `None` before the first.
    state_sum: Option<u64>,
    /// Whether a large `State` value was reported already.
    reported_large: bool,
    /// The gestures the web UI began and did not end yet: a parameter for
    /// each begin that waits for its end.
    open_gestures: Vec<ParameterId>,
}

/// A `State` value above this size is reported once: it is serialized and
/// sent again with every change.
const LARGE_STATE: usize = 1 << 20;

impl EditorSync {
    /// Start keeping an editor in step. Events the plugin emits from now
    /// on reach it. `middle_c` is the plugin's from its `Config`, which the
    /// page gets as `__BEAMER__.middleC`.
    pub fn open(
        model: Arc<dyn SharedModel>,
        dispatch: Arc<dyn WebViewDispatch>,
        events: Option<WebViewEventQueue>,
        middle_c: MiddleC,
    ) -> Self {
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
            dispatch,
            events,
            middle_c,
            last_values,
            state_generations: HashMap::new(),
            state_sum: None,
            reported_large: false,
            open_gestures: Vec::new(),
        }
    }

    /// The script that hands a freshly loaded page its parameters, the
    /// values of the `State` fields, the name of the plugin format
    /// (`"vst3"`, `"auv2"`, `"auv3"`) and the name of middle C (`"C3"` or
    /// `"C4"`).
    pub fn init_script(&mut self, plugin_format: &str) -> String {
        // The page missed the values the senders sent before it loaded.
        if let Some(events) = &self.events {
            events.page_loaded();
        }

        let parameters = params_to_init_json(self.model.parameter_store());
        let format = json_string(plugin_format);
        let middle_c = json_string(self.middle_c.name());

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
             {{\"pluginFormat\":{format},\"middleC\":{middle_c},\"state\":{{{states}}}}})"
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

    /// Handle a message of the page: set what it sets, tell the host
    /// through `host` and return the script that answers it, if any.
    ///
    /// `json` is the message as the page posted it. One that is not a
    /// message of the runtime is logged and changes nothing.
    pub fn handle_message(&mut self, json: &[u8], host: &mut dyn HostEdits) -> Option<String> {
        match PageMessage::parse(json)? {
            PageMessage::SetParameter { id, value } => {
                self.model.parameter_store().set_normalized(id, value);
                host.set_parameter(id, value);
                // Answer with the value in effect at once, so that the
                // display text follows without waiting for the next frame.
                Some(self.parameter_script(id))
            }
            PageMessage::BeginGesture { id } => {
                // The sync ends it if the page goes away first.
                if host.begin_gesture(id) {
                    self.open_gestures.push(id);
                }
                None
            }
            PageMessage::EndGesture { id } => {
                self.gesture_ended(id);
                host.end_gesture(id);
                None
            }
            PageMessage::SetState { path, value } => self.state_script(&path, &value),
            PageMessage::Invoke { method, args, call_id } => {
                let result = self.invoke(&method, &args);
                Some(result_script(call_id, result))
            }
            PageMessage::Event { name, data } => {
                self.dispatch.dispatch_event(&name, &data);
                None
            }
        }
    }

    /// End in the host the gestures the web UI began and did not end. The
    /// wrapper calls it when the page goes away, as the editor closes or
    /// the page loads again: their ends will not come.
    pub fn end_open_gestures(&mut self, host: &mut dyn HostEdits) {
        while let Some(id) = self.open_gestures.pop() {
            host.end_gesture(id);
        }
    }

    /// Answer an invoke call: a built-in method here, any other one by the
    /// plugin.
    fn invoke(&self, method: &str, args: &[Value]) -> Result<Value, String> {
        if method != PARAMETER_TEXT_TO_NORMALIZED {
            return self.dispatch.dispatch_invoke(method, args);
        }
        // The id of the parameter and the text. `null` for text the
        // parameter cannot read and for arguments that do not fit.
        let id = args.first().and_then(Value::as_u64).and_then(|id| ParameterId::try_from(id).ok());
        let text = args.get(1).and_then(Value::as_str);
        let normalized = match (id, text) {
            (Some(id), Some(text)) => self.model.parameter_store().string_to_normalized(id, text),
            _ => None,
        };
        Ok(normalized.map_or(Value::Null, Value::from))
    }

    /// The script that tells the web UI the value of a parameter it set,
    /// as it is in effect now.
    ///
    /// Built after the value was stored and the host was told, which can
    /// change the value (an Audio Unit host rounds it to 32 bits). The
    /// value counts as sent, so the next frame does not send it again.
    fn parameter_script(&mut self, id: ParameterId) -> String {
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

    /// The web UI ended a gesture on a parameter: it no longer waits for
    /// its end. An end without a begin changes nothing.
    fn gesture_ended(&mut self, id: ParameterId) {
        if let Some(index) = self.open_gestures.iter().rposition(|&open| open == id) {
            self.open_gestures.remove(index);
        }
    }
}

impl EditorSync {
    /// Set a `State` field to a value the web UI sent, then answer with
    /// the value in effect.
    ///
    /// A value that does not fit the type of the field leaves the field as
    /// it is and is logged; the answer then corrects the web UI. Returns
    /// `None` for a path that names no field.
    fn state_script(&mut self, path: &str, value: &Value) -> Option<String> {
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
    /// the web UI does not have yet, or of every field with `all`, then note
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

/// The script that settles the promise of an invoke call: `{"ok":value}`
/// or `{"err":message}`.
fn result_script(call_id: u64, result: Result<Value, String>) -> String {
    let outcome = match result {
        Ok(value) => format!("{{\"ok\":{value}}}"),
        Err(message) => format!("{{\"err\":{}}}", json_string(&message)),
    };
    format!("window.__BEAMER__._onResult({call_id},{outcome});")
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
    use std::any::Any;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    use beamer_core::{
        NoParameters, ParameterGroups, ParameterInfo, ParameterRef, ParameterValue, Parameters, Saved,
        State,
    };

    use super::*;

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

        // The second parameter reads a number, the first nothing.
        fn string_to_normalized(&self, id: ParameterId, string: &str) -> Option<ParameterValue> {
            match id {
                20 => string.parse().ok(),
                _ => None,
            }
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

    /// Where the calls of a page go that nobody answers.
    fn no_dispatch() -> Arc<dyn WebViewDispatch> {
        Arc::new(NoParameters)
    }

    /// A host that notes what the editor tells it.
    #[derive(Default)]
    struct RecordingHost {
        calls: Vec<String>,
        /// Whether the host cannot be told of a gesture, as a VST3 host
        /// that has not set its component handler.
        unreachable: bool,
    }

    impl HostEdits for RecordingHost {
        fn begin_gesture(&mut self, id: ParameterId) -> bool {
            if self.unreachable {
                return false;
            }
            self.calls.push(format!("begin {id}"));
            true
        }

        fn set_parameter(&mut self, id: ParameterId, normalized: f64) {
            self.calls.push(format!("set {id} {normalized}"));
        }

        fn end_gesture(&mut self, id: ParameterId) {
            self.calls.push(format!("end {id}"));
        }
    }

    /// Hand the sync a message of the page.
    fn message(sync: &mut EditorSync, host: &mut RecordingHost, json: &str) -> Option<String> {
        sync.handle_message(json.as_bytes(), host)
    }

    fn sync() -> (Arc<TwoParameters>, EditorSync) {
        let parameters = Arc::new(TwoParameters::new());
        let sync = EditorSync::open(Arc::clone(&parameters) as Arc<dyn SharedModel>, no_dispatch(), None, MiddleC::C3);
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
    fn test_gestures_without_an_end_are_ended_once() {
        let (_, mut sync) = sync();
        let mut host = RecordingHost::default();

        // Two begins on one parameter wait for two ends. An end without a
        // begin reaches the host and changes nothing else.
        for json in [
            r#"{"type":"param:begin","id":10}"#,
            r#"{"type":"param:begin","id":20}"#,
            r#"{"type":"param:begin","id":10}"#,
            r#"{"type":"param:end","id":10}"#,
            r#"{"type":"param:end","id":30}"#,
        ] {
            assert_eq!(message(&mut sync, &mut host, json), None);
        }
        assert_eq!(host.calls, ["begin 10", "begin 20", "begin 10", "end 10", "end 30"]);

        // The page goes away: the latest gesture ends first.
        host.calls.clear();
        sync.end_open_gestures(&mut host);
        assert_eq!(host.calls, ["end 20", "end 10"]);
        sync.end_open_gestures(&mut host);
        assert_eq!(host.calls.len(), 2);
    }

    #[test]
    fn test_gesture_the_host_was_not_told_of_is_not_ended() {
        let (_, mut sync) = sync();
        let mut host = RecordingHost { unreachable: true, ..RecordingHost::default() };

        message(&mut sync, &mut host, r#"{"type":"param:begin","id":10}"#);
        sync.end_open_gestures(&mut host);
        assert!(host.calls.is_empty());
    }

    #[test]
    fn test_parameter_the_page_sets_reaches_the_plugin_and_the_host() {
        let (parameters, mut sync) = sync();
        sync.frame_script();
        let mut host = RecordingHost::default();

        assert_eq!(
            message(&mut sync, &mut host, r#"{"type":"param:set","id":20,"value":0.75}"#).as_deref(),
            Some("window.__BEAMER__._onParams({20:[0.75,null,\"0.75\"]});")
        );
        assert_eq!(parameters.get_normalized(20), 0.75);
        assert_eq!(host.calls, ["set 20 0.75"]);
        // The page has the value.
        assert_eq!(sync.frame_script(), None);
    }

    #[test]
    fn test_message_the_runtime_does_not_send_changes_nothing() {
        let (parameters, mut sync) = sync();
        let mut host = RecordingHost::default();

        assert_eq!(message(&mut sync, &mut host, r#"{"type":"param:set","id":20}"#), None);
        assert_eq!(message(&mut sync, &mut host, "not json"), None);
        assert_eq!(parameters.get_normalized(20), 0.0);
        assert!(host.calls.is_empty());
    }

    /// A plugin that answers one method and keeps the events it gets.
    #[derive(Default)]
    struct Echo {
        events: Mutex<Vec<String>>,
    }

    impl WebViewDispatch for Echo {
        fn dispatch_invoke(&self, method: &str, args: &[Value]) -> Result<Value, String> {
            match method {
                "echo" => Ok(args.first().cloned().unwrap_or_default()),
                _ => Err(format!("unknown method: {method}")),
            }
        }

        fn dispatch_event(&self, name: &str, data: &Value) {
            self.events.lock().unwrap().push(format!("{name} {data}"));
        }
    }

    fn echo_sync() -> (Arc<Echo>, EditorSync) {
        let echo = Arc::new(Echo::default());
        let sync = EditorSync::open(
            Arc::new(TwoParameters::new()),
            Arc::clone(&echo) as Arc<dyn WebViewDispatch>,
            None,
            MiddleC::C3,
        );
        (echo, sync)
    }

    #[test]
    fn test_invoke_is_answered_under_its_call_id() {
        let (_, mut sync) = echo_sync();
        let mut host = RecordingHost::default();

        assert_eq!(
            message(&mut sync, &mut host, r#"{"type":"invoke","method":"echo","args":["say \"hi\""],"callId":4}"#)
                .as_deref(),
            Some(r#"window.__BEAMER__._onResult(4,{"ok":"say \"hi\""});"#)
        );
        assert_eq!(
            message(&mut sync, &mut host, r#"{"type":"invoke","method":"nothing","callId":5}"#).as_deref(),
            Some(r#"window.__BEAMER__._onResult(5,{"err":"unknown method: nothing"});"#)
        );
    }

    #[test]
    fn test_built_in_invoke_reads_the_text_of_a_parameter() {
        let (_, mut sync) = echo_sync();
        let mut host = RecordingHost::default();
        let mut invoke = |args: &str| {
            let json = format!(
                r#"{{"type":"invoke","method":"_beamer/paramTextToNormalized","args":{args},"callId":1}}"#
            );
            message(&mut sync, &mut host, &json)
        };

        // The plugin never sees the method: it would reject it.
        assert_eq!(invoke(r#"[20,"0.5"]"#).as_deref(), Some(r#"window.__BEAMER__._onResult(1,{"ok":0.5});"#));
        assert_eq!(invoke(r#"[10,"0.5"]"#).as_deref(), Some(r#"window.__BEAMER__._onResult(1,{"ok":null});"#));
        assert_eq!(invoke("[20]").as_deref(), Some(r#"window.__BEAMER__._onResult(1,{"ok":null});"#));
    }

    #[test]
    fn test_event_of_the_page_reaches_the_plugin() {
        let (echo, mut sync) = echo_sync();
        let mut host = RecordingHost::default();

        assert_eq!(message(&mut sync, &mut host, r#"{"type":"event","name":"zoom","data":2}"#), None);
        assert_eq!(message(&mut sync, &mut host, r#"{"type":"event","name":"ping"}"#), None);
        assert_eq!(*echo.events.lock().unwrap(), ["zoom 2", "ping null"]);
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
        let mut sync = EditorSync::open(parameters as Arc<dyn SharedModel>, no_dispatch(), None, MiddleC::C3);

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
        let mut sync = EditorSync::open(parameters as Arc<dyn SharedModel>, no_dispatch(), Some(events.clone()), MiddleC::C3);

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
    fn test_frame_delivers_the_values_of_the_audio_thread() {
        let events = WebViewEventQueue::new();
        events.claim();
        let mut levels = events.value_sender("levels", [0.0f32; 2]);
        levels.send(&[0.5, 0.25]);

        // The editor ticks before its page loaded: that value reaches
        // nothing.
        let mut sync = EditorSync::open(Arc::new(NoParameters), no_dispatch(), Some(events.clone()), MiddleC::C3);
        assert!(sync.frame_script().is_some());
        assert_eq!(sync.frame_script(), None);

        // The page loaded: the value sent before goes out again.
        sync.init_script("vst3");
        assert_eq!(
            sync.frame_script().as_deref(),
            Some("window.__BEAMER__._onValue(\"levels\",[0.5,0.25]);")
        );
        assert_eq!(sync.frame_script(), None);

        levels.send(&[1.0, 0.0]);
        events.emit("tick", &1);
        assert_eq!(
            sync.frame_script().as_deref(),
            Some(
                "window.__BEAMER__._onEvent(\"tick\",1);\
                 window.__BEAMER__._onValue(\"levels\",[1.0,0.0]);"
            )
        );
    }

    #[test]
    fn test_events_reach_the_editor_only_while_it_is_open() {
        let events = WebViewEventQueue::new();
        events.claim();

        events.emit("before", &1);
        let mut first = EditorSync::open(Arc::new(NoParameters), no_dispatch(), Some(events.clone()), MiddleC::C3);
        assert_eq!(first.frame_script(), None);

        // A second editor of the same plugin keeps the queue open.
        let second = EditorSync::open(Arc::new(NoParameters), no_dispatch(), Some(events.clone()), MiddleC::C3);
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
    fn test_init_script_names_the_plugin_format_and_middle_c() {
        let mut sync = EditorSync::open(Arc::new(NoParameters), no_dispatch(), None, MiddleC::C4);
        assert_eq!(
            sync.init_script("vst3"),
            "window.__BEAMER__._onInit([],{\"pluginFormat\":\"vst3\",\"middleC\":\"C4\",\"state\":{}})"
        );
    }

    /// Two `State` fields (one of them in a nested group) and no
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
        let sync = EditorSync::open(Arc::clone(&states) as Arc<dyn SharedModel>, no_dispatch(), None, MiddleC::C3);
        (states, sync)
    }

    #[test]
    fn test_init_script_carries_the_state_values() {
        let (states, mut sync) = state_sync();
        states.name.set(String::from("say \"hi\""));

        assert_eq!(
            sync.init_script("auv3"),
            "window.__BEAMER__._onInit([],{\"pluginFormat\":\"auv3\",\"middleC\":\"C3\",\
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
        let mut host = RecordingHost::default();

        assert_eq!(
            message(&mut sync, &mut host, r#"{"type":"state:set","id":"zoom","value":200}"#).as_deref(),
            Some("window.__BEAMER__._onState({\"zoom\":200});")
        );
        assert!(host.calls.is_empty());
        assert_eq!(*states.zoom.get(), 200);
        assert_eq!(sync.frame_script(), None);
    }

    #[test]
    fn test_state_that_does_not_fit_is_corrected() {
        let (states, mut sync) = state_sync();
        sync.init_script("vst3");
        states.zoom.set(120);
        sync.frame_script();

        // The field stays as it is. The answer tells the web UI.
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
