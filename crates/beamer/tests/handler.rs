//! `#[beamer::handler]`: the model's methods that the web UI calls and the
//! page events it sends, reached through the dispatch that
//! `#[beamer::model]` generates, with their types in the page's model types.

#![cfg(feature = "derive")]

use std::marker::PhantomData;
use std::sync::Mutex;

use beamer::core::parameter_store::ParameterStore;
use beamer::prelude::*;
use beamer::serde::{Deserialize, Serialize};
use beamer::webview::{ModelTypes, WebViewDispatch};
use serde_json::{json, Value};

/// A pad the page played.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct PadHit {
    pad: u8,
    velocity: f32,
}

/// What the page gets about a pad.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct PadInfo {
    name: String,
    hits: u32,
}

#[beamer::model]
struct PadModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    /// The pads the page played, in order.
    hits: Mutex<Vec<PadHit>>,
    /// How often the page cleared the pads.
    clears: Mutex<u32>,
}

#[beamer::handler]
impl PadModel {
    /// The name of a pad with its number of hits.
    #[method(id = "padInfo")]
    fn pad_info(&self, pad: u8, prefix: String) -> PadInfo {
        let hits = self.hits().iter().filter(|hit| hit.pad == pad).count() as u32;
        PadInfo {
            name: format!("{prefix} {pad}"),
            hits,
        }
    }

    /// The level of a pad, refused for one that does not exist.
    #[method(id = "padLevel")]
    fn pad_level(&self, pad: u8) -> Result<f32, String> {
        if pad < 4 {
            Ok(1.0)
        } else {
            Err(format!("no pad {pad}"))
        }
    }

    /// Forget every hit.
    #[method(id = "reset")]
    fn reset(&self) {
        self.hits.lock().unwrap().clear();
    }

    /// A pad the page played.
    #[page_event(id = "padHit")]
    fn pad_hit(&self, hit: PadHit) {
        self.hits.lock().unwrap().push(hit);
    }

    /// The page cleared the pads.
    #[page_event(id = "cleared")]
    fn cleared(&self) {
        *self.clears.lock().unwrap() += 1;
    }

    // A helper without an attribute: not for the page.
    fn hits(&self) -> Vec<PadHit> {
        self.hits.lock().unwrap().clone()
    }
}

#[test]
fn test_methods_answer_with_their_arguments() {
    let model = PadModel::default();
    model.dispatch_event("padHit", &json!({ "pad": 2, "velocity": 0.5 }));
    assert_eq!(
        model.dispatch_invoke("padInfo", &[json!(2), json!("Pad")]),
        Ok(json!({ "name": "Pad 2", "hits": 1 }))
    );
    assert_eq!(
        model.dispatch_invoke("padLevel", &[json!(1)]),
        Ok(json!(1.0))
    );
    assert_eq!(
        model.dispatch_invoke("padLevel", &[json!(9)]),
        Err(String::from("no pad 9"))
    );
    assert_eq!(model.dispatch_invoke("reset", &[]), Ok(Value::Null));
    assert!(model.hits().is_empty());
}

#[test]
fn test_a_call_that_does_not_fit_is_refused() {
    let model = PadModel::default();
    assert_eq!(
        model.dispatch_invoke("padInfo", &[json!(2)]),
        Err(String::from("method \"padInfo\" takes 2 arguments, got 1"))
    );
    let wrong = model
        .dispatch_invoke("padInfo", &[json!("two"), json!("Pad")])
        .unwrap_err();
    assert!(
        wrong.starts_with("argument 1 of method \"padInfo\": "),
        "{wrong}"
    );
    assert_eq!(
        model.dispatch_invoke("hits", &[]),
        Err(String::from("unknown method: hits"))
    );
}

#[test]
fn test_page_events_reach_their_methods() {
    let model = PadModel::default();
    model.dispatch_event("padHit", &json!({ "pad": 1, "velocity": 1.0 }));
    // Data that does not read is dropped, as is an unknown event.
    model.dispatch_event("padHit", &json!({ "pad": "one" }));
    model.dispatch_event("unknown", &Value::Null);
    model.dispatch_event("cleared", &Value::Null);
    assert_eq!(
        model.hits(),
        [PadHit {
            pad: 1,
            velocity: 1.0
        }]
    );
    assert_eq!(*model.clears.lock().unwrap(), 1);
}

