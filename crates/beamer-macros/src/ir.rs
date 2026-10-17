//! Intermediate representation for the derive macro.
//!
//! This module defines the data structures that represent a parsed model
//! or nested group, after AST parsing but before code generation.

use proc_macro2::Span;

// =============================================================================
// Declarative Attribute Types
// =============================================================================

/// Parsed declarative attributes from `#[parameter(...)]`.
#[derive(Debug, Clone, Default)]
pub struct ParameterAttributes {
    /// Display name (e.g., "Gain")
    pub name: Option<String>,
    /// Default value
    pub default: Option<ParameterDefault>,
    /// Value range
    pub range: Option<RangeSpec>,
    /// Parameter kind (db, hz, percent, etc.)
    pub kind: Option<ParameterKind>,
    /// Short name for constrained UIs
    pub short_name: Option<String>,
    /// Smoothing configuration
    pub smoothing: Option<SmoothingSpec>,
    /// Whether this is a bypass parameter
    pub bypass: bool,
    /// Visual grouping for DAW display (without nested struct).
    /// Parameters with the same group name will appear together in the DAW.
    pub group: Option<String>,
    /// Step size for discrete float parameters.
    pub step: Option<f64>,
}

impl ParameterAttributes {
    /// Check if all required attributes are present for a given parameter type.
    pub fn has_required_for(&self, parameter_type: ParameterType) -> bool {
        match parameter_type {
            ParameterType::Float => {
                self.name.is_some()
                    && self.default.is_some()
                    && (self.range.is_some() || self.kind.as_ref().is_some_and(|k| k.has_fixed_range()))
            }
            ParameterType::Int => {
                self.name.is_some() && self.default.is_some() && self.range.is_some()
            }
            ParameterType::Bool => {
                self.bypass || (self.name.is_some() && self.default.is_some())
            }
            ParameterType::Enum => self.name.is_some(),
        }
    }
}

/// Default value for a parameter.
#[derive(Debug, Clone)]
pub enum ParameterDefault {
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
pub enum ParameterKind {
    // Float kinds
    Db,
    DbLog,
    DbLogOffset,
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

impl ParameterKind {
    /// Parse a kind from a string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "db" => Some(ParameterKind::Db),
            "db_log" => Some(ParameterKind::DbLog),
            "db_log_offset" => Some(ParameterKind::DbLogOffset),
            "hz" => Some(ParameterKind::Hz),
            "ms" => Some(ParameterKind::Ms),
            "seconds" => Some(ParameterKind::Seconds),
            "percent" => Some(ParameterKind::Percent),
            "pan" => Some(ParameterKind::Pan),
            "ratio" => Some(ParameterKind::Ratio),
            "linear" => Some(ParameterKind::Linear),
            "semitones" => Some(ParameterKind::Semitones),
            _ => None,
        }
    }

    /// Check if this kind has a fixed range (doesn't require explicit range attribute).
    pub fn has_fixed_range(&self) -> bool {
        matches!(self, ParameterKind::Percent | ParameterKind::Pan)
    }

