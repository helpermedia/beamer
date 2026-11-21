//! The errors of `#[beamer::handler]`, as doc tests that fail to compile.
//!
//! The model is shared, so a method takes `&self`:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "reset")]
//!     fn reset(&mut self) {}
//! }
//! ```
//!
//! The page's arguments are read into owned values:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "rename")]
//!     fn rename(&self, name: &str) {}
//! }
//! ```
//!
//! A page event has one argument or none. It returns nothing:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[page_event(id = "padHit")]
//!     fn pad_hit(&self, pad: u8, velocity: f32) {}
//! }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[page_event(id = "padHit")]
//!     fn pad_hit(&self, pad: u8) -> bool { true }
//! }
//! ```
//!
//! A name is one method and one page event:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "reset")]
//!     fn reset(&self) {}
//!     #[method(id = "reset")]
//!     fn clear(&self) {}
//! }
//! ```
//!
//! The framework's own methods start with `_beamer/`:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "_beamer/reset")]
//!     fn reset(&self) {}
//! }
//! ```
//!
//! Each marked function has an id:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method]
//!     fn reset(&self) {}
//! }
//! ```
//!
//! One name per function:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "reset")]
//!     #[page_event(id = "reset")]
//!     fn reset(&self) {}
//! }
//! ```
//!
//! The page's types are fixed: no type parameters, no `impl Trait`, nothing async:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "first")]
//!     fn first<T: Default>(&self) {}
//! }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "level")]
//!     fn level(&self) -> impl Clone { 0.5 }
//! }
//! ```
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "load")]
//!     async fn load(&self) {}
//! }
//! ```
//!
//! The attribute goes on the model's own impl block:
//!
//! ```compile_fail
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl Clone for PadModel {
//!     fn clone(&self) -> Self {
//!         PadModel::default()
//!     }
//! }
//! ```
//!
//! The form that compiles:
//!
//! ```
//! # use beamer::prelude::*;
//! # #[beamer::model]
//! # struct PadModel {
//! #     #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
//! #     gain: FloatParameter,
//! # }
//! #[beamer::handler]
//! impl PadModel {
//!     #[method(id = "rename")]
//!     fn rename(&self, name: String) -> Result<(), String> {
//!         Ok(())
//!     }
//!     #[page_event(id = "padHit")]
//!     fn pad_hit(&self, pad: u8) {}
//! }
//! ```
