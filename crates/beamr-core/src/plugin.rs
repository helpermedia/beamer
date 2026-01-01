//! Core plugin trait definitions.

use crate::buffer::{AuxiliaryBuffers, Buffer};
use crate::error::PluginResult;
use crate::midi::{
    KeyswitchInfo, Midi2Controller, MidiBuffer, MidiEvent, MpeInputDeviceSettings,
    NoteExpressionTypeInfo, PhysicalUIMap,
};
use crate::params::Parameters;

// =============================================================================
// Bus Configuration
// =============================================================================

/// Audio bus type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BusType {
    /// Main audio bus (e.g., primary stereo input/output).
    #[default]
    Main,
    /// Auxiliary bus (e.g., sidechain input).
    Aux,
}

/// Information about an audio bus.
#[derive(Debug, Clone)]
pub struct BusInfo {
    /// Display name for the bus (e.g., "Input", "Sidechain").
    pub name: &'static str,
    /// Bus type (main or auxiliary).
    pub bus_type: BusType,
    /// Number of channels in this bus.
    pub channel_count: u32,
    /// Whether the bus is active by default.
    pub is_default_active: bool,
}

impl Default for BusInfo {
    fn default() -> Self {
        Self {
            name: "Main",
            bus_type: BusType::Main,
            channel_count: 2,
            is_default_active: true,
        }
    }
}

impl BusInfo {
    /// Create a stereo main bus.
    pub const fn stereo(name: &'static str) -> Self {
        Self {
            name,
            bus_type: BusType::Main,
            channel_count: 2,
            is_default_active: true,
        }
    }

    /// Create a mono main bus.
    pub const fn mono(name: &'static str) -> Self {
        Self {
            name,
            bus_type: BusType::Main,
            channel_count: 1,
            is_default_active: true,
        }
    }

    /// Create an auxiliary bus (e.g., sidechain).
    pub const fn aux(name: &'static str, channel_count: u32) -> Self {
        Self {
            name,
            bus_type: BusType::Aux,
            channel_count,
            is_default_active: false,
        }
    }
}

// =============================================================================
// AudioProcessor Trait
// =============================================================================

/// Core trait for audio processing logic.
///
/// This trait defines the DSP (Digital Signal Processing) interface that
/// plugin implementations must provide. It is designed to be format-agnostic,
/// meaning the same implementation can be wrapped for VST3, CLAP, or other
/// plugin formats.
///
/// # Thread Safety
///
/// Implementors must be `Send` because the plugin may be moved between threads.
/// The `process` method is called on the audio thread and must be real-time safe:
/// - No allocations
/// - No locks (use lock-free structures)
/// - No syscalls
/// - No unbounded loops
pub trait AudioProcessor: Send {
    /// Called when audio processing setup changes.
    ///
    /// This is called before audio processing begins, whenever the sample rate
    /// or maximum block size changes. Use this to initialize buffers, filters,
    /// or other sample-rate dependent state.
    ///
    /// # Arguments
    /// * `sample_rate` - The sample rate in Hz (e.g., 44100.0, 48000.0)
    /// * `max_buffer_size` - Maximum number of samples per process call
    fn setup(&mut self, sample_rate: f64, max_buffer_size: usize);

