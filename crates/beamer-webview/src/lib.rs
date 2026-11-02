//! WebView editor support for Beamer audio plugins.
//!
//! This crate holds both ends of the conversation between a plugin and its
//! web UI, so that every format wrapper speaks it the same way:
//!
//! - the JavaScript runtime that a page gets as `window.__BEAMER__`
//!   (`beamer_runtime.js`)
//! - what a model offers its web UI: the [`WebViewEventQueue`] with its
//!   [`WebViewValueSender`] and [`WebViewStreamSender`] for the audio
//!   thread, plus the [`WebViewHandler`] for calls and events from the page
//! - the [`EditorSync`] of each open editor, which keeps the page in step
//!   with the plugin
//! - the platform-native WebView that a format wrapper embeds in the
//!   host's view ([`platform`])

use std::ffi::c_void;

pub mod assets;
pub mod editor_sync;
mod error;
pub mod event_queue;
mod ffi;
pub mod mime;
pub mod model;
pub mod parameter_dump;
pub mod platform;
pub mod stream_sender;
pub mod value_sender;

pub use assets::{EmbeddedAsset, EmbeddedAssets};
pub use editor_sync::EditorSync;
pub use error::{Result, WebViewError};
pub use event_queue::WebViewEventQueue;
#[doc(hidden)]
pub use model::__private;
pub use model::{WebViewDispatch, WebViewEvents, WebViewHandler};
pub use parameter_dump::params_to_init_json;
pub use stream_sender::WebViewStreamSender;
pub use value_sender::WebViewValueSender;

// Re-export serde_json so plugins can answer the web UI without adding
// serde_json to their own Cargo.toml.
pub use serde_json;

/// The JavaScript runtime that creates `window.__BEAMER__`.
///
/// Injected at document start before any page code runs. Uses var/function
/// (no ES6) for maximum WebView compatibility.
// Only the macOS WebView injects it so far.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) const BEAMER_RUNTIME_JS: &str = include_str!("beamer_runtime.js");

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

/// Configuration for a WebView editor.
pub struct WebViewConfig<'a> {
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
