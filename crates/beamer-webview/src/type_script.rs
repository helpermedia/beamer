//! Rust types as TypeScript types, for the generated file that describes a
//! model to its web UI.
//!
//! [`TypeScript`] turns a Rust type into the TypeScript type of what
//! `serde_json` writes for it. `#[derive(TypeScript)]` implements it from
//! the serde attributes of a struct or enum; this module implements it for
//! the standard types. A derived type without generics becomes a named
//! declaration of [`TypeScriptTypes`], which renders every declaration it
//! collected as TypeScript source.
//!
//! The mapping:
//!
//! | Rust | TypeScript |
//! |---|---|
//! | `bool` | `boolean` |
//! | integers, `f32`, `f64` | `number` |
//! | `char`, `String`, `str`, `Cow<str>` | `string` |
//! | `()`, unit struct | `null` |
//! | `Option<T>` | `T \| null` |
//! | `Vec<T>`, `VecDeque<T>`, slices, sets | `T[]` |
//! | `[T; N]` | a tuple of N up to 16, `T[]` above |
//! | tuples up to 12 elements | tuples |
//! | maps with string or number keys | `Record<string, V>` |
//! | maps with other keys (unit enums) | `Partial<Record<K, V>>` |
//! | `Box<T>`, `Rc<T>`, `Arc<T>`, `&T` | `T` |
//! | `serde_json::Value` | `unknown` |

use std::any::{type_name, TypeId};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

/// The width that one-line forms (JSDoc, unions, objects) fit in.
const WIDTH: usize = 80;

/// A Rust type that has a TypeScript type: the type of what `serde_json`
/// writes for its values.
///
/// `#[derive(TypeScript)]` implements it from the serde attributes of a
/// struct or enum.
pub trait TypeScript {
    /// The type as TypeScript: a named declaration, which this adds to
    /// `types` on first use, or the type inline.
    fn type_script(types: &mut TypeScriptTypes) -> TsType;
}

/// A TypeScript type.
#[derive(Clone, Debug, PartialEq)]
pub enum TsType {
    /// A keyword type: `number`, `string`, `boolean`, `null`, `unknown` or
    /// `never`.
    Keyword(&'static str),
    /// A string literal type, held as its unquoted text (`noteOn`) and
    /// rendered as a JSON string (`"noteOn"`).
    Literal(String),
    /// A declaration of [`TypeScriptTypes`], by its name.
    Named(String),
    /// An array: `T[]`.
    Array(Box<TsType>),
    /// A tuple: `[A, B]`.
    Tuple(Vec<TsType>),
    /// An object type: `{ a: A; b?: B }`.
    Object(Vec<TsField>),
    /// A union: `A | B`.
    Union(Vec<TsMember>),
    /// An intersection: `A & B`.
    Intersection(Vec<TsType>),
    /// A record: `Record<K, V>`, or `Partial<Record<K, V>>` when `partial`
    /// (not every key of `K` is present).
    Record {
        /// The type of the keys.
        key: Box<TsType>,
        /// The type of the values.
        value: Box<TsType>,
        /// Whether not every key needs to be present.
        partial: bool,
    },
    /// TypeScript written by hand: `#[type_script(type = "...")]`.
    Raw(String),
}

/// A field of an object type.
#[derive(Clone, Debug, PartialEq)]
pub struct TsField {
    /// The name as JSON has it, quoted in the output when it is no valid
    /// identifier.
    pub name: String,
    /// The doc comment, rendered as JSDoc above the field.
    pub doc: Option<String>,
    /// Whether the field may be absent: `name?: type`.
    pub optional: bool,
    /// The type of the field.
    pub ty: TsType,
}

/// A member of a union.
#[derive(Clone, Debug, PartialEq)]
pub struct TsMember {
    /// The doc comment, rendered as JSDoc above the member.
    pub doc: Option<String>,
    /// The type of the member.
    pub ty: TsType,
}

/// The keyword type `null`.
const NULL: TsType = TsType::Keyword("null");

impl TsType {
    /// The type or `null`: `T | null`.
    ///
    /// A type that already takes `null` stays as it is, so an `Option` of an
    /// `Option` is `T | null` too: serde writes `null` for both.
    pub fn nullable(self) -> TsType {
        match self {
            TsType::Keyword("null" | "unknown") => self,
            TsType::Union(mut members) => {
                if !members.iter().any(|member| member.ty == NULL) {
                    members.push(TsMember {
                        doc: None,
                        ty: NULL,
                    });
                }
                TsType::Union(members)
            }
            ty => TsType::Union(vec![
                TsMember { doc: None, ty },
                TsMember {
                    doc: None,
                    ty: NULL,
                },
            ]),
        }
    }

    /// The type as TypeScript source.
    ///
    /// `indent` is the indentation of the line the type starts on: the lines
    /// of a type that breaks over several are indented from it.
    pub fn render(&self, indent: usize) -> String {
        self.layout(indent, indent, 0)
    }

    /// Whether this is a union of more than one member, which needs
    /// parentheses inside an array or an intersection.
    fn is_compound_union(&self) -> bool {
        match self {
            TsType::Union(members) => match members.as_slice() {
                [only] => only.doc.is_none() && only.ty.is_compound_union(),
                _ => true,
            },
            _ => false,
        }
    }

    /// Whether this needs parentheses before `[]`.
    fn needs_parentheses_in_array(&self) -> bool {
        match self {
            TsType::Intersection(parts) => parts.len() > 1,
            TsType::Raw(text) => text.contains(['|', '&', '=']),
            ty => ty.is_compound_union(),
        }
    }

