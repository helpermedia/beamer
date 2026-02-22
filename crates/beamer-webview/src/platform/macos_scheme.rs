//! Custom URL scheme handler for serving embedded assets.
//!
//! Uses `ClassBuilder` instead of `define_class!` so that the ObjC class name
//! can be generated at runtime (one per plugin code). This allows multiple
//! Beamer plugins in the same host process to each register their own scheme
//! handler without colliding on a shared class name.
//!
//! Each handler instance stores a pointer to its plugin's `EmbeddedAssets` as
//! an ivar, so asset lookup is per-instance rather than global.

use std::ffi::{c_void, CString};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, ClassBuilder, Sel};
use objc2::{msg_send, sel, AnyThread, ClassType, MainThreadMarker};
use objc2_foundation::{
    NSData, NSDictionary, NSHTTPURLResponse, NSInteger, NSObject, NSString, NSURL, NSURLRequest,
};

use crate::assets::EmbeddedAssets;
use crate::mime::mime_for_path;

/// Ivar name for the `*const EmbeddedAssets` pointer on each handler instance.
const ASSETS_IVAR: &std::ffi::CStr = c"_beamerAssets";

/// Get or register the per-plugin scheme handler ObjC class.
///
/// The class name is `BeamerSchemeHandler_{AABBCCDD}` where `AABBCCDD` is
/// the hex-encoded 4-byte plugin code. `ClassBuilder::new` returns `None`
/// if the class already exists (e.g. when reopening the same plugin), in
/// which case we look it up and return the existing class.
///
/// Must be called from the main thread (class registration is not thread-safe).
fn scheme_handler_class(plugin_code: [u8; 4]) -> &'static AnyClass {
    let class_name = format!(
        "BeamerSchemeHandler_{:02x}{:02x}{:02x}{:02x}",
        plugin_code[0], plugin_code[1], plugin_code[2], plugin_code[3],
    );
    // Hex encoding is always ASCII, so CString::new cannot fail.
    let c_name = CString::new(class_name.as_str()).expect("hex class name is always valid");

    // Fast path: class was already registered (same plugin reopened, or
    // another instance of the same plugin type).
    if let Some(existing) = AnyClass::get(c_name.as_c_str()) {
        return existing;
    }

    let superclass = NSObject::class();
    let mut builder = match ClassBuilder::new(c_name.as_c_str(), superclass) {
        Some(b) => b,
        // Another thread (or re-entrant call) registered the class between
        // our AnyClass::get check and this point. Look it up again.
        None => {
            return AnyClass::get(c_name.as_c_str())
                .expect("class must exist after ClassBuilder::new returned None");
        }
    };

    // Ivar: raw pointer to the plugin's embedded assets.
    builder.add_ivar::<*const c_void>(ASSETS_IVAR);

    // Declare WKURLSchemeHandler protocol conformance.
    let proto = AnyProtocol::get(c"WKURLSchemeHandler")
        .expect("WKURLSchemeHandler protocol must be available");
    builder.add_protocol(proto);

    // SAFETY: the method signatures match the WKURLSchemeHandler protocol.
    // Raw pointers are used for the receiver to satisfy HRTB requirements.
    unsafe {
        builder.add_method(
            sel!(webView:startURLSchemeTask:),
            start_url_scheme_task
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyObject, *const AnyObject),
        );
        builder.add_method(
            sel!(webView:stopURLSchemeTask:),
            stop_url_scheme_task
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyObject, *const AnyObject),
        );
    }

    builder.register()
}

/// Allocate a scheme handler instance with the given assets.
///
/// The returned object conforms to `WKURLSchemeHandler` and serves files
/// from `assets` when WebKit intercepts a `beamer://` request.
///
/// # Safety
///
/// Must be called from the main thread. `assets` must be `&'static`.
pub unsafe fn new_scheme_handler(
    assets: &'static EmbeddedAssets,
    plugin_code: [u8; 4],
    _mtm: MainThreadMarker,
) -> Retained<AnyObject> {
    let cls = scheme_handler_class(plugin_code);

    // SAFETY: standard ObjC alloc + init pattern on a class we just built.
    let obj: *mut AnyObject = unsafe { msg_send![cls, alloc] };
    // SAFETY: init on a freshly allocated object.
    let obj: *mut AnyObject = unsafe { msg_send![obj, init] };
    assert!(!obj.is_null(), "alloc+init returned nil");

    // Store the assets pointer through the raw pointer before creating the
    // Retained wrapper. This avoids aliasing: Retained would give us
    // &AnyObject (shared ref), but we need a *mut write to the ivar.
    let ivar = cls
        .instance_variable(ASSETS_IVAR)
        .expect("_beamerAssets ivar must exist");
    // SAFETY: obj is a freshly init'd instance of cls, which declares this
    // ivar. No Retained/shared reference exists yet, so the *mut write is sound.
    unsafe {
        let ptr: *mut *const c_void = ivar.load_ptr(&*obj);
        *ptr = assets as *const EmbeddedAssets as *const c_void;
    }

    // SAFETY: alloc+init returned a +1 retained, non-null object.
    unsafe { Retained::from_raw(obj) }.unwrap()
}

