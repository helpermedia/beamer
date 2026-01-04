//! Intermediate representation for the derive macro.
//!
//! This module defines the data structures that represent a parsed parameter
//! struct, after AST parsing but before code generation.

use proc_macro2::Span;

// =============================================================================
// Declarative Attribute Types
// =============================================================================

/// Parsed declarative attributes from `#[param(...)]`.
#[derive(Debug, Clone, Default)]
pub struct ParamAttrs {
    /// Display name (e.g., "Gain")
    pub name: Option<String>,
    /// Default value
    pub default: Option<ParamDefault>,
    /// Value range
    pub range: Option<RangeSpec>,
    /// Parameter kind (db, hz, percent, etc.)
    pub kind: Option<ParamKind>,
    /// Short name for constrained UIs
    pub short_name: Option<String>,
    /// Smoothing configuration
    pub smoothing: Option<SmoothingSpec>,
    /// Whether this is a bypass parameter
    pub bypass: bool,
    /// Visual grouping for DAW display (without nested struct).
    /// Parameters with the same group name will appear together in the DAW.
    pub group: Option<String>,
}

impl ParamAttrs {
    /// Check if all required attributes are present for a given param type.
    pub fn has_required_for(&self, param_type: ParamType) -> bool {
        match param_type {
            ParamType::Float => {
                self.name.is_some()
                    && self.default.is_some()
                    && (self.range.is_some() || self.kind.as_ref().map_or(false, |k| k.has_fixed_range()))
            }
            ParamType::Int => {
                self.name.is_some() && self.default.is_some() && self.range.is_some()
            }
            ParamType::Bool => {
                self.bypass || (self.name.is_some() && self.default.is_some())
            }
            ParamType::Enum => self.name.is_some(),
        }
    }
}

/// Default value for a parameter.
#[derive(Debug, Clone)]
pub enum ParamDefault {
    Float(f64),
    Int(i64),
    Bool(bool),
}

/// Range specification parsed from `range = start..=end`.
#[derive(Debug, Clone)]
pub struct RangeSpec {
    /// Start bound expression (stored for codegen)
    pub start: f64,
    /// End bound expression (stored for codegen)
    pub end: f64,
    /// Span for error reporting
    pub span: Span,
}

/// Parameter kind that determines the constructor and formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    // Float kinds
    Db,
    Hz,
    Ms,
    Seconds,
    Percent,
    Pan,
    Ratio,
    Linear,

    // Int kinds
    Semitones,
}

impl ParamKind {
    /// Parse a kind from a string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "db" => Some(ParamKind::Db),
            "hz" => Some(ParamKind::Hz),
            "ms" => Some(ParamKind::Ms),
            "seconds" => Some(ParamKind::Seconds),
            "percent" => Some(ParamKind::Percent),
            "pan" => Some(ParamKind::Pan),
            "ratio" => Some(ParamKind::Ratio),
            "linear" => Some(ParamKind::Linear),
            "semitones" => Some(ParamKind::Semitones),
            _ => None,
        }
    }

    /// Check if this kind has a fixed range (doesn't require explicit range attribute).
    pub fn has_fixed_range(&self) -> bool {
        matches!(self, ParamKind::Percent | ParamKind::Pan)
    }

    /// Get the fixed range for kinds that have one.
    pub fn fixed_range(&self) -> Option<(f64, f64)> {
        match self {
            ParamKind::Percent => Some((0.0, 1.0)),
            ParamKind::Pan => Some((-1.0, 1.0)),
            _ => None,
        }
    }
}

/// Smoothing specification parsed from `smoothing = "exp:5.0"`.
#[derive(Debug, Clone)]
pub struct SmoothingSpec {
    /// Smoothing style
    pub style: SmoothingStyle,
    /// Time in milliseconds
    pub time_ms: f64,
    /// Span for error reporting
    pub span: Span,
}

/// Smoothing style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmoothingStyle {
    Exponential,
    Linear,
}

impl SmoothingStyle {
    /// Parse from string prefix (e.g., "exp" or "linear").
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "exp" => Some(SmoothingStyle::Exponential),
            "linear" => Some(SmoothingStyle::Linear),
            _ => None,
        }
    }
}

// =============================================================================
// Core IR Types
// =============================================================================

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
    /// Declarative attributes (name, default, range, etc.)
    pub attrs: ParamAttrs,
}

impl ParamFieldIR {
    /// Check if this parameter has all required declarative attributes.
    pub fn has_declarative_attrs(&self) -> bool {
        self.attrs.has_required_for(self.param_type)
    }

    /// Generate the const identifier name for this parameter's VST3 ID.
    ///
    /// E.g., `gain` -> `PARAM_GAIN_VST3_ID`
    pub fn const_name(&self) -> syn::Ident {
        let name = self.field_name.to_string().to_uppercase();
        syn::Ident::new(&format!("PARAM_{}_VST3_ID", name), self.span)
    }
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

    /// Check if all param fields have complete declarative attributes.
    ///
    /// When true, the macro can generate a `Default` implementation.
    pub fn can_generate_default(&self) -> bool {
        self.param_fields().all(|p| p.has_declarative_attrs())
    }

    /// Check if any parameters have flat group attributes.
    pub fn has_flat_groups(&self) -> bool {
        self.param_fields().any(|p| p.attrs.group.is_some())
    }

    /// Get unique flat group names in order of first occurrence.
    pub fn flat_group_names(&self) -> Vec<&str> {
        let mut seen = std::collections::HashSet::new();
        let mut groups = Vec::new();
        for param in self.param_fields() {
            if let Some(ref group) = param.attrs.group {
                if seen.insert(group.as_str()) {
                    groups.push(group.as_str());
                }
            }
        }
        groups
    }
}
