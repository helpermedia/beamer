//! IPC support: WKScriptMessageHandler and WKNavigationDelegate.
//!
//! These ObjC classes use fixed names (`BeamerMessageHandler`,
//! `BeamerNavigationDelegate`) shared across all Beamer plugins in a process.
//! This is safe because the method implementations are identical for every
//! plugin - they simply forward to per-instance callback function pointers
//! stored in ivars. If two plugins built with different Beamer versions load
//! in the same process, the first-registered class wins, but since the
//! signatures and forwarding behavior are the same, this is benign.
//!
//! This differs from the scheme handler (`macos_scheme.rs`) which embeds
//! per-plugin-type assets in ivars and therefore needs unique class names.

use std::ffi::{c_void, CStr};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{msg_send, sel, ClassType, MainThreadMarker};
use objc2_foundation::NSObject;

use crate::{LoadedCallback, MessageCallback};

// ---------------------------------------------------------------------------
// Ivar names
// ---------------------------------------------------------------------------

const MSG_CALLBACK_IVAR: &CStr = c"_beamerMsgCallback";
const MSG_CONTEXT_IVAR: &CStr = c"_beamerMsgContext";
const NAV_LOADED_IVAR: &CStr = c"_beamerNavLoaded";
const NAV_CONTEXT_IVAR: &CStr = c"_beamerNavContext";

// ---------------------------------------------------------------------------
// BeamerMessageHandler (WKScriptMessageHandler)
// ---------------------------------------------------------------------------

/// Get or register the BeamerMessageHandler ObjC class.
fn message_handler_class() -> &'static AnyClass {
    let c_name = c"BeamerMessageHandler";

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

/// Get or register the BeamerNavigationDelegate ObjC class.
fn navigation_delegate_class() -> &'static AnyClass {
    let c_name = c"BeamerNavigationDelegate";

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

    // SAFETY: method signature matches the WKNavigationDelegate protocol.
    unsafe {
        builder.add_method(
            sel!(webView:didFinishNavigation:),
            did_finish_navigation
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyObject, *const AnyObject),
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

/// Allocate a BeamerNavigationDelegate instance.
///
/// # Safety
///
/// Must be called from the main thread.
pub unsafe fn new_navigation_delegate(
    loaded: LoadedCallback,
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
        *ptr = loaded as *const c_void;
        let ptr: *mut *mut c_void = ctx_ivar.load_ptr(&*obj);
        *ptr = context;
    }

    // SAFETY: alloc+init returned a +1 retained, non-null object.
    unsafe { Retained::from_raw(obj) }.unwrap()
}
