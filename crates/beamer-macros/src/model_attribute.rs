//! The `#[beamer::model]` and `#[beamer::group]` attributes.
//!
//! Both generate the same code: `Parameters`, `Saved`, `WebViewEvents`,
//! `ParameterGroups` and `Default`. They differ in what
//! they say about the struct: `model` marks the model of a plugin, `group`
//! a nested part of it that the host shows as a group.
//!
//! An attribute macro is not a derive, so the field attributes
//! (`#[parameter]`, `#[state]`, `#[nested]`) are not registered for it. It
//! reads them and then removes them from the struct it emits.
//!
//! It also runs before `#[cfg]` is applied to the fields, so it cannot tell
//! whether a field with `#[cfg]` exists and refuses one. The whole struct
//! can be behind a `#[cfg]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields};

use crate::parse::FIELD_ATTRIBUTES;
use crate::{codegen, parse, validate};

/// Which of the two attributes is expanded.
#[derive(Clone, Copy)]
pub enum Kind {
    Model,
    Group,
}

impl Kind {
    /// How the attribute is written in messages.
    fn name(self) -> &'static str {
        match self {
            Kind::Model => "#[beamer::model]",
            Kind::Group => "#[beamer::group]",
        }
    }
}

/// Expand `#[beamer::model]` or `#[beamer::group]` on a struct.
///
/// On an error the struct is still emitted, without its field attributes,
/// so that the error is the only one the compiler reports.
pub fn expand(kind: Kind, attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input: DeriveInput = match syn::parse2(item.clone()) {
        Ok(input) => input,
        Err(_) => {
            let error = syn::Error::new_spanned(&item, format!("{} goes on a struct", kind.name()));
            let error = error.to_compile_error();
            return quote! {
                #item
                #error
            };
        }
    };

    let generated = generate(kind, attr, &input).unwrap_or_else(|error| error.to_compile_error());
    remove_field_attributes(&mut input);

    quote! {
        #input
        #generated
    }
}

/// The trait implementations for the struct.
fn generate(kind: Kind, attr: TokenStream, input: &DeriveInput) -> syn::Result<TokenStream> {
    let options = Options::parse(kind, attr)?;
    refuse_cfg_fields(kind, input)?;
    let mut ir = parse::parse(input.clone(), kind.name())?;
    validate::validate(&ir)?;
    if options.no_default {
        ir.generate_default = false;
    } else if let Some(field) = ir.parameter_fields().find(|field| !field.has_declarative_attributes()) {
        return Err(syn::Error::new(
            field.span,
            format!(
                "`{}` lacks what the generated `Default` needs (name, default and a range or \
                 a kind with one); add them, or write `Default` yourself with {}",
                field.field_name,
                kind.name().replace(']', "(no_default)]")
            ),
        ));
    }
    Ok(codegen::generate(&ir))
}

/// Refuse a field with `#[cfg]` or `#[cfg_attr]`: the attribute runs before
/// they are applied, so the generated code would name a field that may not
/// exist.
fn refuse_cfg_fields(kind: Kind, input: &DeriveInput) -> syn::Result<()> {
    let Data::Struct(data) = &input.data else {
        return Ok(());
    };
    for field in &data.fields {
        if let Some(attr) = field
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
        {
            return Err(syn::Error::new_spanned(
                attr,
                format!(
                    "{} cannot see whether a field with #[cfg] exists; \
                     put the whole struct behind the #[cfg] instead",
                    kind.name()
                ),
            ));
        }
    }
    Ok(())
}

/// What can be written between the parentheses of the attribute.
struct Options {
    /// `no_default`: the struct writes its own `Default`.
    no_default: bool,
}

impl Options {
    fn parse(kind: Kind, attr: TokenStream) -> syn::Result<Self> {
        let mut options = Options { no_default: false };
        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("no_default") {
                options.no_default = true;
                Ok(())
            } else {
                Err(meta.error(format!(
                    "unknown option of {}; the only one is `no_default`",
                    kind.name()
                )))
            }
        });
        syn::parse::Parser::parse2(parser, attr)?;
        Ok(options)
    }
}

/// Remove the field attributes, which no derive registers here.
fn remove_field_attributes(input: &mut DeriveInput) {
    let Data::Struct(data) = &mut input.data else {
        return;
    };
    let Fields::Named(fields) = &mut data.fields else {
        return;
    };
    for field in &mut fields.named {
        field
            .attrs
            .retain(|attr| !FIELD_ATTRIBUTES.iter().any(|name| attr.path().is_ident(name)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names of the attributes on each field.
    fn attribute_names(input: &DeriveInput) -> Vec<Vec<String>> {
        let Data::Struct(data) = &input.data else {
            panic!("a struct");
        };
        data.fields
            .iter()
            .map(|field| {
                field
                    .attrs
                    .iter()
                    .map(|attr| quote!(#attr).to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn test_only_the_field_attributes_are_removed() {
        let mut input: DeriveInput = syn::parse_quote! {
            struct Model {
                /// The gain.
                #[parameter(id = "gain")]
                #[allow(dead_code)]
                gain: FloatParameter,
                #[state(id = "zoom")]
                zoom: State<u32>,
                #[nested(group = "Layer")]
                layer: LayerGroup,
                #[serde(skip)]
                other: u32,
            }
        };
        remove_field_attributes(&mut input);

        assert_eq!(
            attribute_names(&input),
            [
                vec![String::from("# [doc = r\" The gain.\"]"), String::from("# [allow (dead_code)]")],
                vec![],
                vec![],
                vec![String::from("# [serde (skip)]")],
            ]
        );
    }

    #[test]
    fn test_options_take_no_default_only() {
        assert!(!Options::parse(Kind::Model, quote!()).expect("no options").no_default);
        assert!(Options::parse(Kind::Model, quote!(no_default)).expect("one option").no_default);

        let error = Options::parse(Kind::Group, quote!(no_defaults)).err().expect("an error");
        assert_eq!(
            error.to_string(),
            "unknown option of #[beamer::group]; the only one is `no_default`"
        );
    }

    #[test]
    fn test_field_with_cfg_is_refused() {
        let input: DeriveInput = syn::parse_quote! {
            struct Model {
                #[cfg(feature = "sidechain")]
                #[parameter(id = "sidechain")]
                sidechain: FloatParameter,
            }
        };
        let error = refuse_cfg_fields(Kind::Model, &input).expect_err("an error");
        assert!(error.to_string().starts_with("#[beamer::model] cannot see whether a field with #[cfg] exists"));
    }

    #[test]
    fn test_incomplete_parameter_without_no_default_is_an_error() {
        let input: DeriveInput = syn::parse_quote! {
            struct Model {
                #[parameter(id = "gain")]
                gain: FloatParameter,
            }
        };
        let error = generate(Kind::Model, quote!(), &input).expect_err("an error");
        assert!(error.to_string().contains("#[beamer::model(no_default)]"));

        assert!(generate(Kind::Model, quote!(no_default), &input).is_ok());
    }
}
