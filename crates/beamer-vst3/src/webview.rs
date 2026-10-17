//! IPlugView wrapper for WebView GUIs.

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use beamer_core::{
    EditorSync, GuiConstraints, GuiDelegate, SharedModel, Size, WebViewDispatch,
    WebViewEvents,
};
use beamer_webview::platform::PlatformWebView;
pub use beamer_webview::WebViewConfig;
use vst3::Steinberg::Vst::{IComponentHandler2, IComponentHandler2Trait};
use vst3::Steinberg::*;
use vst3::{Class, ComRef};

use crate::component_handler::ComponentHandler;

/// Shared context between WebViewPlugView and its IPC callbacks.
///
/// This struct is heap-allocated and pinned. Raw pointers to it are passed
/// as the callback context for the WebView's message and loaded callbacks.
struct IpcContext {
    /// The model of the plugin (trait object for type-erased access). The
    /// view shares it with the plugin for as long as it lives.
    model: Arc<dyn SharedModel>,
    /// The component handler the host set last, shared with the controller.
    handler: Arc<ComponentHandler>,
    /// Where the web UI's invoke calls and events go: the model.
    dispatch: Arc<dyn WebViewDispatch>,
    /// The plugin's queue of events for the web UI, if it has one.
    events: Option<WebViewEvents>,
    /// The `state_generation()` of the model that the host knows
    /// about, shared with the processor.
    known_state_generation: Arc<AtomicU64>,
    /// What keeps the web UI in step while the view shows an editor: set
    /// in attached(), dropped in removed() and with the view.
    sync: Option<EditorSync>,
    /// Pointer to the platform WebView (for evaluate_js calls from callbacks).
    /// Set in attached(), cleared in removed() and when the view is dropped.
    webview: *const PlatformWebView,
    /// NSTimer handle for parameter sync. Null when not running.
    sync_timer: *mut objc2::runtime::AnyObject,
}

impl IpcContext {
    /// Start keeping the web UI in step: the view shows an editor now.
    fn open_sync(&mut self) {
        self.sync = Some(EditorSync::open(Arc::clone(&self.model), self.events.clone()));
    }

    /// Tell the host that the plugin state changed when a `State` field was
    /// set since the last check: by the web UI or by any other thread.
    /// Parameters do not count here: the host learns about their changes
    /// through the edits.
    fn report_state_change(&self) {
        // Without a handler the change stays unreported until there is one.
        // SAFETY: handler is null or a valid COM pointer the slot holds a
        // reference to.
        let Some(handler) = (unsafe { ComRef::from_raw(self.handler.get()) }) else {
            return;
        };
        let generation = self.model.state_generation();
        if self.known_state_generation.swap(generation, Ordering::Relaxed) == generation {
            return;
        }
        // A host without IComponentHandler2 cannot be told.
        if let Some(handler) = handler.cast::<IComponentHandler2>() {
            // SAFETY: handler is a valid COM pointer.
            unsafe { handler.setDirty(1) };
        }
    }
}

/// VST3 IPlugView implementation backed by a platform WebView.
pub struct WebViewPlugView {
    platform: UnsafeCell<Option<PlatformWebView>>,
    config: UnsafeCell<WebViewConfig<'static>>,
    delegate: UnsafeCell<Box<dyn GuiDelegate>>,
    size: UnsafeCell<Size>,
    frame: UnsafeCell<*mut IPlugFrame>,
    /// IPC context, heap-allocated for stable pointer.
    ipc: UnsafeCell<Box<IpcContext>>,
}

// SAFETY: VST3 IPlugView methods are called from the UI thread only.
unsafe impl Send for WebViewPlugView {}
// SAFETY: VST3 IPlugView methods are called from the UI thread only.
unsafe impl Sync for WebViewPlugView {}

