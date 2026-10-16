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

    #[state(id = "zoom")]
    zoom: State<u32>,

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
    use std::ffi::{c_char, CStr};

    use beamer_au::bridge::{
        beamer_au_create_instance, beamer_au_destroy_instance, beamer_au_editor_sync_close,
        beamer_au_editor_sync_frame_script, beamer_au_editor_sync_init_script,
        beamer_au_editor_sync_message, beamer_au_editor_sync_open,
        beamer_au_editor_sync_parameter_script, beamer_au_free_string, beamer_au_param_set_from_ui,
    };
    use beamer_au::factory::register_factory;
    use beamer_au::{AuPluginInstance, AuProcessor};

    fn create() -> Box<dyn AuPluginInstance> {
        Box::new(AuProcessor::<EventsDescriptor>::new())
    }

    /// The text of a script the bridge returned, which is then freed.
    fn text(script: *mut c_char) -> Option<String> {
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
    let gain = EventsParameters::default().gain.id();

    // Before an editor is open, events are dropped.
    events.emit("early", &1);

    let editor = beamer_au_editor_sync_open(instance);
    assert!(!editor.is_null());
    let init = text(beamer_au_editor_sync_init_script(editor, c"auv2".as_ptr())).expect("init");
    assert!(init.starts_with("window.__BEAMER__._onInit([{"));
    assert!(init.ends_with("],{\"pluginFormat\":\"auv2\",\"state\":{\"zoom\":0}})"));

    // The first frame sends every parameter, then nothing changed.
    assert_eq!(
        text(beamer_au_editor_sync_frame_script(editor)),
        Some(format!("window.__BEAMER__._onParams({{{gain}:[0.0,0.0,\"0.00\"]}});"))
    );
    assert_eq!(text(beamer_au_editor_sync_frame_script(editor)), None);

    // A value the web UI set is answered and not sent again.
    beamer_au_param_set_from_ui(instance, gain, 0.5);
    assert_eq!(
        text(beamer_au_editor_sync_parameter_script(editor, gain)),
        Some(format!("window.__BEAMER__._onParams({{{gain}:[0.5,0.5,\"0.50\"]}});"))
    );
    assert_eq!(text(beamer_au_editor_sync_frame_script(editor)), None);

    // A State field the web UI sets, from the message as it sent it.
    let message = br#"{"type":"state:set","id":"zoom","value":150}"#;
    assert_eq!(
        text(beamer_au_editor_sync_message(editor, message.as_ptr(), message.len())).as_deref(),
        Some("window.__BEAMER__._onState({\"zoom\":150});")
    );
    assert_eq!(text(beamer_au_editor_sync_frame_script(editor)), None);
    let other = br#"{"type":"param:begin","id":1}"#;
    assert_eq!(text(beamer_au_editor_sync_message(editor, other.as_ptr(), other.len())), None);

    events.emit("ready", &"now");
    events.emit("count", &2);
    assert_eq!(
        text(beamer_au_editor_sync_frame_script(editor)).as_deref(),
        Some(
            "window.__BEAMER__._onEvent(\"ready\",\"now\");\
             window.__BEAMER__._onEvent(\"count\",2);"
        )
    );
    assert_eq!(text(beamer_au_editor_sync_frame_script(editor)), None);

    // A closed editor gets nothing, and the next one starts with the
    // parameters.
    beamer_au_editor_sync_close(editor);
    events.emit("between", &3);
    let editor = beamer_au_editor_sync_open(instance);
    let first = text(beamer_au_editor_sync_frame_script(editor)).expect("a first frame");
    assert!(!first.contains("between"));

    // A host may destroy the instance before the editor is gone.
    beamer_au_destroy_instance(instance);
    events.emit("after", &4);
    assert!(text(beamer_au_editor_sync_frame_script(editor)).is_some());
    beamer_au_editor_sync_close(editor);

    // No instance and no editor: nothing to open, build or close.
    assert!(beamer_au_editor_sync_open(std::ptr::null_mut()).is_null());
    assert_eq!(text(beamer_au_editor_sync_frame_script(std::ptr::null_mut())), None);
    assert_eq!(text(beamer_au_editor_sync_parameter_script(std::ptr::null_mut(), gain)), None);
    beamer_au_editor_sync_close(std::ptr::null_mut());
}

/// A struct whose queue sits in a nested struct.
#[derive(Parameters)]
struct OuterParameters {
    #[parameter(id = "mix", name = "Mix", default = 1.0, range = 0.0..=1.0)]
    mix: FloatParameter,

    #[nested(group = "Inner")]
    inner: EventsParameters,
}

/// A struct without a queue.
#[derive(Parameters)]
struct QuietParameters {
    #[parameter(id = "mix", name = "Mix", default = 1.0, range = 0.0..=1.0)]
    mix: FloatParameter,
}

#[test]
fn test_derive_finds_the_queue_by_its_type() {
    let parameters = EventsParameters::default();
    let found = Parameters::webview_events(&parameters).expect("the queue is found");
    assert!(std::ptr::eq(found, &parameters.events));

    // Also in a nested struct.
    let outer = OuterParameters::default();
    let found = Parameters::webview_events(&outer).expect("the nested queue is found");
    assert!(std::ptr::eq(found, &outer.inner.events));

    assert!(Parameters::webview_events(&QuietParameters::default()).is_none());
}