    /// Get the fixed range for kinds that have one.
    pub fn fixed_range(&self) -> Option<(f64, f64)> {
        match self {
            ParameterKind::Percent => Some((0.0, 1.0)),
            ParameterKind::Pan => Some((-1.0, 1.0)),
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

/// Intermediate representation of a model struct or of a nested group in
/// one: its parameters, `State` fields, event queue and other fields.
///
/// This captures all the information needed to generate the trait implementations.
#[allow(dead_code)]
pub struct ModelIR {
    /// The struct name (e.g., `GainParameters`)
    pub struct_name: syn::Ident,
    /// Generic parameters, if any
    pub generics: syn::Generics,
    /// All fields in the struct
    pub fields: Vec<FieldIR>,
    /// Span for error reporting
    pub span: Span,
}

/// A single field of a model or of a nested group.
pub enum FieldIR {
    /// A direct parameter field (boxed to reduce enum size)
    Parameter(Box<ParameterFieldIR>),
    /// A nested group (boxed to reduce enum size)
    Nested(Box<NestedFieldIR>),
    /// A `State` field that is saved with the plugin state.
    State(Box<StateFieldIR>),
    /// A field without an attribute. It is not part of the parameters or
    /// the saved state. The generated `Default` initializes it with its
    /// own default.
    Other(syn::Ident),
    /// A `WebViewEvents` field: the plugin's queue of events for the web
    /// UI, which the format wrappers find through
    /// `ModelFields::webview_events`. Initialized like `Other`.
    Events(syn::Ident),
}

/// A field with `#[state(id = "...")]`.
pub struct StateFieldIR {
    /// Field name (e.g., `sample_path`).
    pub field_name: syn::Ident,
    /// The id the value is saved under.
    pub string_id: String,
    /// Span for error reporting.
    pub span: Span,
}

/// A direct parameter field.
#[allow(dead_code)]
pub struct ParameterFieldIR {
    /// Field name (e.g., `gain`)
    pub field_name: syn::Ident,
    /// Parameter type (Float, Int, Bool)
    pub parameter_type: ParameterType,
    /// String ID from `#[parameter(id = "...")]`
    pub string_id: String,
    /// FNV-1a hash of the string ID: the id that hosts see when the
    /// struct is the root. Only used to find collisions within the struct.
    pub hash_id: u32,
    /// Span for error reporting
    pub span: Span,
    /// Declarative attributes (name, default, range, etc.)
    pub attributes: ParameterAttributes,
}

impl ParameterFieldIR {
    /// Check if this parameter has all required declarative attributes.
    pub fn has_declarative_attributes(&self) -> bool {
        self.attributes.has_required_for(self.parameter_type)
    }
}

/// A field that holds a nested group.
#[allow(dead_code)]
pub struct NestedFieldIR {
    /// Field name (e.g., `output`)
    pub field_name: syn::Ident,
    /// Field type (e.g., `OutputParameters`)
    pub field_type: syn::Type,
    /// Group name from `#[nested(group = "...")]`
    pub group_name: String,
    /// Assigned group ID (1-indexed, root is 0)
    pub group_id: i32,
    /// Parent group ID (0 for top-level, parent's group_id for nested-within-nested)
    pub parent_group_id: i32,
    /// Span for error reporting
    pub span: Span,
}

/// The type of a parameter field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterType {
    Float,
    Int,
    Bool,
    Enum,
}

impl ModelIR {
    /// Iterate over all parameter fields (excluding nested).
    pub fn parameter_fields(&self) -> impl Iterator<Item = &ParameterFieldIR> {
        self.fields.iter().filter_map(|f| match f {
            FieldIR::Parameter(p) => Some(p.as_ref()),
            _ => None,
        })
    }

    /// Iterate over all nested fields.
    pub fn nested_fields(&self) -> impl Iterator<Item = &NestedFieldIR> {
        self.fields.iter().filter_map(|f| match f {
            FieldIR::Nested(n) => Some(n.as_ref()),
            _ => None,
        })
    }

    /// Iterate over the `WebViewEvents` fields.
    pub fn events_fields(&self) -> impl Iterator<Item = &syn::Ident> {
        self.fields.iter().filter_map(|f| match f {
            FieldIR::Events(field) => Some(field),
            _ => None,
        })
    }

    /// Iterate over all `State` fields.
    pub fn state_fields(&self) -> impl Iterator<Item = &StateFieldIR> {
        self.fields.iter().filter_map(|f| match f {
            FieldIR::State(s) => Some(s.as_ref()),
            _ => None,
        })
    }

    /// Count of direct parameter fields.
    pub fn parameter_count(&self) -> usize {
        self.parameter_fields().count()
    }

    /// Check if there are any nested fields.
    pub fn has_nested(&self) -> bool {
        self.nested_fields().next().is_some()
    }

    /// Check if all parameter fields have complete declarative attributes.
    ///
    /// When true, the macro can generate a `Default` implementation.
    pub fn can_generate_default(&self) -> bool {
        self.parameter_fields().all(|p| p.has_declarative_attributes())
    }

    /// Check if any parameters have flat group attributes.
    pub fn has_flat_groups(&self) -> bool {
        self.parameter_fields().any(|p| p.attributes.group.is_some())
    }

    /// Get unique flat group names in order of first occurrence.
    pub fn flat_group_names(&self) -> Vec<&str> {
        let mut seen = std::collections::HashSet::new();
        let mut groups = Vec::new();
        for parameter in self.parameter_fields() {
            if let Some(ref group) = parameter.attributes.group {
                if seen.insert(group.as_str()) {
                    groups.push(group.as_str());
                }
            }
        }
        groups
    }
}