impl WebViewPlugView {
    /// Create a new WebView plug view with parameter sync support.
    ///
    /// `component_handler` is the controller's slot for the host's handler,
    /// so that the view always calls the handler the host set last.
    pub fn new(
        config: WebViewConfig<'static>,
        delegate: Box<dyn GuiDelegate>,
        model: Arc<dyn SharedModel>,
        component_handler: Arc<ComponentHandler>,
        dispatch: Arc<dyn WebViewDispatch>,
        events: Option<WebViewEvents>,
        known_state_generation: Arc<AtomicU64>,
    ) -> Self {
        let size = delegate.gui_size();

        Self {
            platform: UnsafeCell::new(None),
            config: UnsafeCell::new(config),
            delegate: UnsafeCell::new(delegate),
            size: UnsafeCell::new(size),
            frame: UnsafeCell::new(std::ptr::null_mut()),
            ipc: UnsafeCell::new(Box::new(IpcContext {
                model,
                handler: component_handler,
                dispatch,
                events,
                known_state_generation,
                sync: None,
                webview: std::ptr::null(),
                sync_timer: std::ptr::null_mut(),
            })),
        }
    }
}

impl Class for WebViewPlugView {
    type Interfaces = (IPlugView,);
}

// ---------------------------------------------------------------------------
// IPC callbacks (extern "C-unwind" for WebView)
// ---------------------------------------------------------------------------

/// Message callback: dispatches JSON messages from JavaScript.
unsafe extern "C-unwind" fn on_message(context: *mut c_void, json: *const u8, len: usize) {
    if context.is_null() || json.is_null() {
        return;
    }

    // SAFETY: context is a valid IpcContext pointer (set in attached()).
    let ipc = unsafe { &mut *(context as *mut IpcContext) };
    // SAFETY: json/len come from the WebView message handler.
    let bytes = unsafe { std::slice::from_raw_parts(json, len) };
    let Ok(json_str) = std::str::from_utf8(bytes) else {
        log::error!("IPC message is not valid UTF-8.");
        return;
    };

    let Ok(msg) = serde_json::from_str::<serde_json::Value>(json_str) else {
        log::warn!("Invalid IPC message JSON: {json_str}");
        return;
    };

    let Some(msg_type) = msg.get("type").and_then(|t| t.as_str()) else {
        return;
    };

    let parameters = ipc.model.parameter_store();

    match msg_type {
        "param:set" => {
            let Some(id) = msg.get("id").and_then(|v| v.as_u64()).map(|v| v as u32) else { return };
            let Some(value) = msg.get("value").and_then(|v| v.as_f64()) else { return };
            parameters.set_normalized(id, value);
            let handler = ipc.handler.get();
            if !handler.is_null() {
                // SAFETY: handler is non-null and is valid COM pointer with valid vtbl.
                unsafe {
                    ((*(*handler).vtbl).performEdit)(handler, id, value);
                }
            }
            // Answer with the value in effect at once, so that the display
            // text follows without waiting for the next frame.
            if let (false, Some(sync)) = (ipc.webview.is_null(), ipc.sync.as_mut()) {
                // SAFETY: webview is valid for the view lifetime.
                let webview = unsafe { &*ipc.webview };
                webview.evaluate_js(&sync.parameter_script(id));
            }
        }
        "state:set" => {
            let Some(path) = msg.get("id").and_then(|v| v.as_str()) else { return };
            let value = msg.get("value").unwrap_or(&serde_json::Value::Null);
            // Answer with the value in effect, which corrects a value that
            // does not fit the field.
            if let (false, Some(sync)) = (ipc.webview.is_null(), ipc.sync.as_mut()) {
                if let Some(script) = sync.state_script(path, value) {
                    // SAFETY: webview is valid for the view lifetime.
                    let webview = unsafe { &*ipc.webview };
                    webview.evaluate_js(&script);
                }
            }
        }
        "param:begin" => {
            let Some(id) = msg.get("id").and_then(|v| v.as_u64()).map(|v| v as u32) else { return };
            let handler = ipc.handler.get();
            if !handler.is_null() {
                // SAFETY: handler is non-null and is valid COM pointer with valid vtbl.
                unsafe {
                    ((*(*handler).vtbl).beginEdit)(handler, id);
                }
            }
        }
        "param:end" => {
            let Some(id) = msg.get("id").and_then(|v| v.as_u64()).map(|v| v as u32) else { return };
            let handler = ipc.handler.get();
            if !handler.is_null() {
                // SAFETY: handler is non-null and is valid COM pointer with valid vtbl.
                unsafe {
                    ((*(*handler).vtbl).endEdit)(handler, id);
                }
            }
        }
        "invoke" => {
            let Some(method) = msg.get("method").and_then(|v| v.as_str()) else { return };
            let args = msg.get("args").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let call_id = msg.get("callId").and_then(|v| v.as_u64()).unwrap_or(0);

            // Handle built-in invokes before dispatching to the model.
            let result = if method == "_beamer/paramTextToNormalized" {
                let param_id = args.first().and_then(|v| v.as_u64()).map(|v| v as u32);
                let text = args.get(1).and_then(|v| v.as_str());
                match (param_id, text) {
                    (Some(id), Some(s)) => match parameters.string_to_normalized(id, s) {
                        Some(norm) => Ok(serde_json::Value::from(norm)),
                        None => Ok(serde_json::Value::Null),
                    },
                    _ => Ok(serde_json::Value::Null),
                }
            } else {
                ipc.dispatch.dispatch_invoke(method, &args)
            };

            // Send result back to JS so the Promise resolves/rejects.
            if !ipc.webview.is_null() {
                // SAFETY: webview pointer is valid for the view lifetime.
                let webview = unsafe { &*ipc.webview };
                let js = match result {
                    Ok(val) => {
                        let json = serde_json::to_string(&val).unwrap_or_else(|_| "null".into());
                        format!("window.__BEAMER__._onResult({call_id},{{\"ok\":{json}}})")
                    }
                    Err(err) => {
                        let escaped = serde_json::to_string(&err).unwrap_or_default();
                        format!("window.__BEAMER__._onResult({call_id},{{\"err\":{escaped}}})")
                    }
                };
                webview.evaluate_js(&js);
            }
        }
        "event" => {
            let Some(name) = msg.get("name").and_then(|v| v.as_str()) else { return };
            let data = msg.get("data").cloned().unwrap_or(serde_json::Value::Null);

            ipc.dispatch.dispatch_event(name, &data);
        }
        _ => {
            log::debug!("Unknown IPC message type: {msg_type}");
        }
    }
}