/// The model's types as the generated test renders them.
fn render<M: Default + ParameterStore + WebViewFields + WebViewDispatch>(
) -> Result<String, Vec<String>> {
    let mut types = ModelTypes::new();
    let model = M::default();
    for index in 0..model.count() {
        if let Some(info) = model.info(index) {
            types.parameter(&info.string_id);
        }
    }
    <M as WebViewFields>::describe_model_types(&mut types, "");
    <M as WebViewDispatch>::dispatch_describe_handler(&mut types);
    types.render("probe")
}

#[test]
fn test_methods_and_page_events_are_in_the_model_types() {
    let rendered = render::<PadModel>().unwrap();
    assert!(
        rendered.ends_with(
            "    methods: {\n\
             \x20     padInfo: { arguments: [number, string]; result: PadInfo };\n\
             \x20     padLevel: { arguments: [number]; result: number };\n\
             \x20     reset: { arguments: []; result: null };\n\
             \x20   };\n\
             \x20   pageEvents: {\n\
             \x20     padHit: PadHit;\n\
             \x20     cleared: null;\n\
             \x20   };\n\
             \x20 }\n\
             }\n"
        ),
        "{rendered}"
    );
}

/// A model without a handler.
#[beamer::model]
struct QuietModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,
}

/// A model with a handler written by hand.
#[beamer::model]
struct ByHandModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,
}

impl WebViewHandler for ByHandModel {
    fn on_invoke(&self, method: &str, _args: &[Value]) -> Result<Value, String> {
        match method {
            "answer" => Ok(json!(42)),
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

#[test]
fn test_without_the_attribute_the_page_stays_loose() {
    for rendered in [
        render::<QuietModel>().unwrap(),
        render::<ByHandModel>().unwrap(),
    ] {
        assert!(!rendered.contains("methods"), "{rendered}");
        assert!(!rendered.contains("pageEvents"), "{rendered}");
    }
    assert_eq!(
        ByHandModel::default().dispatch_invoke("answer", &[]),
        Ok(json!(42))
    );
}

/// A model with a handler and nothing it answers yet.
#[beamer::model]
struct EmptyHandlerModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,
}

#[beamer::handler]
impl EmptyHandlerModel {}

#[test]
fn test_an_empty_handler_allows_no_name() {
    let rendered = render::<EmptyHandlerModel>().unwrap();
    assert!(
        rendered.ends_with("    methods: {};\n    pageEvents: {};\n  }\n}\n"),
        "{rendered}"
    );
}

/// A type without the derive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "beamer::serde")]
struct Plain {
    pad: u8,
}

#[beamer::model]
struct PlainModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,
}

#[beamer::handler]
impl PlainModel {
    #[method(id = "plain")]
    fn plain(&self, plain: Plain) -> Plain {
        plain
    }
}

#[test]
fn test_a_method_type_without_type_script_is_an_error() {
    let errors = render::<PlainModel>().unwrap_err();
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(
        errors[0]
            .contains("The method `plain` takes `handler::Plain` as its argument 1, which has no"),
        "{}",
        errors[0]
    );
    assert!(
        errors[1].contains("The method `plain` returns `handler::Plain`, which has no"),
        "{}",
        errors[1]
    );
}

/// A generic model whose handler holds for every type it takes.
#[beamer::model]
struct GenericModel<T: Send + Sync + 'static> {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    /// What the type parameter stands for, not a parameter.
    kind: PhantomData<fn() -> T>,
}

#[beamer::handler]
impl<T: Send + Sync + 'static> GenericModel<T> {
    #[method(id = "double")]
    fn double(&self, value: f64) -> f64 {
        value * 2.0
    }
}

#[test]
fn test_a_generic_model_has_its_handler() {
    let model = GenericModel::<u8>::default();
    assert_eq!(
        model.dispatch_invoke("double", &[json!(1.5)]),
        Ok(json!(3.0))
    );
    let rendered = render::<GenericModel<u8>>().unwrap();
    assert!(
        rendered.contains("      double: { arguments: [number]; result: number };\n"),
        "{rendered}"
    );
}

/// A loaded sample, under another name for its `Result`.
type Loaded = Result<u8, String>;

#[beamer::model]
struct ShapesModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,

    /// The data of each "maybe" event.
    maybes: Mutex<Vec<Option<u8>>>,
}

#[beamer::handler]
impl ShapesModel {
    /// Nothing to answer, or an error.
    #[method(id = "check")]
    fn check(&self, fail: bool) -> Result<(), String> {
        if fail {
            Err(String::from("failed"))
        } else {
            Ok(())
        }
    }