    /// Process an audio buffer.
    ///
    /// This is the main DSP entry point, called on the audio thread for each
    /// block of audio. The buffer provides input samples and mutable output
    /// buffers for the main bus.
    ///
    /// # Arguments
    ///
    /// * `buffer` - Main audio bus (stereo/surround input and output)
    /// * `aux` - Auxiliary buses (sidechain, aux sends) - ignore if not needed
    ///
    /// # Real-Time Safety
    ///
    /// This method must be real-time safe. Do not allocate, lock mutexes,
    /// or perform any operation with unbounded execution time.
    ///
    /// # Example: Simple Gain
    ///
    /// ```ignore
    /// fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers) {
    ///     let gain = self.params.gain();
    ///     for (input, output) in buffer.zip_channels() {
    ///         for (i, o) in input.iter().zip(output.iter_mut()) {
    ///             *o = *i * gain;
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Example: Sidechain Ducker
    ///
    /// ```ignore
    /// fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers) {
    ///     let duck = aux.sidechain()
    ///         .map(|sc| (sc.rms(0) * 4.0).min(1.0))
    ///         .unwrap_or(0.0);
    ///
    ///     buffer.copy_to_output();
    ///     buffer.apply_output_gain(1.0 - duck * 0.8);
    /// }
    /// ```
    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers);

    /// Called when the plugin is activated or deactivated.
    ///
    /// Activation typically happens when the user inserts the plugin into a
    /// track or opens a project. Deactivation happens when removed or project
    /// is closed.
    ///
    /// Default implementation does nothing.
    fn set_active(&mut self, _active: bool) {}

    /// Get the tail length in samples.
    ///
    /// This indicates how many samples of audio "tail" the plugin produces
    /// after input stops (e.g., reverb decay). Return 0 for no tail, or
    /// `u32::MAX` for infinite tail.
    ///
    /// Default returns 0 (no tail).
    fn tail_samples(&self) -> u32 {
        0
    }

    /// Get the latency in samples.
    ///
    /// If the plugin introduces processing latency (e.g., lookahead limiters),
    /// return the latency in samples here. The host can use this for delay
    /// compensation.
    ///
    /// Default returns 0 (no latency).
    fn latency_samples(&self) -> u32 {
        0
    }

    /// Save the plugin state to bytes.
    ///
    /// This is called when the DAW saves a project or preset. The returned
    /// bytes should contain all state needed to restore the plugin to its
    /// current configuration.
    ///
    /// Default returns an empty vector.
    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(Vec::new())
    }

    /// Load the plugin state from bytes.
    ///
    /// This is called when the DAW loads a project or preset. The data is
    /// the same bytes returned from a previous `save_state` call.
    ///
    /// Default does nothing.
    fn load_state(&mut self, _data: &[u8]) -> PluginResult<()> {
        Ok(())
    }

    // =========================================================================
    // Bus Configuration
    // =========================================================================

    /// Returns the number of audio input buses.
    ///
    /// Default returns 1 (single stereo input).
    fn input_bus_count(&self) -> usize {
        1
    }

    /// Returns the number of audio output buses.
    ///
    /// Default returns 1 (single stereo output).
    fn output_bus_count(&self) -> usize {
        1
    }

    /// Returns information about an input bus.
    ///
    /// Default returns a stereo main bus for index 0.
    fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
        if index == 0 {
            Some(BusInfo::stereo("Input"))
        } else {
            None
        }
    }

    /// Returns information about an output bus.
    ///
    /// Default returns a stereo main bus for index 0.
    fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
        if index == 0 {
            Some(BusInfo::stereo("Output"))
        } else {
            None
        }
    }

    // =========================================================================
    // MIDI Processing
    // =========================================================================

    /// Process MIDI events.
    ///
    /// Called during processing with any incoming MIDI events. Plugins can
    /// transform events and add them to the output buffer, pass them through
    /// unchanged, or consume them entirely.
    ///
    /// # Arguments
    /// * `input` - Slice of incoming MIDI events (sorted by sample_offset)
    /// * `output` - Buffer to write output MIDI events to
    ///
    /// # Real-Time Safety
    ///
    /// This method must be real-time safe. Do not allocate, lock mutexes,
    /// or perform any operation with unbounded execution time.
    ///
    /// # Default Implementation
    ///
    /// The default implementation passes all events through unchanged.
    fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
        for event in input {
            output.push(*event);
        }
    }

    /// Returns whether this plugin processes MIDI events.
    ///
    /// Override to return `true` if your plugin needs MIDI input/output.
    /// This is used by the host to determine event bus configuration.
    ///
    /// Default returns `false`.
    fn wants_midi(&self) -> bool {
        false
    }
}

// =============================================================================
// Plugin Trait
// =============================================================================

