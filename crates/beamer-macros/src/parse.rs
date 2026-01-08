//! AST parsing for the derive macro.
//!
//! This module transforms `syn::DeriveInput` into our intermediate representation.

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Field, Fields};

use beamer_utils::fnv1a_32;
use crate::ir::{
    FieldIR, NestedFieldIR, ParameterAttributes, ParameterDefault, ParameterFieldIR, ParameterKind, ParameterType,
    ParametersIR, RangeSpec, SmoothingSpec, SmoothingStyle,
};
use crate::range_eval;

/// Parse a `DeriveInput` into our intermediate representation.
pub fn parse(input: DeriveInput) -> syn::Result<ParametersIR> {
    // Ensure it's a struct with named fields
    let data_struct = match &input.data {
        Data::Struct(s) => s,
        Data::Enum(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Parameters)] only supports structs, not enums",
            ))
        }
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Parameters)] only supports structs, not unions",
            ))
        }
    };

    let fields = match &data_struct.fields {
        Fields::Named(named) => &named.named,
        Fields::Unnamed(_) => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Parameters)] only supports structs with named fields",
            ))
        }
        Fields::Unit => {
            return Err(syn::Error::new_spanned(
                &input,
                "#[derive(Parameters)] requires at least one field",
            ))
        }
    };

    // Parse each field
    let mut parsed_fields = Vec::new();
    for field in fields {
        if let Some(field_ir) = parse_field(field)? {
            parsed_fields.push(field_ir);
        }
        // Fields without #[parameter] or #[nested] are silently ignored
    }

    if parsed_fields.is_empty() {
        return Err(syn::Error::new_spanned(
            &input,
            "#[derive(Parameters)] requires at least one #[parameter] or #[nested] field",
        ));
    }

    // Assign sequential unit IDs to nested fields
    assign_unit_ids(&mut parsed_fields);

    Ok(ParametersIR {
        struct_name: input.ident.clone(),
        generics: input.generics.clone(),
        fields: parsed_fields,
        span: Span::call_site(),
    })
}

/// Parse a single field, returning None if it has no relevant attributes.
fn parse_field(field: &Field) -> syn::Result<Option<FieldIR>> {
    // Check for #[parameter] attribute
    for attr in &field.attrs {
        if attr.path().is_ident("parameter") {
            return parse_parameter_field(field, attr).map(|p| Some(FieldIR::Parameter(p)));
        }
        if attr.path().is_ident("nested") {
            return parse_nested_field(field, attr).map(|n| Some(FieldIR::Nested(Box::new(n))));
        }
    }

    // Check if this field LOOKS like a parameter type but lacks the attribute
    if let Some(type_name) = extract_type_name(&field.ty) {
        if matches!(
            type_name.as_str(),
            "FloatParameter" | "IntParameter" | "BoolParameter" | "EnumParameter"
        ) {
            return Err(syn::Error::new_spanned(
                field,
                format!(
                    "{} field is missing #[parameter(id = \"...\")] attribute",
                    type_name
                ),
            ));
        }
    }

    Ok(None)
}