    /// An optional argument and result.
    #[method(id = "half")]
    fn half(&self, value: Option<u8>) -> Option<u8> {
        value.map(|value| value / 2)
    }

    /// A `Result` under another name rejects all the same.
    #[method(id = "load")]
    fn load(&self) -> Loaded {
        Err(String::from("no file"))
    }

    /// An id the page writes with a slash, arguments as patterns.
    #[method(id = "pair/sum")]
    fn pair_sum(&self, (a, b): (u8, u8), _: u8) -> u16 {
        u16::from(a) + u16::from(b)
    }

    /// A method only some builds have.
    #[cfg(any())]
    #[method(id = "hidden")]
    fn hidden(&self) -> u8 {
        0
    }

    /// Optional data.
    #[page_event(id = "maybe")]
    fn maybe(&self, data: Option<u8>) {
        self.maybes.lock().unwrap().push(data);
    }

    /// A page event that says it returns nothing.
    // The `-> ()` is what the test is about.
    #[allow(clippy::unused_unit)]
    #[page_event(id = "ping")]
    fn ping(&self) -> () {}
}

#[test]
fn test_results_options_and_patterns() {
    let model = ShapesModel::default();
    assert_eq!(
        model.dispatch_invoke("check", &[json!(false)]),
        Ok(Value::Null)
    );
    assert_eq!(
        model.dispatch_invoke("check", &[json!(true)]),
        Err(String::from("failed"))
    );
    assert_eq!(model.dispatch_invoke("half", &[json!(8)]), Ok(json!(4)));
    assert_eq!(
        model.dispatch_invoke("half", &[Value::Null]),
        Ok(Value::Null)
    );
    assert_eq!(
        model.dispatch_invoke("load", &[]),
        Err(String::from("no file"))
    );
    assert_eq!(
        model.dispatch_invoke("pair/sum", &[json!([1, 2]), json!(0)]),
        Ok(json!(3))
    );
    assert_eq!(
        model.dispatch_invoke("hidden", &[]),
        Err(String::from("unknown method: hidden"))
    );
    assert_eq!(
        model.dispatch_invoke("check", &[]),
        Err(String::from("method \"check\" takes 1 argument, got 0"))
    );
    model.dispatch_event("maybe", &json!(3));
    model.dispatch_event("maybe", &Value::Null);
    model.dispatch_event("ping", &Value::Null);
    assert_eq!(*model.maybes.lock().unwrap(), [Some(3), None]);
}

#[test]
fn test_a_result_under_another_name_is_named_in_the_error() {
    let errors = render::<ShapesModel>().unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("The method `load` returns `core::result::Result<u8, alloc::string::String>` under another name. Write `Result<T, E>`"), "{}", errors[0]);
}

#[beamer::model]
struct RenderedModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    gain: FloatParameter,
}

#[beamer::handler]
impl RenderedModel {
    #[method(id = "check")]
    fn check(&self, fail: bool) -> Result<(), String> {
        if fail {
            Err(String::from("failed"))
        } else {
            Ok(())
        }
    }

    #[method(id = "pair/sum")]
    fn pair_sum(&self, pair: (u8, u8), extra: Option<String>) -> Option<PadInfo> {
        let _ = (pair, extra);
        None
    }

    #[method(id = "describe")]
    fn describe(
        &self,
        first: Vec<PadInfo>,
        second: Vec<PadInfo>,
        third: Vec<PadInfo>,
    ) -> Vec<PadInfo> {
        [first, second, third].concat()
    }

    #[page_event(id = "maybe")]
    fn maybe(&self, data: Option<u8>) {
        let _ = data;
    }
}

#[test]
fn test_quoted_ids_options_and_long_methods_in_the_model_types() {
    let rendered = render::<RenderedModel>().unwrap();
    let methods = &rendered[rendered.find("    methods:").unwrap()..];
    assert_eq!(
        methods,
        "    methods: {\n\
         \x20     check: { arguments: [boolean]; result: null };\n\
         \x20     \"pair/sum\": {\n\
         \x20       arguments: [[number, number], string | null];\n\
         \x20       result: PadInfo | null;\n\
         \x20     };\n\
         \x20     describe: {\n\
         \x20       arguments: [PadInfo[], PadInfo[], PadInfo[]];\n\
         \x20       result: PadInfo[];\n\
         \x20     };\n\
         \x20   };\n\
         \x20   pageEvents: {\n\
         \x20     maybe: number | null;\n\
         \x20   };\n\
         \x20 }\n\
         }\n"
    );
}
