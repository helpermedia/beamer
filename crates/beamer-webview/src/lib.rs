//! WebView GUI support for Beamer audio plugins.
//!
//! Platform-native WebView embedding. This crate provides the shared platform
//! layer used by both VST3 and AU format wrappers.

pub mod assets;
mod error;
mod ffi;
pub mod mime;
pub mod platform;

pub use assets::{EmbeddedAsset, EmbeddedAssets, register_assets};
pub use error::{Result, WebViewError};

/// Content source for a WebView.
pub enum WebViewSource<'a> {
    /// Serve embedded assets via custom URL scheme.
    Assets(&'a EmbeddedAssets),
    /// Navigate to a URL (dev server).
    Url(&'a str),
}

/// Configuration for a WebView GUI.
pub struct WebViewConfig<'a> {
    /// Content source.
    pub source: WebViewSource<'a>,
    /// Whether to enable developer tools.
    pub dev_tools: bool,
}