/// Main plugin trait combining audio processing and parameters.
///
/// This is the primary trait that plugin authors implement to create a complete
/// audio plugin. It combines [`AudioProcessor`] for DSP with a [`Parameters`]
/// collection for host communication.
///
/// # Example
///
/// ```ignore
/// use beamr_core::{Plugin, AudioProcessor, Buffer, AuxiliaryBuffers, Parameters};
///
/// pub struct MyGain {
///     params: MyGainParams,
/// }
///
/// impl AudioProcessor for MyGain {
///     fn setup(&mut self, _sample_rate: f64, _max_buffer_size: usize) {}
///
///     fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers) {
///         let gain = self.params.gain_linear();
///         for (input, output) in buffer.zip_channels() {
///             for (i, o) in input.iter().zip(output.iter_mut()) {
///                 *o = *i * gain;
///             }
///         }
///     }
/// }
///
/// impl Plugin for MyGain {
///     type Params = MyGainParams;
///
///     fn params(&self) -> &Self::Params {
///         &self.params
///     }
///
///     fn create() -> Self {
///         Self { params: MyGainParams::new() }
///     }
/// }
/// ```
pub trait Plugin: AudioProcessor {
    /// The parameter collection type for this plugin.
    type Params: Parameters;

    /// Returns a reference to the plugin's parameters.
    ///
    /// The VST3 wrapper uses this to communicate parameter values with the host.
    fn params(&self) -> &Self::Params;

    /// Creates a new instance of the plugin with default state.
    ///
    /// Called by the host when instantiating the plugin.
    fn create() -> Self
    where
        Self: Sized;

    // =========================================================================
    // MIDI Mapping (IMidiMapping)
    // =========================================================================

    /// Get the parameter ID mapped to a MIDI CC.
    ///
    /// Override this to enable DAW MIDI learn for your parameters. When the
    /// DAW queries which parameter is assigned to a MIDI CC, this method is
    /// called.
    ///
    /// # Arguments
    /// * `bus_index` - MIDI bus index (usually 0)
    /// * `channel` - MIDI channel (0-15), or -1 to query all channels
    /// * `cc` - MIDI CC number (0-127)
    ///
    /// # Returns
    /// `Some(param_id)` if this CC is mapped to a parameter, `None` otherwise.
    ///
    /// # Example
    /// ```ignore
    /// fn midi_cc_to_param(&self, _bus: i32, _channel: i16, cc: u8) -> Option<u32> {
    ///     match cc {
    ///         cc::MOD_WHEEL => Some(PARAM_VIBRATO_DEPTH),
    ///         cc::EXPRESSION => Some(PARAM_VOLUME),
    ///         _ => None,
    ///     }
    /// }
    /// ```
    fn midi_cc_to_param(&self, bus_index: i32, channel: i16, cc: u8) -> Option<u32> {
        let _ = (bus_index, channel, cc);
        None
    }

    // =========================================================================
    // MIDI Learn (IMidiLearn)
    // =========================================================================

    /// Called by DAW when live MIDI CC input is received during learn mode.
    ///
    /// Override this to implement MIDI learn in your plugin UI. When the user
    /// enables "MIDI Learn" mode and moves a MIDI CC knob, the DAW calls this
    /// method so the plugin can map that CC to a parameter.
    ///
    /// # Arguments
    /// * `bus_index` - MIDI bus index (usually 0)
    /// * `channel` - MIDI channel (0-15)
    /// * `cc` - MIDI CC number that was moved
    ///
    /// # Returns
    /// `true` if the input was handled (learned), `false` otherwise.
    fn on_midi_learn(&mut self, bus_index: i32, channel: i16, cc: u8) -> bool {
        let _ = (bus_index, channel, cc);
        false
    }

    // =========================================================================
    // MIDI 2.0 Mapping (IMidiMapping2)
    // =========================================================================

