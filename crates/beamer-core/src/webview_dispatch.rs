//! How the format wrappers pass messages from the web UI to the plugin.
//!
//! A plugin answers `__BEAMER__.invoke()` calls and custom events from
//! JavaScript in its [`Descriptor`]: [`Descriptor::on_invoke()`] and
//! [`Descriptor::on_event()`]. The wrappers reach those methods through
//! [`WebViewDispatch`], which every descriptor implements. Parameter
//! synchronization is automatic and does not go through here.

use crate::plugin::Descriptor;

/// The web UI's way into a plugin, for the format wrappers.
///
/// Implemented for every [`Descriptor`], so a wrapper turns its
/// `Arc<P>` into an `Arc<dyn WebViewDispatch>` without a wrapper type. The
/// methods have their own names, so that calls on a descriptor stay
/// unambiguous.
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

impl<P: Descriptor> WebViewDispatch for P {
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