/// Loaded callback: sends the parameter init dump when the page finishes loading.
unsafe extern "C-unwind" fn on_loaded(context: *mut c_void) {
    if context.is_null() {
        return;
    }

    // SAFETY: context is a valid IpcContext pointer (set in attached()).
    let ipc = unsafe { &mut *(context as *mut IpcContext) };
    let (false, Some(sync)) = (ipc.webview.is_null(), ipc.sync.as_mut()) else {
        return;
    };

    // SAFETY: webview is non-null (checked above) and valid for the view lifetime.
    let webview = unsafe { &*ipc.webview };
    webview.evaluate_js(&sync.init_script("vst3"));
}

/// NSTimer callback for 60 Hz parameter sync.
unsafe extern "C-unwind" fn sync_timer_fired(
    _this: *mut objc2::runtime::AnyObject,
    _cmd: objc2::runtime::Sel,
    timer: *mut objc2::runtime::AnyObject,
) {
    // SAFETY: timer is a valid NSTimer object provided by the Cocoa runtime.
    let user_info: *mut objc2::runtime::AnyObject = unsafe { objc2::msg_send![timer, userInfo] };
    if user_info.is_null() {
        return;
    }

    // SAFETY: userInfo is an NSValue wrapping our context pointer.
    let ptr: *mut c_void = unsafe { objc2::msg_send![user_info, pointerValue] };
    if ptr.is_null() {
        return;
    }

    // SAFETY: ptr is a valid IpcContext pointer stored in the NSValue.
    let ipc = unsafe { &mut *(ptr as *mut IpcContext) };
    // The pointer is null while no WebView is attached.
    if ipc.webview.is_null() {
        return;
    }

    // SAFETY: webview is non-null (checked above) and valid for the view lifetime.
    let webview = unsafe { &*ipc.webview };

    // The parameters that changed since the last frame and the events the
    // plugin emitted.
    if let Some(script) = ipc.sync.as_mut().and_then(EditorSync::frame_script) {
        webview.evaluate_js(&script);
    }

    ipc.report_state_change();
}

