//! Semantic validation for the derive macro.
//!
//! This module validates the parsed IR for semantic correctness.

use std::collections::HashMap;

use crate::ir::{FieldIR, ParamDefault, ParamFieldIR, ParamKind, ParamType, ParamsIR};

/// Validate the IR for semantic correctness.
pub fn validate(ir: &ParamsIR) -> syn::Result<()> {
    check_unique_string_ids(ir)?;
    check_no_hash_collisions(ir)?;
    validate_param_attributes(ir)?;
    Ok(())
}

/// Check that all string IDs are unique.
fn check_unique_string_ids(ir: &ParamsIR) -> syn::Result<()> {
    let mut seen: HashMap<&str, &syn::Ident> = HashMap::new();

    for field in &ir.fields {
        if let FieldIR::Param(param) = field {
            if let Some(first_field) = seen.get(param.string_id.as_str()) {
                return Err(syn::Error::new(
                    param.span,
                    format!(
                        "Duplicate parameter id \"{}\": already used by field `{}`",
                        param.string_id, first_field
                    ),
                ));
            }
            seen.insert(&param.string_id, &param.field_name);
        }
    }

    Ok(())
}

/// Check that no two parameter IDs hash to the same value.
fn check_no_hash_collisions(ir: &ParamsIR) -> syn::Result<()> {
    let mut seen: HashMap<u32, &str> = HashMap::new();

    for field in &ir.fields {
        if let FieldIR::Param(param) = field {
            if let Some(first_id) = seen.get(&param.hash_id) {
                // Hash collision detected
                return Err(syn::Error::new(
                    param.span,
                    format!(
                        "Parameter ID hash collision: \"{}\" and \"{}\" both hash to 0x{:08x}. \
                         Rename one of these parameters to avoid the collision.",
                        param.string_id, first_id, param.hash_id
                    ),
                ));
            }
            seen.insert(param.hash_id, &param.string_id);
        }
    }

    Ok(())
}

// =============================================================================
// Declarative Attribute Validation
// =============================================================================

/// Validate declarative attributes on all parameters.
fn validate_param_attributes(ir: &ParamsIR) -> syn::Result<()> {
    for field in &ir.fields {
        if let FieldIR::Param(param) = field {
            validate_single_param(param)?;
        }
    }
    Ok(())
}

/// Validate a single parameter's declarative attributes.
fn validate_single_param(param: &ParamFieldIR) -> syn::Result<()> {
    // Validate range ordering
    validate_range_ordering(param)?;

    // Validate default is within range
    validate_default_in_range(param)?;

    // Validate smoothing time is positive
    validate_smoothing_time(param)?;

    // Validate kind/type consistency
    validate_kind_type_consistency(param)?;

    Ok(())
}

/// Validate that range start < end.
fn validate_range_ordering(param: &ParamFieldIR) -> syn::Result<()> {
    if let Some(range) = &param.attrs.range {
        if range.start >= range.end {
            return Err(syn::Error::new(
                range.span,
                format!(
                    "invalid range: start ({}) must be less than end ({})",
                    format_number(range.start, param.param_type),
                    format_number(range.end, param.param_type),
                ),
            ));
        }
    }
    Ok(())
}

/// Validate that default value is within the specified range.
fn validate_default_in_range(param: &ParamFieldIR) -> syn::Result<()> {
    let (default_val, range) = match (&param.attrs.default, &param.attrs.range) {
        (Some(d), Some(r)) => (d, r),
        _ => return Ok(()), // Can't validate without both
    };

    let default_f64 = match default_val {
        ParamDefault::Float(v) => *v,
        ParamDefault::Int(v) => *v as f64,
        ParamDefault::Bool(_) => return Ok(()), // Bools don't have ranges
    };

    // Check if default is within range
    if default_f64 < range.start || default_f64 > range.end {
        return Err(syn::Error::new(
            param.span,
            format!(
                "default value {} is outside range {}..={}",
                format_number(default_f64, param.param_type),
                format_number(range.start, param.param_type),
                format_number(range.end, param.param_type),
            ),
        ));
    }

    Ok(())
}

/// Format a number appropriately for the parameter type.
/// FloatParam shows decimals, IntParam shows integers.
fn format_number(value: f64, param_type: ParamType) -> String {
    match param_type {
        ParamType::Float => {
            if value.fract() == 0.0 {
                format!("{:.1}", value) // Show at least one decimal: 100.0
            } else {
                format!("{}", value)
            }
        }
        ParamType::Int => format!("{}", value as i64),
        _ => format!("{}", value),
    }
}

/// Validate that smoothing time is positive.
fn validate_smoothing_time(param: &ParamFieldIR) -> syn::Result<()> {
    if let Some(smoothing) = &param.attrs.smoothing {
        if smoothing.time_ms <= 0.0 {
            return Err(syn::Error::new(
                smoothing.span,
                format!(
                    "smoothing time must be positive, got {} ms",
                    smoothing.time_ms
                ),
            ));
        }
    }
    Ok(())
}

/// Validate that kind is appropriate for the parameter type.
fn validate_kind_type_consistency(param: &ParamFieldIR) -> syn::Result<()> {
    let kind = match param.attrs.kind {
        Some(k) => k,
        None => return Ok(()), // No kind specified, nothing to validate
    };

    // Check for mismatched kinds
    match (param.param_type, kind) {
        // Semitones is for IntParam
        (ParamType::Float, ParamKind::Semitones) => {
            return Err(syn::Error::new(
                param.span,
                "kind 'semitones' should be used with IntParam, not FloatParam",
            ));
        }
        // Float-specific kinds on IntParam
        (ParamType::Int, ParamKind::Db | ParamKind::Hz | ParamKind::Ms | ParamKind::Seconds | ParamKind::Percent | ParamKind::Pan | ParamKind::Ratio) => {
            return Err(syn::Error::new(
                param.span,
                format!(
                    "kind '{:?}' should be used with FloatParam, not IntParam",
                    kind
                ),
            ));
        }
        // Bool/Enum shouldn't have kinds (except bypass which is handled separately)
        (ParamType::Bool, _) if !param.attrs.bypass => {
            return Err(syn::Error::new(
                param.span,
                "BoolParam should not have a 'kind' attribute",
            ));
        }
        (ParamType::Enum, _) => {
            return Err(syn::Error::new(
                param.span,
                "EnumParam should not have a 'kind' attribute",
            ));
        }
        _ => {}
    }

    Ok(())
}
