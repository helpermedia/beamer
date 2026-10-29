//! # beamer-core
//!
//! Core abstractions for the Beamer audio plugin framework.
//!
//! This crate provides platform-agnostic and format-agnostic traits that define
//! the interface for audio plugins. Its only external runtime dependencies are
//! `log`, `serde` and `serde_json`.
//!
//! ## Main Traits
//!
//! - [`Model`] - What every thread shares and the plugin as the host sees it (lives as long as the instance)
//! - [`Processor`] - Core DSP processing trait (created for an audio configuration)
//! - [`Parameters`] - Parameter collection trait
//! - [`Saved`] - Saving and loading the parameters and `State` fields
//! - [`WebViewEvents`] - Where the model's queue of events for the web UI is
//!
//! ## Types
//!
//! - [`Size`] - 2D size in pixels
//! - [`Rect`] - Rectangle in pixels
//! - [`Buffer`] - Main audio I/O buffer
//! - [`AuxiliaryBuffers`] - Sidechain and aux bus access
//! - [`Layout`], [`BusLayout`], [`ChannelSet`] - The channel layouts a plugin supports
//! - [`BusInfo`] - Name, type and default state of an audio bus
//! - [`ActiveBuses`] - The buses a plugin is prepared with
//! - [`ParameterInfo`] - Parameter metadata
//! - [`PluginError`] - Error types
//! - [`MidiEvent`] - MIDI event types
//! - [`Transport`] - DAW transport/timing state
//! - [`ProcessContext`] - Processing context with sample rate and transport

pub mod active_buses;
pub mod allocation_check;
pub mod assets;
pub mod audio_thread;
pub mod buffer;
pub mod buffer_storage;
pub mod bus_config;
pub mod bypass;
pub mod channel_layouts;
pub mod conversion_buffers;
pub mod config;
#[cfg(feature = "diagnostics")]
pub mod diagnostics;
pub mod editor_sync;
pub mod handover;
pub mod host_notifier;
pub mod logging;
pub mod error;
pub mod midi;
pub mod midi1_translator;
pub mod midi_cc_config;
pub mod midi_cc_state;
pub mod model;
pub mod parameter_format;
pub mod parameter_groups;
pub mod parameter_info;
pub mod parameter_range;
pub mod parameter_store;
pub mod parameter_types;
pub mod plugin;
pub mod preset;
pub mod process_context;
pub mod process_warnings;
pub mod sample;
pub mod setup;
pub mod smoothing;
pub mod state;
pub mod state_format;
pub mod sub_blocks;
pub mod types;
pub mod ump;
pub mod webview_dispatch;
pub mod webview_events;
pub mod webview_stream_sender;
pub mod webview_value_sender;

// Re-exports for convenience
pub use active_buses::{ActiveBuses, BusNegotiation, BusRouting};
pub use allocation_check::{AllocationCounts, Reaction};
pub use audio_thread::ProcessScope;
pub use buffer::{AuxiliaryBuffers, AuxInput, AuxOutput, Buffer};
pub use buffer_storage::ProcessBufferStorage;
pub use bus_config::{CachedBusConfig, CachedBusInfo};
pub use channel_layouts::{validate_layouts, BusLayout, ChannelSet, Layout};
pub use assets::{EmbeddedAsset, EmbeddedAssets};
pub use config::{Config, FourCharCode};
pub use conversion_buffers::ConversionBuffers;
pub use editor_sync::EditorSync;
pub use bypass::{BypassAction, BypassHandler, BypassState, CrossfadeCurve};
pub use host_notifier::{HostChangeTarget, HostChanges, HostNotifier};
pub use handover::{Handover, HandoverReader};
pub use logging::LogLevel;
pub use error::{PluginError, PluginResult};
pub use midi::{
    // Basic types
    cc, note_id, note_id_parts, scale, ChannelPressure, ControlChange, MidiBuffer, MidiChannel, MidiEvent,
    MidiEventKind, MidiEventsMut, MidiNote, NoteId, NoteOff, NoteOn, PitchBend, PolyPressure, ProgramChange,
    // Advanced VST3 events
    tuning_semitones, tuning_value, ChordInfo, NoteExpressionInt, NoteExpressionText, NoteExpressionValue,
    ScaleInfo,
    // MIDI 2.0 types
    Midi2Controller,
    // RPN/NRPN types
    rpn, ParameterChange, ParameterNumber, ParameterNumberKind, RELATIVE_14BIT_STEP,
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
    MAX_NOTE_EXPRESSION_TITLE_SIZE, MAX_SCALE_NAME_SIZE,
};
pub use parameter_format::Formatter;
pub use parameter_range::{LinearMapper, LogMapper, LogOffsetMapper, PowerMapper, RangeMapper};
pub use parameter_groups::{GroupId, GroupInfo, ParameterGroups, ROOT_GROUP_ID};
pub use parameter_info::{ParameterFlags, ParameterInfo, ParameterUnit};
pub use model::{Saved, SharedModel, WebViewEvents};
pub use parameter_store::{params_to_init_json, NoParameters, ParameterStore};
pub use parameter_types::{BoolParameter, EnumParameter, EnumParameterValue, FloatParameter, IntParameter, ParameterRef, Parameters};
pub use smoothing::{Smoother, SmoothingStyle};
pub use process_warnings::{ProcessWarning, ProcessWarnings};
pub use state::State;
pub use midi1_translator::Midi1Translator;
pub use midi_cc_config::{controller, MidiCcConfig, MAX_CC_CONTROLLER};
pub use midi_cc_state::{MidiCcState, MIDI_CC_PARAM_BASE};
pub use plugin::{
    BusInfo, BusType, HostSetup, MainInputChannels, MainOutputChannels, MaxBufferSize, Midi1Assignment,
    Midi2Assignment, MidiControllerAssignment, Model, PluginSetup, ProcessMode, Processor, SampleRate,
};
pub use preset::Preset;
pub use process_context::{FrameRate, ProcessContext, Transport};
pub use sample::Sample;
pub use types::{fnv1a_hash, ParameterId, ParameterValue, Rect, Size, MAX_AUX_BUSES, MAX_BUSES, MAX_CHANNELS};
pub use webview_events::WebViewEventQueue;
pub use webview_stream_sender::WebViewStreamSender;
pub use webview_value_sender::WebViewValueSender;
#[doc(hidden)]
pub use webview_dispatch::WebViewDispatch;

// Re-export serde_json so plugins can answer the web UI without adding
// serde_json to their own Cargo.toml.
pub use serde_json;

// Re-export serde for the types a plugin keeps in a `State`. A plugin that
// derives `Serialize` and `Deserialize` without a serde dependency of its
// own adds `#[serde(crate = "beamer::serde")]` to the type.
pub use serde;
