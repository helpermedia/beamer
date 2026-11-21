//! The errors of `#[value]`, `#[stream]` and `#[event]` fields, as doc
//! tests that fail to compile.
//!
//! A name is one thing in the web UI:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! #[beamer::model]
//! struct MeterModel {
//!     #[value(id = "levels")]
//!     levels: WebViewValue<f32>,
//!     #[event(id = "levels")]
//!     levels_event: WebViewEvent<f32>,
//!     events: WebViewEventQueue,
//! }
//! ```
//!
//! The attribute matches the type of the field:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! #[beamer::model]
//! struct MeterModel {
//!     #[value(id = "levels")]
//!     levels: WebViewStream<f32>,
//!     events: WebViewEventQueue,
//! }
//! ```
//!
//! A slash separates the names of nested groups, so an id has none:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! #[beamer::model]
//! struct MeterModel {
//!     #[stream(id = "note/hits")]
//!     hits: WebViewStream<f32>,
//!     events: WebViewEventQueue,
//! }
//! ```
//!
//! A field of one of the three types needs its attribute, which gives it
//! its name:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! #[beamer::model]
//! struct MeterModel {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//!     gain: FloatParameter,
//!     hits: WebViewEvent<f32>,
//!     events: WebViewEventQueue,
//! }
//! ```
//!
//! The form that compiles:
//!
//! ```
//! # use beamer::prelude::*;
//! #[beamer::model]
//! struct MeterModel {
//!     #[value(id = "levels")]
//!     levels: WebViewValue<f32>,
//!     #[stream(id = "hits")]
//!     hits: WebViewStream<f32>,
//!     #[event(id = "loaded")]
//!     loaded: WebViewEvent<f32>,
//!     events: WebViewEventQueue,
//! }
//! ```
