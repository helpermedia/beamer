//! Semantic validation for the derive macro.
//!
//! This module validates the parsed IR for semantic correctness.

use std::collections::HashMap;

use crate::ir::{FieldIR, ParameterDefault, ParamFieldIR, ParamKind, ParamType, ParametersIR};

/// Validate the IR for semantic correctness.
pub fn validate(ir: &ParametersIR) -> syn::Result<()> {
    check_unique_string_ids(ir)?;
    check_no_hash_collisions(ir)?;
    validate_parameter_attributes(ir)?;
    Ok(())
}

/// Check that all string IDs are unique.
fn check_unique_string_ids(ir: &ParametersIR) -> syn::Result<()> {
    let mut seen: HashMap<&str, &syn::Ident> = HashMap::new();

    for field in &ir.fields {
        if let FieldIR::Parameter(parameter) = field {
            if let Some(first_field) = seen.get(parameter.string_id.as_str()) {
                return Err(syn::Error::new(
                    parameter.span,
                    format!(
                        "Duplicate parameter id \"{}\": already used by field `{}`",
                        parameter.string_id, first_field
                    ),
                ));
            }
            seen.insert(&parameter.string_id, &parameter.field_name);
        }
    }

    Ok(())
}

/// Check that no two parameter IDs hash to the same value.
fn check_no_hash_collisions(ir: &ParametersIR) -> syn::Result<()> {
    let mut seen: HashMap<u32, &str> = HashMap::new();

    for field in &ir.fields {
        if let FieldIR::Parameter(parameter) = field {
            if let Some(first_id) = seen.get(&parameter.hash_id) {
                // Hash collision detected
                return Err(syn::Error::new(
                    parameter.span,
                    format!(
                        "Parameter ID hash collision: \"{}\" and \"{}\" both hash to 0x{:08x}. \
                         Rename one of these parameters to avoid the collision.",
                        parameter.string_id, first_id, parameter.hash_id
                    ),
                ));
            }
            seen.insert(parameter.hash_id, &parameter.string_id);
        }
    }

    Ok(())
}

// =============================================================================
// Declarative Attribute Validation
// =============================================================================

/// Validate declarative attributes on all parameters.
fn validate_parameter_attributes(ir: &ParametersIR) -> syn::Result<()> {
    for field in &ir.fields {
        if let FieldIR::Parameter(parameter) = field {
            validate_single_param(parameter)?;
        }
    }
    Ok(())
}

/// Validate a single parameter's declarative attributes.
fn validate_single_param(parameter: &ParamFieldIR) -> syn::Result<()> {
    // Validate range ordering
    validate_range_ordering(parameter)?;

    // Validate default is within range
    validate_default_in_range(parameter)?;

    // Validate smoothing time is positive
    validate_smoothing_time(parameter)?;

    // Validate kind/type consistency
    validate_kind_type_consistency(parameter)?;

    Ok(())
}

/// Validate that range start < end.
fn validate_range_ordering(parameter: &ParamFieldIR) -> syn::Result<()> {
    if let Some(range) = &parameter.attributes.range {
        if range.start >= range.end {
            return Err(syn::Error::new(
                range.span,
                format!(
                    "invalid range: start ({}) must be less than end ({})",
                    format_number(range.start, parameter.parameter_type),
                    format_number(range.end, parameter.parameter_type),
                ),
            ));
        }
    }
    Ok(())
}

/// Validate that default value is within the specified range.
fn validate_default_in_range(parameter: &ParamFieldIR) -> syn::Result<()> {
    let (default_val, range) = match (&parameter.attributes.default, &parameter.attributes.range) {
        (Some(d), Some(r)) => (d, r),
        _ => return Ok(()), // Can't validate without both
    };

    let default_f64 = match default_val {
        ParameterDefault::Float(v) => *v,
        ParameterDefault::Int(v) => *v as f64,
        ParameterDefault::Bool(_) => return Ok(()), // Bools don't have ranges
    };

    // Check if default is within range
    if default_f64 < range.start || default_f64 > range.end {
        return Err(syn::Error::new(
            parameter.span,
            format!(
                "default value {} is outside range {}..={}",
                format_number(default_f64, parameter.parameter_type),
                format_number(range.start, parameter.parameter_type),
                format_number(range.end, parameter.parameter_type),
            ),
        ));
    }

    Ok(())
}

/// Format a number appropriately for the parameter type.
/// FloatParameter shows decimals, IntParameter shows integers.
fn format_number(value: f64, parameter_type: ParamType) -> String {
    match parameter_type {
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
fn validate_smoothing_time(parameter: &ParamFieldIR) -> syn::Result<()> {
    if let Some(smoothing) = &parameter.attributes.smoothing {
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
fn validate_kind_type_consistency(parameter: &ParamFieldIR) -> syn::Result<()> {
    let kind = match parameter.attributes.kind {
        Some(k) => k,
        None => return Ok(()), // No kind specified, nothing to validate
    };

    // Check for mismatched kinds
    match (parameter.parameter_type, kind) {
        // Semitones is for IntParameter
        (ParamType::Float, ParamKind::Semitones) => {
            return Err(syn::Error::new(
                parameter.span,
                "kind 'semitones' should be used with IntParameter, not FloatParameter",
            ));
        }
        // Float-specific kinds on IntParameter
        (ParamType::Int, ParamKind::Db | ParamKind::Hz | ParamKind::Ms | ParamKind::Seconds | ParamKind::Percent | ParamKind::Pan | ParamKind::Ratio) => {
            return Err(syn::Error::new(
                parameter.span,
                format!(
                    "kind '{:?}' should be used with FloatParameter, not IntParameter",
                    kind
                ),
            ));
        }
        // Bool/Enum shouldn't have kinds (except bypass which is handled separately)
        (ParamType::Bool, _) if !parameter.attributes.bypass => {
            return Err(syn::Error::new(
                parameter.span,
                "BoolParameter should not have a 'kind' attribute",
            ));
        }
        (ParamType::Enum, _) => {
            return Err(syn::Error::new(
                parameter.span,
                "EnumParameter should not have a 'kind' attribute",
            ));
        }
        _ => {}
    }

    Ok(())
}
