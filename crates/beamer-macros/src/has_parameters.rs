//! Derive macro for the `HasParameters` trait.
//!
//! This module provides the `#[derive(HasParameters)]` macro that automatically
//! implements the `HasParameters` trait for structs with a `#[parameters]` field.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Type};

/// Derive the `HasParameters` trait for a struct.
///
/// Looks for a field marked with `#[parameters]` and generates the implementation.
pub fn derive_has_parameters_impl(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Only support structs
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    struct_name,
                    "HasParameters can only be derived for structs with named fields",
                ));
            }
            Fields::Unit => {
                return Err(syn::Error::new_spanned(
                    struct_name,
                    "HasParameters cannot be derived for unit structs",
                ));
            }
        },
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                struct_name,
                "HasParameters can only be derived for structs, not enums",
            ));
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                struct_name,
                "HasParameters can only be derived for structs, not unions",
            ));
        }
    };

    // Find the field marked with #[parameters]
    let mut parameters_field: Option<(&Ident, &Type)> = None;

    for field in fields {
        let has_parameters_attr = field.attrs.iter().any(|attr| attr.path().is_ident("parameters"));

        if has_parameters_attr {
            if parameters_field.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "Only one field can be marked with #[parameters]",
                ));
            }

            let field_ident = field
                .ident
                .as_ref()
                .expect("Named fields must have identifiers");
            parameters_field = Some((field_ident, &field.ty));
        }
    }

    let (field_name, field_type) = parameters_field.ok_or_else(|| {
        syn::Error::new_spanned(
            struct_name,
            "No field marked with #[parameters]. Add #[parameters] to the field containing your parameters.\n\
             Example:\n\
             #[derive(HasParameters)]\n\
             struct MyPlugin {\n\
                 #[parameters]\n\
                 parameters: MyParameters,\n\
             }",
        )
    })?;

    Ok(quote! {
        impl #impl_generics ::beamer::core::plugin::HasParameters for #struct_name #ty_generics #where_clause {
            type Parameters = #field_type;

            fn parameters(&self) -> &Self::Parameters {
                &self.#field_name
            }

            fn parameters_mut(&mut self) -> &mut Self::Parameters {
                &mut self.#field_name
            }
        }
    })
}
