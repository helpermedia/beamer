//! Custom WebView message handler trait.
//!
//! Implement [`WebViewHandler`] to handle `invoke()` calls and custom events
//! from JavaScript. Parameter synchronization is automatic and does not
//! require this trait.

/// Handler for custom WebView messages.
///
/// Implement this to handle `invoke()` calls and custom events from
/// JavaScript. Parameter sync is handled automatically and does not
/// require this trait.
pub trait WebViewHandler: Send + Sync {
    /// Handle an invoke call from JavaScript.
    ///
    /// Called on the main thread when JS calls
    /// `__BEAMER__.invoke("method", args...)`.
    /// Return `Ok(value)` to resolve the JS Promise.
    /// Return `Err(message)` to reject the JS Promise.
    fn on_invoke(
        &self,
        _method: &str,
        _args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::Value::Null)
    }

    /// Handle a custom event from JavaScript.
    ///
    /// Called on the main thread when JS calls
    /// `__BEAMER__.emit("name", data)`.
    fn on_event(&self, _name: &str, _data: &serde_json::Value) {}
}