    /// Get all MIDI 1.0 CC assignments for bulk query.
    ///
    /// Override to provide mappings for DAW queries. This is more efficient
    /// than individual `midi_cc_to_param` queries when there are many mappings.
    ///
    /// Default returns empty slice (no mappings).
    fn midi1_assignments(&self) -> &[Midi1Assignment] {
        &[]
    }

    /// Get all MIDI 2.0 controller assignments for bulk query.
    ///
    /// Override to provide MIDI 2.0 Registered/Assignable controller mappings.
    ///
    /// Default returns empty slice (no mappings).
    fn midi2_assignments(&self) -> &[Midi2Assignment] {
        &[]
    }

    // =========================================================================
    // MIDI 2.0 Learn (IMidiLearn2)
    // =========================================================================

    /// Called when MIDI 1.0 CC input is received during learn mode.
    ///
    /// This is the MIDI 2.0 version of `on_midi_learn` with separate methods
    /// for MIDI 1.0 and MIDI 2.0 controllers.
    ///
    /// Default returns `false` (not handled).
    fn on_midi1_learn(&mut self, bus_index: i32, channel: u8, cc: u8) -> bool {
        let _ = (bus_index, channel, cc);
        false
    }

    /// Called when MIDI 2.0 controller input is received during learn mode.
    ///
    /// Override to implement MIDI 2.0 controller learning.
    ///
    /// Default returns `false` (not handled).
    fn on_midi2_learn(&mut self, bus_index: i32, channel: u8, controller: Midi2Controller) -> bool {
        let _ = (bus_index, channel, controller);
        false
    }

    // =========================================================================
    // Note Expression Controller (INoteExpressionController - VST3 SDK 3.5.0)
    // =========================================================================

    /// Returns the number of supported note expression types.
    ///
    /// Override to advertise which note expressions your plugin supports
    /// (e.g., volume, pan, tuning for MPE instruments).
    ///
    /// Default returns 0 (no note expressions).
    fn note_expression_count(&self, bus_index: i32, channel: i16) -> usize {
        let _ = (bus_index, channel);
        0
    }

    /// Returns information about a note expression type by index.
    ///
    /// Override to provide details about each supported expression type.
    ///
    /// Default returns None.
    fn note_expression_info(
        &self,
        bus_index: i32,
        channel: i16,
        index: usize,
    ) -> Option<NoteExpressionTypeInfo> {
        let _ = (bus_index, channel, index);
        None
    }

    /// Converts a normalized note expression value to a display string.
    ///
    /// Override to provide custom formatting (e.g., "2.5 semitones" for tuning).
    ///
    /// Default returns the value as a percentage.
    fn note_expression_value_to_string(
        &self,
        bus_index: i32,
        channel: i16,
        type_id: u32,
        value: f64,
    ) -> String {
        let _ = (bus_index, channel, type_id);
        format!("{:.1}%", value * 100.0)
    }

    /// Parses a string to a normalized note expression value.
    ///
    /// Override to support custom parsing.
    ///
    /// Default returns None (parsing not supported).
    fn note_expression_string_to_value(
        &self,
        bus_index: i32,
        channel: i16,
        type_id: u32,
        string: &str,
    ) -> Option<f64> {
        let _ = (bus_index, channel, type_id, string);
        None
    }

    // =========================================================================
    // Keyswitch Controller (IKeyswitchController - VST3 SDK 3.5.0)
    // =========================================================================

    /// Returns the number of keyswitches (articulations).
    ///
    /// Override for sample libraries and orchestral instruments that
    /// support keyswitching between articulations.
    ///
    /// Default returns 0 (no keyswitches).
    fn keyswitch_count(&self, bus_index: i32, channel: i16) -> usize {
        let _ = (bus_index, channel);
        0
    }

    /// Returns information about a keyswitch by index.
    ///
    /// Override to provide keyswitch details for DAW expression maps.
    ///
    /// Default returns None.
    fn keyswitch_info(&self, bus_index: i32, channel: i16, index: usize) -> Option<KeyswitchInfo> {
        let _ = (bus_index, channel, index);
        None
    }

