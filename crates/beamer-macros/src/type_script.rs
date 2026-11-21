//! Derive macro for the `TypeScript` trait.
//!
//! The derive generates the TypeScript type of what `serde_json` writes for
//! the type. It reads the serde attributes with serde's own parser
//! (`serde_derive_internals`), so every rename and tag rule is serde's.
//! It refuses the serde attributes under which Rust would not read back what
//! it writes plus those that change the shape where it cannot see it, unless
//! `#[type_script(type = "...")]` gives the TypeScript by hand.

use proc_macro2::{Ident, Span, TokenStream};
use quote::{quote, ToTokens};
use serde_derive_internals::ast::{Container, Data, Field, Style, Variant};
use serde_derive_internals::attr::{Default as SerdeDefault, TagType};
use serde_derive_internals::{Ctxt, Derive};
use syn::{DeriveInput, LitStr};

/// The module path segment that serde_derive_internals puts into the
/// `deserialize_with` it adds by itself for borrowed `Cow` fields, which
/// the derive must tell from one the author wrote.
const PRIVATE: &str = "__beamer_type_script_private";

/// The fix the errors about hidden shapes name.
const BY_HAND: &str = "write its TypeScript by hand with #[type_script(type = \"...\")]";

/// What `#[type_script(...)]` says on the type.
#[derive(Default)]
struct ContainerOptions {
    /// The path of the beamer-webview crate, `::beamer::webview` by default.
    crate_path: Option<syn::Path>,
    /// The TypeScript name, the Rust name by default.
    rename: Option<String>,
    /// TypeScript written by hand for the whole type.
    raw: Option<String>,
}

/// Parse and generate the `TypeScript` implementation of a struct or enum.
pub fn derive_type_script_impl(input: DeriveInput) -> syn::Result<TokenStream> {
    let options = container_options(&input)?;
    let field_types = field_options(&input)?;

    let cx = Ctxt::new();
    let private = Ident::new(PRIVATE, Span::call_site());
    let container = Container::from_ast(&cx, &input, Derive::Serialize, &private);
    cx.check()?;
    let Some(container) = container else {
        return Err(syn::Error::new_spanned(
            &input,
            "#[derive(TypeScript)] could not read this type",
        ));
    };

    let crate_path = options
        .crate_path
        .clone()
        .unwrap_or_else(|| syn::parse_quote!(::beamer::webview));
    let ts = quote!(#crate_path::type_script);

    let body = match &options.raw {
        Some(raw) => quote!(#ts::__private::raw(#raw)),
        None => {
            check(&container, &field_types)?;
            let generator = Generator {
                ts: &ts,
                field_types: &field_types,
            };
            generator.container(&container)
        }
    };

    let ident = &input.ident;
    let is_generic = input.generics.type_params().next().is_some()
        || input.generics.const_params().next().is_some();

    let mut generics = input.generics.clone();
    if is_generic {
        let where_clause = generics.make_where_clause();
        for parameter in input.generics.type_params() {
            let parameter = &parameter.ident;
            where_clause
                .predicates
                .push(syn::parse_quote!(#parameter: #ts::TypeScript));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let function_body = if is_generic {
        body
    } else {
        let name = options.rename.clone().unwrap_or_else(|| unraw(ident));
        let doc = option_str(doc_of(&input.attrs).as_deref());
        // The declaration is keyed by the type with every lifetime made
        // 'static, which is the same type to the generated file.
        let lifetimes = input.generics.lifetimes().map(|_| quote!('static));
        let key = if input.generics.lifetimes().next().is_some() {
            quote!(#ident<#(#lifetimes),*>)
        } else {
            quote!(Self)
        };
        quote! {
            types.named::<#key>(#name, #doc, |types| #body)
        }
    };

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #ts::TypeScript for #ident #ty_generics #where_clause {
            #[allow(unused_variables)]
            fn type_script(types: &mut #ts::TypeScriptTypes) -> #ts::TsType {
                #function_body
            }
        }
    })
}

// =============================================================================
// Options
// =============================================================================

/// Read `#[type_script(...)]` on the type; refuse it on variants.
fn container_options(input: &DeriveInput) -> syn::Result<ContainerOptions> {
    let mut options = ContainerOptions::default();
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("type_script"))
    {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate") {
                let value: LitStr = meta.value()?.parse()?;
                options.crate_path = Some(value.parse()?);
            } else if meta.path.is_ident("rename") {
                let value: LitStr = meta.value()?.parse()?;
                options.rename = Some(value.value());
            } else if meta.path.is_ident("type") {
                let value: LitStr = meta.value()?.parse()?;
                options.raw = Some(value.value());
            } else {
                return Err(meta.error(
                    "unknown #[type_script] option on a type: expected `crate`, `rename` or `type`",
                ));
            }
            Ok(())
        })?;
    }
    if let syn::Data::Enum(data) = &input.data {
        for variant in &data.variants {
            if let Some(attr) = variant
                .attrs
                .iter()
                .find(|attr| attr.path().is_ident("type_script"))
            {
                return Err(syn::Error::new_spanned(
                    attr,
                    "#[type_script] is not supported on a variant: put \
                     #[type_script(type = \"...\")] on a field or on the enum",
                ));
            }
        }
    }
    Ok(options)
}

/// The TypeScript written by hand for fields, by the address of the field.
struct FieldTypes(Vec<(*const syn::Field, String)>);

impl FieldTypes {
    /// The TypeScript written by hand for `field`.
    fn get(&self, field: &syn::Field) -> Option<&str> {
        self.0
            .iter()
            .find(|(address, _)| std::ptr::eq(*address, field))
            .map(|(_, raw)| raw.as_str())
    }
}

/// Read `#[type_script(type = "...")]` on every field.
fn field_options(input: &DeriveInput) -> syn::Result<FieldTypes> {
    let fields: Vec<&syn::Field> = match &input.data {
        syn::Data::Struct(data) => data.fields.iter().collect(),
        syn::Data::Enum(data) => data
            .variants
            .iter()
            .flat_map(|variant| variant.fields.iter())
            .collect(),
        syn::Data::Union(_) => Vec::new(),
    };
    let mut types = Vec::new();
    for field in fields {
        for attr in field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("type_script"))
        {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("type") {
                    let value: LitStr = meta.value()?.parse()?;
                    types.push((field as *const syn::Field, value.value()));
                    Ok(())
                } else {
                    Err(meta.error("unknown #[type_script] option on a field: expected `type`"))
                }
            })?;
        }
    }
    Ok(FieldTypes(types))
}

