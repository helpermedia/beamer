//! Derive macro for the `FactoryPresets` trait.
//!
//! This module provides the `#[derive(Presets)]` macro that generates
//! factory preset support for audio plugins.
//!
//! # Example
//!
//! ```ignore
//! #[derive(Presets)]
//! #[preset(parameters = GainParameters)]
//! pub enum GainPresets {
//!     #[preset(name = "Unity", values(gain = 0.0))]
//!     Unity,
//!
//!     #[preset(name = "Quiet", values(gain = -12.0))]
//!     Quiet,
//!
//!     #[preset(name = "Boost", values(gain = 6.0))]
//!     Boost,
//! }
//! ```

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::Parse;
use syn::{Data, DeriveInput, Fields};

use crate::range_eval::eval_literal_expr;

/// Computed FNV-1a hash constant.
const FNV_OFFSET_BASIS: u32 = 2166136261;
const FNV_PRIME: u32 = 16777619;

/// Compute FNV-1a hash at compile time.
fn fnv1a_hash(s: &str) -> u32 {
    let mut hash = FNV_OFFSET_BASIS;
    for byte in s.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// A parameter value within a preset.
struct PresetValueInfo {
    /// Parameter string ID (e.g., "gain")
    param_id: String,
    /// Plain value
    plain_value: f64,
}

/// Information about a single preset variant.
struct PresetVariantInfo {
    /// Display name for the preset
    name: String,
    /// Parameter values for this preset
    values: Vec<PresetValueInfo>,
}

/// Parse and generate FactoryPresets implementation for an enum.
pub fn derive_presets_impl(input: DeriveInput) -> syn::Result<TokenStream> {
    // Ensure it's an enum
    let data_enum = match &input.data {
        Data::Enum(e) => e,
        Data::Struct(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Presets)] only supports enums, not structs",
            ))
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Presets)] only supports enums, not unions",
            ))
        }
    };

    // Parse the #[preset(parameters = Type)] attribute on the enum
    let parameters_type = parse_parameters_attribute(&input.attrs)?;

    // Parse variants
    let mut presets = Vec::new();
    for variant in &data_enum.variants {
        // Ensure it's a unit variant
        match &variant.fields {
            Fields::Unit => {}
            Fields::Named(_) => {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[derive(Presets)] only supports unit variants (no fields)",
                ))
            }
            Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[derive(Presets)] only supports unit variants (no tuple fields)",
                ))
            }
        }

        // Parse the #[preset(name = "...", values(...))] attribute
        let preset_info = parse_preset_variant_attribute(&variant.attrs, &variant.ident)?;
        presets.push(preset_info);
    }

    if presets.is_empty() {
        return Err(syn::Error::new_spanned(
            &input,
            "#[derive(Presets)] requires at least one variant",
        ));
    }

    // Generate the implementation
    let enum_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let count = presets.len();

    // Generate info match arms
    let info_arms: Vec<TokenStream> = presets
        .iter()
        .enumerate()
        .map(|(idx, preset)| {
            let name = &preset.name;
            quote! {
                #idx => Some(::beamer::core::preset::PresetInfo { name: #name }),
            }
        })
        .collect();

    // Generate static arrays for each preset's values and the values match arms
    let mut values_statics = Vec::new();
    let mut values_arms = Vec::new();

    for (idx, preset) in presets.iter().enumerate() {
        let static_name = syn::Ident::new(
            &format!("PRESET_{}_VALUES", idx),
            proc_macro2::Span::call_site(),
        );

        let values: Vec<TokenStream> = preset
            .values
            .iter()
            .map(|v| {
                let hash = fnv1a_hash(&v.param_id);
                let plain = v.plain_value;
                quote! {
                    ::beamer::core::preset::PresetValue {
                        id: #hash,
                        plain_value: #plain,
                    }
                }
            })
            .collect();

        let values_count = values.len();

        values_statics.push(quote! {
            static #static_name: [::beamer::core::preset::PresetValue; #values_count] = [
                #(#values),*
            ];
        });

        values_arms.push(quote! {
            #idx => &#static_name,
        });
    }

    Ok(quote! {
        #(#values_statics)*

        impl #impl_generics ::beamer::core::preset::FactoryPresets for #enum_name #ty_generics #where_clause {
            type Parameters = #parameters_type;

            fn count() -> usize {
                #count
            }

            fn info(index: usize) -> Option<::beamer::core::preset::PresetInfo> {
                match index {
                    #(#info_arms)*
                    _ => None,
                }
            }

            fn values(index: usize) -> &'static [::beamer::core::preset::PresetValue] {
                match index {
                    #(#values_arms)*
                    _ => &[],
                }
            }
        }
    })
}

/// Parse the `#[preset(parameters = Type)]` attribute from the enum.
fn parse_parameters_attribute(attrs: &[syn::Attribute]) -> syn::Result<syn::Type> {
    for attr in attrs {
        if attr.path().is_ident("preset") {
            let mut parameters_type: Option<syn::Type> = None;

            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("parameters") {
                    let value = meta.value()?;
                    parameters_type = Some(value.parse()?);
                    Ok(())
                } else {
                    Err(meta.error("expected `parameters = Type`"))
                }
            })?;

            return parameters_type.ok_or_else(|| {
                syn::Error::new_spanned(attr, "expected #[preset(parameters = Type)]")
            });
        }
    }

    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "#[derive(Presets)] requires #[preset(parameters = YourParametersType)] attribute on the enum",
    ))
}

/// Parse the `#[preset(name = "...", values(...))]` attribute from a variant.
fn parse_preset_variant_attribute(
    attrs: &[syn::Attribute],
    variant_ident: &syn::Ident,
) -> syn::Result<PresetVariantInfo> {
    for attr in attrs {
        if attr.path().is_ident("preset") {
            let mut name: Option<String> = None;
            let mut values: Vec<PresetValueInfo> = Vec::new();

            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    let value = meta.value()?;
                    let lit: syn::LitStr = value.parse()?;
                    name = Some(lit.value());
                    Ok(())
                } else if meta.path.is_ident("values") {
                    // Parse values(...) syntax
                    let content;
                    syn::parenthesized!(content in meta.input);

                    // Parse comma-separated param = value pairs
                    let pairs: syn::punctuated::Punctuated<syn::ExprAssign, syn::Token![,]> =
                        content.parse_terminated(syn::ExprAssign::parse, syn::Token![,])?;

                    for pair in pairs {
                        // Left side should be an identifier (parameter name)
                        let param_id = match pair.left.as_ref() {
                            syn::Expr::Path(path) => {
                                if let Some(ident) = path.path.get_ident() {
                                    ident.to_string()
                                } else {
                                    return Err(syn::Error::new_spanned(
                                        &pair.left,
                                        "expected parameter identifier",
                                    ));
                                }
                            }
                            _ => {
                                return Err(syn::Error::new_spanned(
                                    &pair.left,
                                    "expected parameter identifier",
                                ));
                            }
                        };

                        // Right side should be a numeric literal
                        let plain_value = eval_literal_expr(&pair.right)?.as_f64();

                        values.push(PresetValueInfo {
                            param_id,
                            plain_value,
                        });
                    }

                    Ok(())
                } else {
                    Err(meta.error("expected `name` or `values`"))
                }
            })?;

            let name = name.ok_or_else(|| {
                syn::Error::new_spanned(
                    attr,
                    "preset requires `name = \"...\"` attribute",
                )
            })?;

            return Ok(PresetVariantInfo { name, values });
        }
    }

    Err(syn::Error::new_spanned(
        variant_ident,
        "variant requires #[preset(name = \"...\", values(...))] attribute",
    ))
}
