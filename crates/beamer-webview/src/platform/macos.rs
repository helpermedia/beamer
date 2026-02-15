//! macOS WKWebView implementation.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::NSView;
use objc2_foundation::NSString;
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

use crate::error::{Result, WebViewError};
use crate::WebViewConfig;

/// macOS WebView backed by WKWebView.
pub struct MacosWebView {
    webview: Retained<WKWebView>,
}

impl MacosWebView {
    /// Attach a WKWebView to the given parent NSView.
    ///
    /// # Safety
    ///
    /// `parent` must be a valid `NSView` pointer provided by the VST3 host.
    /// Must be called from the main thread.
    pub unsafe fn attach_to_parent(
        parent: *mut c_void,
        config: &WebViewConfig,
    ) -> Result<Self> {
        if parent.is_null() {
            return Err(WebViewError::CreationFailed("null parent view".into()));
        }

        let mtm = MainThreadMarker::new().ok_or_else(|| {
            WebViewError::CreationFailed("must be called from the main thread".into())
        })?;

        // SAFETY: caller guarantees `parent` is a valid NSView pointer.
        let parent_view: &NSView = unsafe { &*(parent as *const NSView) };
        let frame = parent_view.frame();

        // SAFETY: WKWebViewConfiguration::new is safe when called on the main thread.
        let wk_config = unsafe { WKWebViewConfiguration::new(mtm) };

        // SAFETY: frame and wk_config are valid; we are on the main thread.
        let webview = unsafe {
            WKWebView::initWithFrame_configuration(mtm.alloc(), frame, &wk_config)
        };

        if config.dev_tools {
            // SAFETY: setInspectable is safe to call on a valid WKWebView.
            unsafe { webview.setInspectable(true) };
        }

        let html_string = NSString::from_str(config.html);
        // SAFETY: html_string is a valid NSString; base URL is None.
        unsafe { webview.loadHTMLString_baseURL(&html_string, None) };

        parent_view.addSubview(&webview);

        Ok(Self { webview })
    }

    /// Update the WebView frame.
    pub fn set_frame(&self, x: i32, y: i32, width: i32, height: i32) {
        let frame = objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(x as f64, y as f64),
            objc2_foundation::NSSize::new(width as f64, height as f64),
        );
        self.webview.setFrame(frame);
    }

    /// Remove the WebView from its parent.
    pub fn detach(&mut self) {
        self.webview.removeFromSuperview();
    }
}
