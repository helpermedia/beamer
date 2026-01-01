//! Error types for the BEAMR framework.

use std::fmt;

/// Errors that can occur in BEAMR plugins.
#[derive(Debug)]
pub enum PluginError {
    /// Plugin initialization failed.
    InitializationFailed(String),
    /// Audio processing error.
    ProcessingError(String),
    /// State serialization/deserialization error.
    StateError(String),
    /// Editor/GUI error.
    EditorError(String),
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
            Self::EditorError(msg) => write!(f, "Editor error: {}", msg),
            Self::PlatformError(msg) => write!(f, "Platform error: {}", msg),
            Self::WebViewError(msg) => write!(f, "WebView error: {}", msg),
        }
    }
}

impl std::error::Error for PluginError {}

/// Result type for BEAMR operations.
pub type PluginResult<T> = Result<T, PluginError>;
