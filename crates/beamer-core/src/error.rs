//! Error types for the Beamer framework.

use std::fmt;

/// Errors that can occur in Beamer plugins.
#[derive(Debug)]
pub enum PluginError {
    /// Plugin initialization failed.
    InitializationFailed(String),
    /// Audio processing error.
    ProcessingError(String),
    /// State serialization/deserialization error.
    StateError(String),
    /// GUI error.
    GuiError(String),
    /// Platform-specific error.
    PlatformError(String),
    /// WebView creation or operation failed.
    WebViewError(String),
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InitializationFailed(msg) => write!(f, "Initialization failed: {}", msg),
            Self::ProcessingError(msg) => write!(f, "Processing error: {}", msg),
            Self::StateError(msg) => write!(f, "State error: {}", msg),
            Self::GuiError(msg) => write!(f, "GUI error: {}", msg),
            Self::PlatformError(msg) => write!(f, "Platform error: {}", msg),
            Self::WebViewError(msg) => write!(f, "WebView error: {}", msg),
        }
    }
}

impl std::error::Error for PluginError {}

/// Result type for Beamer operations.
pub type PluginResult<T> = Result<T, PluginError>;
