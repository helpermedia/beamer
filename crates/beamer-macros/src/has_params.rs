//! Derive macro for the `HasParams` trait.
//!
//! This module provides the `#[derive(HasParams)]` macro that automatically
//! implements the `HasParams` trait for structs with a `#[params]` field.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Ident, Type};

/// Derive the `HasParams` trait for a struct.
///
/// Looks for a field marked with `#[params]` and generates the implementation.
pub fn derive_has_params_impl(input: DeriveInput) -> syn::Result<TokenStream> {
    let struct_name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Only support structs
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    struct_name,
                    "HasParams can only be derived for structs with named fields",
                ));
            }
            Fields::Unit => {
                return Err(syn::Error::new_spanned(
                    struct_name,
                    "HasParams cannot be derived for unit structs",
                ));
            }
        },
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                struct_name,
                "HasParams can only be derived for structs, not enums",
            ));
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                struct_name,
                "HasParams can only be derived for structs, not unions",
            ));
        }
    };

    // Find the field marked with #[params]
    let mut params_field: Option<(&Ident, &Type)> = None;

    for field in fields {
        let has_params_attr = field.attrs.iter().any(|attr| attr.path().is_ident("params"));

        if has_params_attr {
            if params_field.is_some() {
                return Err(syn::Error::new_spanned(
                    field,
                    "Only one field can be marked with #[params]",
                ));
            }

            let field_ident = field
                .ident
                .as_ref()
                .expect("Named fields must have identifiers");
            params_field = Some((field_ident, &field.ty));
        }
    }

    let (field_name, field_type) = params_field.ok_or_else(|| {
        syn::Error::new_spanned(
            struct_name,
            "No field marked with #[params]. Add #[params] to the field containing your parameters.\n\
             Example:\n\
             #[derive(HasParams)]\n\
             struct MyPlugin {\n\
                 #[params]\n\
                 params: MyParams,\n\
             }",
        )
    })?;

    Ok(quote! {
        impl #impl_generics ::beamer::core::plugin::HasParams for #struct_name #ty_generics #where_clause {
            type Params = #field_type;

            fn params(&self) -> &Self::Params {
                &self.#field_name
            }

            fn params_mut(&mut self) -> &mut Self::Params {
                &mut self.#field_name
            }
        }
    })
}