    // =========================================================================
    // Physical UI Mapping (INoteExpressionPhysicalUIMapping - VST3 SDK 3.6.11)
    // =========================================================================

    /// Returns mappings from physical UI controllers to note expressions.
    ///
    /// Override to define how MPE controllers (X-axis, Y-axis, Pressure)
    /// map to your plugin's note expression types.
    ///
    /// # Example
    /// ```ignore
    /// fn physical_ui_mappings(&self, _bus: i32, _channel: i16) -> &[PhysicalUIMap] {
    ///     &[
    ///         PhysicalUIMap::y_axis(note_expression::BRIGHTNESS),
    ///         PhysicalUIMap::pressure(note_expression::EXPRESSION),
    ///     ]
    /// }
    /// ```
    ///
    /// Default returns empty slice (no mappings).
    fn physical_ui_mappings(&self, bus_index: i32, channel: i16) -> &[PhysicalUIMap] {
        let _ = (bus_index, channel);
        &[]
    }

    // =========================================================================
    // MPE Wrapper Support (IVst3WrapperMPESupport - VST3 SDK 3.6.12)
    // =========================================================================

    /// Called to enable or disable MPE input processing.
    ///
    /// Override to handle MPE enable/disable notifications from wrappers.
    ///
    /// Default does nothing and returns true.
    fn enable_mpe_input_processing(&mut self, enabled: bool) -> bool {
        let _ = enabled;
        true
    }

    /// Called when the MPE input device settings change.
    ///
    /// Override to receive MPE zone configuration from wrappers.
    ///
    /// Default does nothing and returns true.
    fn set_mpe_input_device_settings(&mut self, settings: MpeInputDeviceSettings) -> bool {
        let _ = settings;
        true
    }
}

// =============================================================================
// MIDI Mapping Types
// =============================================================================

/// Base assignment info for MIDI controller → parameter mapping.
#[derive(Debug, Clone, Copy)]
pub struct MidiControllerAssignment {
    /// Parameter ID this controller maps to.
    pub param_id: u32,
    /// MIDI bus index.
    pub bus_index: i32,
    /// MIDI channel (0-15).
    pub channel: u8,
}

/// MIDI 1.0 CC assignment.
///
/// Maps a MIDI 1.0 Control Change to a parameter.
#[derive(Debug, Clone, Copy)]
pub struct Midi1Assignment {
    /// Base assignment info (param_id, bus, channel).
    pub assignment: MidiControllerAssignment,
    /// CC number (0-127).
    pub controller: u8,
}

impl Midi1Assignment {
    /// Create a new MIDI 1.0 CC assignment.
    pub const fn new(param_id: u32, bus_index: i32, channel: u8, controller: u8) -> Self {
        Self {
            assignment: MidiControllerAssignment {
                param_id,
                bus_index,
                channel,
            },
            controller,
        }
    }

    /// Create an assignment for the default bus and all channels.
    pub const fn simple(param_id: u32, controller: u8) -> Self {
        Self::new(param_id, 0, 0, controller)
    }
}

/// MIDI 2.0 controller assignment.
///
/// Maps a MIDI 2.0 Registered/Assignable Controller to a parameter.
#[derive(Debug, Clone, Copy)]
pub struct Midi2Assignment {
    /// Base assignment info (param_id, bus, channel).
    pub assignment: MidiControllerAssignment,
    /// MIDI 2.0 controller identifier.
    pub controller: Midi2Controller,
}

impl Midi2Assignment {
    /// Create a new MIDI 2.0 controller assignment.
    pub const fn new(
        param_id: u32,
        bus_index: i32,
        channel: u8,
        controller: Midi2Controller,
    ) -> Self {
        Self {
            assignment: MidiControllerAssignment {
                param_id,
                bus_index,
                channel,
            },
            controller,
        }
    }

    /// Create an assignment for the default bus and all channels.
    pub const fn simple(param_id: u32, controller: Midi2Controller) -> Self {
        Self::new(param_id, 0, 0, controller)
    }
}