/// Parse a field with `#[parameter(...)]` attribute.
///
/// Supports both minimal and declarative styles:
/// - Minimal: `#[parameter(id = "gain")]` (requires manual Default)
/// - Declarative: `#[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]`
fn parse_parameter_field(field: &Field, attr: &syn::Attribute) -> syn::Result<ParameterFieldIR> {
    let field_name = field
        .ident
        .clone()
        .ok_or_else(|| syn::Error::new_spanned(field, "Field must have a name"))?;

    // Parse the attribute using syn 2.x API
    let mut string_id: Option<String> = None;
    let mut attributes = ParameterAttributes::default();

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("id") {
            let value: syn::LitStr = meta.value()?.parse()?;
            string_id = Some(value.value());
            Ok(())
        } else if meta.path.is_ident("name") {
            let value: syn::LitStr = meta.value()?.parse()?;
            attributes.name = Some(value.value());
            Ok(())
        } else if meta.path.is_ident("default") {
            attributes.default = Some(parse_default_value(&meta)?);
            Ok(())
        } else if meta.path.is_ident("range") {
            attributes.range = Some(parse_range_spec(&meta)?);
            Ok(())
        } else if meta.path.is_ident("kind") {
            let value: syn::LitStr = meta.value()?.parse()?;
            let kind_str = value.value();
            attributes.kind = Some(ParameterKind::from_str(&kind_str).ok_or_else(|| {
                syn::Error::new_spanned(
                    &value,
                    format!(
                        "unknown kind '{}'. Valid kinds: db, db_log, db_log_offset, hz, ms, seconds, percent, pan, ratio, linear, semitones",
                        kind_str
                    ),
                )
            })?);
            Ok(())
        } else if meta.path.is_ident("short_name") {
            let value: syn::LitStr = meta.value()?.parse()?;
            attributes.short_name = Some(value.value());
            Ok(())
        } else if meta.path.is_ident("smoothing") {
            attributes.smoothing = Some(parse_smoothing_spec(&meta)?);
            Ok(())
        } else if meta.path.is_ident("bypass") {
            // bypass can be `bypass` (flag) or `bypass = true`
            if meta.input.peek(syn::Token![=]) {
                let value: syn::LitBool = meta.value()?.parse()?;
                attributes.bypass = value.value();
            } else {
                attributes.bypass = true;
            }
            Ok(())
        } else if meta.path.is_ident("group") {
            let value: syn::LitStr = meta.value()?.parse()?;
            attributes.group = Some(value.value());
            Ok(())
        } else {
            Err(meta.error(
                "unknown attribute. Expected: id, name, default, range, kind, short_name, smoothing, bypass, group"
            ))
        }
    })?;

    let string_id = string_id.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            format!(
                "#[parameter] on field `{}` requires id attribute: #[parameter(id = \"...\")]",
                field_name
            ),
        )
    })?;

    // Validate that the ID doesn't contain path separators (used for nested group routing)
    if string_id.contains('/') {
        return Err(syn::Error::new_spanned(
            attr,
            format!(
                "parameter id '{}' cannot contain '/' (reserved for nested group path routing)",
                string_id
            ),
        ));
    }

    // Determine parameter type
    let parameter_type = extract_parameter_type(&field.ty).ok_or_else(|| {
        syn::Error::new_spanned(
            &field.ty,
            "#[parameter] can only be used on FloatParameter, IntParameter, BoolParameter, or EnumParameter fields",
        )
    })?;

    // Compute hash
    let hash_id = fnv1a_32(&string_id);

    Ok(ParameterFieldIR {
        field_name,
        parameter_type,
        string_id,
        hash_id,
        span: attr.path().segments[0].ident.span(),
        attributes,
    })
}

/// Parse a default value from `default = <literal>`.
fn parse_default_value(meta: &syn::meta::ParseNestedMeta) -> syn::Result<ParameterDefault> {
    let expr: syn::Expr = meta.value()?.parse()?;
    parse_default_expr(&expr)
}

/// Parse a default value expression.
fn parse_default_expr(expr: &syn::Expr) -> syn::Result<ParameterDefault> {
    match expr {
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Float(f) => {
                let value: f64 = f.base10_parse()?;
                Ok(ParameterDefault::Float(value))
            }
            syn::Lit::Int(i) => {
                let value: i64 = i.base10_parse()?;
                Ok(ParameterDefault::Int(value))
            }
            syn::Lit::Bool(b) => Ok(ParameterDefault::Bool(b.value())),
            _ => Err(syn::Error::new_spanned(
                lit,
                "default must be a float, int, or bool literal",
            )),
        },
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            if let syn::Expr::Lit(lit) = &*unary.expr {
                match &lit.lit {
                    syn::Lit::Float(f) => {
                        let value: f64 = f.base10_parse()?;
                        Ok(ParameterDefault::Float(-value))
                    }
                    syn::Lit::Int(i) => {
                        let value: i64 = i.base10_parse()?;
                        Ok(ParameterDefault::Int(-value))
                    }
                    _ => Err(syn::Error::new_spanned(
                        unary,
                        "expected float or int literal after -",
                    )),
                }
            } else {
                Err(syn::Error::new_spanned(
                    unary,
                    "expected literal after -",
                ))
            }
        }
        _ => Err(syn::Error::new_spanned(
            expr,
            "default must be a literal value (e.g., 0.0, -12, true)",
        )),
    }
}

