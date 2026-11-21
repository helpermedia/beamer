//! The model's types for the page, as `#[beamer::model]` describes them:
//! the paths in nested groups, a type without TypeScript, a generic model.

use beamer::core::parameter_store::ParameterStore;
use beamer::prelude::*;
use beamer::serde::{de::DeserializeOwned, Deserialize, Serialize};
use beamer::webview::ModelTypes;

/// A tab of the editor.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Tab {
    index: u8,
}

/// A type without the derive.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(crate = "beamer::serde")]
struct Plain {
    index: u8,
}

#[beamer::group]
struct NoteGroup {
    #[parameter(id = "level", name = "Level", default = 1.0, range = 0.0..=1.0)]
    level: FloatParameter,
    #[state(id = "tab")]
    tab: State<Tab>,
    #[value(id = "pressed")]
    pressed: WebViewValue<u32>,
    #[stream(id = "hits")]
    hits: WebViewStream<f32>,
    #[event(id = "done")]
    done: WebViewEvent<Tab>,
    #[nested(group = "Deep")]
    deep: DeepGroup,
}

#[beamer::group]
struct DeepGroup {
    #[value(id = "levels")]
    levels: WebViewValue<[f32; 2]>,
}

#[beamer::model]
struct NestedModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,
    #[state(id = "plain")]
    plain: State<Plain>,
    #[nested(group = "Note")]
    note: NoteGroup,
    #[nested(group = "Other")]
    r#type: NoteGroup,
    events: WebViewEventQueue,
}

/// The model's types as the generated test renders them.
fn render<M: Default + ParameterStore + WebViewFields>() -> Result<String, Vec<String>> {
    let mut types = ModelTypes::new();
    let model = M::default();
    for index in 0..model.count() {
        if let Some(info) = model.info(index) {
            types.parameter(&info.string_id);
        }
    }
    M::describe_model_types(&mut types, "");
    types.render("probe")
}

/// The body of the `ModelTypes` augmentation.
fn sections(rendered: &str) -> &str {
    let start = rendered.find("  interface ModelTypes {\n").expect("the augmentation");
    &rendered[start..]
}

#[test]
fn test_nested_paths_are_the_paths_of_the_fields() {
    let rendered = render::<NestedModel>().unwrap();
    assert_eq!(
        sections(&rendered),
        "  interface ModelTypes {\n\
         \x20   parameters: \"gain\" | \"note/level\" | \"type/level\";\n\
         \x20   state: {\n\
         \x20     // Note: `model_types::Plain` has no #[derive(TypeScript)], so the page reads `plain` as unknown.\n\
         \x20     plain: unknown;\n\
         \x20     \"note/tab\": Tab;\n\
         \x20     \"type/tab\": Tab;\n\
         \x20   };\n\
         \x20   values: {\n\
         \x20     \"note/pressed\": number;\n\
         \x20     \"note/deep/levels\": [number, number];\n\
         \x20     \"type/pressed\": number;\n\
         \x20     \"type/deep/levels\": [number, number];\n\
         \x20   };\n\
         \x20   streams: {\n\
         \x20     \"note/hits\": number;\n\
         \x20     \"type/hits\": number;\n\
         \x20   };\n\
         \x20   events: {\n\
         \x20     \"note/done\": Tab;\n\
         \x20     \"type/done\": Tab;\n\
         \x20   };\n\
         \x20 }\n\
         }\n"
    );

    // The names the fields deliver under are the same paths.
    let model = NestedModel::default();
    assert_eq!(model.note.pressed.name(), "note/pressed");
    assert_eq!(model.r#type.deep.levels.name(), "type/deep/levels");
    let mut paths = Vec::new();
    model.for_each_state(&mut |path, _| paths.push(path.to_string()));
    assert_eq!(paths, ["plain", "note/tab", "type/tab"]);
}

#[beamer::model]
struct GenericModel<T: Serialize + DeserializeOwned + Default + Send + Sync + Clone + 'static> {
    #[state(id = "value")]
    value: State<T>,
    #[state(id = "rate")]
    rate: State<f32>,
}

#[beamer::model]
struct BoundModel<T: Serialize + DeserializeOwned + Default + Send + Sync + Clone + TypeScript + 'static> {
    #[state(id = "value")]
    value: State<T>,
}

#[test]
fn test_a_type_parameter_needs_a_type_script_bound() {
    let rendered = render::<GenericModel<Tab>>().unwrap();
    assert!(
        rendered.contains(
            "      // Note: the model cannot see the TypeScript of `model_types::Tab` (its types need #[derive(TypeScript)] and the type parameter `T` a `TypeScript` bound), so the page reads `value` as unknown.\n\
             \x20     value: unknown;\n\
             \x20     rate: number;\n"
        ),
        "{rendered}"
    );
    let rendered = render::<BoundModel<Tab>>().unwrap();
    assert!(rendered.contains("    state: {\n      value: Tab;\n    };\n"), "{rendered}");
}

#[beamer::model]
struct PlainSender {
    #[value(id = "meters")]
    meters: WebViewValue<Plain>,
    events: WebViewEventQueue,
}

#[test]
fn test_a_sender_without_type_script_is_an_error() {
    let errors = render::<PlainSender>().unwrap_err();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("value sender `meters` carries `model_types::Plain`"), "{}", errors[0]);
}