// =============================================================================
// Checks
// =============================================================================

/// Refuse the serde attributes the generated type cannot follow.
fn check(container: &Container, field_types: &FieldTypes) -> syn::Result<()> {
    let mut errors = Errors(None);
    let attrs = &container.attrs;

    let hidden: [(Option<&dyn ToTokens>, &str); 4] = [
        (attrs.type_into().map(|ty| ty as &dyn ToTokens), "into"),
        (attrs.type_from().map(|ty| ty as &dyn ToTokens), "from"),
        (
            attrs.type_try_from().map(|ty| ty as &dyn ToTokens),
            "try_from",
        ),
        (attrs.remote().map(|path| path as &dyn ToTokens), "remote"),
    ];
    for (span, attribute) in hidden {
        if let Some(span) = span {
            errors.push(syn::Error::new_spanned(
                span,
                format!(
                    "#[derive(TypeScript)] cannot see the shape that `#[serde({attribute} = ...)]` \
                     gives this type: {BY_HAND} on the type"
                ),
            ));
        }
    }

    match &container.data {
        Data::Struct(style, fields) => {
            let container_default = !matches!(attrs.default(), SerdeDefault::None);
            for field in fields {
                check_field(&mut errors, field, *style, container_default, field_types);
            }
        }
        Data::Enum(variants) => {
            for variant in variants {
                check_variant(&mut errors, attrs.tag(), variant, field_types);
            }
        }
    }
    errors.finish()
}

/// Errors collected into one.
struct Errors(Option<syn::Error>);

impl Errors {
    /// Add an error.
    fn push(&mut self, error: syn::Error) {
        match &mut self.0 {
            Some(errors) => errors.combine(error),
            None => self.0 = Some(error),
        }
    }

    /// The errors, if there were any.
    fn finish(self) -> syn::Result<()> {
        self.0.map_or(Ok(()), Err)
    }
}

