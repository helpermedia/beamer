//! IPC support: WKScriptMessageHandler and WKNavigationDelegate.
//!
//! The two ObjC classes are registered at runtime under names that are unique
//! to this binary (see `macos_class.rs`), so every plugin binary in a host
//! process uses its own classes and its own method implementations.

use std::ffi::{c_void, CStr};

use objc2::ffi::{
    objc_getAssociatedObject, objc_setAssociatedObject, OBJC_ASSOCIATION_RETAIN_NONATOMIC,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{msg_send, sel, ClassType, MainThreadMarker};
use objc2_foundation::{NSObject, NSProcessInfo, NSURLRequest};

use crate::platform::macos_class::class_name;
use crate::{LoadedCallback, MessageCallback};

// ---------------------------------------------------------------------------
// Ivar names
// ---------------------------------------------------------------------------

const MSG_CALLBACK_IVAR: &CStr = c"_beamerMsgCallback";
const MSG_CONTEXT_IVAR: &CStr = c"_beamerMsgContext";
const NAV_LOADED_IVAR: &CStr = c"_beamerNavLoaded";
const NAV_CONTEXT_IVAR: &CStr = c"_beamerNavContext";
const NAV_TERMINATED_IVAR: &CStr = c"_beamerNavTerminated";

/// Key of the request that loaded the page, an associated object of the
/// navigation delegate (released with it).
static NAV_REQUEST_KEY: u8 = 0;

/// A web content process that ends again within this many seconds of the
/// last end is not replaced: the page itself most likely ends it. Loading it
/// again would repeat that without end.
const RELOAD_GUARD_SECONDS: f64 = 10.0;

// ---------------------------------------------------------------------------
// BeamerMessageHandler (WKScriptMessageHandler)
// ---------------------------------------------------------------------------

/// Get or register the BeamerMessageHandler ObjC class of this binary.
fn message_handler_class() -> &'static AnyClass {
    let c_name = class_name("BeamerMessageHandler", did_receive_script_message as *const () as usize);
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

    // Ivars for callback function pointer and context.
    builder.add_ivar::<*const c_void>(MSG_CALLBACK_IVAR);
    builder.add_ivar::<*mut c_void>(MSG_CONTEXT_IVAR);

    // SAFETY: method signature matches the WKScriptMessageHandler protocol.
    unsafe {
        builder.add_method(
            sel!(userContentController:didReceiveScriptMessage:),
            did_receive_script_message
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyObject, *const AnyObject),
        );
    }

    builder.register()
}

/// `userContentController:didReceiveScriptMessage:` implementation.
unsafe extern "C-unwind" fn did_receive_script_message(
    this: *mut AnyObject,
    _cmd: Sel,
    _controller: *const AnyObject,
    message: *const AnyObject,
) {
    // SAFETY: WebKit provides a valid receiver pointer.
    let this: &AnyObject = unsafe { &*this };
    // SAFETY: WebKit provides a valid message pointer.
    let message: &AnyObject = unsafe { &*message };

    // SAFETY: WKScriptMessage has a `body` property.
    let body: *const AnyObject = unsafe { msg_send![message, body] };
    if body.is_null() {
        return;
    }

    // SAFETY: body is a valid NSString from postMessage(JSON.stringify(...)).
    let utf8: *const u8 = unsafe { msg_send![body, UTF8String] };
    if utf8.is_null() {
        return;
    }
    // SAFETY: NSUTF8StringEncoding = 4; body is a valid NSString.
    let len: usize = unsafe { msg_send![body, lengthOfBytesUsingEncoding: 4u64] };

    // Read callback and context from ivars.
    let callback_ivar = this.class().instance_variable(MSG_CALLBACK_IVAR);
    let context_ivar = this.class().instance_variable(MSG_CONTEXT_IVAR);

    let (Some(cb_ivar), Some(ctx_ivar)) = (callback_ivar, context_ivar) else {
        return;
    };

    // SAFETY: ivar was written in new_message_handler and is never mutated.
    let cb_ptr: *const c_void = unsafe { *cb_ivar.load_ptr::<*const c_void>(this) };
    // SAFETY: ivar was written in new_message_handler and is never mutated.
    let ctx: *mut c_void = unsafe { *ctx_ivar.load_ptr::<*mut c_void>(this) };

    if cb_ptr.is_null() {
        return;
    }

    // SAFETY: cb_ptr was set from a valid MessageCallback function pointer.
    let callback: MessageCallback = unsafe { std::mem::transmute(cb_ptr) };
    // SAFETY: callback and context are valid per new_message_handler contract.
    unsafe { callback(ctx, utf8, len) };
}

