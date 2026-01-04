//! Intermediate representation for the derive macro.
//!
//! This module defines the data structures that represent a parsed parameter
//! struct, after AST parsing but before code generation.

use proc_macro2::Span;

/// Intermediate representation of a parameter struct.
///
/// This captures all the information needed to generate the trait implementations.
#[allow(dead_code)]
pub struct ParamsIR {
    /// The struct name (e.g., `GainParams`)
    pub struct_name: syn::Ident,
    /// Generic parameters, if any
    pub generics: syn::Generics,
    /// All fields in the struct
    pub fields: Vec<FieldIR>,
    /// Span for error reporting
    pub span: Span,
}

/// A single field in the parameter struct.
pub enum FieldIR {
    /// A direct parameter field (FloatParam, IntParam, BoolParam)
    Param(ParamFieldIR),
    /// A nested parameter struct (boxed to reduce enum size)
    Nested(Box<NestedFieldIR>),
}

/// A direct parameter field.
#[allow(dead_code)]
pub struct ParamFieldIR {
    /// Field name (e.g., `gain`)
    pub field_name: syn::Ident,
    /// Parameter type (Float, Int, Bool)
    pub param_type: ParamType,
    /// String ID from `#[param(id = "...")]`
    pub string_id: String,
    /// FNV-1a hash of the string ID
    pub hash_id: u32,
    /// Span for error reporting
    pub span: Span,
}

/// A nested parameter struct field.
#[allow(dead_code)]
pub struct NestedFieldIR {
    /// Field name (e.g., `output`)
    pub field_name: syn::Ident,
    /// Field type (e.g., `OutputParams`)
    pub field_type: syn::Type,
    /// Group name from `#[nested(group = "...")]`
    pub group_name: String,
    /// Assigned unit ID (1-indexed, root is 0)
    pub unit_id: i32,
    /// Parent unit ID (0 for top-level, parent's unit_id for nested-within-nested)
    pub parent_unit_id: i32,
    /// Span for error reporting
    pub span: Span,
}

/// The type of a parameter field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamType {
    Float,
    Int,
    Bool,
    Enum,
}

impl ParamsIR {
    /// Iterate over all parameter fields (excluding nested).
    pub fn param_fields(&self) -> impl Iterator<Item = &ParamFieldIR> {
        self.fields.iter().filter_map(|f| match f {
            FieldIR::Param(p) => Some(p),
            FieldIR::Nested(_) => None,
        })
    }

    /// Iterate over all nested fields.
    pub fn nested_fields(&self) -> impl Iterator<Item = &NestedFieldIR> {
        self.fields.iter().filter_map(|f| match f {
            FieldIR::Param(_) => None,
            FieldIR::Nested(n) => Some(n.as_ref()),
        })
    }

    /// Count of direct parameter fields.
    pub fn param_count(&self) -> usize {
        self.param_fields().count()
    }

    /// Check if there are any nested fields.
    pub fn has_nested(&self) -> bool {
        self.nested_fields().next().is_some()
    }
}

impl ParamFieldIR {
    /// Generate the const identifier name for this parameter's VST3 ID.
    ///
    /// E.g., `gain` -> `PARAM_GAIN_VST3_ID`
    pub fn const_name(&self) -> syn::Ident {
        let name = self.field_name.to_string().to_uppercase();
        syn::Ident::new(&format!("PARAM_{}_VST3_ID", name), self.span)
    }
}