    /// The type on one line, or `None` when it has a doc comment anywhere,
    /// which needs lines of its own.
    fn flat(&self) -> Option<String> {
        Some(match self {
            TsType::Keyword(keyword) => (*keyword).to_string(),
            TsType::Literal(text) => string_literal(text),
            TsType::Named(name) => name.clone(),
            TsType::Raw(text) if text.contains('\n') => return None,
            TsType::Raw(text) => text.clone(),
            TsType::Array(element) => {
                let inner = element.flat()?;
                if element.needs_parentheses_in_array() {
                    format!("({inner})[]")
                } else {
                    format!("{inner}[]")
                }
            }
            TsType::Tuple(elements) => {
                let parts = elements
                    .iter()
                    .map(TsType::flat)
                    .collect::<Option<Vec<_>>>()?;
                format!("[{}]", parts.join(", "))
            }
            TsType::Object(fields) if fields.is_empty() => String::from("{}"),
            TsType::Object(fields) => {
                let mut parts = Vec::with_capacity(fields.len());
                for field in fields {
                    if field.doc.is_some() {
                        return None;
                    }
                    parts.push(format!("{}: {}", field_key(field), field.ty.flat()?));
                }
                format!("{{ {} }}", parts.join("; "))
            }
            TsType::Union(members) if members.is_empty() => String::from("never"),
            TsType::Union(members) => {
                let mut parts = Vec::with_capacity(members.len());
                for member in members {
                    if member.doc.is_some() {
                        return None;
                    }
                    parts.push(member.ty.flat()?);
                }
                parts.join(" | ")
            }
            TsType::Intersection(parts) if parts.is_empty() => String::from("unknown"),
            TsType::Intersection(parts) => {
                let mut texts = Vec::with_capacity(parts.len());
                for part in parts {
                    let text = part.flat()?;
                    if part.is_compound_union() {
                        texts.push(format!("({text})"));
                    } else {
                        texts.push(text);
                    }
                }
                texts.join(" & ")
            }
            TsType::Record {
                key,
                value,
                partial,
            } => {
                let record = format!("Record<{}, {}>", key.flat()?, value.flat()?);
                if *partial {
                    format!("Partial<{record}>")
                } else {
                    record
                }
            }
        })
    }

    /// The type as it starts at `column` on a line indented by `indent`,
    /// with `tail` characters after it on its last line.
    ///
    /// A union that breaks starts with a line break, so whoever places it
    /// after a `:` or `=` leaves out the space.
    fn layout(&self, indent: usize, column: usize, tail: usize) -> String {
        if let Some(flat) = self.flat() {
            if column + flat.len() + tail <= WIDTH {
                return flat;
            }
        }
        match self {
            TsType::Object(fields) if !fields.is_empty() => render_fields(fields, indent),
            TsType::Union(members) => match members.as_slice() {
                [] => String::from("never"),
                [only] if only.doc.is_none() => only.ty.layout(indent, column, tail),
                _ => {
                    let mut out = String::new();
                    let member_indent = indent + 2;
                    for (index, member) in members.iter().enumerate() {
                        out.push('\n');
                        if let Some(doc) = &member.doc {
                            out.push_str(&jsdoc(doc, member_indent));
                        }
                        out.push_str(&spaces(member_indent));
                        out.push_str("| ");
                        let member_tail = if index + 1 == members.len() { tail } else { 0 };
                        // A member that breaks indents from where it starts,
                        // after the bar.
                        out.push_str(&operand(
                            &member.ty,
                            member_indent + 2,
                            member_indent + 2,
                            member_tail,
                            TsType::is_compound_union,
                        ));
                    }
                    out
                }
            },
            TsType::Array(element) => {
                let mut out = operand(
                    element,
                    indent,
                    column,
                    tail + 2,
                    TsType::needs_parentheses_in_array,
                );
                out.push_str("[]");
                out
            }
            TsType::Tuple(elements) if !elements.is_empty() => {
                let mut out = String::from("[\n");
                for element in elements {
                    out.push_str(&spaces(indent + 2));
                    out.push_str(&operand(element, indent + 2, indent + 2, 1, |_| false));
                    out.push_str(",\n");
                }
                out.push_str(&spaces(indent));
                out.push(']');
                out
            }
            TsType::Intersection(parts) if !parts.is_empty() => {
                let mut out = String::new();
                for (index, part) in parts.iter().enumerate() {
                    if index > 0 {
                        out.push_str(" & ");
                    }
                    let part_column = end_column(&out, column);
                    let part_tail = if index + 1 == parts.len() { tail } else { 3 };
                    out.push_str(&operand(
                        part,
                        indent,
                        part_column,
                        part_tail,
                        TsType::is_compound_union,
                    ));
                }
                out
            }
            TsType::Record {
                key,
                value,
                partial,
            } => {
                let mut out = String::from(if *partial {
                    "Partial<Record<"
                } else {
                    "Record<"
                });
                let key_text = key.layout(indent, column + out.len(), 2);
                out.push_str(&key_text);
                out.push_str(", ");
                let closing = if *partial { 2 } else { 1 };
                let value_column = end_column(&out, column);
                out.push_str(&operand(
                    value,
                    indent,
                    value_column,
                    tail + closing,
                    |_| false,
                ));
                out.push_str(if *partial { ">>" } else { ">" });
                out
            }
            // Keywords, literals, names and raw text have one form; a raw
            // text with lines of its own keeps them.
            TsType::Keyword(_) | TsType::Literal(_) | TsType::Named(_) => {
                self.flat().unwrap_or_default()
            }
            TsType::Raw(text) => text.clone(),
            TsType::Object(_) => String::from("{}"),
            TsType::Tuple(_) => String::from("[]"),
            TsType::Intersection(_) => String::from("unknown"),
        }
    }
}

/// The type inside an array, a tuple, an intersection or a record: in
/// parentheses when `parenthesize` says it needs them or when it is a union
/// that breaks, since such a union starts with a line break.
fn operand(
    ty: &TsType,
    indent: usize,
    column: usize,
    tail: usize,
    parenthesize: fn(&TsType) -> bool,
) -> String {
    let needs = parenthesize(ty);
    let text = if needs {
        ty.layout(indent, column + 1, tail + 1)
    } else {
        ty.layout(indent, column, tail)
    };
    if text.starts_with('\n') {
        format!("({text}\n{})", spaces(indent))
    } else if needs {
        format!("({text})")
    } else {
        text
    }
}

/// The fields of an object, one per line, indented two more than `indent`,
/// in braces.
fn render_fields(fields: &[TsField], indent: usize) -> String {
    let field_indent = indent + 2;
    let mut out = String::from("{\n");
    for field in fields {
        if let Some(doc) = &field.doc {
            out.push_str(&jsdoc(doc, field_indent));
        }
        let head = format!("{}{}:", spaces(field_indent), field_key(field));
        out.push_str(&after(&head, &field.ty, field_indent, 1));
        out.push_str(";\n");
    }
    out.push_str(&spaces(indent));
    out.push('}');
    out
}

/// `head` (which ends in `:` or `=`) followed by the type: after a space,
/// or right away when the type breaks into lines starting with a line break.
pub(crate) fn after(head: &str, ty: &TsType, indent: usize, tail: usize) -> String {
    let text = ty.layout(indent, head.len() + 1, tail);
    if text.starts_with('\n') {
        format!("{head}{text}")
    } else {
        format!("{head} {text}")
    }
}

/// The column after `text`, which starts at `column`.
fn end_column(text: &str, column: usize) -> usize {
    match text.rfind('\n') {
        Some(position) => text.len() - position - 1,
        None => column + text.len(),
    }
}

/// `count` spaces.
fn spaces(count: usize) -> String {
    " ".repeat(count)
}

/// A string as a TypeScript string literal, which a JSON string is.
fn string_literal(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| String::from("\"\""))
}