#[allow(non_snake_case)]
impl IPlugViewTrait for WebViewPlugView {
    unsafe fn isPlatformTypeSupported(&self, r#type: FIDString) -> tresult {
        if r#type.is_null() {
            return kResultFalse;
        }
        // SAFETY: type_ is non-null and host provides null-terminated C string.
        let type_str = unsafe { std::ffi::CStr::from_ptr(r#type) };

        #[cfg(target_os = "macos")]
        // SAFETY: kPlatformTypeNSView is a static null-terminated byte literal.
        let supported = type_str == unsafe { std::ffi::CStr::from_ptr(kPlatformTypeNSView) };

        #[cfg(target_os = "windows")]
        // SAFETY: kPlatformTypeHWND is a static null-terminated byte literal.
        let supported = type_str == unsafe { std::ffi::CStr::from_ptr(kPlatformTypeHWND) };

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let supported = false;

        if supported { kResultOk } else { kResultFalse }
    }

    unsafe fn attached(&self, parent: *mut c_void, r#type: FIDString) -> tresult {
        // SAFETY: r#type is forwarded from the host and we are in an unsafe fn.
        if unsafe { self.isPlatformTypeSupported(r#type) } != kResultOk {
            return kResultFalse;
        }

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let platform = unsafe { &mut *self.platform.get() };
        if platform.is_some() {
            return kResultFalse;
        }

        // Set up IPC callbacks in the config.
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let ipc = unsafe { &mut *self.ipc.get() };
        let ipc_ptr = &mut **ipc as *mut IpcContext as *mut c_void;

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let config = unsafe { &mut *self.config.get() };
        config.message_callback = Some(on_message);
        config.loaded_callback = Some(on_loaded);
        config.callback_context = ipc_ptr;

        // SAFETY: parent is a valid platform handle provided by the host.
        match unsafe { PlatformWebView::attach_to_parent(parent, config) } {
            Ok(webview) => {
                *platform = Some(webview);

                // Point the IPC context to the webview for evaluate_js calls.
                ipc.webview = platform.as_ref().unwrap() as *const PlatformWebView;

                // The first frame sends every parameter, and events the
                // plugin emits from now on reach this editor.
                ipc.open_sync();

                // Start 60 Hz sync timer.
                #[cfg(target_os = "macos")]
                {
                    use objc2::msg_send;

                    // SAFETY: NSValue class is always available; wrapping a raw pointer.
                    let ns_value: *mut objc2::runtime::AnyObject = unsafe {
                        msg_send![
                            objc2::runtime::AnyClass::get(c"NSValue").unwrap(),
                            valueWithPointer: ipc_ptr
                        ]
                    };

                    let timer_class = get_or_register_timer_class();
                    // SAFETY: Allocating a new NSObject subclass instance.
                    let target: *mut objc2::runtime::AnyObject = unsafe {
                        msg_send![timer_class, alloc]
                    };
                    // SAFETY: Initializing the allocated NSObject subclass instance.
                    let target: *mut objc2::runtime::AnyObject = unsafe {
                        msg_send![target, init]
                    };

                    // SAFETY: NSTimer class is always available; creating a repeating timer.
                    let timer: *mut objc2::runtime::AnyObject = unsafe {
                        msg_send![
                            objc2::runtime::AnyClass::get(c"NSTimer").unwrap(),
                            timerWithTimeInterval: (1.0 / 60.0f64),
                            target: target,
                            selector: objc2::sel!(beamerSyncTimerFired:),
                            userInfo: ns_value,
                            repeats: true
                        ]
                    };

                    // Schedule in the common modes, so that the UI keeps following
                    // the host while it runs a tracking loop (a drag, a resize or
                    // an open menu). The run loop retains the timer.
                    // SAFETY: NSRunLoop class is always available, the timer is valid
                    // and NSRunLoopCommonModes is a constant of Foundation.
                    unsafe {
                        let run_loop: *mut objc2::runtime::AnyObject = msg_send![
                            objc2::runtime::AnyClass::get(c"NSRunLoop").unwrap(),
                            currentRunLoop
                        ];
                        let _: () = msg_send![
                            run_loop,
                            addTimer: timer,
                            forMode: objc2_foundation::NSRunLoopCommonModes
                        ];
                    }

                    // SAFETY: target is a valid +1 retained object from alloc+init above.
                    // The timer retains the target, so we balance the alloc's +1 here.
                    let _: () = unsafe { msg_send![target, release] };

                    ipc.sync_timer = timer;
                }

                // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
                let delegate = unsafe { &mut *self.delegate.get() };
                delegate.gui_opened();
                kResultOk
            }
            Err(e) => {
                log::error!("Failed to create WebView: {e}");
                // Clear callbacks on failure.
                config.message_callback = None;
                config.loaded_callback = None;
                config.callback_context = std::ptr::null_mut();
                kResultFalse
            }
        }
    }

    unsafe fn removed(&self) -> tresult {
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let delegate = unsafe { &mut *self.delegate.get() };
        delegate.gui_closed();

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let ipc = unsafe { &mut *self.ipc.get() };

        // Stop sync timer.
        #[cfg(target_os = "macos")]
        {
            if !ipc.sync_timer.is_null() {
                // SAFETY: sync_timer is a valid NSTimer; invalidate stops and releases it.
                unsafe {
                    let _: () = objc2::msg_send![ipc.sync_timer, invalidate];
                }
                ipc.sync_timer = std::ptr::null_mut();
            }
        }

        // Clear webview pointer before detaching.
        ipc.webview = std::ptr::null();

        // Without an editor, events are dropped.
        ipc.sync = None;

        // Dropping the platform WebView detaches it from the parent.
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let platform = unsafe { &mut *self.platform.get() };
        *platform = None;
        kResultOk
    }

    unsafe fn onWheel(&self, _distance: f32) -> tresult {
        kResultFalse
    }

    unsafe fn onKeyDown(&self, _key: char16, _keyCode: int16, _modifiers: int16) -> tresult {
        kResultFalse
    }

    unsafe fn onKeyUp(&self, _key: char16, _keyCode: int16, _modifiers: int16) -> tresult {
        kResultFalse
    }

    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let current = unsafe { *self.size.get() };
        // SAFETY: size is non-null (checked above) and host guarantees validity.
        let rect = unsafe { &mut *size };
        rect.left = 0;
        rect.top = 0;
        rect.right = current.width as i32;
        rect.bottom = current.height as i32;
        kResultOk
    }

    unsafe fn onSize(&self, newSize: *mut ViewRect) -> tresult {
        if newSize.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: newSize is non-null (checked above) and host guarantees validity.
        let rect = unsafe { &*newSize };
        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let size = unsafe { &mut *self.size.get() };
        size.width = width;
        size.height = height;

        let new_size = Size::new(width, height);
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let delegate = unsafe { &mut *self.delegate.get() };
        delegate.gui_resized(new_size);

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let platform = unsafe { &*self.platform.get() };
        if let Some(webview) = platform.as_ref() {
            #[cfg(target_os = "macos")]
            webview.set_frame(0, 0, width as i32, height as i32);
            #[cfg(target_os = "windows")]
            webview.set_bounds(0, 0, width as i32, height as i32);
        }

        kResultOk
    }

    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        let frame_ptr = self.frame.get();
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let old_frame = unsafe { *frame_ptr };

        // Release old frame reference.
        if !old_frame.is_null() {
            // SAFETY: old_frame is a valid COM object. IPlugFrame inherits FUnknown.
            unsafe {
                let unknown = old_frame as *mut FUnknown;
                ((*(*unknown).vtbl).release)(unknown);
            };
        }

        // AddRef new frame.
        if !frame.is_null() {
            // SAFETY: frame is a valid COM object provided by the host.
            unsafe {
                let unknown = frame as *mut FUnknown;
                ((*(*unknown).vtbl).addRef)(unknown);
            };
        }

        // SAFETY: Single-threaded access guaranteed by VST3.
        unsafe { *frame_ptr = frame };
        kResultOk
    }