/// Refuse the attributes of a variant the generated type cannot follow,
/// then check its fields.
fn check_variant(errors: &mut Errors, tag: &TagType, variant: &Variant, field_types: &FieldTypes) {
    let attrs = &variant.attrs;
    if matches!(tag, TagType::Internal { .. })
        && !attrs.untagged()
        && matches!(effective_style(variant), Style::Newtype)
        && option_argument(variant.fields[0].ty).is_some()
    {
        errors.push(syn::Error::new_spanned(
            &variant.ident,
            "serde cannot write an `Option` in a newtype variant of an enum with a `tag`: \
             give the variant named fields",
        ));
    }
    let written = !attrs.skip_serializing();
    if attrs.skip_deserializing() && written {
        errors.push(syn::Error::new_spanned(
            &variant.ident,
            "the web UI sends back the values it gets, so Rust has to read every variant it \
             writes: `skip_deserializing` refuses this one when it comes back. Use `skip` to \
             leave the variant out in both directions",
        ));
    }
    if written {
        check_name(errors, &variant.ident, attrs.name(), "variant");
    }
    if let Some((path, attribute)) = written_with(attrs.serialize_with(), attrs.deserialize_with())
    {
        errors.push(syn::Error::new_spanned(
            path,
            format!(
                "#[derive(TypeScript)] cannot see the shape that {attribute} gives this variant: \
                 {BY_HAND} on the enum"
            ),
        ));
    }
    if written {
        for field in &variant.fields {
            check_field(errors, field, variant.style, false, field_types);
        }
    }
}

/// Refuse the attributes of a field the generated type cannot follow.
fn check_field(
    errors: &mut Errors,
    field: &Field,
    style: Style,
    container_default: bool,
    field_types: &FieldTypes,
) {
    let attrs = &field.attrs;
    let span = field_span(field);
    let skipped_both = attrs.skip_serializing() && attrs.skip_deserializing();
    if skipped_both {
        return;
    }
    if attrs.skip_deserializing() {
        errors.push(syn::Error::new_spanned(
            &span,
            "the web UI sends back the values it gets, so Rust has to read every field it \
             writes: `skip_deserializing` drops this one when it comes back. Use `skip` to \
             leave the field out in both directions",
        ));
    }
    // Serde reads a missing `Option` field as `None`, so an `Option` it
    // leaves out when it is `None` needs no default. Not when a function
    // of its own reads the field: then it is required.
    let reads_missing_as_none = leaves_out_none(field)
        && option_argument(field.ty).is_some()
        && attrs
            .deserialize_with()
            .filter(|path| !is_implicit(path))
            .is_none();
    let has_default = container_default
        || !matches!(attrs.default(), SerdeDefault::None)
        || reads_missing_as_none;
    if !has_default {
        let attribute = if attrs.skip_serializing() {
            Some("skip_serializing")
        } else if attrs.skip_serializing_if().is_some() {
            Some("skip_serializing_if")
        } else {
            None
        };
        if let Some(attribute) = attribute {
            errors.push(syn::Error::new_spanned(
                &span,
                format!(
                    "the web UI sends back the values it gets, so Rust has to read them \
                     without the fields it leaves out: `{attribute}` needs \
                     `#[serde(default)]` on this field or on the struct"
                ),
            ));
        }
    }
    if !attrs.skip_serializing() && matches!(style, Style::Struct) {
        check_name(errors, &span, attrs.name(), "field");
    }
    if attrs.skip_serializing() && matches!(style, Style::Tuple | Style::Newtype) {
        errors.push(syn::Error::new_spanned(
            &span,
            "the web UI sends back the values it gets, but Rust reads the elements of a tuple \
             by their place: with `skip_serializing` the next element comes back in this one's \
             place. A newtype variant comes back as a unit variant. Use `skip` to leave the \
             field out in both directions",
        ));
    }
    if attrs.skip_serializing_if().is_some() && matches!(style, Style::Tuple) {
        errors.push(syn::Error::new_spanned(
            &span,
            "a tuple type has a fixed length, but `skip_serializing_if` leaves this element \
             out of the array: give the type named fields, or write its TypeScript by hand \
             with #[type_script(type = \"...\")] on the struct or the enum",
        ));
    }

    if field_types.get(field.original).is_some() {
        return;
    }
    if let Some((path, attribute)) = written_with(attrs.serialize_with(), attrs.deserialize_with())
    {
        errors.push(syn::Error::new_spanned(
            path,
            format!(
                "#[derive(TypeScript)] cannot see the shape that {attribute} gives this field: \
                 {BY_HAND} on the field"
            ),
        ));
    }
    if let Some(path) = attrs.getter() {
        errors.push(syn::Error::new_spanned(
            path,
            format!(
                "#[derive(TypeScript)] cannot see the shape that `getter` gives this field: \
                 {BY_HAND} on the field"
            ),
        ));
    }
}