/// Parse a range specification from `range = start..=end`.
fn parse_range_spec(meta: &syn::meta::ParseNestedMeta) -> syn::Result<RangeSpec> {
    let expr: syn::ExprRange = meta.value()?.parse().map_err(|_| {
        syn::Error::new(
            meta.path.span(),
            "range must be an inclusive range expression like `-60.0..=12.0`",
        )
    })?;

    let start_expr = expr.start.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(&expr, "range must have a start value")
    })?;
    let end_expr = expr.end.as_ref().ok_or_else(|| {
        syn::Error::new_spanned(&expr, "range must have an end value")
    })?;

    // Verify it's an inclusive range
    if !matches!(expr.limits, syn::RangeLimits::Closed(_)) {
        return Err(syn::Error::new_spanned(
            &expr,
            "range must be inclusive (use ..= not ..)",
        ));
    }

    // Evaluate the range bounds
    let (start, end) = range_eval::eval_float_range(start_expr, end_expr)?;

    Ok(RangeSpec {
        start,
        end,
        span: expr.span(),
    })
}

/// Parse a smoothing specification from `smoothing = "exp:5.0"`.
fn parse_smoothing_spec(meta: &syn::meta::ParseNestedMeta) -> syn::Result<SmoothingSpec> {
    let value: syn::LitStr = meta.value()?.parse()?;
    let s = value.value();
    let span = value.span();

    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err(syn::Error::new(
            span,
            "smoothing must be in format 'exp:5.0' or 'linear:10.0'",
        ));
    }

    let style = SmoothingStyle::from_str(parts[0]).ok_or_else(|| {
        syn::Error::new(
            span,
            "smoothing style must be 'exp' or 'linear'",
        )
    })?;

    let time_ms: f64 = parts[1].parse().map_err(|_| {
        syn::Error::new(span, "invalid time value in smoothing (expected number)")
    })?;

    Ok(SmoothingSpec {
        style,
        time_ms,
        span,
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
/// Unit 0 is reserved for root. Flat groups (via `group = "..."`) get IDs 1, 2, 3, ...
/// Nested groups get IDs starting after flat groups.
fn assign_unit_ids(fields: &mut [FieldIR]) {
    // Count flat groups first - they get IDs 1, 2, 3, ...
    let flat_group_count = count_flat_groups(fields);

    // Nested groups start after flat groups
    let mut next_unit_id: i32 = flat_group_count as i32 + 1;

    for field in fields {
        if let FieldIR::Nested(nested) = field {
            nested.unit_id = next_unit_id;
            nested.parent_unit_id = 0; // All top-level for now (recursive nesting is future work)
            next_unit_id += 1;
        }
    }
}

/// Count unique flat group names in the fields.
fn count_flat_groups(fields: &[FieldIR]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for field in fields {
        if let FieldIR::Parameter(p) = field {
            if let Some(ref group) = p.attributes.group {
                seen.insert(group.as_str());
            }
        }
    }
    seen.len()
}

/// Extract the parameter type from a type path.
fn extract_parameter_type(ty: &syn::Type) -> Option<ParameterType> {
    let type_name = extract_type_name(ty)?;
    match type_name.as_str() {
        "FloatParameter" => Some(ParameterType::Float),
        "IntParameter" => Some(ParameterType::Int),
        "BoolParameter" => Some(ParameterType::Bool),
        "EnumParameter" => Some(ParameterType::Enum),
        _ => None,
    }
}

/// Extract the simple type name from a type (e.g., `FloatParameter` from `beamer::FloatParameter`).
fn extract_type_name(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(type_path) = ty {
        // Get the last segment of the path (e.g., `FloatParameter` from `beamer::FloatParameter`)
        if let Some(segment) = type_path.path.segments.last() {
            return Some(segment.ident.to_string());
        }
    }
    None
}
