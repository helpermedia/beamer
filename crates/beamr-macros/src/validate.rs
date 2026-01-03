//! Semantic validation for the derive macro.
//!
//! This module validates the parsed IR for semantic correctness.

use std::collections::HashMap;

use crate::ir::{FieldIR, ParamsIR};

/// Validate the IR for semantic correctness.
pub fn validate(ir: &ParamsIR) -> syn::Result<()> {
    check_unique_string_ids(ir)?;
    check_no_hash_collisions(ir)?;
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
