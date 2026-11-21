//! The model's types for a typed web UI.
//!
//! A plugin whose page is built with a `webview/package.json` gets the ids
//! and value types of its model as TypeScript, in
//! `webview/src/model-types.ts`: the parameter ids, the `State` fields,
//! the senders and the events, each with the type of what it carries. The
//! file augments `ModelTypes` of the beamer-api package, so a typo in an
//! id is a compile error in the page and a value has the model's type.
//!
//! `#[beamer::export]` adds a test to such a plugin that calls [`check`]:
//! it compares the file with the model and fails while they differ. With
//! `BEAMER_WRITE_MODEL_TYPES` set it writes the file instead, which
//! `cargo xtask model-types` and `cargo xtask bundle` do.

use std::path::Path;

use beamer_core::state::State;
use beamer_core::ParameterStore;
use beamer_utils::web_ui::{MODEL_TYPES_NOTE, MODEL_TYPES_PATH, WRITE_MODEL_TYPES_VAR};

use crate::fields::{WebViewEvent, WebViewStream, WebViewValue};
use crate::model::WebViewFields;
use crate::type_script::{after, TsType, TypeScriptTypes};

/// The type a field of the model carries to the page: the value of a
/// `State` field, of a sender or the data of an event. For the code that
/// `#[beamer::model]` generates, which names the field's type.
pub trait Carries {
    /// What the field carries.
    type Value: ?Sized;
}

impl<T> Carries for State<T> {
    type Value = T;
}

impl<T> Carries for WebViewValue<T> {
    type Value = T;
}

impl<T> Carries for WebViewStream<T> {
    type Value = T;
}

impl<T> Carries for WebViewEvent<T> {
    type Value = T;
}

/// One name of a section with the type of what it carries.
struct Entry {
    // The path: the id behind the names of the nested groups.
    path: String,
    // The Rust type, for notes and errors.
    rust_type: &'static str,
    // The model's type parameter that the Rust type names, in a generic
    // model.
    type_parameter: Option<&'static str>,
    // The TypeScript, or `None` for a type without the derive.
    type_script: Option<TsType>,
}

/// The ids and types of a model, as the page's model types list them.
///
/// The generated [`WebViewFields::describe_model_types`] of the model and
/// its nested groups fills it; [`check`] renders it.
pub struct ModelTypes {
    types: TypeScriptTypes,
    parameters: Vec<String>,
    state: Vec<Entry>,
    values: Vec<Entry>,
    streams: Vec<Entry>,
    events: Vec<Entry>,
}

