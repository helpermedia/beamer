//! # Beamer
//!
//! Audio Plugin Framework for Rust.
//!
//! Beamer is a framework for building audio plugins with WebView-based GUIs.
//! It provides safe Rust abstractions that work with multiple plugin formats.
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
//! struct MyGain { parameters: MyParameters }
//!
//! impl AudioProcessor for MyGain {
//!     fn setup(&mut self, _: f64, _: usize) {}
//!     fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
//!         // Your DSP here
//!     }
//! }
//!
//! impl Plugin for MyGain {
//!     type Parameters = MyParameters;
//!     fn parameters(&self) -> &Self::Parameters { &self.parameters }
//!     fn create() -> Self { Self { parameters: MyParameters::new() } }
//! }
//!
//! // Export
//! static CONFIG: PluginConfig = PluginConfig::new("MyGain");
//! static VST3_CONFIG: Vst3Config = Vst3Config::new(vst3::uid(...));
//! export_vst3!(CONFIG, VST3_CONFIG, MyGain);
//! ```

// Re-export sub-crates
pub use beamer_core as core;

#[cfg(feature = "vst3")]
pub use beamer_vst3 as vst3_impl;

/// Re-export of vst3 types needed for plugin configuration.
///
/// This allows examples and plugins to use `beamer::vst3::{uid, Steinberg::TUID}`
/// without adding a direct dependency on the vst3 crate.
#[cfg(feature = "vst3")]
pub mod vst3 {
    pub use ::vst3::{uid, Steinberg};
}

#[cfg(feature = "au")]
pub use beamer_au as au_impl;

// Re-export derive macros when feature is enabled
#[cfg(feature = "derive")]
pub use beamer_macros::Parameters;
#[cfg(feature = "derive")]
pub use beamer_macros::EnumParameter;
#[cfg(feature = "derive")]
pub use beamer_macros::HasParameters;
#[cfg(feature = "derive")]
pub use beamer_macros::Presets;

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
        // Bypass handling
        BypassAction, BypassHandler, BypassState, CrossfadeCurve,
        // Sample trait for generic f32/f64 processing
        Sample,
        // Traits
        AudioProcessor, EditorDelegate, HasParameters, Plugin,
        // Plugin setup types (composable)
        PluginSetup, SampleRate, MaxBufferSize, MainInputChannels, MainOutputChannels,
        AuxInputCount, AuxOutputCount, ProcessMode,
        // Bus configuration
        BusInfo, BusType,
        // Editor types
        EditorConstraints, NoEditor,
        // Parameter metadata
        NoParameters, ParameterFlags, ParameterInfo,
        // Factory presets
        FactoryPresets, NoPresets, PresetInfo, PresetValue,
        // Parameter types
        BoolParameter, EnumParameter, EnumParameterValue, FloatParameter, IntParameter, Formatter, ParameterRef, Parameters,
        // MIDI CC configuration (framework manages runtime state)
        MidiCcConfig,
        // Parameter smoothing
        Smoother, SmoothingStyle,
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
    };

    // Shared plugin configuration (format-agnostic)
    pub use beamer_core::PluginConfig;

    // VST3 implementation (only when feature enabled)
    #[cfg(feature = "vst3")]
    pub use beamer_vst3::{export_vst3, Vst3Config, Vst3Processor};

    // AU implementation (only when feature enabled)
    #[cfg(feature = "au")]
    pub use beamer_au::{export_au, AuConfig, AuProcessor, ComponentType, fourcc};

    // Derive macros for parameters (when feature enabled)
    // These share names with the traits/types they implement, which is allowed
    // because traits and derive macros live in different namespaces.
    #[cfg(feature = "derive")]
    pub use beamer_macros::{EnumParameter, HasParameters, Parameters, Presets};
}
