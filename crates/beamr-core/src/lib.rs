//! # beamr-core
//!
//! Core abstractions for the BEAMR VST3 WebView framework.
//!
//! This crate provides platform-agnostic and format-agnostic traits that define
//! the interface for audio plugins. It has no external dependencies, making it
//! suitable for use in any context.
//!
//! ## Main Traits
//!
//! - [`Plugin`] - Complete plugin trait combining DSP and parameters
//! - [`AudioProcessor`] - Core DSP processing trait
//! - [`Parameters`] - Parameter collection trait
//! - [`EditorDelegate`] - GUI configuration and callbacks
//!
//! ## Types
//!
//! - [`Size`] - 2D size in pixels
//! - [`Rect`] - Rectangle in pixels
//! - [`Buffer`] - Main audio I/O buffer
//! - [`AuxiliaryBuffers`] - Sidechain and aux bus access
//! - [`BusInfo`] - Audio bus configuration
//! - [`ParamInfo`] - Parameter metadata
//! - [`PluginError`] - Error types
//! - [`MidiEvent`] - MIDI event types
//! - [`Transport`] - DAW transport/timing state
//! - [`ProcessContext`] - Processing context with sample rate and transport

pub mod buffer;
pub mod editor;
pub mod error;
pub mod midi;
pub mod params;
pub mod plugin;
pub mod process_context;
pub mod types;

// Re-exports for convenience
#[allow(deprecated)]
pub use buffer::{AudioBuffer, AuxiliaryBuffers, AuxInput, AuxOutput, Buffer, Bus};
pub use editor::{EditorConstraints, EditorDelegate, NoEditor};
pub use error::{PluginError, PluginResult};
pub use midi::{
    // Basic types
    cc, ChannelPressure, ControlChange, MidiBuffer, MidiChannel, MidiEvent, MidiEventKind,
    MidiNote, NoteId, NoteOff, NoteOn, PitchBend, PolyPressure, ProgramChange,
    // Advanced VST3 events
    ChordInfo, NoteExpressionInt, NoteExpressionText, NoteExpressionValue, ScaleInfo, SysEx,
    // MIDI 2.0 types
    Midi2Controller,
    // RPN/NRPN types
    rpn, ParameterNumberKind, ParameterNumberMessage, RpnTracker,
    // Note Expression Controller types (VST3 SDK 3.5.0)
    NoteExpressionTypeFlags, NoteExpressionTypeInfo, NoteExpressionValueDesc,
    // Keyswitch Controller types (VST3 SDK 3.5.0)
    keyswitch_type, KeyswitchInfo,
    // Physical UI Mapping types (VST3 SDK 3.6.11)
    physical_ui, PhysicalUIMap,
    // MPE Support types (VST3 SDK 3.6.12)
    MpeInputDeviceSettings,
    // Constants modules
    note_expression,
    // 14-bit CC utilities
    combine_14bit_cc, combine_14bit_raw, split_14bit_cc, split_14bit_raw,
    // Buffer size constants
    MAX_CHORD_NAME_SIZE, MAX_EXPRESSION_TEXT_SIZE, MAX_KEYSWITCH_TITLE_SIZE,
    MAX_NOTE_EXPRESSION_TITLE_SIZE, MAX_SCALE_NAME_SIZE, MAX_SYSEX_SIZE,
};
pub use params::{NoParams, ParamFlags, ParamInfo, Parameters};
pub use plugin::{
    AudioProcessor, BusInfo, BusType, Midi1Assignment, Midi2Assignment, MidiControllerAssignment,
    Plugin,
};
pub use process_context::{FrameRate, ProcessContext, Transport};
pub use types::{ParamId, ParamValue, Rect, Size};
