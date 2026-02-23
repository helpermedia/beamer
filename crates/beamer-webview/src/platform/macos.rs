//! macOS WKWebView implementation.

use std::ffi::c_void;

use objc2::encode::{Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::MainThreadMarker;
use objc2_app_kit::NSView;

/// Opaque CGColor type with correct encoding for objc2 msg_send.
#[repr(C)]
struct CGColor([u8; 0]);

// SAFETY: CGColorRef has ObjC type encoding ^{CGColor=}.
unsafe impl RefEncode for CGColor {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[]));
}
use objc2_foundation::{NSNumber, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{WKURLSchemeHandler, WKWebView, WKWebViewConfiguration};

use crate::error::{Result, WebViewError};
use crate::platform::macos_scheme::new_scheme_handler;
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

        // Register custom scheme handler for embedded assets.
        if let Some(assets) = config.assets {
            // SAFETY: assets is &'static; new_scheme_handler stores the pointer.
            let handler =
                unsafe { new_scheme_handler(assets, config.plugin_code, mtm) };
            // SAFETY: handler conforms to WKURLSchemeHandler (protocol declared
            // by ClassBuilder, both required methods implemented). The pointer
            // cast is sound because AnyObject has the same layout as
            // ProtocolObject<dyn WKURLSchemeHandler>.
            let handler_proto: &ProtocolObject<dyn WKURLSchemeHandler> = unsafe {
                &*(&*handler as *const _ as *const ProtocolObject<dyn WKURLSchemeHandler>)
            };
            // SAFETY: handler_proto and wk_config are valid; we are on the main thread.
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

        // If a background color is configured, paint it on the parent view's
        // layer so the host's default white doesn't flash while the WKWebView
        // loads content.
        let [r, g, b, a] = config.background_color;
        if r != 0 || g != 0 || b != 0 || a != 0 {
            extern "C" {
                fn CGColorCreateSRGB(
                    red: f64, green: f64, blue: f64, alpha: f64,
                ) -> *const CGColor;
                fn CGColorRelease(color: *const CGColor);
            }
            // SAFETY: parent_view is valid; we are on the main thread.
            // CGColorCreateSRGB returns a +1 retained CGColor (macOS 13+).
            unsafe {
                let _: () = objc2::msg_send![parent_view, setWantsLayer: true];
                let layer: *mut objc2::runtime::AnyObject =
                    objc2::msg_send![parent_view, layer];
                if !layer.is_null() {
                    let cg_color = CGColorCreateSRGB(
                        r as f64 / 255.0,
                        g as f64 / 255.0,
                        b as f64 / 255.0,
                        a as f64 / 255.0,
                    );
                    let _: () = objc2::msg_send![layer, setBackgroundColor: cg_color];
                    CGColorRelease(cg_color);
                }
            }
        }

        // Disable the default white background so the WKWebView is
        // transparent and the parent's color (if set) shows through.
        let key = NSString::from_str("drawsBackground");
        let value = NSNumber::new_bool(false);
        // SAFETY: WKWebView supports KVC for drawsBackground; we are on the main thread.
        let _: () = unsafe { objc2::msg_send![&webview, setValue: &*value, forKey: &*key] };

        if let Some(url) = config.url {
            // Dev server mode: navigate directly to the URL.
            let url_str = NSString::from_str(url);
            let nsurl = NSURL::URLWithString(&url_str).ok_or_else(|| {
                WebViewError::CreationFailed(format!("invalid dev server URL: {url}"))
            })?;
            let request = NSURLRequest::requestWithURL(&nsurl);
            // SAFETY: webview and request are valid; we are on the main thread.
            unsafe { webview.loadRequest(&request) };
        } else if config.assets.is_some() {
            // Production mode: navigate to beamer://localhost/index.html.
            let url_str = NSString::from_str("beamer://localhost/index.html");
            let nsurl = NSURL::URLWithString(&url_str).ok_or_else(|| {
                WebViewError::CreationFailed("failed to create scheme URL".into())
            })?;
            let request = NSURLRequest::requestWithURL(&nsurl);
            // SAFETY: webview and request are valid; we are on the main thread.
            unsafe { webview.loadRequest(&request) };
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
