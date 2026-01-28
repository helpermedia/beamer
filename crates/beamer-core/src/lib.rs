//! # beamer-core
//!
//! Core abstractions for the Beamer audio plugin framework.
//!
//! This crate provides platform-agnostic and format-agnostic traits that define
//! the interface for audio plugins. It has no external dependencies, making it
//! suitable for use in any context.
//!
//! ## Main Traits
//!
//! - [`Descriptor`] - Plugin definition (unprepared state, holds parameters)
//! - [`Processor`] - Core DSP processing trait (prepared state)
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
//! - [`ParameterInfo`] - Parameter metadata
//! - [`PluginError`] - Error types
//! - [`MidiEvent`] - MIDI event types
//! - [`Transport`] - DAW transport/timing state
//! - [`ProcessContext`] - Processing context with sample rate and transport

pub mod buffer;
pub mod buffer_storage;
pub mod bus_config;
pub mod bypass;
pub mod conversion_buffers;
pub mod config;
pub mod editor;
pub mod error;
pub mod midi;
pub mod midi_cc_config;
pub mod midi_cc_state;
pub mod parameter_format;
pub mod parameter_groups;
pub mod parameter_info;
pub mod parameter_range;
pub mod parameter_store;
pub mod parameter_types;
pub mod plugin;
pub mod preset;
pub mod process_context;
pub mod sample;
pub mod setup;
pub mod smoothing;
pub mod sysex_pool;
pub mod types;

// Re-exports for convenience
pub use buffer::{AuxiliaryBuffers, AuxInput, AuxOutput, Buffer};
pub use buffer_storage::ProcessBufferStorage;
pub use bus_config::{CachedBusConfig, CachedBusInfo};
pub use config::Config;
pub use conversion_buffers::ConversionBuffers;
pub use bypass::{BypassAction, BypassHandler, BypassState, CrossfadeCurve};
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
pub use parameter_format::Formatter;
pub use parameter_range::{LinearMapper, LogMapper, LogOffsetMapper, PowerMapper, RangeMapper};
pub use parameter_groups::{GroupId, GroupInfo, ParameterGroups, ROOT_GROUP_ID};
pub use parameter_info::{ParameterFlags, ParameterInfo, ParameterUnit};
pub use parameter_store::{NoParameters, ParameterStore};
pub use parameter_types::{BoolParameter, EnumParameter, EnumParameterValue, FloatParameter, IntParameter, ParameterRef, Parameters};
pub use smoothing::{Smoother, SmoothingStyle};
pub use midi_cc_config::{controller, MidiCcConfig, MAX_CC_CONTROLLER};
pub use midi_cc_state::{MidiCcState, MIDI_CC_PARAM_BASE};
pub use plugin::{
    AuxInputCount, AuxOutputCount, BusInfo, BusLayout, BusType, Descriptor, HasParameters,
    HostSetup, MainInputChannels, MainOutputChannels, MaxBufferSize, Midi1Assignment,
    Midi2Assignment, MidiControllerAssignment, PluginSetup, ProcessMode, Processor, SampleRate,
};
pub use preset::{fnv1a_hash, FactoryPresets, NoPresets, PresetInfo, PresetValue};
pub use process_context::{FrameRate, ProcessContext, Transport};
pub use sample::Sample;
pub use sysex_pool::SysExOutputPool;
pub use types::{ParameterId, ParameterValue, Rect, Size, MAX_AUX_BUSES, MAX_BUSES, MAX_CHANNELS};