/// The key of a field: its name, quoted when it is no valid identifier.
fn field_key(field: &TsField) -> String {
    let mut key = if is_identifier(&field.name) {
        field.name.clone()
    } else {
        string_literal(&field.name)
    };
    if field.optional {
        key.push('?');
    }
    key
}

/// Whether `name` is a valid TypeScript identifier.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' || first == '$' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        }
        _ => false,
    }
}

/// A doc comment as JSDoc at `indent`, ending in a line break: on one line
/// when the doc is one line and fits, else one line per doc line as written.
fn jsdoc(doc: &str, indent: usize) -> String {
    // A `*/` in the text would end the comment early.
    let doc = doc.replace("*/", "*\\/");
    let pad = spaces(indent);
    if !doc.contains('\n') && indent + doc.len() + 7 <= WIDTH {
        return format!("{pad}/** {doc} */\n");
    }
    let mut out = format!("{pad}/**\n");
    for line in doc.lines() {
        if line.is_empty() {
            out.push_str(&format!("{pad} *\n"));
        } else {
            out.push_str(&format!("{pad} * {line}\n"));
        }
    }
    out.push_str(&format!("{pad} */\n"));
    out
}

/// A declaration of [`TypeScriptTypes`].
struct Declaration {
    /// The TypeScript name.
    name: String,
    /// The Rust type it declares, for the error of a second one.
    type_id: TypeId,
    /// The name of the Rust type, for the error of a second one.
    rust_type: &'static str,
    /// The doc comment of the Rust type.
    doc: Option<String>,
    /// The type, `None` while it is being declared.
    ty: Option<TsType>,
}

/// The named declarations that the types of a model refer to, in the order
/// of their first use.
#[derive(Default)]
pub struct TypeScriptTypes {
    /// The declarations in the order of their first use.
    declarations: Vec<Declaration>,
    /// The index in `declarations` of each name.
    by_name: HashMap<String, usize>,
    /// What went wrong: two Rust types under one name.
    errors: Vec<String>,
}

impl TypeScriptTypes {
    /// No declarations yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The declaration `name` of the Rust type `T`: declared on first use
    /// through `declare`, then referred to by name.
    ///
    /// The name is taken before `declare` runs, so a recursive type ends
    /// at its own name. A second Rust type under the same name records an
    /// error that names both Rust types.
    pub fn named<T: ?Sized + 'static>(
        &mut self,
        name: &str,
        doc: Option<&str>,
        declare: impl FnOnce(&mut Self) -> TsType,
    ) -> TsType {
        let type_id = TypeId::of::<T>();
        if let Some(&index) = self.by_name.get(name) {
            let existing = &self.declarations[index];
            if existing.type_id != type_id {
                let error = format!(
                    "two Rust types have the TypeScript name `{name}`: `{}` and `{}`. \
                     Give one of them another one with #[type_script(rename = \"...\")].",
                    existing.rust_type,
                    type_name::<T>()
                );
                if !self.errors.contains(&error) {
                    self.errors.push(error);
                }
            }
            return TsType::Named(name.to_string());
        }

        let index = self.declarations.len();
        self.declarations.push(Declaration {
            name: name.to_string(),
            type_id,
            rust_type: type_name::<T>(),
            doc: doc.map(str::to_string),
            ty: None,
        });
        self.by_name.insert(name.to_string(), index);
        let ty = declare(self);
        self.declarations[index].ty = Some(ty);
        TsType::Named(name.to_string())
    }

    /// The type of the declaration `name`, `None` while it is being declared
    /// or when there is none.
    pub fn declaration(&self, name: &str) -> Option<&TsType> {
        let index = *self.by_name.get(name)?;
        self.declarations[index].ty.as_ref()
    }

    /// Every declaration in the order of first use, separated by blank
    /// lines: `export interface` for an object type, `export type` for any
    /// other, each with the doc comment of its Rust type as JSDoc.
    pub fn render_declarations(&self) -> String {
        let mut out = String::new();
        for declaration in &self.declarations {
            let Some(ty) = &declaration.ty else {
                continue;
            };
            if !out.is_empty() {
                out.push('\n');
            }
            if let Some(doc) = &declaration.doc {
                out.push_str(&jsdoc(doc, 0));
            }
            let name = &declaration.name;
            match ty {
                TsType::Object(fields) if fields.is_empty() => {
                    out.push_str(&format!("export interface {name} {{}}"));
                }
                TsType::Object(fields) => {
                    out.push_str(&format!("export interface {name} "));
                    out.push_str(&render_fields(fields, 0));
                }
                ty => {
                    out.push_str(&after(&format!("export type {name} ="), ty, 0, 1));
                    out.push(';');
                }
            }
            out.push('\n');
        }
        out
    }

    /// What went wrong while the types were declared.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// The type behind any names, `None` for a name still being declared.
    fn resolve<'a>(&'a self, ty: &'a TsType) -> Option<&'a TsType> {
        let mut current = ty;
        // A name that refers to itself through others ends after as many
        // steps as there are declarations.
        for _ in 0..=self.declarations.len() {
            match current {
                TsType::Named(name) => current = self.declaration(name)?,
                ty => return Some(ty),
            }
        }
        None
    }

    /// The record of a map from keys of type `key`: JSON writes every key as
    /// a string, so string and number keys are `Record<string, V>`; others
    /// (unit enums) take only some strings, so their map is `Partial`.
    fn record(&self, key: TsType, value: TsType) -> TsType {
        let (key, partial) = match self.resolve(&key) {
            Some(TsType::Keyword("string" | "number")) => (TsType::Keyword("string"), false),
            Some(TsType::Keyword("boolean")) => (
                TsType::Union(vec![
                    TsMember {
                        doc: None,
                        ty: TsType::Literal(String::from("true")),
                    },
                    TsMember {
                        doc: None,
                        ty: TsType::Literal(String::from("false")),
                    },
                ]),
                true,
            ),
            _ => (key, true),
        };
        TsType::Record {
            key: Box::new(key),
            value: Box::new(value),
            partial,
        }
    }
}