/// Allocate a BeamerMessageHandler instance.
///
/// # Safety
///
/// Must be called from the main thread. `callback` and `context` must remain
/// valid until the handler is removed from the content controller.
pub unsafe fn new_message_handler(
    callback: MessageCallback,
    context: *mut c_void,
    _mtm: MainThreadMarker,
) -> Retained<AnyObject> {
    let cls = message_handler_class();

    // SAFETY: standard ObjC alloc pattern on a class we just built.
    let obj: *mut AnyObject = unsafe { msg_send![cls, alloc] };
    // SAFETY: init on a freshly allocated object.
    let obj: *mut AnyObject = unsafe { msg_send![obj, init] };
    assert!(!obj.is_null(), "alloc+init returned nil");

    // Store callback and context in ivars before creating Retained wrapper.
    let cb_ivar = cls
        .instance_variable(MSG_CALLBACK_IVAR)
        .expect("callback ivar must exist");
    let ctx_ivar = cls
        .instance_variable(MSG_CONTEXT_IVAR)
        .expect("context ivar must exist");

    // SAFETY: obj is a freshly init'd instance; no Retained/shared ref exists yet.
    unsafe {
        let ptr: *mut *const c_void = cb_ivar.load_ptr(&*obj);
        *ptr = callback as *const c_void;
        let ptr: *mut *mut c_void = ctx_ivar.load_ptr(&*obj);
        *ptr = context;
    }

    // SAFETY: alloc+init returned a +1 retained, non-null object.
    unsafe { Retained::from_raw(obj) }.unwrap()
}

// ---------------------------------------------------------------------------
// BeamerNavigationDelegate (WKNavigationDelegate)
// ---------------------------------------------------------------------------

/// Get or register the BeamerNavigationDelegate ObjC class of this binary.
fn navigation_delegate_class() -> &'static AnyClass {
    let c_name = class_name("BeamerNavigationDelegate", did_finish_navigation as *const () as usize);
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

    builder.add_ivar::<*const c_void>(NAV_LOADED_IVAR);
    builder.add_ivar::<*mut c_void>(NAV_CONTEXT_IVAR);
    // System uptime in seconds when the web content process last ended,
    // 0 before it ever did.
    builder.add_ivar::<f64>(NAV_TERMINATED_IVAR);

    // SAFETY: method signatures match the WKNavigationDelegate protocol.
    unsafe {
        builder.add_method(
            sel!(webView:didFinishNavigation:),
            did_finish_navigation
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyObject, *const AnyObject),
        );
        builder.add_method(
            sel!(webViewWebContentProcessDidTerminate:),
            web_content_process_did_terminate
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
        );
    }

    builder.register()
}

/// `webView:didFinishNavigation:` implementation.
unsafe extern "C-unwind" fn did_finish_navigation(
    this: *mut AnyObject,
    _cmd: Sel,
    _webview: *const AnyObject,
    _navigation: *const AnyObject,
) {
    // SAFETY: WebKit provides a valid receiver pointer.
    let this: &AnyObject = unsafe { &*this };

    let loaded_ivar = this.class().instance_variable(NAV_LOADED_IVAR);
    let context_ivar = this.class().instance_variable(NAV_CONTEXT_IVAR);

    let (Some(ld_ivar), Some(ctx_ivar)) = (loaded_ivar, context_ivar) else {
        return;
    };

    // SAFETY: ivar was written in new_navigation_delegate and is never mutated.
    let ld_ptr: *const c_void = unsafe { *ld_ivar.load_ptr::<*const c_void>(this) };
    // SAFETY: ivar was written in new_navigation_delegate and is never mutated.
    let ctx: *mut c_void = unsafe { *ctx_ivar.load_ptr::<*mut c_void>(this) };

    if ld_ptr.is_null() {
        return;
    }

    // SAFETY: ld_ptr was set from a valid LoadedCallback function pointer.
    let callback: LoadedCallback = unsafe { std::mem::transmute(ld_ptr) };
    // SAFETY: callback and context are valid per new_navigation_delegate contract.
    unsafe { callback(ctx) };
}