    unsafe fn canResize(&self) -> tresult {
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let delegate = unsafe { &*self.delegate.get() };
        if delegate.gui_constraints().resizable { kResultOk } else { kResultFalse }
    }

    unsafe fn checkSizeConstraint(&self, rect: *mut ViewRect) -> tresult {
        if rect.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let delegate = unsafe { &*self.delegate.get() };
        let constraints = delegate.gui_constraints();

        // SAFETY: rect is non-null (checked above) and host guarantees validity.
        let r = unsafe { &mut *rect };
        let mut width = (r.right - r.left).max(0) as u32;
        let mut height = (r.bottom - r.top).max(0) as u32;

        width = width.clamp(constraints.min.width, constraints.max.width);
        height = height.clamp(constraints.min.height, constraints.max.height);

        r.right = r.left + width as i32;
        r.bottom = r.top + height as i32;
        kResultOk
    }
}

// Release COM references and clean up IPC when dropped.
// This is a safety net in case removed() was not called by the host.
impl Drop for WebViewPlugView {
    fn drop(&mut self) {
        let ipc = self.ipc.get_mut();

        // Invalidate sync timer if still running.
        #[cfg(target_os = "macos")]
        {
            if !ipc.sync_timer.is_null() {
                // SAFETY: sync_timer is a valid NSTimer.
                unsafe {
                    let _: () = objc2::msg_send![ipc.sync_timer, invalidate];
                }
                ipc.sync_timer = std::ptr::null_mut();
            }
        }

        // Clear webview pointer to prevent stale dereferences.
        ipc.webview = std::ptr::null();

        // Close the event queue if removed() did not.
        ipc.sync = None;

        // Detach the WebView while the IPC context it calls into is still alive,
        // and tell the delegate, which removed() would have done.
        if self.platform.get_mut().take().is_some() {
            self.delegate.get_mut().gui_closed();
        }

        let frame = *self.frame.get_mut();
        if !frame.is_null() {
            // SAFETY: frame is a valid COM object. We hold a reference from setFrame.
            unsafe {
                let unknown = frame as *mut FUnknown;
                ((*(*unknown).vtbl).release)(unknown);
            }
        }
    }
}

