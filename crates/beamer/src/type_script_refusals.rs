//! The serde attributes `#[derive(TypeScript)]` refuses, as doc tests that
//! fail to compile, each beside the form that compiles.
//!
//! The web UI sends back the values it gets, so Rust has to read every
//! value it writes. A name per direction:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(rename(deserialize = "level"))]
//!     gain: f32,
//! }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde", rename_all(deserialize = "camelCase"))]
//! struct Settings {
//!     input_gain: f32,
//! }
//! ```
//!
//! ```
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde", rename_all = "camelCase")]
//! struct Settings {
//!     #[serde(rename = "level", alias = "gain")]
//!     input_gain: f32,
//! }
//! ```
//!
//! A field left out in one direction:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(skip_serializing)]
//!     gain: f32,
//! }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(skip_serializing_if = "is_zero")]
//!     gain: f32,
//! }
//! # fn is_zero(value: &f32) -> bool { *value == 0.0 }
//! ```
//!
//! An `Option` that a function of its own reads is required when it comes
//! back:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! # mod as_text {
//! #     pub fn serialize<S: beamer::serde::Serializer>(value: &Option<f32>, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&format!("{value:?}")) }
//! #     pub fn deserialize<'de, D: beamer::serde::Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> { let text: String = beamer::serde::Deserialize::deserialize(d)?; Ok(text.parse().ok()) }
//! # }
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(skip_serializing_if = "Option::is_none", with = "as_text")]
//!     #[type_script(type = "string")]
//!     gain: Option<f32>,
//! }
//! ```
//!
//! Rust reads unnamed fields by their place, so none is skipped one way:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Range(u8, #[serde(skip_serializing, default)] u8, #[serde(default)] u8);
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! enum Source {
//!     Plain(#[serde(skip_serializing, default)] u32),
//! }
//! ```
//!
//! Serde cannot write an `Option` in a newtype variant with a tag:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde", tag = "type")]
//! enum Choice {
//!     Maybe(Option<u8>),
//! }
//! ```
//!
//! A tuple type keeps its length:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Pair(f32, #[serde(default, skip_serializing_if = "Option::is_none")] Option<f32>);
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(skip_deserializing)]
//!     gain: f32,
//! }
//! ```
//!
//! ```
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(skip_serializing, default)]
//!     unseen: f32,
//!     #[serde(skip_serializing_if = "Option::is_none")]
//!     gain: Option<f32>,
//!     #[serde(skip)]
//!     cache: f32,
//! }
//! ```
//!
//! A shape the derive cannot see, without TypeScript written by hand:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! # mod as_text {
//! #     pub fn serialize<S: beamer::serde::Serializer>(value: &f32, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&value.to_string()) }
//! #     pub fn deserialize<'de, D: beamer::serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> { let text: String = beamer::serde::Deserialize::deserialize(d)?; text.parse().map_err(beamer::serde::de::Error::custom) }
//! # }
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(with = "as_text")]
//!     gain: f32,
//! }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::Serialize;
//! # fn as_text<S: beamer::serde::Serializer>(value: &f32, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&value.to_string()) }
//! #[derive(Serialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(serialize_with = "as_text")]
//!     gain: f32,
//! }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! # fn from_text<'de, D: beamer::serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> { let text: String = beamer::serde::Deserialize::deserialize(d)?; text.parse().map_err(beamer::serde::de::Error::custom) }
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(deserialize_with = "from_text")]
//!     gain: f32,
//! }
//! ```
//!
//! ```
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! # mod as_text {
//! #     pub fn serialize<S: beamer::serde::Serializer>(value: &f32, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&value.to_string()) }
//! #     pub fn deserialize<'de, D: beamer::serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> { let text: String = beamer::serde::Deserialize::deserialize(d)?; text.parse().map_err(beamer::serde::de::Error::custom) }
//! # }
//! # fn to_text<S: beamer::serde::Serializer>(value: &f32, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&value.to_string()) }
//! # fn from_text<'de, D: beamer::serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> { let text: String = beamer::serde::Deserialize::deserialize(d)?; text.parse().map_err(beamer::serde::de::Error::custom) }
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde")]
//! struct Settings {
//!     #[serde(with = "as_text")]
//!     #[type_script(type = "string")]
//!     gain: f32,
//!     #[serde(serialize_with = "to_text", deserialize_with = "from_text")]
//!     #[type_script(type = "string")]
//!     level: f32,
//! }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::Serialize;
//! #[derive(Clone, Serialize, TypeScript)]
//! #[serde(crate = "beamer::serde", into = "f32")]
//! struct Gain {
//!     db: f32,
//! }
//! # impl From<Gain> for f32 { fn from(gain: Gain) -> f32 { gain.db } }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde", from = "f32")]
//! struct Gain {
//!     db: f32,
//! }
//! # impl From<f32> for Gain { fn from(db: f32) -> Gain { Gain { db } } }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde", try_from = "f32")]
//! struct Gain {
//!     db: f32,
//! }
//! # impl TryFrom<f32> for Gain { type Error = String; fn try_from(db: f32) -> Result<Gain, String> { Ok(Gain { db }) } }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # use beamer::serde::Serialize;
//! # mod other { pub struct Gain { db: f32 } impl Gain { pub fn db(&self) -> f32 { self.db } } }
//! #[derive(Serialize, TypeScript)]
//! #[serde(crate = "beamer::serde", remote = "other::Gain")]
//! struct Gain {
//!     #[serde(getter = "other::Gain::db")]
//!     db: f32,
//! }
//! ```
//!
//! ```
//! # use beamer::prelude::*;
//! # use beamer::serde::{Deserialize, Serialize};
//! # mod other { pub struct Gain { db: f32 } impl Gain { pub fn db(&self) -> f32 { self.db } } }
//! #[derive(Clone, Serialize, TypeScript)]
//! #[serde(crate = "beamer::serde", into = "f32")]
//! #[type_script(type = "number")]
//! struct IntoGain {
//!     db: f32,
//! }
//! # impl From<IntoGain> for f32 { fn from(gain: IntoGain) -> f32 { gain.db } }
//!
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde", from = "f32")]
//! #[type_script(type = "number")]
//! struct FromGain {
//!     db: f32,
//! }
//! # impl From<f32> for FromGain { fn from(db: f32) -> FromGain { FromGain { db } } }
//!
//! #[derive(Serialize, Deserialize, TypeScript)]
//! #[serde(crate = "beamer::serde", try_from = "f32")]
//! #[type_script(type = "number")]
//! struct TryFromGain {
//!     db: f32,
//! }
//! # impl TryFrom<f32> for TryFromGain { type Error = String; fn try_from(db: f32) -> Result<TryFromGain, String> { Ok(TryFromGain { db }) } }
//!
//! #[derive(Serialize, TypeScript)]
//! #[serde(crate = "beamer::serde", remote = "other::Gain")]
//! #[type_script(type = "{ db: number }")]
//! struct RemoteGain {
//!     #[serde(getter = "other::Gain::db")]
//!     db: f32,
//! }
//! ```
