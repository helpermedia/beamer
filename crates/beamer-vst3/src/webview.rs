//! IPlugView wrapper for WebView GUIs.

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::fmt::Write;
use std::sync::Arc;

use beamer_core::{GuiConstraints, GuiDelegate, ParameterStore, Size, WebViewHandler};
use beamer_webview::platform::PlatformWebView;
pub use beamer_webview::WebViewConfig;
use vst3::Steinberg::Vst::IComponentHandler;
use vst3::Steinberg::*;
use vst3::Class;

/// Shared context between WebViewPlugView and its IPC callbacks.
///
/// This struct is heap-allocated and pinned. Raw pointers to it are passed
/// as the callback context for the WebView's message and loaded callbacks.
struct IpcContext {
    /// Parameter store (trait object for type-erased access).
    params: *const dyn ParameterStore,
    /// Component handler for host notification. Null until set.
    handler: *mut IComponentHandler,
    /// Custom WebView message handler (invoke/event routing).
    webview_handler: Option<Arc<dyn WebViewHandler>>,
    /// Cached parameter values from the last sync tick.
    /// Index corresponds to ParameterStore::info(index).
    last_values: Vec<f64>,
    /// Pointer to the platform WebView (for evaluate_js calls from callbacks).
    /// Set in attached(), cleared in removed().
    webview: *const PlatformWebView,
    /// NSTimer handle for parameter sync. Null when not running.
    sync_timer: *mut objc2::runtime::AnyObject,
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

/// AddRef a non-null IComponentHandler.
///
/// # Safety
///
/// `handler` must be a valid COM pointer or null.
unsafe fn handler_addref(handler: *mut IComponentHandler) {
    if !handler.is_null() {
        let unknown = handler as *mut FUnknown;
        unsafe { ((*(*unknown).vtbl).addRef)(unknown) };
    }
}

/// Release a non-null IComponentHandler.
///
/// # Safety
///
/// `handler` must be a valid COM pointer or null.
unsafe fn handler_release(handler: *mut IComponentHandler) {
    if !handler.is_null() {
        let unknown = handler as *mut FUnknown;
        unsafe { ((*(*unknown).vtbl).release)(unknown) };
    }
}

impl WebViewPlugView {
    /// Create a new WebView plug view with parameter sync support.
    ///
    /// # Safety
    ///
    /// `params` must be a valid pointer that remains valid for the lifetime
    /// of this view (it points to the plugin's parameter struct which
    /// outlives the editor).
    /// `component_handler` is the IComponentHandler pointer (may be null initially).
    /// If non-null, this function AddRefs it; the view owns a reference until dropped.
    pub unsafe fn new(
        config: WebViewConfig<'static>,
        delegate: Box<dyn GuiDelegate>,
        params: *const dyn ParameterStore,
        component_handler: *mut IComponentHandler,
        webview_handler: Option<Arc<dyn WebViewHandler>>,
    ) -> Self {
        let size = delegate.gui_size();

        // Pre-allocate last_values cache.
        // NAN as sentinel: NAN != NAN guarantees the first sync tick sends all values.
        // SAFETY: Caller guarantees params is valid.
        let param_count = unsafe { &*params }.count();
        let last_values = vec![f64::NAN; param_count];

        // AddRef the handler so the view owns an independent reference.
        // SAFETY: Caller guarantees component_handler is a valid COM pointer or null.
        unsafe { handler_addref(component_handler) };

        Self {
            platform: UnsafeCell::new(None),
            config: UnsafeCell::new(config),
            delegate: UnsafeCell::new(delegate),
            size: UnsafeCell::new(size),
            frame: UnsafeCell::new(std::ptr::null_mut()),
            ipc: UnsafeCell::new(Box::new(IpcContext {
                params,
                handler: component_handler,
                webview_handler,
                last_values,
                webview: std::ptr::null(),
                sync_timer: std::ptr::null_mut(),
            })),
        }
    }

    /// Update the component handler pointer (called when host sets a new handler).
    pub fn set_component_handler(&self, handler: *mut IComponentHandler) {
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let ipc = unsafe { &mut *self.ipc.get() };
        let old = ipc.handler;
        ipc.handler = handler;
        // SAFETY: handler is a valid COM pointer or null per caller contract.
        unsafe {
            handler_addref(handler);
            handler_release(old);
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
    // SAFETY: json/len come from the WebView message handler, guaranteed valid UTF-8 JSON.
    let json_str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(json, len)) };

    let Ok(msg) = serde_json::from_str::<serde_json::Value>(json_str) else {
        log::warn!("Invalid IPC message JSON: {json_str}");
        return;
    };

    let Some(msg_type) = msg.get("type").and_then(|t| t.as_str()) else {
        return;
    };

