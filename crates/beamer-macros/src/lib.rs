//! Derive macros for the Beamer VST3 framework.
//!
//! This crate provides the `#[derive(Params)]` macro for generating parameter
//! trait implementations automatically.
//!
//! # Declarative Parameter Definition
//!
//! Parameters can be defined entirely through attributes - the macro generates
//! the `Default` impl automatically:
//!
//! ```ignore
//! use beamer::prelude::*;
//!
//! #[derive(Params)]
//! pub struct GainParams {
//!     #[param(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     pub gain: FloatParam,
//!
//!     #[param(id = "bypass", bypass = true)]
//!     pub bypass: BoolParam,
//! }
//!
//! // No manual Default impl needed - macro generates everything!
//! ```
//!
//! The macro generates implementations for both `Params` (new system) and
//! `Parameters` (VST3 integration) traits, plus `Default` when all required
//! attributes are present.
//!
//! # Flat Visual Grouping
//!
//! Use `group = "..."` for visual grouping in the DAW without nested structs:
//!
//! ```ignore
//! #[derive(Params)]
//! pub struct SynthParams {
//!     // Filter parameters - grouped visually in DAW
//!     #[param(id = "cutoff", name = "Cutoff", group = "Filter", ...)]
//!     pub cutoff: FloatParam,
//!
//!     #[param(id = "reso", name = "Resonance", group = "Filter", ...)]
//!     pub resonance: FloatParam,
//!
//!     // Output parameters - different visual group
//!     #[param(id = "gain", name = "Gain", group = "Output", ...)]
//!     pub gain: FloatParam,
//! }
//!
//! // Access is flat: params.cutoff, params.resonance, params.gain
//! // But DAW shows them in collapsible "Filter" and "Output" groups
//! ```

use proc_macro::TokenStream;

mod codegen;
mod enum_param;
mod ir;
mod parse;
mod range_eval;
mod validate;

/// Derive macro for implementing parameter traits.
///
/// This macro generates:
/// - `Params` trait implementation (count, iter, by_id, save_state, load_state)
/// - `Parameters` trait implementation (VST3 integration)
/// - `Default` implementation (when declarative attributes are complete)
/// - Compile-time hash collision detection
///
/// # Attributes
///
/// ## Required
/// - `id = "..."` - String ID that gets hashed to u32 for VST3.
///
/// ## Declarative (enables auto-generated Default)
/// - `name = "..."` - Display name
/// - `default = <value>` - Default value (float, int, or bool)
/// - `range = start..=end` - Value range (for FloatParam/IntParam)
/// - `kind = "..."` - Unit type: db, db_log, db_log_offset, hz, ms, seconds, percent, pan, ratio, linear, semitones
/// - `short_name = "..."` - Short name for constrained UIs
/// - `smoothing = "exp:5.0"` - Parameter smoothing (exp or linear)
/// - `bypass` - Mark as bypass parameter (BoolParam only)
/// - `group = "..."` - Visual grouping in DAW without nested struct
///
/// ## Nested Groups
/// - `#[nested(group = "...")]` - For fields containing nested parameter structs
///
/// # Example
///
/// ```ignore
/// #[derive(Params)]
/// pub struct PluginParams {
///     #[param(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
///     pub gain: FloatParam,
///
///     #[param(id = "freq", name = "Frequency", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
///     pub frequency: FloatParam,
///
///     #[nested(group = "Output")]
///     pub output: OutputParams,
/// }
/// ```
#[proc_macro_derive(Params, attributes(param, nested))]
pub fn derive_params(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    match derive_params_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn derive_params_impl(input: syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ir = parse::parse(input)?;
    validate::validate(&ir)?;
    Ok(codegen::generate(&ir))
}

/// Derive macro for implementing `EnumParamValue` trait on enums.
///
/// This macro generates the `EnumParamValue` implementation that allows
/// an enum to be used with `EnumParam<E>` in parameter structs.
///
/// # Requirements
///
/// - The type must be an enum
/// - All variants must be unit variants (no fields)
/// - The enum must also derive `Copy`, `Clone`, and `PartialEq`
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
/// use beamer::EnumParam;
///
/// #[derive(Copy, Clone, PartialEq, EnumParam)]
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
/// // Now FilterType can be used with EnumParam:
/// #[derive(Params)]
/// pub struct FilterParams {
///     #[param(id = "filter_type")]
///     pub filter_type: EnumParam<FilterType>,
/// }
/// ```
#[proc_macro_derive(EnumParam, attributes(name, default))]
pub fn derive_enum_param(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    match enum_param::derive_enum_param_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
