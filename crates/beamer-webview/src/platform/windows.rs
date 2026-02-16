//! Windows WebView2 implementation.

use crate::error::{Result, WebViewError};
use crate::WebViewConfig;

/// Windows WebView backed by WebView2.
pub struct WindowsWebView {
    _private: (),
}

impl WindowsWebView {
    /// Attach a WebView2 to the given parent HWND.
    ///
    /// # Safety
    ///
    /// `parent` must be a valid `HWND` provided by the VST3 host.
    pub unsafe fn attach_to_parent(
        _parent: *mut std::ffi::c_void,
        _config: &WebViewConfig<'_>,
    ) -> Result<Self> {
        Err(WebViewError::PlatformNotSupported)
    }

    /// Update the WebView bounds.
    pub fn set_bounds(&self, _x: i32, _y: i32, _width: i32, _height: i32) {}

    /// Remove the WebView from its parent.
    pub fn detach(&mut self) {}
}
