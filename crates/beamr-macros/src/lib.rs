//! Derive macros for the BEAMR VST3 framework.
//!
//! This crate provides the `#[derive(Params)]` macro for generating parameter
//! trait implementations automatically.
//!
//! # Example
//!
//! ```ignore
//! use beamr::prelude::*;
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
