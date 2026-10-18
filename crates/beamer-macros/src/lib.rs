//! Macros for the Beamer audio plugin framework.
//!
//! - `#[beamer::model]` marks the model of a plugin: the struct every thread
//!   shares, with its parameters, `State` fields and event queue.
//! - `#[beamer::group]` marks a nested group of a model.
//! - `#[beamer::export]` exports the model as a plugin, with its
//!   `Config.toml` and `Presets.toml`.
//! - `#[derive(EnumParameter)]` makes an enum the value of an
//!   `EnumParameter`.
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
mod init;
mod ir;
mod model_attribute;
mod parse;
mod range_eval;
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
/// - the `WebViewEvents` trait implementation (where the event queue of
///   the web UI is)
/// - the `ParameterGroups` trait implementation
/// - `Default`
/// - the ids of every parameter (`assign_ids`), and a compile error for two
///   ids in a struct that are equal or hash to the same value
///
/// The plugin then implements `Model` for the struct: how it creates its
/// processor, its buses and MIDI behavior and its answers to the web UI.
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
///   percent, pan, ratio, linear, semitones
/// - `short_name = "..."`: short name for constrained UIs
/// - `smoothing = "exp:5.0"`: parameter smoothing (exp or linear)
/// - `bypass`: mark as the bypass parameter (BoolParameter only)
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
///   returning it, and `set_group_ids()` when it has nested or flat groups.
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

/// Mark a struct as a nested group of a model: a part of it that the host
/// shows as a group.
///
/// The model holds it in a field with `#[nested(group = "...")]`. It takes
/// the same field attributes and options as [`macro@model`] and generates
/// the same code.
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

/// Attribute macro that generates plugin configuration and entry points.
///
/// Place `#[beamer::export]` on your model struct to automatically generate:
/// - `pub static CONFIG: Config` from `Config.toml` in the crate root
/// - Factory presets from `Presets.toml` (if present)
/// - `export_plugin!` call with the correct arguments
///
/// # Requirements
///
/// - A `Config.toml` file must exist in the crate root (next to `Cargo.toml`)
/// - The struct must be marked with `#[beamer::model]` and implement `Model`
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
pub fn export(_attr: TokenStream, item: TokenStream) -> TokenStream {
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