// ---------------------------------------------------------------------------
// ObjC method implementations
// ---------------------------------------------------------------------------

/// Read the `_beamerAssets` ivar from a handler instance.
///
/// # Safety
///
/// `this` must be a valid instance of a scheme handler class built by
/// `scheme_handler_class`.
unsafe fn load_assets(this: &AnyObject) -> Option<&'static EmbeddedAssets> {
    let ivar = this.class().instance_variable(ASSETS_IVAR)?;
    // SAFETY: the ivar was written in `new_scheme_handler` and is never mutated.
    let raw: *const c_void = unsafe { *ivar.load_ptr::<*const c_void>(this) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: raw was set from a valid &'static EmbeddedAssets in new_scheme_handler.
    Some(unsafe { &*(raw as *const EmbeddedAssets) })
}

/// `webView:startURLSchemeTask:` implementation.
unsafe extern "C-unwind" fn start_url_scheme_task(
    this: *mut AnyObject,
    _cmd: Sel,
    _webview: *const AnyObject,
    task: *const AnyObject,
) {
    // SAFETY: WebKit provides a valid receiver pointer.
    let this: &AnyObject = unsafe { &*this };
    // SAFETY: WebKit provides a valid task pointer.
    let task: &AnyObject = unsafe { &*task };

    // SAFETY: this is a valid scheme handler instance with an assets ivar.
    let Some(assets) = (unsafe { load_assets(this) }) else {
        return;
    };

    // SAFETY: task conforms to WKURLSchemeTask; request returns a valid object.
    let request: *const NSURLRequest = unsafe { msg_send![task, request] };
    // SAFETY: request is a valid NSURLRequest.
    let url_opt: Option<Retained<NSURL>> = unsafe { msg_send![request, URL] };
    let Some(url) = url_opt else { return };

    // Use NSURL::path() which returns the decoded path component, stripping
    // query strings, fragments and percent-encoding.
    let Some(ns_path) = url.path() else { return };
    let full_path = ns_path.to_string();
    let path = full_path.strip_prefix('/').unwrap_or(&full_path);
    let path = if path.is_empty() { "index.html" } else { path };

    // Keep absoluteString for the HTTP response URL.
    let url_string = url.absoluteString().map(|s| s.to_string());
    let response_url = url_string.as_deref().unwrap_or("beamer://localhost/");

    let (data, mime) = match assets.get(path) {
        Some(d) => (d, mime_for_path(path)),
        None => {
            log::warn!("asset not found: {path}");
            respond(task, response_url, 404, "text/plain", b"Not Found");
            return;
        }
    };

    respond(task, response_url, 200, mime, data);
}

/// `webView:stopURLSchemeTask:` implementation.
///
/// No-op: our `start` handler is fully synchronous and never yields the run
/// loop, so `stop` can only be called after `didFinish` has already been sent.
unsafe extern "C-unwind" fn stop_url_scheme_task(
    _this: *mut AnyObject,
    _cmd: Sel,
    _webview: *const AnyObject,
    _task: *const AnyObject,
) {
}

/// Send an HTTP response back to the scheme task.
fn respond(task: &AnyObject, url_string: &str, status: i32, mime: &str, body: &[u8]) {
    let Some(ns_url) = NSURL::URLWithString(&NSString::from_str(url_string)) else {
        log::error!("failed to construct response URL: {url_string}");
        return;
    };

    let key = NSString::from_str("Content-Type");
    let val = NSString::from_str(mime);
    let headers: Retained<NSDictionary<NSString, NSString>> =
        NSDictionary::from_slices(&[&*key], &[&*val]);

    let Some(response) = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
        NSHTTPURLResponse::alloc(),
        &ns_url,
        status as NSInteger,
        None,
        Some(&headers),
    ) else {
        log::error!("failed to construct HTTP response for: {url_string}");
        return;
    };

    let ns_data = NSData::with_bytes(body);

    // SAFETY: response and data are valid; task has not been stopped.
    unsafe {
        let _: () = msg_send![task, didReceiveResponse: &*response];
        let _: () = msg_send![task, didReceiveData: &*ns_data];
        let _: () = msg_send![task, didFinish];
    }
}