// =============================================================================
// Standard types
// =============================================================================

/// Implement `TypeScript` for types that are a keyword.
macro_rules! keyword {
    ($keyword:literal: $($ty:ty),*) => {
        $(
            impl TypeScript for $ty {
                fn type_script(_: &mut TypeScriptTypes) -> TsType {
                    TsType::Keyword($keyword)
                }
            }
        )*
    };
}

keyword!("boolean": bool);
keyword!("number": i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize, f32, f64);
keyword!("string": char, String, str);
keyword!("null": ());
keyword!("unknown": serde_json::Value);

impl<T: ?Sized> TypeScript for PhantomData<T> {
    fn type_script(_: &mut TypeScriptTypes) -> TsType {
        NULL
    }
}

impl<T: TypeScript> TypeScript for Option<T> {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        T::type_script(types).nullable()
    }
}

/// Implement `TypeScript` for wrappers that serde writes as what they hold.
macro_rules! transparent {
    ($($ty:ty),*) => {
        $(
            impl<T: ?Sized + TypeScript> TypeScript for $ty {
                fn type_script(types: &mut TypeScriptTypes) -> TsType {
                    T::type_script(types)
                }
            }
        )*
    };
}

transparent!(&T, &mut T, Box<T>, Rc<T>, Arc<T>);

impl<T: ?Sized + ToOwned + TypeScript> TypeScript for Cow<'_, T> {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        T::type_script(types)
    }
}

/// Implement `TypeScript` for sequences that serde writes as arrays.
macro_rules! array {
    ($($ty:ty),*) => {
        $(
            impl<T: TypeScript> TypeScript for $ty {
                fn type_script(types: &mut TypeScriptTypes) -> TsType {
                    TsType::Array(Box::new(T::type_script(types)))
                }
            }
        )*
    };
}

array!([T], Vec<T>, VecDeque<T>, BTreeSet<T>);

impl<T: TypeScript, S> TypeScript for HashSet<T, S> {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        TsType::Array(Box::new(T::type_script(types)))
    }
}

/// The longest array written as a tuple.
const LONGEST_TUPLE_ARRAY: usize = 16;

impl<T: TypeScript, const N: usize> TypeScript for [T; N] {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        let element = T::type_script(types);
        if N <= LONGEST_TUPLE_ARRAY {
            TsType::Tuple(vec![element; N])
        } else {
            TsType::Array(Box::new(element))
        }
    }
}

/// Implement `TypeScript` for tuples.
macro_rules! tuple {
    ($($name:ident),+) => {
        impl<$($name: TypeScript),+> TypeScript for ($($name,)+) {
            fn type_script(types: &mut TypeScriptTypes) -> TsType {
                TsType::Tuple(vec![$($name::type_script(types)),+])
            }
        }
    };
}

tuple!(A);
tuple!(A, B);
tuple!(A, B, C);
tuple!(A, B, C, D);
tuple!(A, B, C, D, E);
tuple!(A, B, C, D, E, F);
tuple!(A, B, C, D, E, F, G);
tuple!(A, B, C, D, E, F, G, H);
tuple!(A, B, C, D, E, F, G, H, I);
tuple!(A, B, C, D, E, F, G, H, I, J);
tuple!(A, B, C, D, E, F, G, H, I, J, K);
tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

impl<K: TypeScript, V: TypeScript, S> TypeScript for HashMap<K, V, S> {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        let key = K::type_script(types);
        let value = V::type_script(types);
        types.record(key, value)
    }
}

impl<K: TypeScript, V: TypeScript> TypeScript for BTreeMap<K, V> {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        let key = K::type_script(types);
        let value = V::type_script(types);
        types.record(key, value)
    }
}

impl TypeScript for serde_json::Map<String, serde_json::Value> {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        types.record(TsType::Keyword("string"), TsType::Keyword("unknown"))
    }
}

// =============================================================================
// Beamer types
// =============================================================================

// The types of beamer-core that a `State` can hold. beamer-core does not
// know this trait, so they are declared here as the derive would declare
// them from their serde attributes.

impl TypeScript for beamer_core::CurvePoint {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        types.named::<Self>(
            "CurvePoint",
            Some("A point of a `Curve`, both coordinates from 0.0 to 1.0."),
            |types| {
                TsType::Object(vec![
                    __private::field("x", Some("The value that goes in."), false, f64::type_script(types)),
                    __private::field("y", Some("The value that comes out."), false, f64::type_script(types)),
                ])
            },
        )
    }
}

impl TypeScript for beamer_core::CurveShape {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        types.named::<Self>("CurveShape", Some("How a `Curve` runs between its points."), |_| {
            TsType::Union(vec![
                __private::member(
                    Some("Straight lines from point to point."),
                    __private::literal("linear"),
                ),
                __private::member(
                    Some("Smooth through every point without overshooting them: a monotone cubic."),
                    __private::literal("smooth"),
                ),
            ])
        })
    }
}

impl TypeScript for beamer_core::Curve {
    fn type_script(types: &mut TypeScriptTypes) -> TsType {
        types.named::<Self>(
            "Curve",
            Some(
                "A curve through points, from 0.0 to 1.0 in and out.\n\
                 At least two points, sorted by x and none at the same x.",
            ),
            |types| {
                TsType::Object(vec![
                    __private::field("shape", None, false, beamer_core::CurveShape::type_script(types)),
                    __private::field("points", None, false, <Vec<beamer_core::CurvePoint>>::type_script(types)),
                ])
            },
        )
    }
}

