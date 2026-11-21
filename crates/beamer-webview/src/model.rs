//! What the web UI reaches in a model.
//!
//! - [`WebViewFields`] reaches the fields of a model and its nested groups
//!   that the web UI knows: the queue of events, the declared senders and
//!   events, the ids and types the page's model types list.
//! - [`WebViewHandler`] answers `__BEAMER__.invoke()` calls and custom
//!   events from JavaScript. `#[beamer::handler]` on an impl block of the
//!   model generates it when the web UI uses them.
//! - [`WebViewDispatch`] is how the format wrappers reach the handler.
//!
//! `#[beamer::model]` generates `WebViewFields` and `WebViewDispatch`,
//! `#[beamer::group]` generates `WebViewFields`. The generated dispatch
//! calls the model's `WebViewHandler` when it has one. Without one it
//! rejects every call as an unknown method and ignores the events.
//! Parameter synchronization is automatic and does not go through here.

use beamer_core::NoParameters;
use serde_json::Value;

use crate::event_queue::WebViewEventQueue;
use crate::model_types::ModelTypes;

/// The fields of a model or nested group that the web UI knows: the queue
/// of events, the declared senders and events (see
/// [`fields`](crate::fields)) and the ids and types of the page's model
/// types.
///
/// `#[beamer::model]` and `#[beamer::group]` generate this for a model and
/// each of its nested groups. A hand-written model or group adds an empty
/// `impl WebViewFields for MyModel {}`, or returns its queue.
pub trait WebViewFields {
    /// The plugin's queue of events for the web UI: a [`WebViewEventQueue`]
    /// field of this struct, or else the first one of a nested group in
    /// field order.
    ///
    /// The format wrappers deliver what is emitted into it while an editor
    /// is open. The default is for structs without one.
    fn webview_queue(&self) -> Option<&WebViewEventQueue> {
        None
    }

    /// Connect the declared senders and events of this struct and its
    /// nested groups to `queue`, each under its name behind `prefix`.
    ///
    /// The model's `assign_ids` calls this with the model's own queue, so
    /// that a sender made in `prepare()` delivers. The default is for
    /// structs without such fields.
    fn connect_webview(&mut self, prefix: &str, queue: &WebViewEventQueue) {
        let _ = (prefix, queue);
    }

    /// Add the ids and types of this struct and its nested groups to
    /// `types`, each path behind `prefix`: the parameters, the `State`
    /// fields, the senders and the events. The test that
    /// `#[beamer::export]` adds compares them with the page's model types.
    fn describe_model_types(types: &mut ModelTypes, prefix: &str)
    where
        Self: Sized,
    {
        let _ = (types, prefix);
    }
}

/// How a model answers its web UI: `__BEAMER__.invoke()` calls and custom
/// events from JavaScript.
///
/// `#[beamer::handler]` on an impl block of the model generates it from the
/// methods marked `#[method(id = "...")]` and `#[page_event(id = "...")]`,
/// with their types in the page's model types:
///
/// ```ignore
/// #[beamer::handler]
/// impl SamplerModel {
///     /// The name of the loaded sample.
///     #[method(id = "sampleName")]
///     fn sample_name(&self) -> String {
///         (*self.sample_name.get()).clone()
///     }
/// }
/// ```
///
/// A hand-written impl matches on the names. Its methods are not in the
/// page's model types, so the page's `invoke()` and `emit()` stay loose.
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

    /// Add the methods and page events with their types to `types`.
    /// `#[beamer::handler]` generates it. The default adds nothing, which
    /// leaves the page's `invoke()` and `emit()` loose.
    fn describe_handler(types: &mut ModelTypes)
    where
        Self: Sized,
    {
        let _ = types;
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

    /// Add the methods and page events of the plugin's handler to `types`,
    /// for the page's model types. Nothing for a model without one.
    fn dispatch_describe_handler(types: &mut ModelTypes)
    where
        Self: Sized,
    {
        let _ = types;
    }
}

/// The answer to an invoke call of a method the plugin does not know.
#[doc(hidden)]
pub fn unknown_method(method: &str) -> String {
    format!("unknown method: {method}")
}

