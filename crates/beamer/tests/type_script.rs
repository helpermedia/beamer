//! `#[derive(TypeScript)]` against what `serde_json` writes: each type's
//! TypeScript as text plus sample values that must conform to it.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use beamer::prelude::*;
use beamer::serde::{Deserialize, Serialize};
use beamer::webview::type_script::{TsType, TypeScriptTypes};
use serde_json::Value;

// =============================================================================
// Conformance
// =============================================================================

/// Whether `value` is a value of `ty`, with `path` naming where in the value
/// it is for the message.
///
/// `exact` refuses keys an object type does not name; an intersection checks
/// its parts without it, then the keys against all parts.
fn conforms(
    value: &Value,
    ty: &TsType,
    types: &TypeScriptTypes,
    exact: bool,
    path: &str,
) -> Result<(), String> {
    let fail = || Err(format!("{path}: {value} is no {}", ty.render(0)));
    match ty {
        TsType::Keyword("number") if value.is_number() => Ok(()),
        TsType::Keyword("string") if value.is_string() => Ok(()),
        TsType::Keyword("boolean") if value.is_boolean() => Ok(()),
        TsType::Keyword("null") if value.is_null() => Ok(()),
        TsType::Keyword("unknown") => Ok(()),
        TsType::Keyword(_) => fail(),
        TsType::Literal(text) if value.as_str() == Some(text) => Ok(()),
        TsType::Literal(_) => fail(),
        TsType::Named(name) => {
            let declared = types
                .declaration(name)
                .ok_or_else(|| format!("{path}: no declaration {name}"))?;
            conforms(value, declared, types, exact, path)
        }
        TsType::Array(element) => match value {
            Value::Array(items) => items.iter().enumerate().try_for_each(|(index, item)| {
                conforms(item, element, types, true, &format!("{path}[{index}]"))
            }),
            _ => fail(),
        },
        TsType::Tuple(elements) => match value {
            Value::Array(items) if items.len() == elements.len() => items
                .iter()
                .zip(elements)
                .enumerate()
                .try_for_each(|(index, (item, element))| {
                    conforms(item, element, types, true, &format!("{path}[{index}]"))
                }),
            _ => fail(),
        },
        TsType::Object(fields) => {
            let Value::Object(map) = value else {
                return fail();
            };
            for field in fields {
                match map.get(&field.name) {
                    Some(item) => conforms(
                        item,
                        &field.ty,
                        types,
                        true,
                        &format!("{path}.{}", field.name),
                    )?,
                    None if field.optional => {}
                    None => return Err(format!("{path}: {value} lacks `{}`", field.name)),
                }
            }
            if exact {
                if let Some(extra) = map
                    .keys()
                    .find(|key| !fields.iter().any(|field| &field.name == *key))
                {
                    return Err(format!(
                        "{path}: {value} has `{extra}`, which {} lacks",
                        ty.render(0)
                    ));
                }
            }
            Ok(())
        }
        TsType::Union(members) => {
            let mut reasons = Vec::new();
            for member in members {
                match conforms(value, &member.ty, types, exact, path) {
                    Ok(()) => return Ok(()),
                    Err(reason) => reasons.push(reason),
                }
            }
            Err(format!(
                "{path}: {value} matches no member of {}: {reasons:?}",
                ty.render(0)
            ))
        }
        TsType::Intersection(parts) => {
            for part in parts {
                conforms(value, part, types, false, path)?;
            }
            if let (true, Value::Object(map), Some(known)) = (exact, value, known_keys(ty, types)) {
                if let Some(extra) = map.keys().find(|key| !known.contains(*key)) {
                    return Err(format!(
                        "{path}: {value} has `{extra}`, which {} lacks",
                        ty.render(0)
                    ));
                }
            }
            Ok(())
        }
        TsType::Record {
            key,
            value: value_type,
            ..
        } => {
            let Value::Object(map) = value else {
                return fail();
            };
            for (name, item) in map {
                conforms(
                    &Value::String(name.clone()),
                    key,
                    types,
                    true,
                    &format!("{path} key"),
                )?;
                conforms(item, value_type, types, true, &format!("{path}.{name}"))?;
            }
            Ok(())
        }
        TsType::Raw(_) => Ok(()),
    }
}