// =============================================================================
// For generated code
// =============================================================================

/// What code generated by `#[derive(TypeScript)]` and the model macros
/// calls.
///
/// [`TypeProbe`](__private::TypeProbe) asks whether a concrete type
/// implements [`TypeScript`] without requiring it:
/// `(&TypeProbe::<T>::new()).type_script_if_any(types)` takes
/// [`HasTypeScript`](__private::HasTypeScript) when `T` implements it.
/// Otherwise only [`LacksTypeScript`](__private::LacksTypeScript) fits, one
/// reference further. The generated code knows the type, so the choice is
/// made when it is compiled.
#[doc(hidden)]
pub mod __private {
    use std::marker::PhantomData;

    use super::{TsField, TsMember, TsType, TypeScript, TypeScriptTypes, NULL};

    /// A type to ask about.
    pub struct TypeProbe<T: ?Sized>(PhantomData<T>);

    impl<T: ?Sized> TypeProbe<T> {
        /// A probe of `T`.
        #[allow(clippy::new_without_default)]
        pub const fn new() -> Self {
            Self(PhantomData)
        }
    }

    /// The TypeScript of a type that has one.
    pub trait HasTypeScript {
        /// The type as TypeScript.
        fn type_script_if_any(&self, types: &mut TypeScriptTypes) -> Option<TsType>;
    }

    impl<T: ?Sized + TypeScript> HasTypeScript for TypeProbe<T> {
        fn type_script_if_any(&self, types: &mut TypeScriptTypes) -> Option<TsType> {
            Some(T::type_script(types))
        }
    }

    /// The answer for a type without TypeScript.
    pub trait LacksTypeScript {
        /// Nothing: the type has no TypeScript.
        fn type_script_if_any(&self, types: &mut TypeScriptTypes) -> Option<TsType> {
            let _ = types;
            None
        }
    }

    impl<T: ?Sized> LacksTypeScript for &TypeProbe<T> {}

    /// A field of an object type.
    pub fn field(name: &str, doc: Option<&str>, optional: bool, ty: TsType) -> TsField {
        TsField {
            name: name.to_string(),
            doc: doc.map(str::to_string),
            optional,
            ty,
        }
    }

    /// A member of a union.
    pub fn member(doc: Option<&str>, ty: TsType) -> TsMember {
        TsMember {
            doc: doc.map(str::to_string),
            ty,
        }
    }

    /// A string literal type.
    pub fn literal(text: &str) -> TsType {
        TsType::Literal(text.to_string())
    }

    /// TypeScript written by hand.
    pub fn raw(text: &str) -> TsType {
        TsType::Raw(text.to_string())
    }

    /// Fields beside flattened types: one object, or its intersection with
    /// them.
    pub fn with_flattened(
        types: &TypeScriptTypes,
        fields: Vec<TsField>,
        flattened: Vec<TsType>,
    ) -> TsType {
        if flattened.is_empty() {
            return TsType::Object(fields);
        }
        let mut parts = Vec::with_capacity(flattened.len() + 1);
        if !fields.is_empty() {
            parts.push(TsType::Object(fields));
        }
        parts.extend(flattened.into_iter().map(|ty| in_object(types, ty)));
        if parts.len() == 1 {
            parts.remove(0)
        } else {
            TsType::Intersection(parts)
        }
    }

    /// The newtype variant of an internally tagged enum: the tag in the
    /// object of the content, or beside it.
    pub fn tagged_newtype(types: &TypeScriptTypes, tag: TsField, content: TsType) -> TsType {
        match in_object(types, content) {
            TsType::Object(mut fields) => {
                fields.insert(0, tag);
                TsType::Object(fields)
            }
            content => TsType::Intersection(vec![TsType::Object(vec![tag]), content]),
        }
    }

    /// The type as serde writes it into an object: flattened into one or as
    /// the content of an internally tagged newtype variant. There `null` (a
    /// `None` or a unit struct) adds no field and the unit variant `"U"` of
    /// an externally tagged enum is `{ U: null }`, also in the members of a
    /// union. A type that changes is written out in place of its name.
    fn in_object(types: &TypeScriptTypes, ty: TsType) -> TsType {
        in_object_unless(types, &ty, &mut Vec::new()).unwrap_or(ty)
    }

