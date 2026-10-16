//! Keeping an open web UI in step with its plugin.
//!
//! Each open editor has an [`EditorSync`]. The format wrapper creates it
//! when the editor opens and drops it when the editor closes. At every
//! frame of the editor (about 60 times a second, on the main thread) the
//! wrapper asks it for a script that brings the web UI up to date: the
//! parameters that changed since the last frame and the events the plugin
//! emitted. The wrapper runs the script in the WebView.
//!
//! The scripts call the JavaScript runtime of the WebView
//! (`window.__BEAMER__`). The format wrappers build them here, so that
//! they send the same messages.

use std::fmt::Write;
use std::sync::Arc;

use crate::parameter_store::{params_to_init_json, ParameterStore};
use crate::types::ParameterId;
use crate::webview_events::WebViewEvents;

/// The link between one open editor and the plugin it shows.
///
/// For the main thread of the editor. Holds the parameters and the event
/// queue itself, so that it keeps working when the editor outlives the
/// plugin instance.
pub struct EditorSync {
    /// The parameters of the plugin.
    parameters: Arc<dyn ParameterStore>,
    /// The plugin's queue of events for the web UI, if it has one.
    events: Option<WebViewEvents>,
    /// The bits of the normalized value of each parameter the web UI
    /// knows, by index as in `ParameterStore::info`. `None` until it was
    /// sent.
    last_values: Vec<Option<u64>>,
}

impl EditorSync {
    /// Start keeping an editor in step. Events the plugin emits from now
    /// on reach it.
    pub fn open(parameters: Arc<dyn ParameterStore>, events: Option<WebViewEvents>) -> Self {
        if let Some(events) = &events {
            events.editor_opened();
        }
        // Nothing was sent yet, so the first frame sends every parameter.
        let last_values = vec![None; parameters.count()];
        Self {
            parameters,
            events,
            last_values,
        }
    }

    /// The parameters of the plugin.
    pub fn parameters(&self) -> &dyn ParameterStore {
        &*self.parameters
    }

    /// The script that hands a freshly loaded page its parameters and the
    /// name of the plugin format (`"vst3"`, `"au"`).
    pub fn init_script(&self, plugin_format: &str) -> String {
        let parameters = params_to_init_json(&*self.parameters);
        let format = json_string(plugin_format);
        format!("window.__BEAMER__._onInit({parameters},{{\"pluginFormat\":{format}}})")
    }

    /// The script of one frame: the parameters that changed since the last
    /// frame, then the events that wait. `None` when nothing changed.
    pub fn frame_script(&mut self) -> Option<String> {
        let mut script = String::new();
        let parameters = &*self.parameters;

        let mut any = false;
        for (index, last) in self.last_values.iter_mut().enumerate() {
            let Some(info) = parameters.info(index) else {
                continue;
            };
            let value = parameters.get_normalized(info.id);
            // Bits, so that a value that stays NaN is not sent again.
            if *last == Some(value.to_bits()) {
                continue;
            }
            *last = Some(value.to_bits());
            script.push_str(if any { "," } else { "window.__BEAMER__._onParams({" });
            any = true;
            write_parameter(&mut script, parameters, info.id, value);
        }
        if any {
            script.push_str("});");
        }

        if let Some(events) = self.events.as_ref().and_then(WebViewEvents::take_script) {
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
        let parameters = &*self.parameters;
        let value = parameters.get_normalized(id);
        let index = (0..parameters.count())
            .find(|&index| parameters.info(index).is_some_and(|info| info.id == id));
        if let Some(last) = index.and_then(|index| self.last_values.get_mut(index)) {
            *last = Some(value.to_bits());
        }

        let mut script = String::from("window.__BEAMER__._onParams({");
        write_parameter(&mut script, parameters, id, value);
        script.push_str("});");
        script
    }
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

    use super::*;
    use crate::parameter_info::ParameterInfo;
    use crate::parameter_store::NoParameters;
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

    fn sync() -> (Arc<TwoParameters>, EditorSync) {
        let parameters = Arc::new(TwoParameters::new());
        let sync = EditorSync::open(Arc::clone(&parameters) as Arc<dyn ParameterStore>, None);
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
        let mut sync = EditorSync::open(parameters as Arc<dyn ParameterStore>, None);

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
        let events = WebViewEvents::new();
        events.claim();
        let parameters = Arc::new(TwoParameters::new());
        let mut sync = EditorSync::open(parameters as Arc<dyn ParameterStore>, Some(events.clone()));

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
        let events = WebViewEvents::new();
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
        let sync = EditorSync::open(Arc::new(NoParameters), None);
        assert_eq!(
            sync.init_script("vst3"),
            "window.__BEAMER__._onInit([],{\"pluginFormat\":\"vst3\"})"
        );
    }
}