/// The functions written for serde, with the attributes they come from:
/// `with` when there are both.
fn written_with<'a>(
    serialize: Option<&'a syn::ExprPath>,
    deserialize: Option<&'a syn::ExprPath>,
) -> Option<(&'a syn::ExprPath, &'static str)> {
    let serialize = serialize.filter(|path| !is_implicit(path));
    let deserialize = deserialize.filter(|path| !is_implicit(path));
    match (serialize, deserialize) {
        (Some(path), Some(_)) => {
            Some((path, "`with` (or `serialize_with` and `deserialize_with`)"))
        }
        (Some(path), None) => Some((path, "`serialize_with`")),
        (None, Some(path)) => Some((path, "`deserialize_with`")),
        (None, None) => None,
    }
}

/// Refuse a name that differs between writing and reading.
fn check_name(
    errors: &mut Errors,
    span: &dyn ToTokens,
    name: &serde_derive_internals::name::MultiName,
    what: &str,
) {
    let written = &name.serialize_name().value;
    let read = &name.deserialize_name().value;
    if written != read {
        errors.push(syn::Error::new_spanned(
            span,
            format!(
                "the web UI sends back the values it gets, so Rust has to read the names it \
                 writes: this {what} is written as `{written}` but read as `{read}`. Give \
                 `rename` or `rename_all` one name for both directions, without \
                 `serialize = ...` and `deserialize = ...`"
            ),
        ));
    }
}

/// What an error about a field points to: its name, or the whole field
/// when it has none.
fn field_span(field: &Field) -> TokenStream {
    match &field.original.ident {
        Some(ident) => ident.to_token_stream(),
        None => field.original.to_token_stream(),
    }
}

/// Whether serde_derive_internals added this path by itself (for a
/// borrowed `Cow`).
fn is_implicit(path: &&syn::ExprPath) -> bool {
    path.path
        .segments
        .iter()
        .nth(1)
        .is_some_and(|segment| segment.ident == PRIVATE)
}

// =============================================================================
// Generation
// =============================================================================

/// What generates the expression of the TypeScript type.
struct Generator<'a> {
    /// The path of the `type_script` module.
    ts: &'a TokenStream,
    /// The TypeScript written by hand for fields.
    field_types: &'a FieldTypes,
}

