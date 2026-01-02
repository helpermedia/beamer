//! # BEAMR
//!
//! VST3 Framework for Rust.
//!
//! BEAMR is a framework for building VST3 audio plugins with WebView-based GUIs.
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
//! use beamr::prelude::*;
//! use beamr::vst3_impl::{Vst3Processor, vst3};
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
pub use beamr_core as core;
pub use beamr_vst3 as vst3_impl;

/// Prelude module for convenient imports.
///
/// Import everything you need to build a plugin:
/// ```rust,ignore
/// use beamr::prelude::*;
/// ```
pub mod prelude {
    // Core traits and types
    pub use beamr_core::{
        // Buffer types
        AuxiliaryBuffers, AuxInput, AuxOutput, Buffer,
        // Traits
        AudioProcessor, EditorDelegate, Parameters, Plugin,
        // Bus configuration
        BusInfo, BusType,
        // Editor types
        EditorConstraints, NoEditor,
        // Parameter types
        NoParams, ParamFlags, ParamInfo,
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
    pub use beamr_vst3::{export_vst3, PluginConfig, Vst3Processor};
}
