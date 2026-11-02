//! What the web UI reaches in a model.
//!
//! - [`WebViewEvents`] tells where the model's queue of events for the web
//!   UI is.
//! - [`WebViewHandler`] answers `__BEAMER__.invoke()` calls and custom
//!   events from JavaScript. A plugin implements it when its web UI uses
//!   them.
//! - [`WebViewDispatch`] is how the format wrappers reach the handler.
//!
//! `#[beamer::model]` generates `WebViewEvents` and `WebViewDispatch`,
//! `#[beamer::group]` generates `WebViewEvents`. The generated dispatch
//! calls the model's `WebViewHandler` when it has one. Without one it
//! rejects every call as an unknown method and ignores the events.
//! Parameter synchronization is automatic and does not go through here.

use beamer_core::NoParameters;
use serde_json::Value;

use crate::event_queue::WebViewEventQueue;

/// Where the queue of events for the web UI is: a [`WebViewEventQueue`]
/// field of this struct, or else the first one of a nested group in field
/// order.
///
/// `#[beamer::model]` and `#[beamer::group]` generate this for a model and
/// each of its nested groups. A hand-written model or group adds an empty
/// `impl WebViewEvents for MyModel {}`, or returns its queue.
pub trait WebViewEvents {
    /// The plugin's queue of events for the web UI.
    ///
    /// The format wrappers deliver what is emitted into it while an editor
    /// is open. `#[beamer::model]` and `#[beamer::group]` find the field by its type; the
    /// default is for structs without one.
    fn webview_events(&self) -> Option<&WebViewEventQueue> {
        None
    }
}

/// How a model answers its web UI: `__BEAMER__.invoke()` calls and custom
/// events from JavaScript.
///
/// A plugin whose web UI calls `invoke()` or emits events implements it for
/// its model:
///
/// ```ignore
/// impl WebViewHandler for SamplerModel {
///     fn on_invoke(&self, method: &str, args: &[serde_json::Value]) -> Result<serde_json::Value, String> {
///         match method {
///             "getSampleName" => Ok(serde_json::json!(*self.sample_name.get())),
///             _ => Err(format!("unknown method: {method}")),
///         }
///     }
/// }
/// ```
///
/// Both methods are called on the main thread, never on the audio thread.
///
/// The dispatch that `#[beamer::model]` generates finds this impl when it
/// is compiled. For a generic model that is an impl for every type the
/// struct takes, `impl<T: Send + Sync + 'static> WebViewHandler for
/// MyModel<T>` for a struct `MyModel<T: Send + Sync + 'static>`. An impl for
/// one type only (`MyModel<f32>`) or with bounds the struct does not have
/// compiles but is never called.
pub trait WebViewHandler {
    /// Answer an invoke call from JavaScript.
    ///
    /// Called when the web UI calls `__BEAMER__.invoke("method", args...)`.
    /// Return `Ok(value)` to resolve the JavaScript promise with `value`,
    /// `Err(message)` to reject it.
    ///
    /// The default rejects every call as an unknown method. An override
    /// ends its `match` the same way for the methods it does not know.
    fn on_invoke(&self, method: &str, args: &[Value]) -> Result<Value, String> {
        let _ = args;
        Err(unknown_method(method))
    }

    /// Handle a custom event from JavaScript.
    ///
    /// Called when the web UI calls `__BEAMER__.emit("name", data)`. The
    /// default does nothing.
    fn on_event(&self, name: &str, data: &Value) {
        let _ = (name, data);
    }
}

/// The web UI's way into a plugin, for the format wrappers: its
/// [`WebViewHandler`], if it has one.
///
/// `#[beamer::model]` generates it. A hand-written model adds an empty
/// `impl WebViewDispatch for MyModel {}`, which rejects every call and
/// ignores the events, or forwards to its handler. The methods have their
/// own names, so that calls on a model stay unambiguous.
pub trait WebViewDispatch: Send + Sync {
    /// Pass an invoke call from JavaScript to the plugin.
    fn dispatch_invoke(&self, method: &str, args: &[Value]) -> Result<Value, String> {
        let _ = args;
        Err(unknown_method(method))
    }

