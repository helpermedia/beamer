//! Custom URL scheme handler for serving embedded assets.
//!
//! Implements `WKURLSchemeHandler` to intercept `beamer://` requests and serve
//! files from the global asset table.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, AnyThread, MainThreadMarker, MainThreadOnly};
use objc2_foundation::{
    NSData, NSDictionary, NSHTTPURLResponse, NSInteger, NSObject, NSObjectProtocol, NSString, NSURL,
};
use objc2_web_kit::{WKURLSchemeHandler, WKURLSchemeTask, WKWebView};

use crate::assets::get_asset;
use crate::mime::mime_for_path;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "BeamerSchemeHandler"]
    pub struct BeamerSchemeHandler;

    unsafe impl NSObjectProtocol for BeamerSchemeHandler {}

    unsafe impl WKURLSchemeHandler for BeamerSchemeHandler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start_scheme_task(
            &self,
            _webview: &WKWebView,
            task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
            // SAFETY: task is a valid WKURLSchemeTask provided by WebKit.
            let request = unsafe { task.request() };
            let Some(url) = request.URL() else {
                return;
            };
            let url_string = url.absoluteString().unwrap().to_string();

            // Strip "beamer://localhost/" prefix to get relative path
            let path = url_string
                .strip_prefix("beamer://localhost/")
                .filter(|p| !p.is_empty())
                .unwrap_or("index.html");

            let (data, mime) = match get_asset(path) {
                Some(data) => (data, mime_for_path(path)),
                None => {
                    log::warn!("asset not found: {path}");
                    respond(task, &url_string, 404, "text/plain", b"Not Found");
                    return;
                }
            };

            respond(task, &url_string, 200, mime, data);
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop_scheme_task(
            &self,
            _webview: &WKWebView,
            _task: &ProtocolObject<dyn WKURLSchemeTask>,
        ) {
            // No-op for synchronous responses
        }
    }
);

impl BeamerSchemeHandler {
    /// Create a new scheme handler instance.
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let obj = mtm.alloc::<Self>();
        // SAFETY: NSObject's init is safe to call on a freshly allocated object.
        unsafe { objc2::msg_send![obj, init] }
    }
}

/// Send a response to a scheme task.
fn respond(
    task: &ProtocolObject<dyn WKURLSchemeTask>,
    url_string: &str,
    status: i32,
    mime: &str,
    body: &[u8],
) {
    let ns_url = NSURL::URLWithString(&NSString::from_str(url_string)).unwrap();

    let content_type_key = NSString::from_str("Content-Type");
    let content_type_val = NSString::from_str(mime);
    let headers: Retained<NSDictionary<NSString, NSString>> =
        NSDictionary::from_slices(&[&*content_type_key], &[&*content_type_val]);

    let response = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
        NSHTTPURLResponse::alloc(),
        &ns_url,
        status as NSInteger,
        None,
        Some(&headers),
    )
    .unwrap();

    let ns_data = NSData::with_bytes(body);

    // SAFETY: response and data are valid; task has not been finished yet.
    unsafe {
        task.didReceiveResponse(&response);
        task.didReceiveData(&ns_data);
        task.didFinish();
    }
}
