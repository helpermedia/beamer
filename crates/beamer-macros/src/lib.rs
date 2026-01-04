//! Derive macros for the Beamer VST3 framework.
//!
//! This crate provides the `#[derive(Params)]` macro for generating parameter
//! trait implementations automatically.
//!
//! # Example
//!
//! ```ignore
//! use beamer::prelude::*;
//!
//! #[derive(Params)]
//! pub struct GainParams {
//!     #[param(id = "gain")]
//!     pub gain: FloatParam,
//! }
//!
//! impl Default for GainParams {
//!     fn default() -> Self {
//!         Self {
//!             gain: FloatParam::db("Gain", 0.0, -60.0..=12.0),
//!         }
//!     }
//! }
//! ```
//!
//! The macro generates implementations for both `Params` (new system) and
//! `Parameters` (VST3 integration) traits.

use proc_macro::TokenStream;

mod codegen;
mod enum_param;
mod fnv;
mod ir;
mod parse;
mod validate;

/// Derive macro for implementing parameter traits.
///
/// This macro generates:
/// - `Params` trait implementation (count, iter, by_id, save_state, load_state)
/// - `Parameters` trait implementation (VST3 integration)
/// - Compile-time hash collision detection
///
/// # Attributes
///
/// - `#[param(id = "...")]` - Required on every `FloatParam`, `IntParam`, or `BoolParam` field.
///   The ID is a string that gets hashed to a u32 for VST3 compatibility.
///
/// - `#[nested(group = "...")]` - Applied to fields containing nested parameter structs.
///   The group name is used for VST3 unit hierarchy.
///
/// # Example
///
/// ```ignore
/// #[derive(Params)]
/// pub struct PluginParams {
///     #[param(id = "gain")]
///     pub gain: FloatParam,
///
///     #[param(id = "freq")]
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
