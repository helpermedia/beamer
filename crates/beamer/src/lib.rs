//! # Beamer
//!
//! Audio Plugin Framework for Rust.
//!
//! Beamer is a framework for building audio plugins (AU, VST3).
//! It provides safe Rust abstractions that work with multiple plugin formats.
//!
//! ## Quick Start
//!
//! A plugin has two structs: the model, which every thread shares, and the
//! processor, which the audio thread owns. The `#[beamer::export]`
//! attribute on the model reads `Config.toml` at compile time and generates
//! the plugin metadata and the entry points for the enabled formats.
//!
// The example needs the macros, so it is only compiled as a doctest when
// the `derive` feature is enabled.
#![cfg_attr(feature = "derive", doc = "```rust")]
#![cfg_attr(not(feature = "derive"), doc = "```rust,ignore")]
//! use beamer::prelude::*;
//!
//! // 1. Model: what every thread shares. It holds the parameters and
//! //    describes the plugin to the host. It lives as long as the plugin
//! //    instance.
//! # // The attribute reads Config.toml, which this crate does not have, so
//! # // the doctest comments it out. The rendered page still shows the line.
//! # /*
//! #[beamer::export]
//! # */
//! #[beamer::model]
//! struct GainModel {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     gain: FloatParameter,
//! }
//!
//! impl Model for GainModel {
//!     // A plain gain needs no setup. Use SampleRate for delays and
//!     // MaxBufferSize for FFT.
//!     type Setup = ();
//!     type Processor = GainProcessor;
//!
//!     fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
//!         GainProcessor { model: self }
//!     }
//! }
//!
//! // 2. Processor: what the audio thread owns. It shares the model with the
//! //    host and the UI.
//! struct GainProcessor {
//!     model: Arc<GainModel>,
//! }
//!
//! impl Processor for GainProcessor {
//!     fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
//!         let gain = self.model.gain.as_linear() as f32;
//!         // The buffer is processed in place: each channel holds its input.
//!         for channel in buffer.channels_mut() {
//!             for sample in channel {
//!                 *sample *= gain;
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! The plugin crate is a dynamic library with one feature per format, named
//! `au` and `vst3`. The export macro checks these names in the plugin crate, so
//! the entry points are only generated for the enabled features:
//!
//! ```toml
//! [lib]
//! crate-type = ["cdylib"]
//!
//! [features]
//! au = ["beamer/au"]
//! vst3 = ["beamer/vst3"]
//! ```
//!
//! `Config.toml` in the crate root, next to `Cargo.toml`:
//!
//! ```toml
//! name = "My Gain Plugin"
//! category = "effect"
//! subcategories = ["dynamics"]
//! manufacturer_code = "Manu"
//! plugin_code = "gain"
//! vendor = "My Company"
//! url = "https://example.com"
//! email = "support@example.com"
//! ```
//!
//! See [BUILDING.md](https://github.com/helpermedia/beamer/blob/main/docs/BUILDING.md)
//! for building and bundling a plugin. The
//! [repository](https://github.com/helpermedia/beamer) also has the reference
//! and the example plugins.

// Re-export sub-crates
pub use beamer_core as core;

/// Plugin setup types for declaring host information requirements.
///
/// See [`beamer_core::setup`] for documentation and examples.
pub use beamer_core::setup;

// serde, for the types a plugin keeps in a `State`. A plugin without a
// serde dependency of its own derives through this path:
// `#[serde(crate = "beamer::serde")]`.
pub use beamer_core::serde;

#[cfg(feature = "vst3")]
pub use beamer_vst3 as vst3_impl;

/// Re-export of vst3 types needed for plugin configuration.
///
/// This allows examples and plugins to use `beamer::vst3::Steinberg::TUID`
/// without adding a direct dependency on the vst3 crate.
#[cfg(feature = "vst3")]
pub mod vst3 {
    pub use ::vst3::Steinberg;
    pub use ::vst3::Steinberg::TUID;
}

#[cfg(feature = "au")]
pub use beamer_au as au_impl;

// Re-export the macros when the feature is enabled
#[cfg(feature = "derive")]
pub use beamer_macros::EnumParameter;
#[cfg(feature = "derive")]
pub use beamer_macros::export;
#[cfg(feature = "derive")]
pub use beamer_macros::{group, model};

