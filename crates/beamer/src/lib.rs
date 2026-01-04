//! # Beamer
//!
//! VST3 Framework for Rust.
//!
//! Beamer is a framework for building VST3 audio plugins with WebView-based GUIs.
//! It provides safe Rust abstractions over the VST3 SDK.
//!
//! ## Architecture
//!
//! ```text
//! Your Plugin (implements Plugin trait)
//!        ↓
//! Vst3Processor<P> (generic VST3 wrapper)
//!        ↓
//! VST3 COM interfaces
//! ```
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use beamer::prelude::*;
//! use beamer::vst3_impl::{Vst3Processor, vst3};
//!
//! // Define your plugin
//! struct MyGain { params: MyParams }
//!
//! impl AudioProcessor for MyGain {
//!     fn setup(&mut self, _: f64, _: usize) {}
//!     fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &ProcessContext) {
//!         // Your DSP here
//!     }
//! }
//!
//! impl Plugin for MyGain {
//!     type Params = MyParams;
//!     fn params(&self) -> &Self::Params { &self.params }
//!     fn create() -> Self { Self { params: MyParams::new() } }
//! }
//!
//! // Export
//! static CONFIG: PluginConfig = PluginConfig::new("MyGain", MY_UID);
//! export_vst3!(CONFIG, Vst3Processor<MyGain>);
//! ```

// Re-export sub-crates
pub use beamer_core as core;
pub use beamer_vst3 as vst3_impl;

// Re-export derive macros when feature is enabled
#[cfg(feature = "derive")]
pub use beamer_macros::Params;
#[cfg(feature = "derive")]
pub use beamer_macros::EnumParam;

/// Prelude module for convenient imports.
///
/// Import everything you need to build a plugin:
/// ```rust,ignore
/// use beamer::prelude::*;
/// ```
pub mod prelude {
    // Core traits and types
    pub use beamer_core::{
        // Buffer types
        AuxiliaryBuffers, AuxInput, AuxOutput, Buffer,
        // Sample trait for generic f32/f64 processing
        Sample,
        // Traits
        AudioProcessor, EditorDelegate, Parameters, Plugin,
        // Bus configuration
        BusInfo, BusType,
        // Editor types
        EditorConstraints, NoEditor,
        // Parameter types (legacy)
        NoParams, ParamFlags, ParamInfo,
        // New parameter types (Phase 1)
        BoolParam, EnumParam, EnumParamValue, FloatParam, IntParam, Formatter, ParamRef, Params,
        // Parameter smoothing
        Smoother, SmoothingStyle,
        // VST3 Unit system (parameter groups)
        UnitId, UnitInfo, Units, ROOT_UNIT_ID,
        // Range mapping
        LinearMapper, LogMapper, RangeMapper,
        // Error types
        PluginError, PluginResult,
        // Geometry
        Rect, Size,
        // MIDI types
        MidiBuffer, MidiChannel, MidiEvent, MidiEventKind, MidiNote, NoteId, NoteOff, NoteOn,
        // Process context and transport
        FrameRate, ProcessContext, Transport,
    };

    // VST3 implementation
    pub use beamer_vst3::{export_vst3, PluginConfig, Vst3Processor};

    // Derive macros for parameters (when feature enabled)
    #[cfg(feature = "derive")]
    pub use beamer_macros::Params as DeriveParams;
    #[cfg(feature = "derive")]
    pub use beamer_macros::EnumParam as DeriveEnumParam;
}
