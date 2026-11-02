//! Events from Rust to the web UI: what a plugin emits into the queue of
//! its model reaches the editor through the format wrappers.

#![cfg(feature = "derive")]

use std::cell::RefCell;

use beamer::prelude::*;

static CONFIG: Config = Config::new("Events", Category::Effect, "Bmer", "evnt");

thread_local! {
    /// The queue of the model that was created last on this thread.
    static LAST_EVENTS: RefCell<Option<WebViewEventQueue>> = const { RefCell::new(None) };
}

/// The plugin's end of the queue of the instance a wrapper just created.
fn last_events() -> WebViewEventQueue {
    LAST_EVENTS
        .with(|events| events.borrow().clone())
        .expect("a model was created on this thread")
}

/// A model that notes its queue when it is created, so that a test finds
/// the queue of the instance a wrapper just created. Its `Default` is
/// written by hand for that.
#[beamer::model(no_default)]
struct EventsModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    #[state(id = "zoom")]
    zoom: State<u32>,

    events: WebViewEventQueue,
}

impl Default for EventsModel {
    fn default() -> Self {
        let mut model = Self {
            gain: FloatParameter::new("Gain", 0.0, 0.0..=1.0),
            zoom: State::default(),
            events: WebViewEventQueue::default(),
        };
        model.assign_ids("");
        LAST_EVENTS.with(|events| *events.borrow_mut() = Some(model.events.clone()));
        model
    }
}

impl Model for EventsModel {
    type Setup = ();
    type Processor = Silence;

    fn prepare(self: Arc<Self>, _: ()) -> Silence {
        Silence
    }
}

struct Silence;

impl Processor for Silence {
    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {}
}

#[test]
fn test_queue_in_the_model_starts_empty() {
    let model = EventsModel::default();

    // No wrapper took this queue, so nothing is kept.
    model.events.emit("lost", &1);
    assert_eq!(model.events.take_script(), None);
}

#[test]
fn test_vst3_delivers_the_queue_of_its_instance() {
    let wrapper = beamer_vst3::Vst3Processor::<EventsModel>::new(&CONFIG);
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
    use std::ffi::{c_char, c_void, CStr};

    use beamer_au::bridge::{
        beamer_au_create_instance, beamer_au_destroy_instance, beamer_au_editor_sync_close,
        beamer_au_editor_sync_end_open_gestures, beamer_au_editor_sync_frame_script,
        beamer_au_editor_sync_init_script, beamer_au_editor_sync_message, beamer_au_editor_sync_open,
        beamer_au_free_string, BeamerAuEditorHost, BeamerAuEditorSync,
    };
    use beamer_au::factory::register_factory;
    use beamer_au::{AuPluginInstance, AuProcessor};

    fn create() -> Box<dyn AuPluginInstance> {
        Box::new(AuProcessor::<EventsModel>::new())
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

    /// The edits the host was told about, noted by the functions of the
    /// editor's host. The context is the list.
    type Edits = RefCell<Vec<String>>;

    fn note(context: *mut c_void, edit: String) {
        // SAFETY: The context is the test's list, which outlives the calls.
        unsafe { &*(context as *const Edits) }.borrow_mut().push(edit);
    }

    unsafe extern "C" fn begin_gesture(context: *mut c_void, param_id: u32) {
        note(context, format!("begin {param_id}"));
    }

    unsafe extern "C" fn set_parameter(context: *mut c_void, param_id: u32, normalized: f64) {
        note(context, format!("set {param_id} {normalized}"));
    }

    unsafe extern "C" fn end_gesture(context: *mut c_void, param_id: u32) {
        note(context, format!("end {param_id}"));
    }

    let edits = Edits::default();
    let host = BeamerAuEditorHost {
        context: &edits as *const Edits as *mut c_void,
        begin_gesture: Some(begin_gesture),
        set_parameter: Some(set_parameter),
        end_gesture: Some(end_gesture),
    };
    // A message of the page, with the answer of the editor sync.
    let message = |editor: *mut BeamerAuEditorSync, json: &str| {
        text(beamer_au_editor_sync_message(editor, json.as_ptr(), json.len(), &host))
    };

    // One plugin per process: this test is the only one that registers.
    register_factory(create, &CONFIG);
    let instance = beamer_au_create_instance();
    assert!(!instance.is_null());
    let events = last_events();
    let gain = EventsModel::default().gain.id();

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

    // A value the web UI sets reaches the plugin and the host. It is
    // answered and not sent again.
    let set = format!(r#"{{"type":"param:set","id":{gain},"value":0.5}}"#);
    assert_eq!(
        message(editor, &set),
        Some(format!("window.__BEAMER__._onParams({{{gain}:[0.5,0.5,\"0.50\"]}});"))
    );
    assert_eq!(*edits.borrow(), [format!("set {gain} 0.5")]);
    assert_eq!(text(beamer_au_editor_sync_frame_script(editor)), None);

    // A State field the web UI sets, from the message as it sent it.
    assert_eq!(
        message(editor, r#"{"type":"state:set","id":"zoom","value":150}"#).as_deref(),
        Some("window.__BEAMER__._onState({\"zoom\":150});")
    );
    assert_eq!(text(beamer_au_editor_sync_frame_script(editor)), None);

    // A gesture the page left open ends in the host when the page goes
    // away.
    edits.borrow_mut().clear();
    let begin = format!(r#"{{"type":"param:begin","id":{gain}}}"#);
    assert_eq!(message(editor, &begin), None);
    beamer_au_editor_sync_end_open_gestures(editor, &host);
    assert_eq!(*edits.borrow(), [format!("begin {gain}"), format!("end {gain}")]);

    // A method the plugin does not know.
    assert_eq!(
        message(editor, r#"{"type":"invoke","method":"getInfo","callId":2}"#).as_deref(),
        Some(r#"window.__BEAMER__._onResult(2,{"err":"unknown method: getInfo"});"#)
    );

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

    // A closed editor gets nothing. The next one starts with the
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

    // No instance and no editor: nothing to open, build, handle, end or
    // close.
    edits.borrow_mut().clear();
    assert!(beamer_au_editor_sync_open(std::ptr::null_mut()).is_null());
    assert_eq!(text(beamer_au_editor_sync_frame_script(std::ptr::null_mut())), None);
    assert_eq!(message(std::ptr::null_mut(), &set), None);
    beamer_au_editor_sync_end_open_gestures(std::ptr::null_mut(), &host);
    beamer_au_editor_sync_close(std::ptr::null_mut());
    assert!(edits.borrow().is_empty());
}

/// A model whose queue sits in a nested model.
#[beamer::model]
struct OuterModel {
    #[parameter(id = "mix", name = "Mix", default = 1.0, range = 0.0..=1.0)]
    mix: FloatParameter,

    #[nested(group = "Inner")]
    inner: EventsModel,
}

/// A model without a queue.
#[beamer::model]
struct QuietModel {
    #[parameter(id = "mix", name = "Mix", default = 1.0, range = 0.0..=1.0)]
    mix: FloatParameter,
}

#[test]
fn test_derive_finds_the_queue_by_its_type() {
    let model = EventsModel::default();
    let found = WebViewEvents::webview_events(&model).expect("the queue is found");
    assert!(std::ptr::eq(found, &model.events));

    // Also in a nested model.
    let outer = OuterModel::default();
    let found = WebViewEvents::webview_events(&outer).expect("the nested queue is found");
    assert!(std::ptr::eq(found, &outer.inner.events));

    assert!(WebViewEvents::webview_events(&QuietModel::default()).is_none());
}