/// The keys an object type names, `None` when it takes any.
fn known_keys(ty: &TsType, types: &TypeScriptTypes) -> Option<Vec<String>> {
    match ty {
        TsType::Object(fields) => Some(fields.iter().map(|field| field.name.clone()).collect()),
        TsType::Named(name) => known_keys(types.declaration(name)?, types),
        TsType::Intersection(parts) => {
            let mut keys = Vec::new();
            for part in parts {
                keys.extend(known_keys(part, types)?);
            }
            Some(keys)
        }
        TsType::Union(members) => {
            let mut keys = Vec::new();
            for member in members {
                keys.extend(known_keys(&member.ty, types)?);
            }
            Some(keys)
        }
        _ => None,
    }
}

/// The type of `T` with its declarations.
fn declare<T: TypeScript>() -> (TsType, TypeScriptTypes) {
    let mut types = TypeScriptTypes::new();
    let ty = T::type_script(&mut types);
    assert!(types.errors().is_empty(), "{:?}", types.errors());
    (ty, types)
}

/// The type of `T` inline.
fn inline<T: TypeScript>() -> String {
    declare::<T>().0.render(0)
}

/// The declarations the type of `T` made.
fn declarations<T: TypeScript>() -> String {
    declare::<T>().1.render_declarations()
}

/// Check that what `serde_json` writes for each value conforms to the type
/// of `T`.
fn check<T: TypeScript + Serialize>(values: &[T]) {
    let (ty, types) = declare::<T>();
    for value in values {
        let json = serde_json::to_value(value).unwrap();
        if let Err(reason) = conforms(&json, &ty, &types, true, "value") {
            panic!("{reason}\n{}", types.render_declarations());
        }
    }
}

/// Check that a value does not conform, so the checker refuses what it must.
fn refuse<T: TypeScript>(json: Value) {
    let (ty, types) = declare::<T>();
    assert!(
        conforms(&json, &ty, &types, true, "value").is_err(),
        "{json} conforms to {}",
        ty.render(0)
    );
}

/// Check that each value comes back as it went out.
fn round_trip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
    values: &[T],
) {
    for value in values {
        let json = serde_json::to_value(value).unwrap();
        let back: T = serde_json::from_value(json).unwrap();
        assert_eq!(&back, value);
    }
}

// =============================================================================
// The mapping, row by row
// =============================================================================

#[test]
fn test_bool_is_boolean() {
    assert_eq!(inline::<bool>(), "boolean");
    check(&[true, false]);
    refuse::<bool>(Value::from(1));
}

#[test]
fn test_numbers_are_number() {
    assert_eq!(
        inline::<(i8, u16, i32, u64, i128, usize, f32, f64)>()
            .matches("number")
            .count(),
        8
    );
    check(&[(
        i8::MIN,
        u16::MAX,
        -3i32,
        u64::MAX,
        -5i128,
        usize::MAX,
        0.5f32,
        -1e300f64,
    )]);
    refuse::<f64>(Value::Null);
}