    /// Pass a custom event from JavaScript to the plugin.
    fn dispatch_event(&self, name: &str, data: &Value) {
        let _ = (name, data);
    }
}

/// The answer to an invoke call of a method the plugin does not know.
fn unknown_method(method: &str) -> String {
    format!("unknown method: {method}")
}

impl WebViewEvents for NoParameters {}

impl WebViewDispatch for NoParameters {}

/// What the dispatch that `#[beamer::model]` generates uses to find the
/// handler of a model.
///
/// `(&ModelRef(model)).invoke(..)` takes [`Implemented`](__private::Implemented)
/// when the model implements [`WebViewHandler`]. Otherwise only
/// [`Missing`](__private::Missing) fits, one reference further. The
/// generated code knows the type of the model, so the choice is made when
/// it is compiled.
#[doc(hidden)]
pub mod __private {
    use serde_json::Value;

    use super::{unknown_method, WebViewHandler};

    /// A model on its way to its handler.
    pub struct ModelRef<'a, T>(pub &'a T);

    /// The way to the handler of a model that has one.
    pub trait Implemented {
        fn invoke(&self, method: &str, args: &[Value]) -> Result<Value, String>;
        fn event(&self, name: &str, data: &Value);
    }

    impl<T: WebViewHandler> Implemented for ModelRef<'_, T> {
        fn invoke(&self, method: &str, args: &[Value]) -> Result<Value, String> {
            self.0.on_invoke(method, args)
        }

        fn event(&self, name: &str, data: &Value) {
            self.0.on_event(name, data);
        }
    }

    /// The answers of a model without a handler.
    pub trait Missing {
        fn invoke(&self, method: &str, args: &[Value]) -> Result<Value, String> {
            let _ = args;
            Err(unknown_method(method))
        }

        fn event(&self, name: &str, data: &Value) {
            let _ = (name, data);
        }
    }

    impl<T> Missing for &ModelRef<'_, T> {}
}

/// Implement the web traits for a model without a web UI: no event queue
/// and no handler. For test models in the format wrappers, which cannot use
/// `#[beamer::model]`.
#[doc(hidden)]
#[macro_export]
macro_rules! __without_web_ui {
    ($model:ty) => {
        impl $crate::WebViewEvents for $model {}

        impl $crate::WebViewDispatch for $model {}
    };
}

// The calls take the form of the generated dispatch: the reference is what
// lets a model without a handler reach `Missing`.
#[cfg(test)]
#[allow(clippy::needless_borrow)]
mod tests {
    use super::__private::{Implemented as _, Missing as _, ModelRef};
    use super::*;

    /// A model whose handler answers one method.
    struct Answering;

    impl WebViewHandler for Answering {
        fn on_invoke(&self, method: &str, _args: &[Value]) -> Result<Value, String> {
            match method {
                "answer" => Ok(Value::from(42)),
                _ => Err(unknown_method(method)),
            }
        }
    }

    /// A model without a handler.
    struct Silent;

    #[test]
    fn test_handler_is_found_when_the_model_has_one() {
        assert_eq!((&ModelRef(&Answering)).invoke("answer", &[]), Ok(Value::from(42)));
        assert_eq!(
            (&ModelRef(&Answering)).invoke("other", &[]),
            Err(String::from("unknown method: other"))
        );
    }

    #[test]
    fn test_model_without_a_handler_rejects_every_call() {
        assert_eq!(
            (&ModelRef(&Silent)).invoke("answer", &[]),
            Err(String::from("unknown method: answer"))
        );
        (&ModelRef(&Silent)).event("ignored", &Value::Null);
    }
}
