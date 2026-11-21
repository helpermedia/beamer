//! Macros for the Beamer audio plugin framework.
//!
//! - `#[beamer::model]` marks the model of a plugin: the struct every thread
//!   shares, with its parameters, `State` fields and event queue.
//! - `#[beamer::group]` marks a nested group of a model.
//! - `#[beamer::handler]` marks the impl block whose methods answer the web
//!   UI's `invoke()` calls and page events.
//! - `#[beamer::export]` exports the model as a plugin, with its
//!   `Config.toml`.
//! - `#[derive(EnumParameter)]` makes an enum the value of an
//!   `EnumParameter`.
//! - `#[derive(TypeScript)]` gives a type the TypeScript of what
//!   `serde_json` writes for it, for the web UI.
//!
//! # Declarative Parameters
//!
//! Parameters are defined entirely through attributes; the macro generates
//! the `Default` impl:
//!
//! ```ignore
//! use beamer::prelude::*;
//!
//! #[beamer::export]
//! #[beamer::model]
//! pub struct GainModel {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     pub gain: FloatParameter,
//!
//!     #[parameter(id = "bypass", bypass)]
//!     pub bypass: BoolParameter,
//! }
//! ```
//!
//! # Flat Visual Grouping
//!
//! Use `group = "..."` for visual grouping in the DAW without nested groups:
//!
//! ```ignore
//! #[beamer::model]
//! pub struct SynthModel {
//!     // Filter parameters, grouped visually in the DAW.
//!     #[parameter(id = "cutoff", name = "Cutoff", group = "Filter", ...)]
//!     pub cutoff: FloatParameter,
//!
//!     #[parameter(id = "reso", name = "Resonance", group = "Filter", ...)]
//!     pub resonance: FloatParameter,
//!
//!     // Output parameters, a different visual group.
//!     #[parameter(id = "gain", name = "Gain", group = "Output", ...)]
//!     pub gain: FloatParameter,
//! }
//!
//! // Access is flat: model.cutoff, model.resonance, model.gain.
//! // The DAW shows them in collapsible "Filter" and "Output" groups.
//! ```

use proc_macro::TokenStream;

mod codegen;
mod config_file;
mod enum_parameter;
mod handler;
mod init;
mod ir;
mod model_attribute;
mod parse;
mod range_eval;
mod type_script;
mod validate;