/// Simple `GuiDelegate` backed by fixed size and constraints.
pub struct StaticGuiDelegate {
    size: Size,
    constraints: GuiConstraints,
}

impl StaticGuiDelegate {
    pub fn new(size: Size, constraints: GuiConstraints) -> Self {
        Self { size, constraints }
    }
}

impl GuiDelegate for StaticGuiDelegate {
    fn gui_size(&self) -> Size {
        self.size
    }

    fn gui_constraints(&self) -> GuiConstraints {
        self.constraints
    }
}

// ---------------------------------------------------------------------------
// NSTimer helper class
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn get_or_register_timer_class() -> &'static objc2::runtime::AnyClass {
    use objc2::runtime::{AnyClass, ClassBuilder};
    use objc2_foundation::NSObject;
    use objc2::ClassType;

    // The name is unique to this binary, so that plugin binaries in one host
    // process do not share the class and each other's timer callback.
    let c_name = beamer_webview::platform::macos_class::class_name(
        "BeamerSyncTimerTarget",
        sync_timer_fired as *const () as usize,
    );
    let c_name = c_name.as_c_str();

    if let Some(existing) = AnyClass::get(c_name) {
        return existing;
    }

    let superclass = NSObject::class();
    let mut builder = match ClassBuilder::new(c_name, superclass) {
        Some(b) => b,
        None => {
            return AnyClass::get(c_name)
                .expect("class must exist after ClassBuilder::new returned None");
        }
    };

    // SAFETY: sync_timer_fired has the correct signature for an ObjC method
    // receiving (self, _cmd, timer). AnyObject is the correct callee type
    // for a dynamically registered class.
    unsafe {
        builder.add_method::<objc2::runtime::AnyObject, _>(
            objc2::sel!(beamerSyncTimerFired:),
            sync_timer_fired
                as unsafe extern "C-unwind" fn(
                    *mut objc2::runtime::AnyObject,
                    objc2::runtime::Sel,
                    *mut objc2::runtime::AnyObject,
                ),
        );
    }

    builder.register()
}