#[test]
fn test_text_is_string() {
    assert_eq!(
        inline::<(char, String, &str, Cow<'_, str>)>(),
        "[string, string, string, string]"
    );
    check(&[('x', String::from("text"), "slice", Cow::Borrowed("cow"))]);
}

/// A unit struct.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Nothing;

#[test]
fn test_unit_and_unit_struct_are_null() {
    assert_eq!(inline::<()>(), "null");
    assert_eq!(inline::<Nothing>(), "Nothing");
    assert_eq!(
        declarations::<Nothing>(),
        "/** A unit struct. */\nexport type Nothing = null;\n"
    );
    check(&[((), Nothing)]);
    round_trip(&[Nothing]);
}

/// Values that may be absent.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Maybe {
    /// Written as `null` when absent.
    tempo: Option<f64>,
    /// Left out when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    /// Left out when zero.
    #[serde(default, skip_serializing_if = "is_zero")]
    offset: u32,
    /// An option of an option is null once.
    nested: Option<Option<bool>>,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

#[test]
fn test_option_is_nullable_and_skipped_fields_optional() {
    assert_eq!(
        declarations::<Maybe>(),
        "/** Values that may be absent. */\n\
         export interface Maybe {\n  \
           /** Written as `null` when absent. */\n  \
           tempo: number | null;\n  \
           /** Left out when absent. */\n  \
           host?: string;\n  \
           /** Left out when zero. */\n  \
           offset?: number;\n  \
           /** An option of an option is null once. */\n  \
           nested: boolean | null;\n\
         }\n"
    );
    let values = [
        Maybe {
            tempo: None,
            host: None,
            offset: 0,
            nested: None,
        },
        Maybe {
            tempo: Some(120.0),
            host: Some(String::from("Logic")),
            offset: 3,
            nested: Some(Some(true)),
        },
    ];
    check(&values);
    round_trip(&values);
    refuse::<Maybe>(serde_json::json!({ "tempo": null, "host": null, "nested": null }));
}

#[test]
fn test_sequences_and_sets_are_arrays() {
    type Sequences = (
        Vec<f32>,
        VecDeque<u8>,
        HashSet<String>,
        BTreeSet<char>,
        Box<[bool]>,
    );
    assert_eq!(
        inline::<Sequences>(),
        "[number[], number[], string[], string[], boolean[]]"
    );
    check::<Sequences>(&[(
        vec![1.0, 2.0],
        VecDeque::from([3]),
        HashSet::from([String::from("a")]),
        BTreeSet::from(['b']),
        Box::new([true]),
    )]);
    refuse::<Vec<f32>>(serde_json::json!([1, "two"]));
}

#[test]
fn test_fixed_arrays_are_tuples() {
    assert_eq!(inline::<[f32; 4]>(), "[number, number, number, number]");
    assert_eq!(inline::<[u8; 32]>(), "number[]");
    check(&[[0.1f32, 0.2, 0.3, 0.4]]);
    check(&[[7u8; 32]]);
    refuse::<[f32; 2]>(serde_json::json!([1.0]));
}

#[test]
fn test_tuples_are_tuples() {
    assert_eq!(
        inline::<(u8, String, Option<bool>)>(),
        "[number, string, boolean | null]"
    );
    check(&[
        (1u8, String::from("x"), None::<bool>),
        (2, String::new(), Some(true)),
    ]);
}

/// A unit enum as the keys of a map.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TypeScript,
)]
#[serde(crate = "beamer::serde", rename_all = "camelCase")]
enum Band {
    Low,
    HighMid,
}

#[test]
fn test_maps_are_records() {
    assert_eq!(inline::<HashMap<String, f32>>(), "Record<string, number>");
    assert_eq!(inline::<BTreeMap<u8, String>>(), "Record<string, string>");
    assert_eq!(
        inline::<BTreeMap<Band, f32>>(),
        "Partial<Record<Band, number>>"
    );
    assert_eq!(
        declarations::<BTreeMap<Band, f32>>(),
        "/** A unit enum as the keys of a map. */\nexport type Band = \"low\" | \"highMid\";\n"
    );
    check(&[HashMap::from([(String::from("a"), 1.0f32)])]);
    check(&[BTreeMap::from([(3u8, String::from("three"))])]);
    check(&[BTreeMap::from([(Band::HighMid, 0.5f32)]), BTreeMap::new()]);
    refuse::<BTreeMap<Band, f32>>(serde_json::json!({ "mid": 1.0 }));
}

#[test]
fn test_pointers_are_what_they_hold() {
    assert_eq!(
        inline::<(Box<f32>, Rc<String>, Arc<bool>, &u8)>(),
        "[number, string, boolean, number]"
    );
    // Serde writes `Rc` and `Arc` only with its `rc` feature.
    check(&[(Box::new(1.0f32), &3u8)]);
}

/// Settings as a page shows them.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", rename_all = "camelCase")]
struct Settings {
    /// The input gain in dB.
    input_gain: f32,
    #[serde(rename = "mode")]
    mode_name: String,
    #[serde(skip)]
    cache: u32,
    #[serde(flatten)]
    display: Display,
}

/// How the editor looks.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", rename_all = "camelCase")]
struct Display {
    dark_theme: bool,
    zoom: f32,
}

#[test]
fn test_struct_is_an_interface() {
    assert_eq!(
        declarations::<Settings>(),
        "/** Settings as a page shows them. */\n\
         export type Settings = {\n  \
           /** The input gain in dB. */\n  \
           inputGain: number;\n  \
           mode: string;\n\
         } & Display;\n\
         \n\
         /** How the editor looks. */\n\
         export interface Display {\n  \
           darkTheme: boolean;\n  \
           zoom: number;\n\
         }\n"
    );
    let values = [Settings {
        input_gain: -6.0,
        mode_name: String::from("vintage"),
        cache: 0,
        display: Display {
            dark_theme: true,
            zoom: 1.5,
        },
    }];
    check(&values);
    round_trip(&values);
    refuse::<Settings>(
        serde_json::json!({ "inputGain": 1, "mode": "x", "darkTheme": true, "zoom": 1, "cache": 0 }),
    );
}

