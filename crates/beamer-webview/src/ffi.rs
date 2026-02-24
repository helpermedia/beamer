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
    use crate::WebViewConfig;

    /// Read 4 bytes from a pointer, or return `[0; 4]` if null.
    unsafe fn read_rgba(ptr: *const u8) -> [u8; 4] {
        if ptr.is_null() {
            [0; 4]
        } else {
            // SAFETY: caller guarantees ptr points to at least 4 readable bytes.
            unsafe { [*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)] }
        }
    }

    /// Create a WebView serving embedded assets via custom scheme.
    ///
    /// Each plugin must pass its 4-byte plugin code so that the scheme handler
    /// gets a unique ObjC class name (avoiding collisions when multiple Beamer
    /// plugins are loaded in the same host process).
    ///
    /// Returns an opaque handle, or null on failure.
    ///
    /// # Safety
    ///
    /// - `parent` must be a valid `NSView*` pointer
    /// - `assets` must be a valid `*const EmbeddedAssets` with `'static` lifetime
    /// - `plugin_code` must point to exactly 4 ASCII bytes
    /// - `background_color` must point to 4 bytes (RGBA) or be null
    /// - Must be called from the main thread
    #[no_mangle]
    pub extern "C" fn beamer_webview_create(
        parent: *mut c_void,
        assets: *const c_void,
        plugin_code: *const u8,
        dev_tools: bool,
        background_color: *const u8,
    ) -> *mut c_void {
        if parent.is_null() || assets.is_null() || plugin_code.is_null() {
            return ptr::null_mut();
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: caller guarantees plugin_code points to 4 bytes.
            let code = unsafe {
                [
                    *plugin_code,
                    *plugin_code.add(1),
                    *plugin_code.add(2),
                    *plugin_code.add(3),
                ]
            };

            // SAFETY: caller guarantees assets is a valid *const EmbeddedAssets
            // with 'static lifetime (it comes from Config.gui_assets).
            let assets_ref: &'static crate::assets::EmbeddedAssets =
                unsafe { &*(assets as *const crate::assets::EmbeddedAssets) };

            // SAFETY: read_rgba handles null safely.
            let bg = unsafe { read_rgba(background_color) };

            let config = WebViewConfig {
                plugin_code: code,
                assets: Some(assets_ref),
                url: None,
                dev_tools,
                background_color: bg,
                message_callback: None,
                loaded_callback: None,
                callback_context: std::ptr::null_mut(),
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
    /// - `plugin_code` must point to exactly 4 ASCII bytes
    /// - `background_color` must point to 4 bytes (RGBA) or be null
    /// - Must be called from the main thread
    #[no_mangle]
    pub extern "C" fn beamer_webview_create_url(
        parent: *mut c_void,
        url: *const c_char,
        plugin_code: *const u8,
        dev_tools: bool,
        background_color: *const u8,
    ) -> *mut c_void {
        if parent.is_null() || url.is_null() || plugin_code.is_null() {
            return ptr::null_mut();
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: caller guarantees url is a valid C string.
            let url_str = unsafe { CStr::from_ptr(url) }.to_str().ok()?;

            // SAFETY: caller guarantees plugin_code points to 4 bytes.
            let code = unsafe {
                [
                    *plugin_code,
                    *plugin_code.add(1),
                    *plugin_code.add(2),
                    *plugin_code.add(3),
                ]
            };

            // SAFETY: read_rgba handles null safely.
            let bg = unsafe { read_rgba(background_color) };

            let config = WebViewConfig {
                plugin_code: code,
                assets: None,
                url: Some(url_str),
                dev_tools,
                background_color: bg,
                message_callback: None,
                loaded_callback: None,
                callback_context: std::ptr::null_mut(),
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

    /// Evaluate JavaScript in the WebView.
    ///
    /// # Safety
    ///
    /// - `handle` must be a valid pointer from `beamer_webview_create`
    /// - `script` must point to `len` bytes of valid UTF-8
    /// - Must be called from the main thread
    #[no_mangle]
    pub extern "C" fn beamer_webview_eval_js(
        handle: *mut c_void,
        script: *const u8,
        len: usize,
    ) {
        if handle.is_null() || script.is_null() {
            log::warn!("beamer_webview_eval_js: null handle or script pointer");
            return;
        }

        let _ = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: caller guarantees handle is valid.
            let webview = unsafe { &*(handle as *const MacosWebView) };
            // SAFETY: caller guarantees script points to len bytes of valid UTF-8.
            let script_str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(script, len)) };
            webview.evaluate_js(script_str);
        }));
    }

    /// Create a WebView with IPC callbacks.
    ///
    /// Extended version of `beamer_webview_create` that accepts message and
    /// loaded callbacks for IPC support.
    ///
    /// # Safety
    ///
    /// Same requirements as `beamer_webview_create`, plus:
    /// - `message_callback` must be a valid function pointer or null
    /// - `loaded_callback` must be a valid function pointer or null
    /// - `callback_context` must remain valid until the WebView is destroyed
    #[no_mangle]
    pub extern "C" fn beamer_webview_create_with_ipc(
        parent: *mut c_void,
        assets: *const c_void,
        plugin_code: *const u8,
        dev_tools: bool,
        background_color: *const u8,
        message_callback: Option<crate::MessageCallback>,
        loaded_callback: Option<crate::LoadedCallback>,
        callback_context: *mut c_void,
    ) -> *mut c_void {
        if parent.is_null() || assets.is_null() || plugin_code.is_null() {
            return ptr::null_mut();
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: caller guarantees plugin_code points to 4 bytes.
            let code = unsafe {
                [
                    *plugin_code,
                    *plugin_code.add(1),
                    *plugin_code.add(2),
                    *plugin_code.add(3),
                ]
            };

            // SAFETY: caller guarantees assets is a valid *const EmbeddedAssets
            // with 'static lifetime.
            let assets_ref: &'static crate::assets::EmbeddedAssets =
                unsafe { &*(assets as *const crate::assets::EmbeddedAssets) };

            // SAFETY: read_rgba handles null safely.
            let bg = unsafe { read_rgba(background_color) };

            let config = WebViewConfig {
                plugin_code: code,
                assets: Some(assets_ref),
                url: None,
                dev_tools,
                background_color: bg,
                message_callback,
                loaded_callback,
                callback_context,
            };

            // SAFETY: caller guarantees parent is a valid NSView pointer on main thread.
            let webview = unsafe { MacosWebView::attach_to_parent(parent, &config) }.ok()?;

            let boxed = Box::new(webview);
            Some(Box::into_raw(boxed) as *mut c_void)
        }));

        result.unwrap_or(None).unwrap_or(ptr::null_mut())
    }

    /// Create a WebView with IPC callbacks in URL (dev server) mode.
    ///
    /// # Safety
    ///
    /// Same requirements as `beamer_webview_create_url`, plus IPC callback requirements.
    #[no_mangle]
    pub extern "C" fn beamer_webview_create_url_with_ipc(
        parent: *mut c_void,
        url: *const c_char,
        plugin_code: *const u8,
        dev_tools: bool,
        background_color: *const u8,
        message_callback: Option<crate::MessageCallback>,
        loaded_callback: Option<crate::LoadedCallback>,
        callback_context: *mut c_void,
    ) -> *mut c_void {
        if parent.is_null() || url.is_null() || plugin_code.is_null() {
            return ptr::null_mut();
        }

        let result = catch_unwind(AssertUnwindSafe(|| {
            // SAFETY: caller guarantees url is a valid C string.
            let url_str = unsafe { CStr::from_ptr(url) }.to_str().ok()?;

            // SAFETY: caller guarantees plugin_code points to 4 bytes.
            let code = unsafe {
                [
                    *plugin_code,
                    *plugin_code.add(1),
                    *plugin_code.add(2),
                    *plugin_code.add(3),
                ]
            };

            // SAFETY: read_rgba handles null safely.
            let bg = unsafe { read_rgba(background_color) };

            let config = WebViewConfig {
                plugin_code: code,
                assets: None,
                url: Some(url_str),
                dev_tools,
                background_color: bg,
                message_callback,
                loaded_callback,
                callback_context,
            };

            // SAFETY: caller guarantees parent is a valid NSView pointer on main thread.
            let webview = unsafe { MacosWebView::attach_to_parent(parent, &config) }.ok()?;

            let boxed = Box::new(webview);
            Some(Box::into_raw(boxed) as *mut c_void)
        }));

        result.unwrap_or(None).unwrap_or(ptr::null_mut())
    }
}