/// `webViewWebContentProcessDidTerminate:` implementation.
///
/// WebKit runs the page in a process of its own. When that process ends (the
/// system reclaims its memory, or WebKit crashes), the view stays blank. This
/// loads the page again, which reaches `did_finish_navigation` like any other
/// load, unless the process ended shortly before as well.
unsafe extern "C-unwind" fn web_content_process_did_terminate(
    this: *mut AnyObject,
    _cmd: Sel,
    webview: *mut AnyObject,
) {
    // SAFETY: WebKit provides a valid receiver pointer.
    let this: &AnyObject = unsafe { &*this };

    let Some(terminated_ivar) = this.class().instance_variable(NAV_TERMINATED_IVAR) else {
        return;
    };

    let now = NSProcessInfo::processInfo().systemUptime();

    // SAFETY: the ivar is an f64, read and written only here, on the main
    // thread where WebKit calls the delegate.
    let previous = unsafe {
        let ptr: *mut f64 = terminated_ivar.load_ptr(this);
        std::mem::replace(&mut *ptr, now)
    };

    if previous > 0.0 && now - previous < RELOAD_GUARD_SECONDS {
        log::error!(
            "The web content process of the editor ended again within \
             {RELOAD_GUARD_SECONDS} s; the editor stays blank until it is opened again"
        );
        return;
    }

    log::warn!("The web content process of the editor ended; loading the page again");
    // SAFETY: WebKit provides a valid WKWebView; we are on the main thread.
    let mut navigation: *mut AnyObject = unsafe { msg_send![webview, reload] };
    if navigation.is_null() {
        // The process ended before the first page committed, so the web view
        // has nothing to reload: load the request of that page again.
        // SAFETY: the key holds an NSURLRequest or nothing, set only in
        // `new_navigation_delegate`. It lives as long as the delegate.
        let request = unsafe {
            objc_getAssociatedObject(this, &NAV_REQUEST_KEY as *const u8 as *const c_void)
        };
        if !request.is_null() {
            // SAFETY: webview is a valid WKWebView and request a valid
            // NSURLRequest; we are on the main thread.
            navigation = unsafe { msg_send![webview, loadRequest: request] };
        }
    }
    if navigation.is_null() {
        log::error!(
            "The page of the editor could not be loaded again; \
             the editor stays blank until it is opened again"
        );
    }
}

/// Allocate a BeamerNavigationDelegate instance.
///
/// # Safety
///
/// Must be called from the main thread. `loaded` and `context` must remain
/// valid until the delegate is removed from the web view.
pub unsafe fn new_navigation_delegate(
    loaded: Option<LoadedCallback>,
    request: Option<&NSURLRequest>,
    context: *mut c_void,
    _mtm: MainThreadMarker,
) -> Retained<AnyObject> {
    let cls = navigation_delegate_class();

    // SAFETY: standard ObjC alloc pattern on a class we just built.
    let obj: *mut AnyObject = unsafe { msg_send![cls, alloc] };
    // SAFETY: init on a freshly allocated object.
    let obj: *mut AnyObject = unsafe { msg_send![obj, init] };
    assert!(!obj.is_null(), "alloc+init returned nil");

    let ld_ivar = cls
        .instance_variable(NAV_LOADED_IVAR)
        .expect("loaded ivar must exist");
    let ctx_ivar = cls
        .instance_variable(NAV_CONTEXT_IVAR)
        .expect("context ivar must exist");

    // SAFETY: obj is a freshly init'd instance; no Retained/shared ref exists yet.
    unsafe {
        let ptr: *mut *const c_void = ld_ivar.load_ptr(&*obj);
        *ptr = loaded.map_or(std::ptr::null(), |loaded| loaded as *const c_void);
        let ptr: *mut *mut c_void = ctx_ivar.load_ptr(&*obj);
        *ptr = context;
    }

    // The delegate retains the request that loads the page, for a load
    // after an ended web content process.
    if let Some(request) = request {
        // SAFETY: obj is a valid object; the key is a static address.
        unsafe {
            objc_setAssociatedObject(
                obj,
                &NAV_REQUEST_KEY as *const u8 as *const c_void,
                request as *const NSURLRequest as *mut AnyObject,
                OBJC_ASSOCIATION_RETAIN_NONATOMIC,
            );
        }
    }

    // SAFETY: alloc+init returned a +1 retained, non-null object.
    unsafe { Retained::from_raw(obj) }.unwrap()
}