impl Default for ModelTypes {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelTypes {
    /// No ids yet.
    pub fn new() -> Self {
        Self {
            types: TypeScriptTypes::new(),
            parameters: Vec::new(),
            state: Vec::new(),
            values: Vec::new(),
            streams: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Add the string id of a parameter.
    pub fn parameter(&mut self, id: &str) {
        self.parameters.push(id.to_string());
    }

    /// Add a `State` field at `path`. `rust_type` names the type of its
    /// value for the notes. `type_parameter` is the model's type parameter
    /// that the type names, in a generic model: its TypeScript needs a
    /// `TypeScript` bound there. `type_script` gives the TypeScript of the
    /// value, `None` for a type without it.
    pub fn state(
        &mut self,
        path: String,
        rust_type: &'static str,
        type_parameter: Option<&'static str>,
        type_script: impl FnOnce(&mut TypeScriptTypes) -> Option<TsType>,
    ) {
        let type_script = type_script(&mut self.types);
        self.state.push(Entry { path, rust_type, type_parameter, type_script });
    }

    /// Add a value sender `name`, as [`state`](Self::state) does.
    pub fn value(
        &mut self,
        name: String,
        rust_type: &'static str,
        type_parameter: Option<&'static str>,
        type_script: impl FnOnce(&mut TypeScriptTypes) -> Option<TsType>,
    ) {
        let type_script = type_script(&mut self.types);
        self.values.push(Entry { path: name, rust_type, type_parameter, type_script });
    }

    /// Add a stream sender `name` with the type of one value, as
    /// [`state`](Self::state) does. The page receives arrays of them.
    pub fn stream(
        &mut self,
        name: String,
        rust_type: &'static str,
        type_parameter: Option<&'static str>,
        type_script: impl FnOnce(&mut TypeScriptTypes) -> Option<TsType>,
    ) {
        let type_script = type_script(&mut self.types);
        self.streams.push(Entry { path: name, rust_type, type_parameter, type_script });
    }

    /// Add an event `name` with the type of its data, as
    /// [`state`](Self::state) does.
    pub fn event(
        &mut self,
        name: String,
        rust_type: &'static str,
        type_parameter: Option<&'static str>,
        type_script: impl FnOnce(&mut TypeScriptTypes) -> Option<TsType>,
    ) {
        let type_script = type_script(&mut self.types);
        self.events.push(Entry { path: name, rust_type, type_parameter, type_script });
    }

    /// The model's types as the page's `model-types.ts`, or what keeps
    /// them from being written: a sender or event whose type has no
    /// TypeScript, two Rust types under one TypeScript name.
    pub fn render(&self, package: &str) -> Result<String, Vec<String>> {
        let mut errors: Vec<String> = self.types.errors().to_vec();
        let lacking = |kind: &str, entries: &[Entry], errors: &mut Vec<String>| {
            for entry in entries.iter().filter(|entry| entry.type_script.is_none()) {
                errors.push(match entry.type_parameter {
                    Some(parameter) => format!(
                        "The {kind} `{}` carries `{}`, whose TypeScript the model cannot see: \
                         its types need #[derive(TypeScript)] and the model's type parameter \
                         `{parameter}` a `TypeScript` bound.",
                        entry.path, entry.rust_type
                    ),
                    None => format!(
                        "The {kind} `{}` carries `{}`, which has no #[derive(TypeScript)]. A \
                         {kind} exists for the page only: derive it next to `Serialize`.",
                        entry.path, entry.rust_type
                    ),
                });
            }
        };
        lacking("value sender", &self.values, &mut errors);
        lacking("stream sender", &self.streams, &mut errors);
        lacking("event", &self.events, &mut errors);
        if !errors.is_empty() {
            return Err(errors);
        }

        let mut out = format!(
            "// Generated from the plugin's model. Do not edit: `cargo test` fails\n\
             // while this file differs from the model. `cargo xtask bundle` writes\n\
             // it, or on its own:\n\
             //\n\
             //   cargo xtask model-types {package}\n\n"
        );
        let declarations = self.types.render_declarations();
        if declarations.trim().is_empty() {
            // A module, so that `declare module` augments beamer-api's
            // types instead of replacing them.
            out.push_str("export {};\n");
        } else {
            out.push_str(declarations.trim_end());
            out.push('\n');
        }
        out.push_str("\ndeclare module \"@helpermedia/beamer-api\" {\n  interface ModelTypes {\n");
        out.push_str(&render_parameters(&self.parameters));
        out.push_str(&render_section("state", &self.state));
        out.push_str(&render_section("values", &self.values));
        out.push_str(&render_section("streams", &self.streams));
        out.push_str(&render_section("events", &self.events));
        out.push_str("  }\n}\n");
        Ok(out)
    }
}

/// The `parameters` line: a union of the ids on one line when it fits,
/// one id per line otherwise, `never` for a model without parameters.
fn render_parameters(ids: &[String]) -> String {
    if ids.is_empty() {
        return "    parameters: never;\n".to_string();
    }
    let literals: Vec<String> = ids.iter().map(|id| json_string(id)).collect();
    let line = format!("    parameters: {};", literals.join(" | "));
    if line.len() <= 80 {
        return format!("{line}\n");
    }
    let mut out = "    parameters:\n".to_string();
    for (index, literal) in literals.iter().enumerate() {
        let end = if index + 1 == literals.len() { ";" } else { "" };
        out.push_str(&format!("      | {literal}{end}\n"));
    }
    out
}

/// A section of names with their types, `{}` when it has none. A type
/// without TypeScript is `unknown`, with a note that says why.
fn render_section(section: &str, entries: &[Entry]) -> String {
    if entries.is_empty() {
        return format!("    {section}: {{}};\n");
    }
    let mut out = format!("    {section}: {{\n");
    for entry in entries {
        let unknown = TsType::Keyword("unknown");
        let type_script = match &entry.type_script {
            Some(type_script) => type_script,
            None => {
                let reason = match entry.type_parameter {
                    Some(parameter) => format!(
                        "the model cannot see the TypeScript of `{}` (its types need \
                         #[derive(TypeScript)] and the type parameter `{parameter}` a \
                         `TypeScript` bound)",
                        entry.rust_type
                    ),
                    None => format!("`{}` has no #[derive(TypeScript)]", entry.rust_type),
                };
                out.push_str(&format!(
                    "      {MODEL_TYPES_NOTE}{reason}, so the page reads `{}` as unknown.\n",
                    entry.path
                ));
                &unknown
            }
        };
        let head = format!("      {}:", property_name(&entry.path));
        out.push_str(&after(&head, type_script, 6, 1));
        out.push_str(";\n");
    }
    out.push_str("    };\n");
    out
}

/// A name as a TypeScript property: as it is when it is an identifier,
/// quoted otherwise (a path with a slash).
fn property_name(name: &str) -> String {
    let mut chars = name.chars();
    let identifier = chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if identifier {
        name.to_string()
    } else {
        json_string(name)
    }
}

/// A string as JSON, which is a TypeScript string literal as well.
fn json_string(text: &str) -> String {
    // Serializing a string cannot fail.
    serde_json::to_string(text).unwrap_or_else(|_| String::from("\"\""))
}

/// Compare the page's model types of the package in `manifest_dir` with
/// the model `M`, or write them with `BEAMER_WRITE_MODEL_TYPES` set.
///
/// The test that `#[beamer::export]` adds calls this. It panics while the
/// file differs from the model, with the command that writes it. It also
/// panics when the model's types cannot be written.
pub fn check<M>(manifest_dir: &str, package: &str)
where
    M: Default + ParameterStore + WebViewFields,
{
    let mut types = ModelTypes::new();
    // The parameter ids in the order of the init dump, from the model the
    // host gets.
    let model = M::default();
    for index in 0..model.count() {
        if let Some(info) = model.info(index) {
            types.parameter(&info.string_id);
        }
    }
    M::describe_model_types(&mut types, "");

    let rendered = match types.render(package) {
        Ok(rendered) => rendered,
        Err(errors) => panic!("The model's types of {package} cannot be written:\n- {}", errors.join("\n- ")),
    };

    let path = Path::new(manifest_dir).join(MODEL_TYPES_PATH);
    // A checkout that turned the line ends into CRLF still matches.
    let current = std::fs::read_to_string(&path).map(|text| text.replace("\r\n", "\n"));
    let matches = current.as_deref().ok() == Some(rendered.as_str());
    if std::env::var_os(WRITE_MODEL_TYPES_VAR).is_some() {
        // An unchanged file keeps its time, so the page's tools see no
        // change.
        if matches {
            return;
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("Failed to create {}: {e}", dir.display()));
        }
        std::fs::write(&path, &rendered).unwrap_or_else(|e| panic!("Failed to write {}: {e}", path.display()));
        return;
    }

    if !matches {
        let state = if current.is_ok() { "differs from the model" } else { "is missing" };
        panic!(
            "{} {state}. Write it with `cargo xtask model-types {package}` (`cargo xtask bundle` \
             writes it too) and commit it with the change of the model.",
            path.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(_: &mut TypeScriptTypes) -> Option<TsType> {
        Some(TsType::Keyword("number"))
    }

    fn missing(_: &mut TypeScriptTypes) -> Option<TsType> {
        None
    }

    #[test]
    fn test_a_model_without_anything_is_still_a_module() {
        let rendered = ModelTypes::new().render("gain").unwrap();
        assert!(rendered.contains("\nexport {};\n"), "{rendered}");
        assert!(rendered.contains("    parameters: never;\n"), "{rendered}");
        assert!(rendered.contains("    state: {};\n    values: {};\n    streams: {};\n    events: {};\n"), "{rendered}");
    }

    #[test]
    fn test_parameters_go_one_per_line_when_they_do_not_fit() {
        assert_eq!(render_parameters(&["gain".to_string()]), "    parameters: \"gain\";\n");
        let many: Vec<String> = (0..12).map(|index| format!("parameter_{index}")).collect();
        let rendered = render_parameters(&many);
        assert!(rendered.starts_with("    parameters:\n      | \"parameter_0\"\n"), "{rendered}");
        assert!(rendered.ends_with("      | \"parameter_11\";\n"), "{rendered}");
    }

    #[test]
    fn test_paths_with_a_slash_are_quoted() {
        let mut types = ModelTypes::new();
        types.value("note/pressed".to_string(), "u32", None, number);
        types.value("pressed".to_string(), "u32", None, number);
        let rendered = types.render("midi-transform").unwrap();
        assert!(rendered.contains("      \"note/pressed\": number;\n      pressed: number;\n"), "{rendered}");
    }

    #[test]
    fn test_a_state_type_without_type_script_is_unknown_with_a_note() {
        let mut types = ModelTypes::new();
        types.state("tab".to_string(), "drum_synth::Tab", None, missing);
        let rendered = types.render("drum-synth").unwrap();
        assert!(
            rendered.contains(&format!(
                "      {MODEL_TYPES_NOTE}`drum_synth::Tab` has no #[derive(TypeScript)], so the page reads `tab` as unknown.\n      tab: unknown;\n"
            )),
            "{rendered}"
        );
    }

    #[test]
    fn test_a_sender_or_event_without_type_script_is_an_error() {
        let mut types = ModelTypes::new();
        types.value("meters".to_string(), "compressor::Meters", None, missing);
        types.stream("hits".to_string(), "drum_synth::Hit", None, missing);
        types.event("sampleRate".to_string(), "f64", None, number);
        let errors = types.render("compressor").unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[0].contains("value sender `meters` carries `compressor::Meters`"), "{}", errors[0]);
        assert!(errors[1].contains("stream sender `hits`"), "{}", errors[1]);
    }

    #[test]
    fn test_a_type_parameter_without_a_bound_is_named() {
        let mut types = ModelTypes::new();
        types.state("tab".to_string(), "demo::Tab", Some("T"), missing);
        let rendered = types.render("demo").unwrap();
        assert!(
            rendered.contains(&format!(
                "      {MODEL_TYPES_NOTE}the model cannot see the TypeScript of `demo::Tab` (its types need #[derive(TypeScript)] and the type parameter `T` a `TypeScript` bound), so the page reads `tab` as unknown.\n"
            )),
            "{rendered}"
        );
        types.value("meters".to_string(), "demo::Meters", Some("M"), missing);
        let errors = types.render("demo").unwrap_err();
        assert!(errors[0].contains("the model's type parameter `M` a `TypeScript` bound"), "{}", errors[0]);
    }

    #[test]
    fn test_a_union_that_breaks_starts_on_the_next_line() {
        let mut types = ModelTypes::new();
        let long = |_: &mut TypeScriptTypes| {
            Some(TsType::Union(
                (0..8)
                    .map(|index| crate::type_script::__private::member(None, TsType::Literal(format!("variant_{index}"))))
                    .collect(),
            ))
        };
        types.state("mode".to_string(), "demo::Mode", None, long);
        let rendered = types.render("demo").unwrap();
        assert!(rendered.contains("      mode:\n        | \"variant_0\"\n"), "{rendered}");
        assert!(!rendered.lines().any(|line| line.ends_with(' ')), "{rendered}");
    }
}
