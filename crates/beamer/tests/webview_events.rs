//! Events from Rust to the web UI: what a plugin emits into the queue of
//! its parameter struct reaches the editor through the format wrappers.

#![cfg(feature = "derive")]

use std::cell::RefCell;

use beamer::prelude::*;

static CONFIG: Config = Config::new("Events", Category::Effect, "Bmer", "evnt");

thread_local! {
    /// The queue of the descriptor that was created last on this thread.
    static LAST_EVENTS: RefCell<Option<WebViewEvents>> = const { RefCell::new(None) };
}

/// The plugin's end of the queue of the instance a wrapper just created.
fn last_events() -> WebViewEvents {
    LAST_EVENTS
        .with(|events| events.borrow().clone())
        .expect("a descriptor was created on this thread")
}

#[derive(Parameters)]
struct EventsParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    events: WebViewEvents,
}

#[derive(HasParameters)]
struct EventsDescriptor {
    #[parameters]
    parameters: Arc<EventsParameters>,
}

impl Default for EventsDescriptor {
    fn default() -> Self {
        let parameters = Arc::new(EventsParameters::default());
        LAST_EVENTS.with(|events| *events.borrow_mut() = Some(parameters.events.clone()));
        Self { parameters }
    }
}

impl Descriptor for EventsDescriptor {
    type Setup = ();
    type Processor = Silence;

    fn prepare(&self, _: ()) -> Silence {
        Silence
    }

    fn webview_events(&self) -> Option<&WebViewEvents> {
        Some(&self.parameters.events)
    }
}

struct Silence;

impl Processor for Silence {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

#[test]
fn test_queue_in_the_parameter_struct_starts_empty() {
    let parameters = EventsParameters::default();

    // No wrapper took this queue, so nothing is kept.
    parameters.events.emit("lost", &1);
    assert_eq!(parameters.events.take_script(), None);
}

#[test]
fn test_vst3_delivers_the_queue_of_its_instance() {
    let wrapper = beamer_vst3::Vst3Processor::<EventsDescriptor>::new(&CONFIG);
    let events = last_events();

    // Before an editor is open, events are dropped.
    events.emit("early", &1);
    assert_eq!(events.take_script(), None);

    // What the view does when it is attached, on its timer and when it is
    // removed.
    events.editor_opened();
    events.emit("ready", &"now");
    assert_eq!(
        events.take_script().as_deref(),
        Some("window.__BEAMER__._onEvent(\"ready\",\"now\");")
    );
    events.editor_closed();

    drop(wrapper);
}

#[cfg(target_os = "macos")]
#[test]
fn test_au_delivers_the_queue_of_its_instance() {
    use std::ffi::CStr;

    use beamer_au::bridge::{
        beamer_au_create_instance, beamer_au_destroy_instance, beamer_au_free_string,
        beamer_au_webview_events_close, beamer_au_webview_events_open,
        beamer_au_webview_events_take,
    };
    use beamer_au::factory::register_factory;
    use beamer_au::{AuPluginInstance, AuProcessor};

    fn create() -> Box<dyn AuPluginInstance> {
        Box::new(AuProcessor::<EventsDescriptor>::new())
    }

    /// What the editor's timer gets from one call.
    fn take(editor: *mut beamer_au::bridge::BeamerAuWebViewEvents) -> Option<String> {
        let script = beamer_au_webview_events_take(editor);
        if script.is_null() {
            return None;
        }
        // SAFETY: The bridge returned a null-terminated string that it owns
        // until it is freed below.
        let text = unsafe { CStr::from_ptr(script) }.to_string_lossy().into_owned();
        beamer_au_free_string(script);
        Some(text)
    }

    // One plugin per process: this test is the only one that registers.
    register_factory(create, &CONFIG);
    let instance = beamer_au_create_instance();
    assert!(!instance.is_null());
    let events = last_events();

    // Before an editor is open, events are dropped.
    events.emit("early", &1);

    let editor = beamer_au_webview_events_open(instance);
    assert!(!editor.is_null());
    assert_eq!(take(editor), None);

    events.emit("ready", &"now");
    events.emit("count", &2);
    assert_eq!(
        take(editor).as_deref(),
        Some(
            "window.__BEAMER__._onEvent(\"ready\",\"now\");\
             window.__BEAMER__._onEvent(\"count\",2);"
        )
    );
    assert_eq!(take(editor), None);

    // A closed editor gets nothing, and the next one starts empty.
    beamer_au_webview_events_close(editor);
    events.emit("between", &3);
    let editor = beamer_au_webview_events_open(instance);
    assert_eq!(take(editor), None);

    // A host may destroy the instance before the editor is gone.
    beamer_au_destroy_instance(instance);
    events.emit("after", &4);
    assert!(take(editor).is_some());
    beamer_au_webview_events_close(editor);

    // No instance and no editor: nothing to open, take or close.
    assert!(beamer_au_webview_events_open(std::ptr::null_mut()).is_null());
    assert_eq!(take(std::ptr::null_mut()), None);
    beamer_au_webview_events_close(std::ptr::null_mut());
}
