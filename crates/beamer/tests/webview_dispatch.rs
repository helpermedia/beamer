//! Calls from the web UI reach the model: the format wrappers pass
//! `invoke()` calls and custom events on through `WebViewDispatch`.

#![cfg(feature = "derive")]

use std::sync::Mutex;

use beamer::core::WebViewDispatch;
use beamer::prelude::*;

struct Silence;

impl Processor for Silence {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

/// A plugin that answers nothing from the web UI.
#[beamer::model]
struct QuietModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,
}

impl Model for QuietModel {
    type Setup = ();
    type Processor = Silence;

    fn prepare(self: Arc<Self>, _: ()) -> Silence {
        Silence
    }
}

/// A plugin that answers one method and keeps the events it gets.
#[beamer::model]
struct EchoModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    /// The name and data of each event, in order.
    events: Mutex<Vec<(String, serde_json::Value)>>,
}

impl Model for EchoModel {
    type Setup = ();
    type Processor = Silence;

    fn prepare(self: Arc<Self>, _: ()) -> Silence {
        Silence
    }

    fn on_invoke(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        match method {
            "echo" => Ok(args.first().cloned().unwrap_or_default()),
            _ => Err(format!("unknown method: {method}")),
        }
    }

    fn on_event(&self, name: &str, data: &serde_json::Value) {
        self.events
            .lock()
            .expect("no test panics while holding it")
            .push((name.to_owned(), data.clone()));
    }
}

/// The model as the wrappers hold it for the web UI.
fn dispatch<P: Model>(model: P) -> Arc<dyn WebViewDispatch> {
    Arc::new(model)
}

#[test]
fn test_unknown_method_rejects_by_default() {
    let quiet = dispatch(QuietModel::default());

    // The promise in JavaScript rejects with this message.
    assert_eq!(
        quiet.dispatch_invoke("getInfo", &[]),
        Err(String::from("unknown method: getInfo")),
    );
}

#[test]
fn test_events_are_ignored_by_default() {
    let quiet = dispatch(QuietModel::default());

    quiet.dispatch_event("ignored", &serde_json::json!(1));
}

#[test]
fn test_invoke_reaches_the_model() {
    let echo = dispatch(EchoModel::default());

    assert_eq!(
        echo.dispatch_invoke("echo", &[serde_json::json!("hello")]),
        Ok(serde_json::json!("hello")),
    );
}

#[test]
fn test_events_reach_the_model() {
    let echo = Arc::new(EchoModel::default());
    let as_dispatch: Arc<dyn WebViewDispatch> = Arc::clone(&echo) as Arc<dyn WebViewDispatch>;

    as_dispatch.dispatch_event("zoom", &serde_json::json!(2));
    as_dispatch.dispatch_event("theme", &serde_json::json!("dark"));

    let events = echo.events.lock().expect("no test panics while holding it");
    assert_eq!(
        *events,
        [
            (String::from("zoom"), serde_json::json!(2)),
            (String::from("theme"), serde_json::json!("dark")),
        ],
    );
}