/// A level in dB.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Decibels(f32);

/// A name, written as the string it holds.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", transparent)]
struct Label {
    text: String,
    #[serde(skip)]
    cached_width: u32,
}

#[test]
fn test_newtype_and_transparent_are_the_inner_type() {
    assert_eq!(
        declarations::<Decibels>(),
        "/** A level in dB. */\nexport type Decibels = number;\n"
    );
    assert_eq!(
        declarations::<Label>(),
        "/** A name, written as the string it holds. */\nexport type Label = string;\n"
    );
    check(&[Decibels(-3.0)]);
    check(&[Label {
        text: String::from("Kick"),
        cached_width: 0,
    }]);
    round_trip(&[Decibels(-3.0)]);
}

/// A point on the curve.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Point(f32, f32, #[serde(skip)] u8);

#[test]
fn test_tuple_struct_is_a_tuple() {
    assert_eq!(
        declarations::<Point>(),
        "/** A point on the curve. */\nexport type Point = [number, number];\n"
    );
    check(&[Point(0.0, 1.0, 0)]);
}

// =============================================================================
// Enums
// =============================================================================

/// Every kind of variant, externally tagged.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
enum External {
    Unit,
    Newtype(f32),
    Tuple(u8, String),
    Struct {
        a: bool,
    },
    #[serde(skip)]
    #[allow(dead_code)]
    Hidden,
    #[serde(rename = "renamed")]
    Other,
}

#[test]
fn test_enum_externally_tagged() {
    assert_eq!(
        declarations::<External>(),
        "/** Every kind of variant, externally tagged. */\n\
         export type External =\n  \
           | \"Unit\"\n  \
           | { Newtype: number }\n  \
           | { Tuple: [number, string] }\n  \
           | { Struct: { a: boolean } }\n  \
           | \"renamed\";\n"
    );
    let values = [
        External::Unit,
        External::Newtype(1.0),
        External::Tuple(2, String::from("x")),
        External::Struct { a: true },
        External::Other,
    ];
    check(&values);
    round_trip(&values);
    refuse::<External>(Value::from("Hidden"));
}

/// Variants with a tag field.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", tag = "type", rename_all = "camelCase")]
enum Internal {
    /// A unit variant.
    Start,
    NoteOn {
        #[serde(rename = "noteNumber")]
        note: u8,
        velocity: f32,
    },
    Wrapped(Display),
    Empty(Nothing),
}

#[test]
fn test_enum_internally_tagged() {
    assert_eq!(
        declarations::<Internal>(),
        "/** Variants with a tag field. */\n\
         export type Internal =\n  \
           /** A unit variant. */\n  \
           | { type: \"start\" }\n  \
           | { type: \"noteOn\"; noteNumber: number; velocity: number }\n  \
           | { type: \"wrapped\" } & Display\n  \
           | { type: \"empty\" };\n\
         \n\
         /** How the editor looks. */\n\
         export interface Display {\n  \
           darkTheme: boolean;\n  \
           zoom: number;\n\
         }\n\
         \n\
         /** A unit struct. */\n\
         export type Nothing = null;\n"
    );
    let values = [
        Internal::Start,
        Internal::NoteOn {
            note: 60,
            velocity: 0.5,
        },
        Internal::Wrapped(Display {
            dark_theme: false,
            zoom: 1.0,
        }),
        Internal::Empty(Nothing),
    ];
    check(&values);
    round_trip(&values);
    refuse::<Internal>(serde_json::json!({ "type": "noteOn", "note": 60, "velocity": 0.5 }));
}

/// Variants with a tag and their content beside it.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", tag = "t", content = "c")]
enum Adjacent {
    Unit,
    Newtype(f32),
    Tuple(u8, u8),
    Struct { a: bool },
}

