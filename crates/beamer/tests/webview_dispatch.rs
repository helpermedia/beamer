//! Calls from the web UI reach the descriptor: the format wrappers pass
//! `invoke()` calls and custom events on through `WebViewDispatch`.

#![cfg(feature = "derive")]

use std::sync::Mutex;

use beamer::core::{NoParameters, WebViewDispatch};
use beamer::prelude::*;

struct Silence;

impl Processor for Silence {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

/// A plugin that answers nothing from the web UI.
#[derive(Default, HasParameters)]
struct QuietDescriptor {
    #[parameters]
    parameters: Arc<NoParameters>,
}

impl Descriptor for QuietDescriptor {
    type Setup = ();
    type Processor = Silence;

    fn prepare(&self, _: ()) -> Silence {
        Silence
    }
}

/// A plugin that answers one method and keeps the events it gets.
#[derive(Default, HasParameters)]
struct EchoDescriptor {
    #[parameters]
    parameters: Arc<NoParameters>,
    /// The name and data of each event, in order.
    events: Mutex<Vec<(String, serde_json::Value)>>,
}

impl Descriptor for EchoDescriptor {
    type Setup = ();
    type Processor = Silence;

    fn prepare(&self, _: ()) -> Silence {
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

/// The descriptor as the wrappers hold it for the web UI.
fn dispatch<P: Descriptor>(descriptor: P) -> Arc<dyn WebViewDispatch> {
    Arc::new(descriptor)
}

#[test]
fn test_unknown_method_rejects_by_default() {
    let quiet = dispatch(QuietDescriptor::default());

    // The promise in JavaScript rejects with this message.
    assert_eq!(
        quiet.dispatch_invoke("getInfo", &[]),
        Err(String::from("unknown method: getInfo")),
    );
}

#[test]
fn test_events_are_ignored_by_default() {
    let quiet = dispatch(QuietDescriptor::default());

    quiet.dispatch_event("ignored", &serde_json::json!(1));
}

#[test]
fn test_invoke_reaches_the_descriptor() {
    let echo = dispatch(EchoDescriptor::default());

    assert_eq!(
        echo.dispatch_invoke("echo", &[serde_json::json!("hello")]),
        Ok(serde_json::json!("hello")),
    );
}

#[test]
fn test_events_reach_the_descriptor() {
    let echo = Arc::new(EchoDescriptor::default());
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
