//! C-ABI exports for the AU ObjC wrapper.
//!
//! These functions wrap `MacosWebView` so the Au Objective-C template can
//! create and manage a WKWebView without reimplementing the setup logic.
//! The same Rust platform code is shared by both Vst3 (via direct Rust calls)
//! and Au (via these C-ABI functions).

#[cfg(target_os = "macos")]
mod macos_ffi {
    use std::ffi::{c_char, c_void, CStr};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::ptr;

    use crate::platform::macos::MacosWebView;
    use crate::{WebViewConfig, WebViewSource};

    /// Create a WebView serving embedded assets via custom scheme.
    ///
    /// Assets must be registered via `register_assets()` before calling this.
    /// Returns an opaque handle, or null on failure.
    ///
    /// # Safety
    ///
    /// - `parent` must be a valid `NSView*` pointer
    /// - Must be called from the main thread
    #[no_mangle]
    pub extern "C" fn beamer_webview_create(
        parent: *mut c_void,
        dev_tools: bool,
    ) -> *mut c_void {
        if parent.is_null() {
            return ptr::null_mut();
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            // The scheme handler reads from GLOBAL_ASSETS (set via register_assets()),
            // not from config.source. The Assets variant here only selects the
            // beamer:// navigation path in attach_to_parent.
            let empty = beamer_core::EmbeddedAssets::new(&[]);
            let config = WebViewConfig {
                source: WebViewSource::Assets(&empty),
                dev_tools,
            };

            // SAFETY: caller guarantees parent is a valid NSView pointer on main thread.
            let webview = unsafe { MacosWebView::attach_to_parent(parent, &config) }.ok()?;

            let boxed = Box::new(webview);
            Some(Box::into_raw(boxed) as *mut c_void)
        }));

        result.unwrap_or(None).unwrap_or(ptr::null_mut())
    }

    /// Create a WebView that loads from a URL (dev server mode).
    ///
    /// Returns an opaque handle, or null on failure.
    ///
    /// # Safety
    ///
    /// - `parent` must be a valid `NSView*` pointer
    /// - `url` must be a valid null-terminated UTF-8 C string
    /// - Must be called from the main thread
    #[no_mangle]
    pub extern "C" fn beamer_webview_create_url(
        parent: *mut c_void,
        url: *const c_char,
        dev_tools: bool,
    ) -> *mut c_void {
        if parent.is_null() || url.is_null() {
            return ptr::null_mut();
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: caller guarantees url is a valid C string.
            let url_str = unsafe { CStr::from_ptr(url) }.to_str().ok()?;

            let config = WebViewConfig {
                source: WebViewSource::Url(url_str),
                dev_tools,
            };

            // SAFETY: caller guarantees parent is a valid NSView pointer on main thread.
            let webview = unsafe { MacosWebView::attach_to_parent(parent, &config) }.ok()?;

            let boxed = Box::new(webview);
            Some(Box::into_raw(boxed) as *mut c_void)
        }));

        result.unwrap_or(None).unwrap_or(ptr::null_mut())
    }

    /// Update the WebView frame.
    ///
    /// # Safety
    ///
    /// `handle` must be a valid pointer returned by `beamer_webview_create`.
    #[no_mangle]
    pub extern "C" fn beamer_webview_set_frame(
        handle: *mut c_void,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        if handle.is_null() {
            return;
        }

        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: caller guarantees handle is valid.
            let webview = unsafe { &*(handle as *const MacosWebView) };
            webview.set_frame(x, y, width, height);
        }));
    }

    /// Detach and destroy the WebView.
    ///
    /// # Safety
    ///
    /// `handle` must be a valid pointer returned by `beamer_webview_create`,
    /// and must not be used after this call.
    #[no_mangle]
    pub extern "C" fn beamer_webview_destroy(handle: *mut c_void) {
        if handle.is_null() {
            return;
        }

        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: caller guarantees handle is valid and won't be reused.
            let mut webview = unsafe { Box::from_raw(handle as *mut MacosWebView) };
            webview.detach();
            // Box drops here, releasing the WKWebView
        }));
    }
}
