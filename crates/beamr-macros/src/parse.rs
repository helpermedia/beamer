//! AST parsing for the derive macro.
//!
//! This module transforms `syn::DeriveInput` into our intermediate representation.

use proc_macro2::Span;
use syn::{Data, DeriveInput, Field, Fields};

use crate::fnv::fnv1a_32;
use crate::ir::{FieldIR, NestedFieldIR, ParamFieldIR, ParamType, ParamsIR};

/// Parse a `DeriveInput` into our intermediate representation.
pub fn parse(input: DeriveInput) -> syn::Result<ParamsIR> {
    // Ensure it's a struct with named fields
    let data_struct = match &input.data {
        Data::Struct(s) => s,
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Params)] only supports structs, not enums",
            ))
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Params)] only supports structs, not unions",
            ))
        }
    };

    let fields = match &data_struct.fields {
        Fields::Named(named) => &named.named,
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Params)] only supports structs with named fields",
            ))
        }
        Fields::Unit => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Params)] requires at least one field",
            ))
        }
    };

    // Parse each field
    let mut parsed_fields = Vec::new();
    for field in fields {
        if let Some(field_ir) = parse_field(field)? {
            parsed_fields.push(field_ir);
        }
        // Fields without #[param] or #[nested] are silently ignored
    }

    if parsed_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            &input,
            "#[derive(Params)] requires at least one #[param] or #[nested] field",
        ));
    }

    // Assign sequential unit IDs to nested fields
    assign_unit_ids(&mut parsed_fields);

    Ok(ParamsIR {
        struct_name: input.ident.clone(),
        generics: input.generics.clone(),
        fields: parsed_fields,
        span: Span::call_site(),
    })
}

/// Parse a single field, returning None if it has no relevant attributes.
fn parse_field(field: &Field) -> syn::Result<Option<FieldIR>> {
    // Check for #[param] attribute
    for attr in &field.attrs {
        if attr.path().is_ident("param") {
            return parse_param_field(field, attr).map(|p| Some(FieldIR::Param(p)));
        }
        if attr.path().is_ident("nested") {
            return parse_nested_field(field, attr).map(|n| Some(FieldIR::Nested(n)));
        }
    }

    // Check if this field LOOKS like a parameter type but lacks the attribute
    if let Some(type_name) = extract_type_name(&field.ty) {
        if matches!(type_name.as_str(), "FloatParam" | "IntParam" | "BoolParam") {
            return Err(syn::Error::new_spanned(
                field,
                format!(
                    "{} field is missing #[param(id = \"...\")] attribute",
                    type_name
                ),
            ));
        }
    }

    Ok(None)
}

/// Parse a field with `#[param(id = "...")]` attribute.
fn parse_param_field(field: &Field, attr: &syn::Attribute) -> syn::Result<ParamFieldIR> {
    let field_name = field
        .ident
        .clone()
        .ok_or_else(|| syn::Error::new_spanned(field, "Field must have a name"))?;

    // Parse the attribute using syn 2.x API
    let mut string_id: Option<String> = None;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("id") {
            let value: syn::LitStr = meta.value()?.parse()?;
            string_id = Some(value.value());
            Ok(())
        } else {
            Err(meta.error("expected `id = \"...\"`"))
        }
    })?;

    let string_id = string_id.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            format!(
                "#[param] on field `{}` requires id attribute: #[param(id = \"...\")]",
                field_name
            ),
        )
    })?;

    // Determine parameter type
    let param_type = extract_param_type(&field.ty).ok_or_else(|| {
        syn::Error::new_spanned(
            &field.ty,
            "#[param] can only be used on FloatParam, IntParam, or BoolParam fields",
        )
    })?;

    // Compute hash
    let hash_id = fnv1a_32(&string_id);

    Ok(ParamFieldIR {
        field_name,
        param_type,
        string_id,
        hash_id,
        span: attr.path().segments[0].ident.span(),
    })
}

/// Parse a field with `#[nested(group = "...")]` attribute.
fn parse_nested_field(field: &Field, attr: &syn::Attribute) -> syn::Result<NestedFieldIR> {
    let field_name = field
        .ident
        .clone()
        .ok_or_else(|| syn::Error::new_spanned(field, "Field must have a name"))?;

    // Parse the attribute using syn 2.x API
    let mut group_name: Option<String> = None;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("group") {
            let value: syn::LitStr = meta.value()?.parse()?;
            group_name = Some(value.value());
            Ok(())
        } else {
            Err(meta.error("expected `group = \"...\"`"))
        }
    })?;

    let group_name = group_name.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            format!(
                "#[nested] on field `{}` requires group attribute: #[nested(group = \"...\")]",
                field_name
            ),
        )
    })?;

    Ok(NestedFieldIR {
        field_name,
        field_type: field.ty.clone(),
        group_name,
        unit_id: 0,         // Assigned later by assign_unit_ids()
        parent_unit_id: 0,  // Assigned later by assign_unit_ids()
        span: attr.path().segments[0].ident.span(),
    })
}

/// Assign sequential unit IDs to nested fields.
///
/// Unit 0 is reserved for root, so nested groups get IDs 1, 2, 3, ...
fn assign_unit_ids(fields: &mut [FieldIR]) {
    let mut next_unit_id: i32 = 1;

    for field in fields {
        if let FieldIR::Nested(nested) = field {
            nested.unit_id = next_unit_id;
            nested.parent_unit_id = 0; // All top-level for now (recursive nesting is future work)
            next_unit_id += 1;
        }
    }
}

/// Extract the parameter type from a type path.
fn extract_param_type(ty: &syn::Type) -> Option<ParamType> {
    let type_name = extract_type_name(ty)?;
    match type_name.as_str() {
        "FloatParam" => Some(ParamType::Float),
        "IntParam" => Some(ParamType::Int),
        "BoolParam" => Some(ParamType::Bool),
        _ => None,
    }
}

/// Extract the simple type name from a type (e.g., `FloatParam` from `beamr::FloatParam`).
fn extract_type_name(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(type_path) = ty {
        // Get the last segment of the path (e.g., `FloatParam` from `beamr::FloatParam`)
        if let Some(segment) = type_path.path.segments.last() {
            return Some(segment.ident.to_string());
        }
    }
    None
}