    /// The type in an object, `None` when it stays the same. `visited`
    /// holds the names of the unions on the way, so that a union that
    /// holds itself ends.
    fn in_object_unless(
        types: &TypeScriptTypes,
        ty: &TsType,
        visited: &mut Vec<String>,
    ) -> Option<TsType> {
        let name = match ty {
            TsType::Named(name) if visited.contains(name) => return None,
            TsType::Named(name) => Some(name),
            _ => None,
        };
        match types.resolve(ty)? {
            TsType::Literal(variant) => {
                Some(TsType::Object(vec![field(variant, None, false, NULL)]))
            }
            TsType::Keyword("null") => Some(TsType::Object(Vec::new())),
            TsType::Union(members) => {
                visited.extend(name.cloned());
                let changed: Vec<Option<TsType>> = members
                    .iter()
                    .map(|member| in_object_unless(types, &member.ty, visited))
                    .collect();
                if name.is_some() {
                    visited.pop();
                }
                if changed.iter().all(Option::is_none) {
                    return None;
                }
                Some(TsType::Union(
                    members
                        .iter()
                        .zip(changed)
                        .map(|(member, changed)| TsMember {
                            doc: member.doc.clone(),
                            ty: changed.unwrap_or_else(|| member.ty.clone()),
                        })
                        .collect(),
                ))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::needless_borrow)]
mod tests {
    use super::__private::{HasTypeScript as _, LacksTypeScript as _, TypeProbe};
    use super::*;

    /// The type of `T`, inline, with the declarations it made.
    fn type_of<T: ?Sized + TypeScript>() -> (String, TypeScriptTypes) {
        let mut types = TypeScriptTypes::new();
        let ty = T::type_script(&mut types);
        (ty.render(0), types)
    }

    /// The type of `T` rendered inline.
    fn ts<T: ?Sized + TypeScript>() -> String {
        type_of::<T>().0
    }

    fn field(name: &str, ty: TsType) -> TsField {
        TsField {
            name: name.to_string(),
            doc: None,
            optional: false,
            ty,
        }
    }

    fn member(ty: TsType) -> TsMember {
        TsMember { doc: None, ty }
    }

    fn literal(text: &str) -> TsType {
        TsType::Literal(text.to_string())
    }

    const NUMBER: TsType = TsType::Keyword("number");
    const STRING: TsType = TsType::Keyword("string");

    #[test]
    fn test_keywords() {
        assert_eq!(ts::<bool>(), "boolean");
        for number in [
            ts::<i8>(),
            ts::<i16>(),
            ts::<i32>(),
            ts::<i64>(),
            ts::<i128>(),
            ts::<isize>(),
            ts::<u8>(),
            ts::<u16>(),
            ts::<u32>(),
            ts::<u64>(),
            ts::<u128>(),
            ts::<usize>(),
            ts::<f32>(),
            ts::<f64>(),
        ] {
            assert_eq!(number, "number");
        }
        assert_eq!(ts::<char>(), "string");
        assert_eq!(ts::<String>(), "string");
        assert_eq!(ts::<str>(), "string");
        assert_eq!(ts::<Cow<'static, str>>(), "string");
        assert_eq!(ts::<()>(), "null");
        assert_eq!(ts::<PhantomData<u8>>(), "null");
        assert_eq!(ts::<serde_json::Value>(), "unknown");
    }

    #[test]
    fn test_option_takes_null_once() {
        assert_eq!(ts::<Option<u8>>(), "number | null");
        assert_eq!(ts::<Option<Option<u8>>>(), "number | null");
        assert_eq!(ts::<Option<()>>(), "null");
        assert_eq!(ts::<Option<serde_json::Value>>(), "unknown");
        assert_eq!(ts::<Vec<Option<bool>>>(), "(boolean | null)[]");
    }

    #[test]
    fn test_wrappers_are_what_they_hold() {
        assert_eq!(ts::<&str>(), "string");
        assert_eq!(ts::<&mut u8>(), "number");
        assert_eq!(ts::<Box<str>>(), "string");
        assert_eq!(ts::<Rc<bool>>(), "boolean");
        assert_eq!(ts::<Arc<String>>(), "string");
        assert_eq!(ts::<Cow<'static, [u8]>>(), "number[]");
    }

    #[test]
    fn test_sequences_are_arrays() {
        assert_eq!(ts::<Vec<f32>>(), "number[]");
        assert_eq!(ts::<VecDeque<String>>(), "string[]");
        assert_eq!(ts::<[u8]>(), "number[]");
        assert_eq!(ts::<HashSet<u8>>(), "number[]");
        assert_eq!(ts::<BTreeSet<char>>(), "string[]");
        assert_eq!(ts::<Vec<Vec<bool>>>(), "boolean[][]");
    }

    #[test]
    fn test_arrays_are_tuples_up_to_sixteen() {
        assert_eq!(ts::<[f32; 0]>(), "[]");
        assert_eq!(ts::<[f32; 2]>(), "[number, number]");
        let sixteen = ts::<[u8; 16]>();
        assert_eq!(sixteen.matches("number").count(), 16);
        assert!(sixteen.starts_with('['));
        assert_eq!(ts::<[u8; 17]>(), "number[]");
    }

    #[test]
    fn test_tuples() {
        assert_eq!(ts::<(u8,)>(), "[number]");
        assert_eq!(ts::<(u8, String, bool)>(), "[number, string, boolean]");
        let twelve = ts::<(u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, u8)>();
        assert_eq!(twelve.matches("number").count(), 12);
    }

    #[test]
    fn test_maps_are_records() {
        assert_eq!(ts::<HashMap<String, f32>>(), "Record<string, number>");
        assert_eq!(ts::<BTreeMap<u32, bool>>(), "Record<string, boolean>");
        assert_eq!(ts::<BTreeMap<char, ()>>(), "Record<string, null>");
        assert_eq!(
            ts::<HashMap<bool, u8>>(),
            "Partial<Record<\"true\" | \"false\", number>>"
        );
        assert_eq!(
            ts::<serde_json::Map<String, serde_json::Value>>(),
            "Record<string, unknown>"
        );
    }

    /// A unit enum as a derive declares it.
    struct Tab;

    impl TypeScript for Tab {
        fn type_script(types: &mut TypeScriptTypes) -> TsType {
            types.named::<Self>("Tab", None, |_| {
                TsType::Union(vec![member(literal("pads")), member(literal("mixer"))])
            })
        }
    }

    /// A newtype of a string as a derive declares it.
    struct Id;

    impl TypeScript for Id {
        fn type_script(types: &mut TypeScriptTypes) -> TsType {
            types.named::<Self>("Id", None, |_| STRING)
        }
    }

    #[test]
    fn test_maps_of_named_keys() {
        assert_eq!(ts::<BTreeMap<Tab, u8>>(), "Partial<Record<Tab, number>>");
        assert_eq!(ts::<HashMap<Id, u8>>(), "Record<string, number>");
    }

    #[test]
    fn test_type_probe_tells_types_with_type_script_from_others() {
        struct Plain;

        let mut types = TypeScriptTypes::new();
        assert_eq!(
            (&TypeProbe::<f32>::new()).type_script_if_any(&mut types),
            Some(NUMBER)
        );
        assert_eq!(
            (&TypeProbe::<Tab>::new()).type_script_if_any(&mut types),
            Some(TsType::Named(String::from("Tab")))
        );
        assert_eq!(
            (&TypeProbe::<Plain>::new()).type_script_if_any(&mut types),
            None
        );
        assert_eq!(
            (&TypeProbe::<str>::new()).type_script_if_any(&mut types),
            Some(STRING)
        );
    }

    /// A list that holds itself, as a derive declares it.
    struct Node;

    impl TypeScript for Node {
        fn type_script(types: &mut TypeScriptTypes) -> TsType {
            types.named::<Self>("Node", Some("A node of a tree."), |types| {
                TsType::Object(vec![
                    field("value", NUMBER),
                    field("children", <Vec<Node>>::type_script(types)),
                    field("parent", <Option<Box<Node>>>::type_script(types)),
                ])
            })
        }
    }

    #[test]
    fn test_curve_is_what_serde_writes() {
        let (inline, types) = type_of::<beamer_core::Curve>();
        assert_eq!(inline, "Curve");
        assert_eq!(
            types.render_declarations(),
            "/**\n \
             * A curve through points, from 0.0 to 1.0 in and out.\n \
             * At least two points, sorted by x and none at the same x.\n \
             */\n\
             export interface Curve {\n  shape: CurveShape;\n  points: CurvePoint[];\n}\n\
             \n\
             /** How a `Curve` runs between its points. */\n\
             export type CurveShape =\n  \
             /** Straight lines from point to point. */\n  \
             | \"linear\"\n  \
             /** Smooth through every point without overshooting them: a monotone cubic. */\n  \
             | \"smooth\";\n\
             \n\
             /** A point of a `Curve`, both coordinates from 0.0 to 1.0. */\n\
             export interface CurvePoint {\n  \
             /** The value that goes in. */\n  x: number;\n  \
             /** The value that comes out. */\n  y: number;\n}\n"
        );
        assert!(types.errors().is_empty());
    }

    #[test]
    fn test_named_ends_recursive_types() {
        let (inline, types) = type_of::<Node>();
        assert_eq!(inline, "Node");
        assert_eq!(
            types.render_declarations(),
            "/** A node of a tree. */\n\
             export interface Node {\n  value: number;\n  children: Node[];\n  parent: Node | null;\n}\n"
        );
        assert!(types.errors().is_empty());
    }

    mod other {
        use super::super::{TsType, TypeScript, TypeScriptTypes};

        /// Another type named `Tab` in TypeScript.
        pub struct Tab;

        impl TypeScript for Tab {
            fn type_script(types: &mut TypeScriptTypes) -> TsType {
                types.named::<Self>("Tab", None, |_| TsType::Keyword("number"))
            }
        }
    }

    #[test]
    fn test_named_reports_two_rust_types_under_one_name() {
        let mut types = TypeScriptTypes::new();
        Tab::type_script(&mut types);
        Tab::type_script(&mut types);
        assert!(types.errors().is_empty());
        assert_eq!(
            other::Tab::type_script(&mut types),
            TsType::Named(String::from("Tab"))
        );
        other::Tab::type_script(&mut types);
        assert_eq!(types.errors().len(), 1);
        let error = &types.errors()[0];
        assert!(error.contains("`Tab`"), "{error}");
        assert!(error.contains("type_script::tests::Tab"), "{error}");
        assert!(error.contains("type_script::tests::other::Tab"), "{error}");
        assert!(error.contains("#[type_script(rename"), "{error}");
        // The first type keeps the name.
        assert_eq!(
            types.render_declarations(),
            "export type Tab = \"pads\" | \"mixer\";\n"
        );
    }

    #[test]
    fn test_declarations_in_order_of_first_use() {
        let mut types = TypeScriptTypes::new();
        Id::type_script(&mut types);
        Node::type_script(&mut types);
        Id::type_script(&mut types);
        Tab::type_script(&mut types);
        let rendered = types.render_declarations();
        let id = rendered.find("export type Id").unwrap();
        let node = rendered.find("export interface Node").unwrap();
        let tab = rendered.find("export type Tab").unwrap();
        assert!(id < node && node < tab);
        assert!(rendered.contains("string;\n\n/** A node"));
    }

    #[test]
    fn test_jsdoc_on_one_line_when_it_fits() {
        assert_eq!(jsdoc("Short.", 0), "/** Short. */\n");
        assert_eq!(jsdoc("Short.", 4), "    /** Short. */\n");
        // 80 columns exactly still fit.
        let fits = "x".repeat(73);
        assert_eq!(jsdoc(&fits, 0), format!("/** {fits} */\n"));
        let long = "x".repeat(74);
        assert_eq!(jsdoc(&long, 0), format!("/**\n * {long}\n */\n"));
        assert_eq!(jsdoc(&fits, 2), format!("  /**\n   * {fits}\n   */\n"));
    }

    #[test]
    fn test_jsdoc_keeps_lines_as_written() {
        assert_eq!(
            jsdoc("First line\nsecond line.\n\nAfter a gap.", 2),
            "  /**\n   * First line\n   * second line.\n   *\n   * After a gap.\n   */\n"
        );
        assert_eq!(jsdoc("Ends */ early.", 0), "/** Ends *\\/ early. */\n");
    }

    fn declare(name: &str, doc: Option<&str>, ty: TsType) -> String {
        struct Any;
        let mut types = TypeScriptTypes::new();
        types.named::<Any>(name, doc, |_| ty);
        types.render_declarations()
    }

    #[test]
    fn test_union_on_one_line_when_it_fits() {
        let union = TsType::Union(vec![
            member(literal("constant_power")),
            member(literal("linear")),
        ]);
        assert_eq!(
            declare("PanLaw", None, union),
            "export type PanLaw = \"constant_power\" | \"linear\";\n"
        );
    }

    #[test]
    fn test_union_on_many_lines_when_too_long() {
        let names = [
            "threshold",
            "ratio",
            "attack",
            "release",
            "knee",
            "lookahead",
            "makeup",
        ];
        let union = TsType::Union(names.iter().map(|name| member(literal(name))).collect());
        assert_eq!(
            declare("Id", None, union.clone()),
            "export type Id =\n  | \"threshold\"\n  | \"ratio\"\n  | \"attack\"\n  | \"release\"\n  \
             | \"knee\"\n  | \"lookahead\"\n  | \"makeup\";\n"
        );
        // In a field, the members indent from the field.
        let object = TsType::Object(vec![field("parameters", union)]);
        assert!(declare("Model", None, object)
            .starts_with("export interface Model {\n  parameters:\n    | \"threshold\"\n"));
    }

    #[test]
    fn test_union_on_many_lines_when_a_member_has_a_doc() {
        let union = TsType::Union(vec![
            TsMember {
                doc: Some(String::from("The first.")),
                ty: literal("a"),
            },
            member(TsType::Object(vec![
                field("type", literal("b")),
                field("value", NUMBER),
            ])),
        ]);
        assert_eq!(
            declare("Choice", None, union),
            "export type Choice =\n  /** The first. */\n  | \"a\"\n  | { type: \"b\"; value: number };\n"
        );
    }

    #[test]
    fn test_object_member_that_breaks_indents_from_its_start() {
        let union = TsType::Union(vec![
            TsMember {
                doc: None,
                ty: TsType::Object(vec![TsField {
                    name: String::from("value"),
                    doc: Some(String::from("The value.")),
                    optional: false,
                    ty: NUMBER,
                }]),
            },
            member(TsType::Object(vec![field("type", literal("reset"))])),
        ]);
        assert_eq!(
            declare("Event", None, union),
            "export type Event =\n  | {\n      /** The value. */\n      value: number;\n    }\n  | { type: \"reset\" };\n"
        );
    }

    #[test]
    fn test_object_inline_on_one_line_when_it_fits() {
        let ty = TsType::Object(vec![field("type", literal("reset"))]);
        assert_eq!(ty.render(0), "{ type: \"reset\" }");
        let ty = TsType::Object(vec![
            TsField {
                name: String::from("a b"),
                doc: None,
                optional: true,
                ty: NUMBER,
            },
            field("$ok_1", STRING),
            field("1st", STRING),
        ]);
        assert_eq!(
            ty.render(0),
            "{ \"a b\"?: number; $ok_1: string; \"1st\": string }"
        );
        assert_eq!(TsType::Object(Vec::new()).render(0), "{}");
    }

    #[test]
    fn test_object_inline_breaks_with_a_doc_or_when_too_long() {
        let documented = TsType::Object(vec![TsField {
            name: String::from("gain"),
            doc: Some(String::from("The gain.")),
            optional: false,
            ty: NUMBER,
        }]);
        assert_eq!(
            documented.render(2),
            "{\n    /** The gain. */\n    gain: number;\n  }"
        );
        let long = TsType::Object(
            (0..8)
                .map(|index| field(&format!("field{index}"), STRING))
                .collect(),
        );
        let rendered = long.render(0);
        assert!(rendered.starts_with("{\n  field0: string;\n"));
        assert!(rendered.ends_with("  field7: string;\n}"));
    }

    #[test]
    fn test_interface_breaks_always() {
        let ty = TsType::Object(vec![field("a", NUMBER)]);
        assert_eq!(
            declare("A", None, ty),
            "export interface A {\n  a: number;\n}\n"
        );
        assert_eq!(
            declare("E", None, TsType::Object(Vec::new())),
            "export interface E {}\n"
        );
    }

    #[test]
    fn test_arrays_parenthesize_unions_and_intersections() {
        let union = TsType::Union(vec![member(NUMBER), member(STRING)]);
        assert_eq!(
            TsType::Array(Box::new(union.clone())).render(0),
            "(number | string)[]"
        );
        let intersection = TsType::Intersection(vec![
            TsType::Named(String::from("A")),
            TsType::Named(String::from("B")),
        ]);
        assert_eq!(TsType::Array(Box::new(intersection)).render(0), "(A & B)[]");
        assert_eq!(
            TsType::Array(Box::new(TsType::Union(vec![member(NUMBER)]))).render(0),
            "number[]"
        );
        assert_eq!(
            TsType::Intersection(vec![union, TsType::Named(String::from("C"))]).render(0),
            "(number | string) & C"
        );
        assert_eq!(
            TsType::Array(Box::new(TsType::Raw(String::from("A | B")))).render(0),
            "(A | B)[]"
        );
    }

    #[test]
    fn test_empty_union_is_never() {
        assert_eq!(TsType::Union(Vec::new()).render(0), "never");
        assert_eq!(
            declare("Never", None, TsType::Union(Vec::new())),
            "export type Never = never;\n"
        );
    }

    #[test]
    fn test_records_and_literals() {
        let record = TsType::Record {
            key: Box::new(TsType::Named(String::from("Tab"))),
            value: Box::new(NUMBER),
            partial: true,
        };
        assert_eq!(record.render(0), "Partial<Record<Tab, number>>");
        assert_eq!(literal("say \"hi\"").render(0), "\"say \\\"hi\\\"\"");
        assert_eq!(TsType::Raw(String::from("Date")).render(0), "Date");
    }

    #[test]
    fn test_long_tuple_breaks() {
        let tuple = TsType::Tuple(vec![TsType::Named("x".repeat(30)); 3]);
        let name = "x".repeat(30);
        assert_eq!(
            tuple.render(0),
            format!("[\n  {name},\n  {name},\n  {name},\n]")
        );
    }

    #[test]
    fn test_flattened_and_tagged_newtype() {
        use super::__private::{tagged_newtype, with_flattened};

        let a = TsType::Named(String::from("A"));
        let types = TypeScriptTypes::new();
        assert_eq!(with_flattened(&types, Vec::new(), vec![a.clone()]), a);
        assert_eq!(
            with_flattened(&types, vec![field("x", NUMBER)], vec![a.clone()]).render(0),
            "{ x: number } & A"
        );
        // A flattened `None` writes no field.
        assert_eq!(
            with_flattened(&types, vec![field("x", NUMBER)], vec![a.clone().nullable()]).render(0),
            "{ x: number } & (A | {})"
        );
        // A unit variant inside an object is an object.
        let units = TsType::Union(vec![member(literal("u")), member(a.clone())]);
        assert_eq!(
            with_flattened(&types, vec![field("x", NUMBER)], vec![units.clone()]).render(0),
            "{ x: number } & ({ u: null } | A)"
        );
        let tag = field("type", literal("a"));
        assert_eq!(
            tagged_newtype(&types, tag.clone(), NULL).render(0),
            "{ type: \"a\" }"
        );
        assert_eq!(
            tagged_newtype(&types, tag.clone(), units).render(0),
            "{ type: \"a\" } & ({ u: null } | A)"
        );
        assert_eq!(
            tagged_newtype(&types, tag, a).render(0),
            "{ type: \"a\" } & A"
        );
    }
}