#[test]
fn test_enum_adjacently_tagged() {
    assert_eq!(
        declarations::<Adjacent>(),
        "/** Variants with a tag and their content beside it. */\n\
         export type Adjacent =\n  \
           | { t: \"Unit\" }\n  \
           | { t: \"Newtype\"; c: number }\n  \
           | { t: \"Tuple\"; c: [number, number] }\n  \
           | { t: \"Struct\"; c: { a: boolean } };\n"
    );
    let values = [
        Adjacent::Unit,
        Adjacent::Newtype(1.0),
        Adjacent::Tuple(1, 2),
        Adjacent::Struct { a: false },
    ];
    check(&values);
    round_trip(&values);
}

/// Variants by their shape alone.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", untagged)]
enum Untagged {
    Struct { a: bool },
    Tuple(u8, u8),
    Newtype(String),
    Unit,
}

#[test]
fn test_enum_untagged() {
    assert_eq!(
        declarations::<Untagged>(),
        "/** Variants by their shape alone. */\n\
         export type Untagged = { a: boolean } | [number, number] | string | null;\n"
    );
    let values = [
        Untagged::Struct { a: true },
        Untagged::Tuple(1, 2),
        Untagged::Newtype(String::from("x")),
        Untagged::Unit,
    ];
    check(&values);
    round_trip(&values);
}

/// One variant without its tag.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", rename_all = "lowercase")]
enum PartlyUntagged {
    Named(u8),
    #[serde(untagged)]
    Raw(String),
}

#[test]
fn test_untagged_variant_in_a_tagged_enum() {
    assert_eq!(
        declarations::<PartlyUntagged>(),
        "/** One variant without its tag. */\nexport type PartlyUntagged = { named: number } | string;\n"
    );
    let values = [
        PartlyUntagged::Named(1),
        PartlyUntagged::Raw(String::from("x")),
    ];
    check(&values);
    round_trip(&values);
}

/// A newtype variant whose field serde skips: a unit variant.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
enum SkippedContent {
    Plain(#[serde(skip)] u32),
    Kept(u32),
}

/// The same with a tag field.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", tag = "t")]
enum TaggedSkippedContent {
    Plain(#[serde(skip)] u32),
}

#[test]
fn test_newtype_variant_with_a_skipped_field_is_a_unit_variant() {
    assert_eq!(
        declarations::<SkippedContent>(),
        "/** A newtype variant whose field serde skips: a unit variant. */\n\
         export type SkippedContent = \"Plain\" | { Kept: number };\n"
    );
    let values = [SkippedContent::Plain(0), SkippedContent::Kept(1)];
    check(&values);
    round_trip(&values);
    assert_eq!(
        declarations::<TaggedSkippedContent>(),
        "/** The same with a tag field. */\nexport type TaggedSkippedContent = { t: \"Plain\" };\n"
    );
    check(&[TaggedSkippedContent::Plain(0)]);
    round_trip(&[TaggedSkippedContent::Plain(0)]);
}

/// A mode by its variant's name.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
enum Mode {
    Off,
    Level(u8),
}

/// The fields of what it flattens beside its own.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Flattening {
    z: u8,
    #[serde(flatten)]
    zoom: Option<Zoom>,
    #[serde(flatten)]
    mode: Mode,
}

/// A zoom factor.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Zoom {
    factor: u8,
}

/// A mode in a variant with a tag field.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", tag = "type")]
enum Holder {
    Mode(Mode),
}

#[test]
fn test_values_inside_an_object() {
    // A flattened `None` writes no field; a unit variant writes its name
    // as a key.
    assert_eq!(inline::<Flattening>(), "Flattening");
    assert_eq!(
        declare::<Flattening>()
            .1
            .declaration("Flattening")
            .unwrap()
            .render(0),
        "{ z: number } & (Zoom | {}) & ({ Off: null } | { Level: number })"
    );
    let values = [
        Flattening {
            z: 1,
            zoom: None,
            mode: Mode::Off,
        },
        Flattening {
            z: 1,
            zoom: Some(Zoom { factor: 2 }),
            mode: Mode::Level(3),
        },
    ];
    check(&values);
    round_trip(&values);
    assert_eq!(
        declare::<Holder>()
            .1
            .declaration("Holder")
            .unwrap()
            .render(0),
        "{ type: \"Mode\" } & ({ Off: null } | { Level: number })"
    );
    let values = [Holder::Mode(Mode::Off), Holder::Mode(Mode::Level(3))];
    check(&values);
    round_trip(&values);
}