impl Generator<'_> {
    /// The type of the whole struct or enum.
    fn container(&self, container: &Container) -> TokenStream {
        let ts = self.ts;
        match &container.data {
            Data::Struct(style, fields) => {
                if container.attrs.transparent() {
                    if let Some(field) = fields.iter().find(|field| field.attrs.transparent()) {
                        return self.field_type(field);
                    }
                }
                match style {
                    Style::Struct => {
                        let mut tag = Vec::new();
                        if let TagType::Internal { tag: name } = container.attrs.tag() {
                            let value = &container.attrs.name().serialize_name().value;
                            tag.push(self.tag_field(name, value));
                        }
                        self.object(tag, fields)
                    }
                    Style::Tuple => self.tuple(fields),
                    Style::Newtype => self.field_type(&fields[0]),
                    Style::Unit => quote!(#ts::TsType::Keyword("null")),
                }
            }
            Data::Enum(variants) => {
                let members = variants
                    .iter()
                    .filter(|variant| !variant.attrs.skip_serializing())
                    .map(|variant| {
                        let ty = self.variant(container.attrs.tag(), variant);
                        let doc = option_str(doc_of(&variant.original.attrs).as_deref());
                        quote!(#ts::__private::member(#doc, #ty))
                    });
                quote!(#ts::TsType::Union(::std::vec![#(#members),*]))
            }
        }
    }

    /// The type of one variant in the representation of its enum.
    fn variant(&self, tag: &TagType, variant: &Variant) -> TokenStream {
        let ts = self.ts;
        let name = &variant.attrs.name().serialize_name().value;
        let untagged = TagType::None;
        let tag = if variant.attrs.untagged() {
            &untagged
        } else {
            tag
        };
        match (tag, effective_style(variant)) {
            (TagType::External, Style::Unit) => quote!(#ts::__private::literal(#name)),
            (TagType::External, _) => {
                let content = self.content(variant);
                quote!(#ts::TsType::Object(::std::vec![
                    #ts::__private::field(#name, ::core::option::Option::None, false, #content)
                ]))
            }
            (TagType::Internal { tag }, Style::Struct) => {
                let tag = self.tag_field(tag, name);
                self.object(vec![tag], &variant.fields)
            }
            (TagType::Internal { tag }, Style::Newtype) => {
                let tag = self.tag_field(tag, name);
                let content = self.field_type(&variant.fields[0]);
                quote!({
                    let content = #content;
                    #ts::__private::tagged_newtype(types, #tag, content)
                })
            }
            // Serde refuses tuple variants in an internally tagged enum.
            (TagType::Internal { tag }, _) | (TagType::Adjacent { tag, .. }, Style::Unit) => {
                let tag = self.tag_field(tag, name);
                quote!(#ts::TsType::Object(::std::vec![#tag]))
            }
            (
                TagType::Adjacent {
                    tag,
                    content: content_name,
                },
                _,
            ) => {
                let tag = self.tag_field(tag, name);
                let content = self.content(variant);
                quote!(#ts::TsType::Object(::std::vec![
                    #tag,
                    #ts::__private::field(#content_name, ::core::option::Option::None, false, #content)
                ]))
            }
            (TagType::None, Style::Unit) => quote!(#ts::TsType::Keyword("null")),
            (TagType::None, _) => self.content(variant),
        }
    }

    /// The content of a variant with fields, as an untagged enum writes it.
    fn content(&self, variant: &Variant) -> TokenStream {
        match effective_style(variant) {
            Style::Struct => self.object(Vec::new(), &variant.fields),
            Style::Tuple => self.tuple(&variant.fields),
            Style::Newtype => self.field_type(&variant.fields[0]),
            Style::Unit => {
                let ts = self.ts;
                quote!(#ts::TsType::Keyword("null"))
            }
        }
    }

    /// The field of a tag: its name and the name of the variant.
    fn tag_field(&self, tag: &str, value: &str) -> TokenStream {
        let ts = self.ts;
        quote!(#ts::__private::field(
            #tag,
            ::core::option::Option::None,
            false,
            #ts::__private::literal(#value)
        ))
    }

    /// Named fields, after the given ones: an object, or its intersection
    /// with flattened fields.
    fn object(&self, mut leading: Vec<TokenStream>, fields: &[Field]) -> TokenStream {
        let ts = self.ts;
        let mut flattened = Vec::new();
        for field in fields
            .iter()
            .filter(|field| !field.attrs.skip_serializing())
        {
            let ty = self.field_type(field);
            if field.attrs.flatten() {
                flattened.push(ty);
                continue;
            }
            let name = &field.attrs.name().serialize_name().value;
            let doc = option_str(doc_of(&field.original.attrs).as_deref());
            let optional = field.attrs.skip_serializing_if().is_some();
            // An `Option` that serde leaves out when it is `None` has the
            // type of what it holds: `null` only when that takes it, as
            // `Some(None)` does.
            let held = option_argument(field.ty).filter(|_| {
                leaves_out_none(field) && self.field_types.get(field.original).is_none()
            });
            let ty = match held {
                Some(held) => quote!(<#held as #ts::TypeScript>::type_script(types)),
                None => ty,
            };
            leading.push(quote!(#ts::__private::field(#name, #doc, #optional, #ty)));
        }
        if flattened.is_empty() {
            quote!(#ts::TsType::Object(::std::vec![#(#leading),*]))
        } else {
            quote!({
                let fields = ::std::vec![#(#leading),*];
                let flattened = ::std::vec![#(#flattened),*];
                #ts::__private::with_flattened(types, fields, flattened)
            })
        }
    }

    /// Unnamed fields as a tuple.
    fn tuple(&self, fields: &[Field]) -> TokenStream {
        let ts = self.ts;
        let elements = fields
            .iter()
            .filter(|field| !field.attrs.skip_serializing())
            .map(|field| self.field_type(field));
        quote!(#ts::TsType::Tuple(::std::vec![#(#elements),*]))
    }

    /// The type of a field: written by hand, or its Rust type's.
    fn field_type(&self, field: &Field) -> TokenStream {
        let ts = self.ts;
        match self.field_types.get(field.original) {
            Some(raw) => quote!(#ts::__private::raw(#raw)),
            None => {
                let ty = field.ty;
                quote!(<#ty as #ts::TypeScript>::type_script(types))
            }
        }
    }
}

/// The style serde writes a variant in: a newtype variant whose field it
/// skips is a unit variant.
fn effective_style(variant: &Variant) -> Style {
    match variant.style {
        Style::Newtype if variant.fields[0].attrs.skip_serializing() => Style::Unit,
        style => style,
    }
}

/// Whether serde leaves the field out when it is `None`:
/// `skip_serializing_if = "Option::is_none"` or another `is_none`.
fn leaves_out_none(field: &Field) -> bool {
    field
        .attrs
        .skip_serializing_if()
        .and_then(|path| path.path.segments.last())
        .is_some_and(|segment| segment.ident == "is_none")
}

/// The type an `Option` holds, when `ty` is one by its name.
fn option_argument(ty: &syn::Type) -> Option<&syn::Type> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    match arguments.args.first()? {
        syn::GenericArgument::Type(ty) if arguments.args.len() == 1 => Some(ty),
        _ => None,
    }
}

/// The doc comment of an item: its `///` lines without the one space after
/// the slashes, `None` when it has none.
fn doc_of(attrs: &[syn::Attribute]) -> Option<String> {
    let mut lines = Vec::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("doc")) {
        if let syn::Meta::NameValue(name_value) = &attr.meta {
            if let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(text),
                ..
            }) = &name_value.value
            {
                let text = text.value();
                for line in text.split('\n') {
                    let line = line.strip_prefix(' ').unwrap_or(line);
                    lines.push(line.trim_end().to_string());
                }
            }
        }
    }
    while lines.first().is_some_and(String::is_empty) {
        lines.remove(0);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// An optional string as tokens.
fn option_str(text: Option<&str>) -> TokenStream {
    match text {
        Some(text) => quote!(::core::option::Option::Some(#text)),
        None => quote!(::core::option::Option::None),
    }
}

/// The name of an identifier without `r#`.
fn unraw(ident: &Ident) -> String {
    let name = ident.to_string();
    match name.strip_prefix("r#") {
        Some(name) => name.to_string(),
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The errors of the derive for `input`, as one string.
    fn errors(input: TokenStream) -> String {
        let input: DeriveInput = syn::parse2(input).unwrap();
        match derive_type_script_impl(input) {
            Ok(_) => String::new(),
            Err(errors) => errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    #[test]
    fn test_plain_types_derive() {
        assert_eq!(errors(quote! { struct A { a: f32, b: Option<u8> } }), "");
        assert_eq!(errors(quote! { enum E { A, B(u8), C { c: bool } } }), "");
        assert_eq!(
            errors(quote! {
                #[serde(default)]
                struct D { #[serde(skip_serializing_if = "Option::is_none")] a: Option<u8> }
            }),
            ""
        );
        assert_eq!(
            errors(quote! { struct S { #[serde(skip)] cache: u8, #[serde(alias = "old")] a: u8 } }),
            ""
        );
        assert_eq!(
            errors(quote! { struct C<'a> { #[serde(borrow)] text: std::borrow::Cow<'a, str> } }),
            ""
        );
    }

    #[test]
    fn test_a_name_per_direction_is_refused() {
        let error = errors(quote! { struct A { #[serde(rename(deserialize = "b"))] a: u8 } });
        assert!(error.contains("written as `a` but read as `b`"), "{error}");
        let error = errors(quote! {
            #[serde(rename_all(serialize = "camelCase"))]
            struct A { long_name: u8 }
        });
        assert!(
            error.contains("written as `longName` but read as `long_name`"),
            "{error}"
        );
        let error = errors(quote! {
            #[serde(rename_all(deserialize = "lowercase"))]
            enum E { A }
        });
        assert!(
            error.contains("variant is written as `A` but read as `a`"),
            "{error}"
        );
        // The same rule both ways is one name.
        assert_eq!(
            errors(quote! {
                #[serde(rename_all(serialize = "camelCase", deserialize = "camelCase"))]
                struct A { long_name: u8 }
            }),
            ""
        );
    }

    #[test]
    fn test_skipping_one_direction_is_refused() {
        let error = errors(quote! { struct A { #[serde(skip_serializing)] a: u8 } });
        assert!(
            error.contains("`skip_serializing` needs `#[serde(default)]`"),
            "{error}"
        );
        let error = errors(quote! { struct A { #[serde(skip_serializing_if = "is_zero")] a: u8 } });
        assert!(
            error.contains("`skip_serializing_if` needs `#[serde(default)]`"),
            "{error}"
        );
        let error = errors(quote! { struct A { #[serde(skip_deserializing)] a: u8 } });
        assert!(
            error.contains("`skip_deserializing` drops this one"),
            "{error}"
        );
        let error = errors(quote! { enum E { #[serde(skip_deserializing)] A } });
        assert!(error.contains("every variant it writes"), "{error}");
        assert_eq!(
            errors(quote! { struct A { #[serde(skip_serializing, default)] a: u8 } }),
            ""
        );
        assert_eq!(
            errors(quote! { enum E { #[serde(skip_serializing)] A, B } }),
            ""
        );
    }

    #[test]
    fn test_hidden_shapes_are_refused_without_a_hand_written_type() {
        for attribute in ["with", "serialize_with", "deserialize_with"] {
            let attribute = Ident::new(attribute, Span::call_site());
            let error = errors(quote! { struct A { #[serde(#attribute = "x")] a: u8 } });
            assert!(
                error.contains("#[type_script(type = \"...\")] on the field"),
                "{error}"
            );
            assert_eq!(
                errors(quote! {
                    struct A { #[serde(#attribute = "x")] #[type_script(type = "string")] a: u8 }
                }),
                ""
            );
        }
        for attribute in ["into", "from", "try_from"] {
            let attribute = Ident::new(attribute, Span::call_site());
            let error = errors(quote! { #[serde(#attribute = "B")] struct A { a: u8 } });
            assert!(
                error.contains(&format!("`#[serde({attribute} = ...)]`")),
                "{error}"
            );
            assert!(error.contains("on the type"), "{error}");
            assert_eq!(
                errors(
                    quote! { #[serde(#attribute = "B")] #[type_script(type = "number")] struct A { a: u8 } }
                ),
                ""
            );
        }
        let error =
            errors(quote! { #[serde(remote = "B")] struct A { #[serde(getter = "B::a")] a: u8 } });
        assert!(error.contains("`#[serde(remote = ...)]`"), "{error}");
        assert!(error.contains("`getter`"), "{error}");
        let error = errors(quote! { enum E { #[serde(with = "x")] A(u8) } });
        assert!(error.contains("on the enum"), "{error}");
    }

    #[test]
    fn test_unknown_options_are_refused() {
        let error = errors(quote! { #[type_script(name = "B")] struct A; });
        assert!(
            error.contains("expected `crate`, `rename` or `type`"),
            "{error}"
        );
        let error = errors(quote! { struct A { #[type_script(rename = "b")] a: u8 } });
        assert!(error.contains("expected `type`"), "{error}");
        let error = errors(quote! { enum E { #[type_script(type = "x")] A } });
        assert!(error.contains("not supported on a variant"), "{error}");
    }

    #[test]
    fn test_doc_lines_lose_the_space_after_the_slashes() {
        let input: DeriveInput = syn::parse2(quote! {
            /// First line
            ///   indented.
            ///
            /// After a gap.
            struct A;
        })
        .unwrap();
        assert_eq!(
            doc_of(&input.attrs).unwrap(),
            "First line\n  indented.\n\nAfter a gap."
        );
    }
}
