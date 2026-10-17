//! How the format wrappers pass messages from the web UI to the plugin.
//!
//! A plugin answers `__BEAMER__.invoke()` calls and custom events from
//! JavaScript in its [`Model`]: [`Model::on_invoke()`] and
//! [`Model::on_event()`]. The wrappers reach those methods through
//! [`WebViewDispatch`], which every model implements. Parameter
//! synchronization is automatic and does not go through here.

use crate::plugin::Model;

/// The web UI's way into a plugin, for the format wrappers.
///
/// Implemented for every [`Model`], so a wrapper turns its `Arc<P>` into an
/// `Arc<dyn WebViewDispatch>` without a wrapper type. The methods have their
/// own names, so that calls on a model stay unambiguous.
#[doc(hidden)]
pub trait WebViewDispatch: Send + Sync {
    /// Pass an invoke call from JavaScript to the plugin.
    fn dispatch_invoke(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String>;

    /// Pass a custom event from JavaScript to the plugin.
    fn dispatch_event(&self, name: &str, data: &serde_json::Value);
}

impl<P: Model> WebViewDispatch for P {
    fn dispatch_invoke(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        self.on_invoke(method, args)
    }

    fn dispatch_event(&self, name: &str, data: &serde_json::Value) {
        self.on_event(name, data);
    }
}