/// An `Option` that serde leaves out when it is `None`, without a default.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct LeftOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    gain: Option<f32>,
    // `Some(None)` writes `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    twice: Option<Option<f32>>,
}

#[test]
fn test_option_left_out_when_none() {
    assert_eq!(
        declare::<LeftOut>()
            .1
            .declaration("LeftOut")
            .unwrap()
            .render(0),
        "{ gain?: number; twice?: number | null }"
    );
    check(&[
        LeftOut {
            gain: None,
            twice: None,
        },
        LeftOut {
            gain: Some(1.0),
            twice: Some(None),
        },
        LeftOut {
            gain: None,
            twice: Some(Some(1.0)),
        },
    ]);
    round_trip(&[
        LeftOut {
            gain: None,
            twice: None,
        },
        LeftOut {
            gain: Some(1.0),
            twice: Some(Some(1.0)),
        },
    ]);
}

/// Variants by their shape, one of them an enum of its own.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", untagged)]
enum Shapes {
    Nothing,
    Zoomed(Zoom),
    Moded(Mode),
}

/// An untagged enum flattened through an `Option`.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct DeepFlattening {
    z: u8,
    #[serde(flatten)]
    shape: Option<Shapes>,
}

#[test]
fn test_unions_inside_an_object_at_every_depth() {
    assert_eq!(
        declare::<DeepFlattening>()
            .1
            .declaration("DeepFlattening")
            .unwrap()
            .render(0),
        "{ z: number } & ({} | Zoom | { Off: null } | { Level: number } | {})"
    );
    check(&[
        DeepFlattening { z: 1, shape: None },
        DeepFlattening {
            z: 1,
            shape: Some(Shapes::Zoomed(Zoom { factor: 2 })),
        },
        DeepFlattening {
            z: 1,
            shape: Some(Shapes::Moded(Mode::Off)),
        },
        DeepFlattening {
            z: 1,
            shape: Some(Shapes::Moded(Mode::Level(3))),
        },
    ]);
}

/// An `Option` of an `Option` behind a pointer, left out when it is `None`.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct BoxedTwice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    boxed: Option<Box<Option<u8>>>,
}

#[test]
fn test_option_holding_an_option_behind_a_pointer_keeps_null() {
    assert_eq!(
        declare::<BoxedTwice>()
            .1
            .declaration("BoxedTwice")
            .unwrap()
            .render(0),
        "{ boxed?: number | null }"
    );
    check(&[
        BoxedTwice { boxed: None },
        BoxedTwice {
            boxed: Some(Box::new(None)),
        },
    ]);
}

/// A struct with the tag of its name.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", tag = "kind")]
struct TaggedStruct {
    value: u8,
}

#[test]
fn test_struct_with_a_tag() {
    assert_eq!(
        declarations::<TaggedStruct>(),
        "/** A struct with the tag of its name. */\n\
         export interface TaggedStruct {\n  kind: \"TaggedStruct\";\n  value: number;\n}\n"
    );
    check(&[TaggedStruct { value: 1 }]);
    round_trip(&[TaggedStruct { value: 1 }]);
}

/// An enum without variants.
#[derive(Serialize, TypeScript)]
#[serde(crate = "beamer::serde")]
enum Never {}

#[test]
fn test_enum_without_variants_is_never() {
    assert_eq!(
        declarations::<Never>(),
        "/** An enum without variants. */\nexport type Never = never;\n"
    );
}

// =============================================================================
// Generics, names and written types
// =============================================================================

/// A value with its time.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Timed<T> {
    time: f64,
    value: T,
}

#[test]
fn test_generic_type_inline_at_each_use() {
    assert_eq!(inline::<Timed<f32>>(), "{ time: number; value: number }");
    assert_eq!(inline::<Timed<Band>>(), "{ time: number; value: Band }");
    assert_eq!(declarations::<Timed<f32>>(), "");
    assert_eq!(
        inline::<Vec<Timed<Option<u8>>>>(),
        "{ time: number; value: number | null }[]"
    );
    check(&[Timed {
        time: 1.0,
        value: Band::Low,
    }]);
    round_trip(&[Timed {
        time: 1.0,
        value: 0.5f32,
    }]);
}

/// A borrowed name, declared like any type without generics.
#[derive(Serialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Borrowed<'a> {
    name: &'a str,
}