/// Mark a struct as the model of a plugin: what every thread of a plugin
/// instance shares.
///
/// The attribute generates:
/// - the `Parameters` trait implementation (count, iter, by_id, groups,
///   smoothing), which also makes the struct a `ParameterStore` for the
///   host
/// - the `Saved` trait implementation (parameters and `State` fields,
///   saving and loading)
/// - the `WebViewFields` trait implementation (where the event queue of
///   the web UI is, the connection of the declared senders and events to
///   it and the ids and types of the page's model types)
/// - the `WebViewDispatch` trait implementation, which passes the calls and
///   events of the web UI to the model's `WebViewHandler` when it has one
/// - the `ParameterGroups` trait implementation
/// - `Default`
/// - the ids of every parameter (`assign_ids`), with a compile error for two
///   ids in a struct that are equal or hash to the same value
///
/// The plugin then implements `Model` for the struct: how it creates its
/// processor and its buses and MIDI behavior. A plugin whose web UI calls
/// `invoke()` or emits events also implements `WebViewHandler` for it.
///
/// # Field Attributes
///
/// ## `#[parameter(...)]`
/// - `id = "..."` (required): string ID. A parameter is known by its path:
///   this id behind the names of the nested fields it sits in
///   (`osc1/attack`). The hash of the path is the u32 that hosts see.
/// - `name = "..."`: display name
/// - `default = <value>`: default value (float, int or bool)
/// - `range = start..=end`: value range (for FloatParameter/IntParameter)
/// - `kind = "..."`: unit type: db, db_log, db_log_offset, hz, ms, seconds,
///   percent, pan, ratio, linear, semitones, note_number
/// - `short_name = "..."`: short name for constrained UIs
/// - `smoothing = "exp:5.0"`: parameter smoothing (exp or linear)
/// - `bypass`: mark as the bypass parameter (BoolParameter only). A host's
///   own bypass button sets it, so the plugin keeps processing (and
///   receiving MIDI) while bypassed; see `BoolParameter::bypass()`
/// - `group = "..."`: visual grouping in the DAW without a nested group
///
/// The generated `Default` takes each parameter from these: every parameter
/// needs a name, a default and a range (or a kind with one), unless the
/// struct uses `no_default`.
///
/// ## `#[nested(group = "...")]`
/// A field that holds a `#[beamer::group]` struct, shown to the host as a
/// group with this name.
///
/// ## `#[state(id = "...")]`
/// A `State<T>` field: a value that is saved with the plugin state under
/// this id. Ids share one namespace with parameter ids.
///
/// ## `#[value(id = "...")]`, `#[stream(id = "...")]`, `#[event(id = "...")]`
/// A sender or an event for the web UI: a `WebViewValue<T>` for the newest
/// value from the audio thread, a `WebViewStream<T>` for every value in
/// order, a `WebViewEvent<T>` for a notification from a host or UI thread.
/// The id is the name the page knows it by, behind the names of the nested
/// fields it sits in. The three share one namespace of their own. The
/// model connects them to its `WebViewEventQueue` when it assigns the ids;
/// `prepare()` makes the senders from them (`self.meters.sender(initial)`).
///
/// ## Other fields
/// A `WebViewEventQueue` field is the event queue of the web UI. A field
/// without an attribute (a `Handover`, a cache) is not part of the
/// parameters or the saved state; the generated `Default` initializes it
/// with its own default.
///
/// A field cannot have `#[cfg]`: the attribute runs before `#[cfg]` is
/// applied to the fields. Put the whole struct behind the `#[cfg]`.
///
/// # Options
///
/// - `#[beamer::model(no_default)]`: generate no `Default`, for a model
///   with a plain field that has none or a parameter set up by hand. Write
///   `Default` yourself and call `assign_ids("")` on the value before
///   returning it, also `set_group_ids()` when it has nested or flat groups.
///
/// # Example
///
/// ```ignore
/// #[beamer::export]
/// #[beamer::model]
/// pub struct GainModel {
///     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
///     pub gain: FloatParameter,
///
///     #[nested(group = "Output")]
///     pub output: OutputGroup,
/// }
///
/// impl Model for GainModel {
///     type Setup = ();
///     type Processor = GainProcessor;
///
///     fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
///         GainProcessor { model: self }
///     }
/// }
/// ```
///
/// `#[beamer::export]` and `#[beamer::model]` can be written in either
/// order.
#[proc_macro_attribute]
pub fn model(attr: TokenStream, item: TokenStream) -> TokenStream {
    model_attribute::expand(model_attribute::Kind::Model, attr.into(), item.into()).into()
}

