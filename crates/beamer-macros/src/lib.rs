//! Derive macros for the Beamer audio plugin framework.
//!
//! This crate provides the `#[derive(Parameters)]` macro for generating parameter
//! trait implementations automatically.
//!
//! # Declarative Parameter Descriptor
//!
//! Parameters can be defined entirely through attributes - the macro generates
//! the `Default` impl automatically:
//!
//! ```ignore
//! use beamer::prelude::*;
//!
//! #[derive(Parameters)]
//! pub struct GainParameters {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     pub gain: FloatParameter,
//!
//!     #[parameter(id = "bypass", bypass = true)]
//!     pub bypass: BoolParameter,
//! }
//!
//! // No manual Default impl needed - macro generates everything!
//! ```
//!
//! The macro generates implementations for both `Parameters` (high-level) and
//! `ParameterStore` (host integration) traits, plus `Default` when all required
//! attributes are present.
//!
//! # Flat Visual Grouping
//!
//! Use `group = "..."` for visual grouping in the DAW without nested structs:
//!
//! ```ignore
//! #[derive(Parameters)]
//! pub struct SynthParameters {
//!     // Filter parameters - grouped visually in DAW
//!     #[parameter(id = "cutoff", name = "Cutoff", group = "Filter", ...)]
//!     pub cutoff: FloatParameter,
//!
//!     #[parameter(id = "reso", name = "Resonance", group = "Filter", ...)]
//!     pub resonance: FloatParameter,
//!
//!     // Output parameters - different visual group
//!     #[parameter(id = "gain", name = "Gain", group = "Output", ...)]
//!     pub gain: FloatParameter,
//! }
//!
//! // Access is flat: parameters.cutoff, parameters.resonance, parameters.gain
//! // But DAW shows them in collapsible "Filter" and "Output" groups
//! ```

use proc_macro::TokenStream;

mod codegen;
mod enum_parameter;
mod has_parameters;
mod ir;
mod parse;
mod presets;
mod range_eval;
mod validate;

/// Derive macro for implementing parameter traits.
///
/// This macro generates:
/// - `Parameters` trait implementation (count, iter, by_id, save_state, load_state)
/// - `ParameterStore` trait implementation (host integration)
/// - `Default` implementation (when declarative attributes are complete)
/// - Compile-time hash collision detection
///
/// # Attributes
///
/// ## Required
/// - `id = "..."` - String ID that gets hashed to u32.
///
/// ## Declarative (enables auto-generated Default)
/// - `name = "..."` - Display name
/// - `default = <value>` - Default value (float, int, or bool)
/// - `range = start..=end` - Value range (for FloatParameter/IntParameter)
/// - `kind = "..."` - Unit type: db, db_log, db_log_offset, hz, ms, seconds, percent, pan, ratio, linear, semitones
/// - `short_name = "..."` - Short name for constrained UIs
/// - `smoothing = "exp:5.0"` - Parameter smoothing (exp or linear)
/// - `bypass` - Mark as bypass parameter (BoolParameter only)
/// - `group = "..."` - Visual grouping in DAW without nested struct
///
/// ## Nested Groups
/// - `#[nested(group = "...")]` - For fields containing nested parameter structs
///
/// # Example
///
/// ```ignore
/// #[derive(Parameters)]
/// pub struct PluginParameters {
///     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
///     pub gain: FloatParameter,
///
///     #[parameter(id = "freq", name = "Frequency", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
///     pub frequency: FloatParameter,
///
///     #[nested(group = "Output")]
///     pub output: OutputParameters,
/// }
/// ```
#[proc_macro_derive(Parameters, attributes(parameter, nested))]
pub fn derive_parameters(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    match derive_parameters_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn derive_parameters_impl(input: syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ir = parse::parse(input)?;
    validate::validate(&ir)?;
    Ok(codegen::generate(&ir))
}

/// Derive macro for implementing `EnumParameterValue` trait on enums.
///
/// This macro generates the `EnumParameterValue` implementation that allows
/// an enum to be used with `EnumParameter<E>` in parameter structs.
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
/// #[derive(Parameters)]
/// pub struct FilterParameters {
///     #[parameter(id = "filter_type")]
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

/// Derive macro for implementing the `HasParameters` trait.
///
/// This macro generates the `HasParameters` implementation for structs that hold
/// parameter collections. It eliminates the boilerplate of implementing
/// `parameters()` and `parameters_mut()` on both Descriptor and Processor types.
///
/// # Usage
///
/// Mark the field containing your parameters with `#[parameters]`:
///
/// ```ignore
/// use beamer::prelude::*;
///
/// #[derive(Default, HasParameters)]
/// pub struct GainPlugin {
///     #[parameters]
///     parameters: GainParameters,
/// }
///
/// #[derive(HasParameters)]
/// pub struct GainProcessor {
///     #[parameters]
///     parameters: GainParameters,
///     // other processor-only fields...
/// }
/// ```
///
/// # Requirements
///
/// - The struct must have named fields
/// - Exactly one field must be marked with `#[parameters]`
/// - The marked field's type must implement `ParameterStore`, `ParameterGroups`, and `Parameters`
///
/// # What It Generates
///
/// ```ignore
/// impl HasParameters for GainPlugin {
///     type Parameters = GainParameters;
///
///     fn parameters(&self) -> &Self::Parameters {
///         &self.parameters
///     }
///
///     fn parameters_mut(&mut self) -> &mut Self::Parameters {
///         &mut self.parameters
///     }
/// }
/// ```
#[proc_macro_derive(HasParameters, attributes(parameters))]
pub fn derive_has_parameters(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    match has_parameters::derive_has_parameters_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Derive macro for implementing `FactoryPresets` trait on preset enums.
///
/// This macro generates the `FactoryPresets` implementation that allows
/// factory presets to be integrated into VST3 and AU plugin wrappers.
///
/// # Container Attribute
///
/// The enum must have a `#[preset(parameters = Type)]` attribute specifying
/// which parameter struct the presets apply to.
///
/// # Variant Attributes
///
/// Each variant must have a `#[preset(name = "...", values(...))]` attribute:
/// - `name = "..."` - Display name shown in the DAW's preset browser
/// - `values(param = value, ...)` - Parameter values using string IDs
///
/// Values are specified in plain units (dB, Hz, etc.) and automatically
/// converted to normalized values at runtime.
///
/// # Example
///
/// ```ignore
/// use beamer::Presets;
///
/// #[derive(Presets)]
/// #[preset(parameters = GainParameters)]
/// pub enum GainPresets {
///     #[preset(name = "Unity", values(gain = 0.0))]
///     Unity,
///
///     #[preset(name = "Quiet", values(gain = -12.0))]
///     Quiet,
///
///     #[preset(name = "Boost", values(gain = 6.0))]
///     Boost,
/// }
/// ```
///
/// # Sparse Presets
///
/// Presets can specify only some parameters - unspecified parameters keep
/// their current values when the preset is applied:
///
/// ```ignore
/// #[preset(name = "High Resonance", values(resonance = 0.9))]
/// HighResonance,  // Only changes resonance, leaves other params alone
/// ```
#[proc_macro_derive(Presets, attributes(preset))]
pub fn derive_presets(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as syn::DeriveInput);

    match presets::derive_presets_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}
