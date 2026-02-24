//! WebView GUI support for Beamer audio plugins.
//!
//! Platform-native WebView embedding. This crate provides the shared platform
//! layer used by both VST3 and AU format wrappers.

use std::ffi::c_void;

pub mod assets;
mod error;
mod ffi;
pub mod mime;
pub mod platform;

pub use assets::{EmbeddedAsset, EmbeddedAssets};
pub use error::{Result, WebViewError};

/// Callback fired when JavaScript sends a message to native code.
///
/// `json` is a pointer to a UTF-8 JSON string of `len` bytes (not null-terminated).
/// Called on the main thread.
pub type MessageCallback =
    unsafe extern "C-unwind" fn(context: *mut c_void, json: *const u8, len: usize);

/// Callback fired when the WebView finishes loading initial content.
///
/// Called on the main thread.
pub type LoadedCallback = unsafe extern "C-unwind" fn(context: *mut c_void);

/// Configuration for a WebView GUI.
pub struct WebViewConfig<'a> {
    /// 4-byte plugin subtype code used to generate a unique ObjC class name
    /// per plugin type so multiple plugins can coexist in the same host process.
    pub plugin_code: [u8; 4],
    /// Embedded web assets. When set, the WebView navigates to
    /// `beamer://localhost/index.html` and a per-instance scheme handler
    /// serves files from this table.
    pub assets: Option<&'static EmbeddedAssets>,
    /// Dev server URL. When set, the WebView navigates here instead of
    /// using the custom scheme handler. The lifetime allows FFI paths to
    /// pass a short-lived reference without claiming `'static`.
    pub url: Option<&'a str>,
    /// Whether to enable developer tools.
    pub dev_tools: bool,
    /// Background color (RGBA, 0-255) painted on the parent view's layer
    /// while web content loads. All-zero means no override.
    pub background_color: [u8; 4],
    /// Callback for messages from JavaScript. May be null.
    pub message_callback: Option<MessageCallback>,
    /// Callback when the page finishes loading. May be null.
    pub loaded_callback: Option<LoadedCallback>,
    /// Context pointer passed to callbacks.
    pub callback_context: *mut c_void,
}
