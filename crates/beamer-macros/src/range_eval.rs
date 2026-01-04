//! Compile-time evaluation of range expressions for declarative parameter attributes.
//!
//! This module provides functions to extract numeric values from `syn::Expr` nodes
//! representing range bounds (e.g., `-60.0..=12.0`).

use syn::spanned::Spanned;

/// A numeric literal value extracted from a syn expression.
#[derive(Debug, Clone, Copy)]
pub enum LiteralValue {
    Float(f64),
    Int(i64),
}

impl LiteralValue {
    /// Convert to f64, casting integers if needed.
    pub fn as_f64(self) -> f64 {
        match self {
            LiteralValue::Float(f) => f,
            LiteralValue::Int(i) => i as f64,
        }
    }
}

/// Evaluate a literal expression to extract its numeric value.
///
/// Handles:
/// - Float literals: `60.0`, `0.5`
/// - Integer literals: `60`, `127`
/// - Negative expressions: `-60.0`, `-24`
///
/// # Errors
///
/// Returns an error if the expression is not a numeric literal or negated literal.
pub fn eval_literal_expr(expr: &syn::Expr) -> syn::Result<LiteralValue> {
    match expr {
        // Direct literal: 60.0, 127
        syn::Expr::Lit(lit) => eval_lit(&lit.lit),

        // Negation: -60.0, -24
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            let inner = eval_literal_expr(&unary.expr)?;
            Ok(match inner {
                LiteralValue::Float(f) => LiteralValue::Float(-f),
                LiteralValue::Int(i) => LiteralValue::Int(-i),
            })
        }

        // Group expression: (60.0)
        syn::Expr::Paren(paren) => eval_literal_expr(&paren.expr),

        _ => Err(syn::Error::new(
            expr.span(),
            "expected numeric literal (e.g., `60.0`, `-24`)",
        )),
    }
}

/// Evaluate a syn::Lit to a LiteralValue.
fn eval_lit(lit: &syn::Lit) -> syn::Result<LiteralValue> {
    match lit {
        syn::Lit::Float(f) => {
            let value: f64 = f.base10_parse().map_err(|e| {
                syn::Error::new(f.span(), format!("invalid float literal: {}", e))
            })?;
            Ok(LiteralValue::Float(value))
        }
        syn::Lit::Int(i) => {
            let value: i64 = i.base10_parse().map_err(|e| {
                syn::Error::new(i.span(), format!("invalid integer literal: {}", e))
            })?;
            Ok(LiteralValue::Int(value))
        }
        _ => Err(syn::Error::new(
            lit.span(),
            "expected float or integer literal",
        )),
    }
}

/// Evaluate a range expression and extract start and end as f64.
///
/// # Arguments
///
/// * `start` - The start bound expression
/// * `end` - The end bound expression
///
/// # Returns
///
/// A tuple of (start, end) as f64 values.
pub fn eval_float_range(
    start: &syn::Expr,
    end: &syn::Expr,
) -> syn::Result<(f64, f64)> {
    let start_val = eval_literal_expr(start)?;
    let end_val = eval_literal_expr(end)?;
    Ok((start_val.as_f64(), end_val.as_f64()))
}