#[test]
fn test_type_with_lifetimes_is_declared() {
    assert_eq!(inline::<Borrowed<'_>>(), "Borrowed");
    check(&[Borrowed { name: "x" }]);
}

/// A tree of nodes.
#[derive(Debug, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Tree {
    children: Vec<Tree>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<Box<Tree>>,
}

#[test]
fn test_recursive_type() {
    assert_eq!(
        declarations::<Tree>(),
        "/** A tree of nodes. */\nexport interface Tree {\n  children: Tree[];\n  parent?: Tree;\n}\n"
    );
    let values = [Tree {
        children: vec![Tree {
            children: Vec::new(),
            parent: None,
        }],
        parent: None,
    }];
    check(&values);
    round_trip(&values);
}

mod first {
    use super::*;

    /// One kind of channel.
    #[derive(Serialize, TypeScript)]
    #[serde(crate = "beamer::serde")]
    pub struct Channel(pub u8);
}

mod second {
    use super::*;

    /// Another kind of channel.
    #[derive(Serialize, TypeScript)]
    #[serde(crate = "beamer::serde")]
    pub struct Channel(pub String);

    /// The same, under another name.
    #[derive(Serialize, TypeScript)]
    #[serde(crate = "beamer::serde")]
    #[type_script(rename = "NamedChannel")]
    pub struct Renamed(pub String);
}

#[test]
fn test_two_types_under_one_name_are_an_error() {
    let mut types = TypeScriptTypes::new();
    first::Channel::type_script(&mut types);
    second::Channel::type_script(&mut types);
    assert_eq!(types.errors().len(), 1);
    let error = &types.errors()[0];
    assert!(
        error.contains("first::Channel") && error.contains("second::Channel"),
        "{error}"
    );

    let mut types = TypeScriptTypes::new();
    first::Channel::type_script(&mut types);
    assert_eq!(
        second::Renamed::type_script(&mut types),
        TsType::Named(String::from("NamedChannel"))
    );
    assert!(types.errors().is_empty());
    assert!(types
        .render_declarations()
        .contains("export type NamedChannel = string;"));
}

/// A time, written by hand.
#[derive(Serialize, TypeScript)]
#[serde(crate = "beamer::serde")]
#[type_script(type = "`${number}:${number}`")]
struct Clock {
    #[allow(dead_code)]
    hours: u8,
}

/// Fields written by hand.
#[derive(Serialize, TypeScript)]
#[serde(crate = "beamer::serde")]
struct Written {
    #[serde(serialize_with = "as_text")]
    #[type_script(type = "string")]
    gain: f32,
    clock: Clock,
}