/// Answer the web UI from an impl block of the model: `invoke()` calls and
/// the events the page emits.
///
/// Generates `WebViewHandler` from the methods marked with an attribute,
/// with their types in the page's model types. Methods without one stay
/// helpers of the block.
///
/// - `#[method(id = "...")]`: the page calls it with
///   `invoke("id", ...arguments)`. The arguments after `&self` are the
///   positional arguments of `invoke()`, read with serde into owned values.
///   The result is what the promise resolves to; a `Result` rejects it with
///   the error's text.
/// - `#[page_event(id = "...")]`: the page sends it with
///   `emit("id", data)`. Its one argument after `&self` is the data, none
///   for an event without data. It returns nothing.
///
/// The ids are written as the page writes them (camelCase). Each type of an
/// argument, a result or an event's data needs `TypeScript`
/// (`#[derive(TypeScript)]` next to `Serialize` and `Deserialize`) for a
/// page with generated model types. Both are called on the main thread,
/// never on the audio thread.
///
/// A `Result` is written `Result<T, E>` in the signature, so that the
/// page's types read `T` from it: under another name (an alias) it still
/// rejects the promise, but the page's types cannot see its value. A
/// `#[cfg]` on a marked function applies to its answer and its types too.
/// On a generic model the block's bounds are the struct's own: the
/// dispatch does not find a handler that holds for some types only.
///
/// # Example
///
/// ```ignore
/// #[beamer::handler]
/// impl SamplerModel {
///     /// The name of the loaded sample.
///     #[method(id = "sampleName")]
///     fn sample_name(&self) -> String {
///         (*self.sample_name.get()).clone()
///     }
///
///     /// Load the sample at `path`.
///     #[method(id = "loadSample")]
///     fn load_sample(&self, path: String) -> Result<(), String> {
///         self.loader.load(&path)
///     }
///
///     /// A pad the page played.
///     #[page_event(id = "padHit")]
///     fn pad_hit(&self, hit: PadHit) {
///         self.pads.push(hit);
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    handler::expand(attr.into(), item.into()).into()
}

/// Mark a struct as a nested group of a model: a part of it that the host
/// shows as a group.
///
/// The model holds it in a field with `#[nested(group = "...")]`. It takes
/// the same field attributes and options as [`macro@model`] and generates
/// the same code, except `WebViewDispatch`: the calls of the web UI go to
/// the model.
///
/// # Example
///
/// ```ignore
/// #[beamer::group]
/// pub struct FilterGroup {
///     #[parameter(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz")]
///     pub cutoff: FloatParameter,
/// }
/// ```
#[proc_macro_attribute]
pub fn group(attr: TokenStream, item: TokenStream) -> TokenStream {
    model_attribute::expand(model_attribute::Kind::Group, attr.into(), item.into()).into()
}

