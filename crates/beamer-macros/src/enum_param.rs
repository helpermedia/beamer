//! Derive macro for `EnumParamValue` trait.
//!
//! This module implements the `#[derive(EnumParam)]` macro that generates
//! the `EnumParamValue` trait implementation for enums, enabling them to
//! be used with `EnumParam<E>` in parameter structs.
//!
//! # Example
//!
//! ```ignore
//! #[derive(Copy, Clone, PartialEq, EnumParam)]
//! pub enum FilterType {
//!     #[name = "Low Pass"]
//!     LowPass,
//!     #[name = "High Pass"]
//!     HighPass,
//!     BandPass,  // Uses "BandPass" as display name
//! }
//! ```

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

/// Information about a single enum variant.
struct VariantInfo {
    /// The variant identifier (e.g., `LowPass`)
    ident: syn::Ident,
    /// The display name (from `#[name = "..."]` or the identifier)
    display_name: String,
    /// Whether this variant is marked as the default
    is_default: bool,
}

/// Parse and generate EnumParamValue implementation for an enum.
pub fn derive_enum_param_impl(input: DeriveInput) -> syn::Result<TokenStream> {
    // Ensure it's an enum
    let data_enum = match &input.data {
        Data::Enum(e) => e,
        Data::Struct(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(EnumParam)] only supports enums, not structs",
            ))
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(EnumParam)] only supports enums, not unions",
            ))
        }
    };

    // Parse variants
    let mut variants = Vec::new();
    for variant in &data_enum.variants {
        // Ensure it's a unit variant (no fields)
        match &variant.fields {
            Fields::Unit => {}
            Fields::Named(_) => {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[derive(EnumParam)] only supports unit variants (no fields)",
                ))
            }
            Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    variant,
                    "#[derive(EnumParam)] only supports unit variants (no tuple fields)",
                ))
            }
        }

        // Extract display name from #[name = "..."] attribute
        let display_name = extract_name_attribute(&variant.attrs)?
            .unwrap_or_else(|| variant.ident.to_string());

        // Check for #[default] attribute
        let is_default = has_default_attribute(&variant.attrs);

        variants.push(VariantInfo {
            ident: variant.ident.clone(),
            display_name,
            is_default,
        });
    }

    if variants.is_empty() {
        return Err(syn::Error::new_spanned(
            &input,
            "#[derive(EnumParam)] requires at least one variant",
        ));
    }

    // Find default variant (validate only one is marked)
    let default_indices: Vec<usize> = variants
        .iter()
        .enumerate()
        .filter(|(_, v)| v.is_default)
        .map(|(i, _)| i)
        .collect();

    if default_indices.len() > 1 {
        return Err(syn::Error::new_spanned(
            &input,
            "#[derive(EnumParam)] only one variant can be marked as #[default]",
        ));
    }

    // Default to first variant (index 0) if none specified
    let default_index = default_indices.first().copied().unwrap_or(0);

    // Generate the implementation
    let enum_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let count = variants.len();

    // Generate from_index match arms
    let from_index_arms: Vec<TokenStream> = variants
        .iter()
        .enumerate()
        .map(|(idx, v)| {
            let ident = &v.ident;
            quote! { #idx => Some(#enum_name::#ident), }
        })
        .collect();

    // Generate to_index match arms
    let to_index_arms: Vec<TokenStream> = variants
        .iter()
        .enumerate()
        .map(|(idx, v)| {
            let ident = &v.ident;
            quote! { #enum_name::#ident => #idx, }
        })
        .collect();

    // Generate name match arms
    let name_arms: Vec<TokenStream> = variants
        .iter()
        .enumerate()
        .map(|(idx, v)| {
            let name = &v.display_name;
            quote! { #idx => #name, }
        })
        .collect();

    // Generate names array
    let names_array: Vec<&str> = variants.iter().map(|v| v.display_name.as_str()).collect();

    // Get the default variant ident for default_value()
    let default_ident = &variants[default_index].ident;

    Ok(quote! {
        impl #impl_generics ::beamer::core::param_types::EnumParamValue for #enum_name #ty_generics #where_clause {
            const COUNT: usize = #count;
            const DEFAULT_INDEX: usize = #default_index;

            fn from_index(index: usize) -> Option<Self> {
                match index {
                    #(#from_index_arms)*
                    _ => None,
                }
            }

            fn to_index(self) -> usize {
                match self {
                    #(#to_index_arms)*
                }
            }

            fn default_value() -> Self {
                #enum_name::#default_ident
            }

            fn name(index: usize) -> &'static str {
                match index {
                    #(#name_arms)*
                    _ => "",
                }
            }

            fn names() -> &'static [&'static str] {
                &[#(#names_array),*]
            }
        }
    })
}

/// Extract the display name from a `#[name = "..."]` attribute.
fn extract_name_attribute(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    for attr in attrs {
        if attr.path().is_ident("name") {
            // Parse #[name = "..."] syntax
            let name_value: syn::MetaNameValue = attr.meta.require_name_value()?.clone();
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(lit_str),
                ..
            }) = &name_value.value
            {
                return Ok(Some(lit_str.value()));
            } else {
                return Err(syn::Error::new_spanned(
                    &name_value.value,
                    "expected string literal for #[name = \"...\"]",
                ));
            }
        }
    }
    Ok(None)
}

/// Check if a variant has the `#[default]` attribute.
fn has_default_attribute(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("default"))
}