/// Generate plugin entry points for all enabled formats (AU, VST3).
///
/// Plugins normally do not call this macro themselves. `#[beamer::export]` on
/// the model struct generates the `CONFIG` static from `Config.toml`, then
/// calls this macro with it and the model. The macro in turn calls the
/// export macro of every enabled format crate.
///
/// # Arguments
///
/// * `$config` - The [`Config`](beamer_core::Config) static containing all plugin metadata
/// * `$plugin` - The model type implementing the [`Model`](beamer_core::Model) trait,
///   with its factory presets in [`Model::PRESETS`](beamer_core::Model::PRESETS)
///
/// # Example
///
/// In outline, what `#[beamer::export]` generates for a model `MyModel`:
///
/// ```rust,ignore
/// pub static CONFIG: Config = Config::new("My Plugin", Category::Effect, "Mfgr", "plgn")
///     .with_vendor("My Company")
///     .with_version(env!("CARGO_PKG_VERSION"));
///
/// export_plugin!(CONFIG, MyModel);
/// ```
#[macro_export]
macro_rules! export_plugin {
    ($config:expr, $plugin:ty) => {
        #[cfg(feature = "au")]
        $crate::au_impl::export_au!($config, $plugin);

        #[cfg(feature = "vst3")]
        $crate::vst3_impl::export_vst3!($config, $plugin);
    };
}

/// Prelude module for convenient imports.
///
/// Import everything you need to build a plugin:
/// ```rust,ignore
/// use beamer::prelude::*;
/// ```
pub mod prelude {
    // Core traits and types
    // The model of a plugin lives in an `Arc`.
    pub use std::sync::Arc;

    pub use beamer_core::{
        // Asset types
        EmbeddedAsset, EmbeddedAssets,
        // Buffer types
        AuxiliaryBuffers, AuxInput, AuxOutput, Buffer,
        // Bypass handling
        BypassAction, BypassHandler, BypassState, CrossfadeCurve,
        // Sample trait for generic f32/f64 processing
        Sample,
        // Traits
        GuiDelegate, Model, Processor, Saved, WebViewEvents,
        // Plugin setup types (composable)
        PluginSetup, SampleRate, MaxBufferSize, MainInputChannels, MainOutputChannels,
        AuxInputCount, AuxOutputCount, ProcessMode,
        // Bus configuration
        BusInfo, BusType,
        // GUI types
        GuiConstraints, NoGui,
        // Parameter metadata
        NoParameters, ParameterFlags, ParameterInfo,
        // Factory presets
        Preset,
        // Parameter types
        BoolParameter, EnumParameter, EnumParameterValue, FloatParameter, IntParameter, Formatter, ParameterRef, Parameters,
        // MIDI CC configuration (framework manages runtime state)
        MidiCcConfig,
        // Parameter smoothing
        Smoother, SmoothingStyle,
        // Values for the audio thread and state beyond parameters
        Handover, HandoverReader, State,
        // Parameter group system
        GroupId, GroupInfo, ParameterGroups, ROOT_GROUP_ID,
        // Range mapping
        LinearMapper, LogMapper, LogOffsetMapper, PowerMapper, RangeMapper,
        // Error types
        PluginError, PluginResult,
        // Geometry
        Rect, Size,
        // MIDI types
        ChannelPressure, ControlChange, MidiBuffer, MidiChannel, MidiEvent, MidiEventKind,
        MidiNote, NoteId, NoteOff, NoteOn, PitchBend, PolyPressure, ProgramChange,
        // Process context and transport
        FrameRate, ProcessContext, Transport,
        // FourCharCode
        FourCharCode,
        // WebView support
        WebViewEventQueue, serde_json,
        // For the types kept in a `State`
        serde,
    };

    // Plugin configuration
    pub use beamer_core::{Config, config::Category, config::Subcategory};

    // Unified export macro
    pub use crate::export_plugin;

    // The enum derive (when the feature is enabled). It shares its name with
    // the type it is used for, which is allowed because types and derive
    // macros live in different namespaces.
    #[cfg(feature = "derive")]
    pub use beamer_macros::EnumParameter;
}
