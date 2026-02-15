//! Error types for WebView operations.

/// Errors that can occur during WebView operations.
#[derive(Debug)]
pub enum WebViewError {
    /// The current platform is not supported.
    PlatformNotSupported,
    /// WebView creation failed.
    CreationFailed(String),
    /// A WebView is already attached.
    AlreadyAttached,
    /// No WebView is currently attached.
    NotAttached,
}

impl std::fmt::Display for WebViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PlatformNotSupported => write!(f, "platform not supported"),
            Self::CreationFailed(msg) => write!(f, "webview creation failed: {msg}"),
            Self::AlreadyAttached => write!(f, "webview already attached"),
            Self::NotAttached => write!(f, "no webview attached"),
        }
    }
}

impl std::error::Error for WebViewError {}

/// Result type for WebView operations.
pub type Result<T> = std::result::Result<T, WebViewError>;