/// Derive macro for implementing `EnumParameterValue` trait on enums.
///
/// This macro generates the `EnumParameterValue` implementation that allows
/// an enum to be used with `EnumParameter<E>` in models and their nested groups.
///
/// # Requirements
///
/// - The type must be an enum
/// - All variants must be unit variants (no fields)
/// - The enum must also derive `Copy`, `Clone` and `PartialEq`
///
/// # Attributes
///
/// - `#[name = "..."]` - Optional display name for a variant. If not specified,
///   the variant identifier is used as the display name.
/// - `#[default]` - Mark a variant as the default. If not specified, the first
///   variant is used. Only one variant can be marked as default.
///
/// # Example
///
/// ```ignore
/// use beamer::EnumParameter;
///
/// #[derive(Copy, Clone, PartialEq, EnumParameter)]
/// pub enum FilterType {
///     #[name = "Low Pass"]
///     LowPass,
///     #[default]
///     #[name = "High Pass"]
///     HighPass,
///     #[name = "Band Pass"]
///     BandPass,
///     Notch,  // Uses "Notch" as display name
/// }
///
/// // Now FilterType can be used with EnumParameter:
/// #[beamer::group]
/// pub struct FilterGroup {
///     #[parameter(id = "filter_type", name = "Filter Type")]
///     pub filter_type: EnumParameter<FilterType>,
/// }
/// ```
#[proc_macro_derive(EnumParameter, attributes(name, default))]
pub fn derive_enum_parameter(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    match enum_parameter::derive_enum_parameter_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derive macro for the `TypeScript` trait: the TypeScript type of what
/// `serde_json` writes for a struct or enum, for the file that describes a
/// model to its web UI.
///
/// The derive reads the serde attributes with serde's own parser, so
/// renames, `skip`, `flatten`, `transparent` and the four representations
/// of an enum (externally tagged, `tag`, `tag` with `content`, `untagged`)
/// come out as serde writes them. A field with `skip_serializing_if` is
/// optional (`name?: T`). A type without generics becomes a named
/// declaration with its doc comments as JSDoc; a generic type is written
/// inline at each use and needs `TypeScript` for its type parameters.
///
/// # Attributes
///
/// - `#[type_script(rename = "Name")]` on the type: the TypeScript name,
///   the Rust name by default. Two Rust types under one name are an error
///   of the generated file.
/// - `#[type_script(type = "...")]` on the type or a field: TypeScript
///   written by hand in place of the derived type.
/// - `#[type_script(crate = "path")]` on the type: the path of
///   beamer-webview, `::beamer::webview` by default.
///
/// # Refused serde attributes
///
/// The web UI sends back the values it gets, so Rust has to read every
/// value it writes. The derive refuses a different name per direction
/// (`rename(deserialize = ...)`, `rename_all(deserialize = ...)`),
/// `skip_serializing` or `skip_serializing_if` without `default` and
/// `skip_deserializing` (`skip` is fine). An `Option` field with
/// `skip_serializing_if = "Option::is_none"` needs no `default`: serde
/// reads a missing `Option` as `None`. A tuple type keeps its length, so
/// its fields take no `skip_serializing_if`. It also refuses the attributes
/// that change the shape where it cannot see it (`with`,
/// `serialize_with`, `deserialize_with`, `getter`, `into`, `from`,
/// `try_from`, `remote`), unless `#[type_script(type = "...")]` is on that
/// field or type.
///
/// # Example
///
/// ```ignore
/// use beamer::prelude::*;
/// use beamer::serde::Serialize;
///
/// /// The levels as the editor's meters show them.
/// #[derive(Serialize, TypeScript)]
/// #[serde(crate = "beamer::serde", rename_all = "camelCase")]
/// pub struct Meters {
///     /// The held gain reduction in dB, a positive number.
///     pub reduction_db: f32,
/// }
/// ```
///
/// declares
///
/// ```ts
/// /** The levels as the editor's meters show them. */
/// export interface Meters {
///   /** The held gain reduction in dB, a positive number. */
///   reductionDb: number;
/// }
/// ```
#[proc_macro_derive(TypeScript, attributes(type_script))]
pub fn derive_type_script(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    match type_script::derive_type_script_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Attribute macro that generates plugin configuration and entry points.
///
/// Place `#[beamer::export]` on your model struct to automatically generate:
/// - `pub static CONFIG: Config` from `Config.toml` in the crate root
/// - `export_plugin!` call with the correct arguments
///
/// # Requirements
///
/// - A `Config.toml` file must exist in the crate root (next to `Cargo.toml`)
/// - The struct must be marked with `#[beamer::model]` and implement `Model`
///
/// The factory presets of the plugin are `Model::PRESETS`.
///
/// # Example
///
/// ```ignore
/// use beamer::prelude::*;
///
/// #[beamer::export]
/// #[beamer::model]
/// pub struct GainModel {
///     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
///     pub gain: FloatParameter,
/// }
///
/// impl Model for GainModel { /* ... */ }
/// // No CONFIG static or export_plugin! needed!
/// ```
#[proc_macro_attribute]
pub fn export(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        let attr = proc_macro2::TokenStream::from(attr);
        return syn::Error::new_spanned(attr, "#[beamer::export] takes no arguments")
            .to_compile_error()
            .into();
    }

    let input = syn::parse_macro_input!(item as syn::DeriveInput);
    let struct_ident = input.ident.clone();

    match init::export_impl(struct_ident) {
        Ok(generated) => {
            let original: proc_macro2::TokenStream = {
                let ts: TokenStream = quote::quote! { #input }.into();
                ts.into()
            };
            let combined = quote::quote! {
                #original
                #generated
            };
            combined.into()
        }
        Err(err) => {
            let err = syn::Error::new(input.ident.span(), err);
            let original: proc_macro2::TokenStream = {
                let ts: TokenStream = quote::quote! { #input }.into();
                ts.into()
            };
            let error_tokens = err.to_compile_error();
            let combined = quote::quote! {
                #original
                #error_tokens
            };
            combined.into()
        }
    }
}