#[cfg(test)]
mod tests {
    use beamer_core::{NoParameters, ParameterStore};
    use vst3::Steinberg::Vst::IComponentHandler;

    use super::*;

    /// A plugin that knows no invoke methods and ignores events.
    struct NoDispatch;

    impl WebViewDispatch for NoDispatch {
        fn dispatch_invoke(
            &self,
            method: &str,
            _args: &[serde_json::Value],
        ) -> Result<serde_json::Value, String> {
            Err(format!("unknown method: {method}"))
        }

        fn dispatch_event(&self, _name: &str, _data: &serde_json::Value) {}
    }

    /// The context of a view of a plugin with this event queue.
    fn context(events: Option<WebViewEvents>) -> IpcContext {
        IpcContext {
            model: Arc::new(NoParameters),
            handler: Arc::new(ComponentHandler::new()),
            dispatch: Arc::new(NoDispatch),
            events,
            known_state_generation: Arc::new(AtomicU64::new(0)),
            sync: None,
            webview: std::ptr::null(),
            sync_timer: std::ptr::null_mut(),
        }
    }

    /// The script of the next frame of a view.
    fn frame(ipc: &mut IpcContext) -> Option<String> {
        ipc.sync.as_mut().and_then(EditorSync::frame_script)
    }

    #[test]
    fn test_view_opens_and_closes_the_event_queue_once() {
        let events = WebViewEvents::new();
        events.claim();
        let mut ipc = context(Some(events.clone()));

        events.emit("before", &1);
        assert_eq!(frame(&mut ipc), None);

        // attached(), then a timer tick.
        ipc.open_sync();
        events.emit("open", &2);
        assert_eq!(
            frame(&mut ipc).as_deref(),
            Some("window.__BEAMER__._onEvent(\"open\",2);")
        );

        // removed(), then the drop of the view.
        ipc.sync = None;
        ipc.sync = None;
        events.emit("closed", &3);
        assert_eq!(frame(&mut ipc), None);

        // A second view of the same plugin was not closed by the first.
        let mut second = context(Some(events.clone()));
        second.open_sync();
        ipc.open_sync();
        ipc.sync = None;
        events.emit("second", &4);
        assert!(frame(&mut second).is_some());
    }

    /// A component handler of a host that counts the calls of setDirty.
    #[derive(Default)]
    struct DirtyRecorder {
        dirty: std::sync::atomic::AtomicUsize,
    }

    impl Class for DirtyRecorder {
        type Interfaces = (IComponentHandler, IComponentHandler2);
    }

    impl vst3::Steinberg::Vst::IComponentHandlerTrait for DirtyRecorder {
        unsafe fn beginEdit(&self, _id: vst3::Steinberg::Vst::ParamID) -> tresult {
            kResultOk
        }

        unsafe fn performEdit(
            &self,
            _id: vst3::Steinberg::Vst::ParamID,
            _value: vst3::Steinberg::Vst::ParamValue,
        ) -> tresult {
            kResultOk
        }

        unsafe fn endEdit(&self, _id: vst3::Steinberg::Vst::ParamID) -> tresult {
            kResultOk
        }