fn as_text<S: beamer::serde::Serializer>(value: &f32, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

#[test]
fn test_types_written_by_hand() {
    assert_eq!(
        declarations::<Written>(),
        "/** Fields written by hand. */\n\
         export interface Written {\n  gain: string;\n  clock: Clock;\n}\n\
         \n\
         /** A time, written by hand. */\n\
         export type Clock = `${number}:${number}`;\n"
    );
}

/// The probe of the model macros: a derived type has TypeScript, others
/// none.
#[test]
#[allow(clippy::needless_borrow)]
fn test_type_probe_on_derived_types() {
    use beamer::webview::type_script::__private::{
        HasTypeScript as _, LacksTypeScript as _, TypeProbe,
    };

    struct Plain;

    let mut types = TypeScriptTypes::new();
    assert_eq!(
        (&TypeProbe::<Decibels>::new()).type_script_if_any(&mut types),
        Some(TsType::Named(String::from("Decibels")))
    );
    assert_eq!(
        (&TypeProbe::<Plain>::new()).type_script_if_any(&mut types),
        None
    );
}

// =============================================================================
// The examples' types
// =============================================================================

/// A note or a CC as it came in and as it went out, sent to the editor,
/// which lights up the notes that sound and shows the last CC. As JSON an
/// object with its `type` ("noteOn", "noteOff", "cc" or "reset") and the
/// fields in camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(
    crate = "beamer::serde",
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Transformed {
    /// A note started: its pitch in and out, `None` when it was filtered
    /// out. Velocities from 0.0 to 1.0.
    NoteOn {
        channel: u8,
        pitch: u8,
        out_pitch: Option<u8>,
        velocity: f32,
        out_velocity: f32,
    },
    /// The note on `pitch` ended, by its incoming pitch.
    NoteOff { channel: u8, pitch: u8 },
    /// A CC in and out. Values from 0.0 to 1.0.
    Cc {
        channel: u8,
        controller: u8,
        value: f64,
        out_controller: u8,
        out_value: f64,
    },
    /// The host reset the plugin: no note sounds any more.
    Reset,
}

#[test]
fn test_midi_transform_transformed() {
    assert_eq!(
        declarations::<Transformed>(),
        r#"/**
 * A note or a CC as it came in and as it went out, sent to the editor,
 * which lights up the notes that sound and shows the last CC. As JSON an
 * object with its `type` ("noteOn", "noteOff", "cc" or "reset") and the
 * fields in camelCase.
 */
export type Transformed =
  /**
   * A note started: its pitch in and out, `None` when it was filtered
   * out. Velocities from 0.0 to 1.0.
   */
  | {
      type: "noteOn";
      channel: number;
      pitch: number;
      outPitch: number | null;
      velocity: number;
      outVelocity: number;
    }
  /** The note on `pitch` ended, by its incoming pitch. */
  | { type: "noteOff"; channel: number; pitch: number }
  /** A CC in and out. Values from 0.0 to 1.0. */
  | {
      type: "cc";
      channel: number;
      controller: number;
      value: number;
      outController: number;
      outValue: number;
    }
  /** The host reset the plugin: no note sounds any more. */
  | { type: "reset" };
"#
    );
    let values = [
        Transformed::NoteOn {
            channel: 0,
            pitch: 60,
            out_pitch: Some(62),
            velocity: 0.5,
            out_velocity: 0.4,
        },
        Transformed::NoteOn {
            channel: 1,
            pitch: 61,
            out_pitch: None,
            velocity: 1.0,
            out_velocity: 0.0,
        },
        Transformed::NoteOff {
            channel: 0,
            pitch: 60,
        },
        Transformed::Cc {
            channel: 0,
            controller: 1,
            value: 0.5,
            out_controller: 2,
            out_value: 0.25,
        },
        Transformed::Reset,
    ];
    check(&values);
    round_trip(&values);
}

/// What the editor needs to know of the processor, sent from the audio
/// thread.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", rename_all = "camelCase")]
pub struct Status {
    /// The sample rate the host prepared the plugin for, which turns the
    /// lookahead into samples.
    pub sample_rate: f64,
    /// Whether the detector keys on the sidechain: the parameter asks for
    /// it, the bus is on and the host passes its samples.
    pub keyed_on_sidechain: bool,
}

/// The levels as the editor's meters show them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", rename_all = "camelCase")]
pub struct Meters {
    /// The held peak of the level the detector keys on (the main input or
    /// the sidechain) in dB, at least the floor of the meters.
    pub key_db: f32,
    /// The held gain reduction in dB, a positive number.
    pub reduction_db: f32,
    /// The auto makeup gain in effect in dB, 0 while auto makeup is off.
    pub auto_makeup_db: f32,
}

#[test]
fn test_compressor_status_and_meters() {
    let mut types = TypeScriptTypes::new();
    Status::type_script(&mut types);
    Meters::type_script(&mut types);
    assert_eq!(
        types.render_declarations(),
        r#"/**
 * What the editor needs to know of the processor, sent from the audio
 * thread.
 */
export interface Status {
  /**
   * The sample rate the host prepared the plugin for, which turns the
   * lookahead into samples.
   */
  sampleRate: number;
  /**
   * Whether the detector keys on the sidechain: the parameter asks for
   * it, the bus is on and the host passes its samples.
   */
  keyedOnSidechain: boolean;
}

/** The levels as the editor's meters show them. */
export interface Meters {
  /**
   * The held peak of the level the detector keys on (the main input or
   * the sidechain) in dB, at least the floor of the meters.
   */
  keyDb: number;
  /** The held gain reduction in dB, a positive number. */
  reductionDb: number;
  /** The auto makeup gain in effect in dB, 0 while auto makeup is off. */
  autoMakeupDb: number;
}
"#
    );
    let status = [Status {
        sample_rate: 48_000.0,
        keyed_on_sidechain: true,
    }];
    let meters = [Meters {
        key_db: -60.0,
        reduction_db: 3.5,
        auto_makeup_db: 0.0,
    }];
    check(&status);
    check(&meters);
    round_trip(&status);
    round_trip(&meters);
}