impl WebViewFields for NoParameters {}

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
    use std::marker::PhantomData;

    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use serde_json::Value;

    pub use super::unknown_method;
    use super::WebViewHandler;
    use crate::model_types::ModelTypes;

    /// A model on its way to its handler.
    pub struct ModelRef<'a, T>(pub &'a T);

    /// The type of a model on its way to the description of its handler,
    /// chosen as [`ModelRef`] is.
    pub struct HandlerOf<T>(PhantomData<fn() -> T>);

    impl<T> HandlerOf<T> {
        #[allow(clippy::new_without_default)]
        pub fn new() -> Self {
            Self(PhantomData)
        }
    }

    /// The description of a model's handler.
    pub trait Described {
        fn describe(&self, types: &mut ModelTypes);
    }

    impl<T: WebViewHandler> Described for HandlerOf<T> {
        fn describe(&self, types: &mut ModelTypes) {
            T::describe_handler(types);
        }
    }

    /// Nothing to describe for a model without a handler.
    pub trait Undescribed {
        fn describe(&self, types: &mut ModelTypes) {
            let _ = types;
        }
    }

    impl<T> Undescribed for &HandlerOf<T> {}

    /// The answer to a call with the wrong number of arguments.
    pub fn check_argument_count(method: &str, expected: usize, args: &[Value]) -> Result<(), String> {
        if args.len() == expected {
            Ok(())
        } else {
            let arguments = if expected == 1 { "argument" } else { "arguments" };
            Err(format!("method {method:?} takes {expected} {arguments}, got {}", args.len()))
        }
    }

    /// The argument at `index` of a call of `method`, read with serde.
    pub fn argument<T: DeserializeOwned>(method: &str, index: usize, value: &Value) -> Result<T, String> {
        T::deserialize(value).map_err(|error| format!("argument {} of method {method:?}: {error}", index + 1))
    }

    /// The data of the page event `name`, read with serde. Data that does
    /// not read is logged. The event is dropped then.
    pub fn page_event_data<T: DeserializeOwned>(name: &str, data: &Value) -> Option<T> {
        match T::deserialize(data) {
            Ok(data) => Some(data),
            Err(error) => {
                log::error!("The data of the page event {name:?} does not read: {error}");
                None
            }
        }
    }

    /// What a method returned, on its way to the page. `(&Answer(result))
    /// .answer(..)` takes [`AnswerResult`] for a `Result`, whatever its name,
    /// and [`AnswerValue`] for any other value, one reference further.
    pub struct Answer<T>(pub T);

    /// The answer of a method that returns a `Result`: its value, or its
    /// error's text, which rejects the page's promise.
    pub trait AnswerResult {
        fn answer(&self, method: &str, reported: &std::sync::atomic::AtomicBool) -> Result<Value, String>;
    }

    impl<T: Serialize, E: std::fmt::Display> AnswerResult for Answer<Result<T, E>> {
        fn answer(&self, method: &str, reported: &std::sync::atomic::AtomicBool) -> Result<Value, String> {
            match &self.0 {
                Ok(value) => answer(method, value, reported),
                Err(error) => Err(error.to_string()),
            }
        }
    }

    /// The answer of a method that returns a value.
    pub trait AnswerValue {
        fn answer(&self, method: &str, reported: &std::sync::atomic::AtomicBool) -> Result<Value, String>;
    }

    impl<T: Serialize> AnswerValue for &Answer<T> {
        fn answer(&self, method: &str, reported: &std::sync::atomic::AtomicBool) -> Result<Value, String> {
            answer(method, &self.0, reported)
        }
    }

    /// The answer of `method` to the page. Debug builds report a number
    /// JSON does not hold as it is, once per method (`reported`).
    fn answer<T: Serialize + ?Sized>(
        method: &str,
        result: &T,
        reported: &std::sync::atomic::AtomicBool,
    ) -> Result<Value, String> {
        #[cfg(debug_assertions)]
        if !reported.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(found) = beamer_core::json_check::unfaithful_number(result) {
                if !reported.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    beamer_core::json_check::report(&format!("The method {method:?}"), &found);
                }
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = reported;
        serde_json::to_value(result).map_err(|error| format!("the result of method {method:?}: {error}"))
    }

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
        impl $crate::WebViewFields for $model {}

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