        unsafe fn restartComponent(&self, _flags: int32) -> tresult {
            kResultOk
        }
    }

    impl IComponentHandler2Trait for DirtyRecorder {
        unsafe fn setDirty(&self, state: TBool) -> tresult {
            if state != 0 {
                self.dirty.fetch_add(1, Ordering::Relaxed);
            }
            kResultOk
        }

        unsafe fn requestOpenEditor(&self, _name: FIDString) -> tresult {
            kNotImplemented
        }

        unsafe fn startGroupEdit(&self) -> tresult {
            kNotImplemented
        }

        unsafe fn finishGroupEdit(&self) -> tresult {
            kNotImplemented
        }
    }

    /// The model of a plugin with one `State` field and no parameters.
    #[derive(Default)]
    struct OneState {
        zoom: beamer_core::State<u32>,
    }

    impl beamer_core::ParameterGroups for OneState {}

    impl beamer_core::Parameters for OneState {
        fn count(&self) -> usize {
            0
        }

        fn iter(&self) -> Box<dyn Iterator<Item = &dyn beamer_core::ParameterRef> + '_> {
            Box::new(std::iter::empty())
        }

        fn by_id(&self, _id: u32) -> Option<&dyn beamer_core::ParameterRef> {
            None
        }
    }

    impl beamer_core::ModelFields for OneState {
        fn state_generation(&self) -> u64 {
            self.zoom.generation()
        }
    }

    impl ParameterStore for OneState {
        fn count(&self) -> usize {
            0
        }

        fn info(&self, _index: usize) -> Option<&beamer_core::ParameterInfo> {
            None
        }

        fn get_normalized(&self, _id: u32) -> f64 {
            0.0
        }

        fn set_normalized(&self, _id: u32, _value: f64) {}

        fn normalized_to_string(&self, _id: u32, _normalized: f64) -> String {
            String::new()
        }

        fn string_to_normalized(&self, _id: u32, _string: &str) -> Option<f64> {
            None
        }

        fn normalized_to_plain(&self, _id: u32, normalized: f64) -> f64 {
            normalized
        }

        fn plain_to_normalized(&self, _id: u32, plain: f64) -> f64 {
            plain
        }
    }

    #[test]
    fn test_view_tells_the_host_when_the_state_changed() {
        let model = Arc::new(OneState::default());
        let host = vst3::ComWrapper::new(DirtyRecorder::default());
        let handler = host
            .as_com_ref::<IComponentHandler>()
            .expect("the recorder is an IComponentHandler")
            .as_ptr();
        let known = Arc::new(AtomicU64::new(beamer_core::ModelFields::state_generation(&*model)));
        let slot = Arc::new(ComponentHandler::new());
        let ipc = IpcContext {
            model: Arc::clone(&model) as Arc<dyn SharedModel>,
            handler: Arc::clone(&slot),
            known_state_generation: Arc::clone(&known),
            ..context(None)
        };
        let dirty = || host.dirty.load(Ordering::Relaxed);

        // A State field is set while the host has not set its handler yet.
        model.zoom.set(2);
        ipc.report_state_change();
        assert_eq!(dirty(), 0);

        // The host sets its handler after the view was created: the view
        // uses it, and the change is reported now.
        // SAFETY: handler is a valid COM pointer, and the test is one thread.
        unsafe { slot.set(handler) };
        ipc.report_state_change();
        assert_eq!(dirty(), 1);
        ipc.report_state_change();
        assert_eq!(dirty(), 1);

        // A state the host loaded is one it knows.
        model.zoom.set(3);
        known.store(beamer_core::ModelFields::state_generation(&*model), Ordering::Relaxed);
        ipc.report_state_change();
        assert_eq!(dirty(), 1);
    }

    #[test]
    fn test_view_of_a_plugin_without_events() {
        let mut ipc = context(None);
        ipc.open_sync();
        assert_eq!(frame(&mut ipc), None);
        ipc.sync = None;
    }
}