    // SAFETY: params pointer remains valid for the lifetime of the view.
    let params = unsafe { &*ipc.params };

    match msg_type {
        "param:set" => {
            let Some(id) = msg.get("id").and_then(|v| v.as_u64()).map(|v| v as u32) else { return };
            let Some(value) = msg.get("value").and_then(|v| v.as_f64()) else { return };
            params.set_normalized(id, value);
            if !ipc.handler.is_null() {
                // SAFETY: handler is non-null and is valid COM pointer with valid vtbl.
                unsafe {
                    ((*(*ipc.handler).vtbl).performEdit)(ipc.handler, id, value);
                }
            }
        }
        "param:begin" => {
            let Some(id) = msg.get("id").and_then(|v| v.as_u64()).map(|v| v as u32) else { return };
            if !ipc.handler.is_null() {
                // SAFETY: handler is non-null and is valid COM pointer with valid vtbl.
                unsafe {
                    ((*(*ipc.handler).vtbl).beginEdit)(ipc.handler, id);
                }
            }
        }
        "param:end" => {
            let Some(id) = msg.get("id").and_then(|v| v.as_u64()).map(|v| v as u32) else { return };
            if !ipc.handler.is_null() {
                // SAFETY: handler is non-null and is valid COM pointer with valid vtbl.
                unsafe {
                    ((*(*ipc.handler).vtbl).endEdit)(ipc.handler, id);
                }
            }
        }
        "invoke" => {
            let Some(method) = msg.get("method").and_then(|v| v.as_str()) else { return };
            let args = msg.get("args").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let call_id = msg.get("callId").and_then(|v| v.as_u64()).unwrap_or(0);

            let result = match &ipc.webview_handler {
                Some(handler) => handler.on_invoke(method, &args),
                None => Ok(serde_json::Value::Null),
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

            if let Some(handler) = &ipc.webview_handler {
                handler.on_event(name, &data);
            }
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
    let ipc = unsafe { &*(context as *const IpcContext) };
    if ipc.webview.is_null() {
        return;
    }

    // SAFETY: params and webview pointers remain valid for the view lifetime.
    let params = unsafe { &*ipc.params };
    // SAFETY: webview is non-null (checked above) and valid for the view lifetime.
    let webview = unsafe { &*ipc.webview };

    let json_array = beamer_core::params_to_init_json(params);
    let js = format!("window.__BEAMER__._onInit({json_array})");
    webview.evaluate_js(&js);
}

/// NSTimer callback for 60Hz parameter sync.
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
    let ptr: *const objc2::runtime::AnyObject = unsafe { objc2::msg_send![user_info, pointerValue] };
    if ptr.is_null() {
        return;
    }

    // SAFETY: ptr is a valid IpcContext pointer stored in the NSValue.
    let ipc = unsafe { &mut *(ptr as *mut IpcContext) };
    // Guard against timer firing after webview detach but before invalidation.
    if ipc.webview.is_null() {
        return;
    }

    // SAFETY: params and webview pointers remain valid for the view lifetime.
    let params = unsafe { &*ipc.params };
    // SAFETY: webview is non-null (checked above) and valid for the view lifetime.
    let webview = unsafe { &*ipc.webview };

    // Poll and push changed parameters.
    let mut script = String::new();
    let mut any_changed = false;

    let count = params.count();
    for i in 0..count {
        let Some(info) = params.info(i) else { continue };
        let val = params.get_normalized(info.id);
        if i < ipc.last_values.len() && val != ipc.last_values[i] {
            ipc.last_values[i] = val;
            if !any_changed {
                script.push_str("window.__BEAMER__._onParams({");
                any_changed = true;
            } else {
                script.push(',');
            }
            let _ = write!(script, "{}:{}", info.id, val);
        }
    }

    if any_changed {
        script.push_str("})");
        webview.evaluate_js(&script);
    }
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

                // Reset cached values so the first sync tick sends everything.
                for v in &mut ipc.last_values {
                    *v = f64::NAN;
                }

                // Start 60Hz sync timer.
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
                            scheduledTimerWithTimeInterval: (1.0 / 60.0f64),
                            target: target,
                            selector: objc2::sel!(beamerSyncTimerFired:),
                            userInfo: ns_value,
                            repeats: true
                        ]
                    };

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

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let platform = unsafe { &mut *self.platform.get() };
        if let Some(webview) = platform.as_mut() {
            webview.detach();
        }
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

        // Release our AddRef'd IComponentHandler reference.
        // SAFETY: handler was AddRef'd in new() or set_component_handler().
        unsafe { handler_release(ipc.handler) };
        ipc.handler = std::ptr::null_mut();

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

    let c_name = c"BeamerSyncTimerTarget";

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
