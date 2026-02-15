//! WebView GUI support for Beamer audio plugins.
//!
//! Platform-native WebView embedding. This crate provides the shared platform
//! layer used by both VST3 and AU format wrappers.

mod error;
pub mod platform;

pub use error::{Result, WebViewError};

/// Configuration for a WebView editor.
pub struct WebViewConfig {
    /// HTML content to load.
    pub html: &'static str,
    /// Whether to enable developer tools.
    pub dev_tools: bool,
}
