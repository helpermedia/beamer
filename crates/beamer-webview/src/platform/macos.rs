//! macOS WKWebView implementation.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::MainThreadMarker;
use objc2_app_kit::NSView;
use objc2_foundation::{NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKURLSchemeHandler, WKWebView, WKWebViewConfiguration};

use crate::error::{Result, WebViewError};
use crate::platform::macos_scheme::BeamerSchemeHandler;
use crate::{WebViewConfig, WebViewSource};

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
        config: &WebViewConfig<'_>,
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

        // Register custom scheme handler for embedded assets
        if matches!(config.source, WebViewSource::Assets(_)) {
            let handler = BeamerSchemeHandler::new(mtm);
            let handler_proto: &ProtocolObject<dyn WKURLSchemeHandler> =
                ProtocolObject::from_ref(&*handler);
            // SAFETY: handler and wk_config are valid; we are on the main thread.
            unsafe {
                wk_config.setURLSchemeHandler_forURLScheme(
                    Some(handler_proto),
                    &NSString::from_str("beamer"),
                );
            };
        }

        // SAFETY: frame and wk_config are valid; we are on the main thread.
        let webview = unsafe {
            WKWebView::initWithFrame_configuration(mtm.alloc(), frame, &wk_config)
        };

        if config.dev_tools {
            // SAFETY: setInspectable is safe to call on a valid WKWebView.
            unsafe { webview.setInspectable(true) };
        }

        match &config.source {
            WebViewSource::Assets(_) => {
                // Navigate to beamer://localhost/index.html
                let url_str = NSString::from_str("beamer://localhost/index.html");
                let nsurl = NSURL::URLWithString(&url_str).ok_or_else(|| {
                    WebViewError::CreationFailed("failed to create scheme URL".into())
                })?;
                let request = NSURLRequest::requestWithURL(&nsurl);
                // SAFETY: webview and request are valid; we are on the main thread.
                unsafe { webview.loadRequest(&request) };
            }
            WebViewSource::Url(url) => {
                // Navigate to dev server URL
                let url_str = NSString::from_str(url);
                let nsurl = NSURL::URLWithString(&url_str).ok_or_else(|| {
                    WebViewError::CreationFailed(format!("invalid dev server URL: {url}"))
                })?;
                let request = NSURLRequest::requestWithURL(&nsurl);
                // SAFETY: webview and request are valid; we are on the main thread.
                unsafe { webview.loadRequest(&request) };
            }
        }

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
