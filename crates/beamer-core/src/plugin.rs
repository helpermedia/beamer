//! Core plugin trait definitions.
//!
//! A plugin is two structs:
//!
//! - **The model** ([`Model`]): what every thread shares. It holds the
//!   parameters, the `State` fields and everything else the host, the UI and
//!   the audio thread use. It describes buses and MIDI behavior to the host
//!   and answers the web UI. The framework keeps it in one [`Arc`] for the
//!   life of the plugin instance.
//!
//! - **The processor** ([`Processor`]): what the audio thread owns. It is
//!   created by [`Model::prepare()`] with the real sample rate and buffer
//!   configuration, and dropped when the host releases the audio resources or
//!   changes the configuration.
//!
//! This design eliminates placeholder values by making it impossible to process audio
//! until proper configuration is available.

use std::sync::Arc;

use crate::buffer::{AuxiliaryBuffers, Buffer};
use crate::midi::{
    KeyswitchInfo, Midi2Controller, MidiBuffer, MidiEvent, MpeInputDeviceSettings,
    NoteExpressionTypeInfo, PhysicalUIMap,
};
use crate::midi_cc_config::MidiCcConfig;
use crate::model::ModelFields;
use crate::parameter_store::ParameterStore;
use crate::process_context::ProcessContext;

// =============================================================================
// Plugin Setup Types - Composable Host Information
// =============================================================================

/// Internal: All information the host provides at initialization.
///
/// This is passed to `PluginSetup::extract()` so each setup type can
/// extract only what it needs.
#[derive(Clone, Debug)]
pub struct HostSetup {
    /// Sample rate in Hz (e.g., 44100.0, 48000.0, 96000.0)
    pub sample_rate: f64,
    /// Maximum number of samples per process() call
    pub max_buffer_size: usize,
    /// Bus layout information
    pub layout: BusLayout,
    /// Processing mode (realtime vs offline)
    pub process_mode: ProcessMode,
}

impl HostSetup {
    /// Create a new HostSetup with the given values.
    pub fn new(sample_rate: f64, max_buffer_size: usize, layout: BusLayout, process_mode: ProcessMode) -> Self {
        Self {
            sample_rate,
            max_buffer_size,
            layout,
            process_mode,
        }
    }
}

/// Trait for plugin setup requirements.
///
/// Plugins declare what host information they need via [`Model::Setup`].
/// The framework extracts only the requested data from the host.
///
/// # Composable Setup Types
///
/// Request exactly what you need using individual types or tuples:
///
/// ```ignore
/// type Setup = ();                                  // No setup needed
/// type Setup = SampleRate;                          // Just sample rate
/// type Setup = (SampleRate, MaxBufferSize);         // Sample rate + buffer size
/// type Setup = (SampleRate, MainOutputChannels);    // Sample rate + channels
/// ```
///
/// # Available Types
///
/// | Type | Value | Use Case |
/// |------|-------|----------|
/// | `()` | - | Stateless plugins (gain, pan) |
/// | [`SampleRate`] | `f64` | Time-based DSP (delay, filter, envelope) |
/// | [`MaxBufferSize`] | `usize` | FFT, lookahead buffers |
/// | [`MainInputChannels`] | `u32` | Per-channel input processing |
/// | [`MainOutputChannels`] | `u32` | Per-channel output state |
/// | [`AuxInputCount`] | `usize` | Sidechain-aware processing |
/// | [`AuxOutputCount`] | `usize` | Multi-bus output |
/// | [`ProcessMode`] | enum | Quality settings for offline rendering |
pub trait PluginSetup: Clone + Send + 'static {
    /// Extract this setup from the host-provided information.
    fn extract(host: &HostSetup) -> Self;
}

// =============================================================================
// Unit Type - No Setup Needed
// =============================================================================

/// Use `()` for stateless plugins that don't depend on sample rate.
///
/// # Example
///
/// ```ignore
/// impl Model for GainModel {
///     type Setup = ();
///     fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
///         GainProcessor { model: self }
///     }
/// }
/// ```
impl PluginSetup for () {
    fn extract(_: &HostSetup) -> Self {}
}

// =============================================================================
// Individual Setup Types
// =============================================================================

/// Sample rate in Hz.
///
/// Use for most plugins with time-dependent DSP: delays, filters,
/// envelopes, etc. This is the most common choice.
///
/// # Example
///
/// ```ignore
/// impl Model for DelayModel {
///     type Setup = SampleRate;
///     fn prepare(self: Arc<Self>, sample_rate: SampleRate) -> DelayProcessor {
///         let buffer_size = (MAX_DELAY_SECONDS * sample_rate.0) as usize;
///         DelayProcessor {
///             sample_rate: sample_rate.0,
///             buffer: vec![0.0; buffer_size],
///         }
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SampleRate(pub f64);

impl SampleRate {
    /// Get the sample rate in Hz.
    #[inline]
    pub fn hz(&self) -> f64 {
        self.0
    }
}

impl PluginSetup for SampleRate {
    fn extract(host: &HostSetup) -> Self {
        SampleRate(host.sample_rate)
    }
}

/// Maximum buffer size in samples.
///
/// Use for plugins that need to pre-allocate processing buffers,
/// like FFT-based processors or lookahead limiters.
///
/// # Example
///
/// ```ignore
/// impl Model for FftModel {
///     type Setup = (SampleRate, MaxBufferSize);
///     fn prepare(self: Arc<Self>, (sr, mbs): (SampleRate, MaxBufferSize)) -> FftProcessor {
///         FftProcessor {
///             sample_rate: sr.0,
///             fft_buffer: vec![0.0; mbs.0],
///         }
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaxBufferSize(pub usize);

impl PluginSetup for MaxBufferSize {
    fn extract(host: &HostSetup) -> Self {
        MaxBufferSize(host.max_buffer_size)
    }
}

/// Number of channels on the main input bus.
///
/// Use for plugins that need per-channel input processing,
/// like channel-specific EQ or surround processors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainInputChannels(pub u32);

impl PluginSetup for MainInputChannels {
    fn extract(host: &HostSetup) -> Self {
        MainInputChannels(host.layout.main_input_channels)
    }
}

/// Number of channels on the main output bus.
///
/// Use for plugins that need per-channel output state allocation,
/// like surround processors or multi-channel analyzers.
///
/// # Example
///
/// ```ignore
/// impl Model for SurroundModel {
///     type Setup = (SampleRate, MainOutputChannels);
///     fn prepare(self: Arc<Self>, (sr, channels): (SampleRate, MainOutputChannels)) -> SurroundProcessor {
///         SurroundProcessor {
///             sample_rate: sr.0,
///             per_channel_state: vec![State::new(); channels.0 as usize],
///         }
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MainOutputChannels(pub u32);

impl PluginSetup for MainOutputChannels {
    fn extract(host: &HostSetup) -> Self {
        MainOutputChannels(host.layout.main_output_channels)
    }
}

/// Number of auxiliary input buses.
///
/// Use for sidechain-aware plugins that need to know
/// how many aux inputs are available.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuxInputCount(pub usize);

impl PluginSetup for AuxInputCount {
    fn extract(host: &HostSetup) -> Self {
        AuxInputCount(host.layout.aux_input_count)
    }
}

/// Number of auxiliary output buses.
///
/// Use for multi-bus output plugins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuxOutputCount(pub usize);

impl PluginSetup for AuxOutputCount {
    fn extract(host: &HostSetup) -> Self {
        AuxOutputCount(host.layout.aux_output_count)
    }
}

/// Processing mode (realtime vs offline).
///
/// Use for plugins that want different quality settings
/// for realtime vs offline rendering (e.g., higher quality
/// oversampling when rendering offline).
///
/// Hosts report it in every plugin format. When a host changes the mode,
/// the plugin is prepared again with the new one.
///
/// # Example
///
/// ```ignore
/// impl Model for OversamplingModel {
///     type Setup = (SampleRate, ProcessMode);
///     fn prepare(self: Arc<Self>, (sr, mode): (SampleRate, ProcessMode)) -> OversamplingProcessor {
///         let oversampling = match mode {
///             ProcessMode::Realtime | ProcessMode::Prefetch => 2,
///             ProcessMode::Offline => 8,
///         };
///         OversamplingProcessor { sample_rate: sr.0, oversampling }
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProcessMode {
    /// Normal realtime playback.
    #[default]
    Realtime,
    /// Offline rendering (bounce, export).
    Offline,
    /// Prefetch (used by some hosts for look-ahead). VST3 only: Audio Units
    /// know realtime and offline rendering.
    Prefetch,
}

impl PluginSetup for ProcessMode {
    fn extract(host: &HostSetup) -> Self {
        host.process_mode
    }
}

// =============================================================================
// Tuple Implementations - Compose Multiple Setup Types
// =============================================================================

impl<A, B> PluginSetup for (A, B)
where
    A: PluginSetup,
    B: PluginSetup,
{
    fn extract(host: &HostSetup) -> Self {
        (A::extract(host), B::extract(host))
    }
}

impl<A, B, C> PluginSetup for (A, B, C)
where
    A: PluginSetup,
    B: PluginSetup,
    C: PluginSetup,
{
    fn extract(host: &HostSetup) -> Self {
        (A::extract(host), B::extract(host), C::extract(host))
    }
}

impl<A, B, C, D> PluginSetup for (A, B, C, D)
where
    A: PluginSetup,
    B: PluginSetup,
    C: PluginSetup,
    D: PluginSetup,
{
    fn extract(host: &HostSetup) -> Self {
        (A::extract(host), B::extract(host), C::extract(host), D::extract(host))
    }
}

impl<A, B, C, D, E> PluginSetup for (A, B, C, D, E)
where
    A: PluginSetup,
    B: PluginSetup,
    C: PluginSetup,
    D: PluginSetup,
    E: PluginSetup,
{
    fn extract(host: &HostSetup) -> Self {
        (A::extract(host), B::extract(host), C::extract(host), D::extract(host), E::extract(host))
    }
}

// =============================================================================
// Bus Layout Information
// =============================================================================

/// Bus layout information for plugins that need channel configuration.
///
/// This is used internally to populate the individual setup types
/// like [`MainInputChannels`], [`MainOutputChannels`], etc.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BusLayout {
    /// Number of channels on the main input bus
    pub main_input_channels: u32,
    /// Number of channels on the main output bus
    pub main_output_channels: u32,
    /// Number of auxiliary input buses
    pub aux_input_count: usize,
    /// Number of auxiliary output buses
    pub aux_output_count: usize,
}

impl BusLayout {
    /// Create a stereo (2 in, 2 out) layout with no aux buses.
    pub const fn stereo() -> Self {
        Self {
            main_input_channels: 2,
            main_output_channels: 2,
            aux_input_count: 0,
            aux_output_count: 0,
        }
    }

    /// Create a layout from the bus configuration a model declares.
    pub fn from_model<P: Model>(model: &P) -> Self {
        Self {
            main_input_channels: model
                .input_bus_info(0)
                .map(|b| b.channel_count)
                .unwrap_or(2),
            main_output_channels: model
                .output_bus_info(0)
                .map(|b| b.channel_count)
                .unwrap_or(2),
            aux_input_count: model.input_bus_count().saturating_sub(1),
            aux_output_count: model.output_bus_count().saturating_sub(1),
        }
    }
}

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
// Processor Trait
// =============================================================================

/// The prepared processor - ready for audio processing.
///
/// This trait defines the DSP (Digital Signal Processing) interface that
/// plugin implementations must provide. It is designed to be format-agnostic,
/// meaning the same implementation can be wrapped for AU, VST3 or other
/// plugin formats.
///
/// A `Processor` is created by calling [`Model::prepare()`] with the
/// audio configuration, so it holds valid configuration from the start - no
/// placeholder values. It belongs to the audio thread. Everything that other
/// threads need to see or change lives in the shared model, of which the
/// processor keeps its own `Arc`.
///
/// # Lifecycle
///
/// ```text
/// Model::default() -> Model (in an Arc, lives as long as the plugin instance)
///                      |
///                      v  Model::prepare(self: Arc<Self>, setup)
///                      |
///                      v
///                Processor (ready for audio)
///                      |
///                      v  the host releases the audio resources or
///                      |  changes the configuration
///                      v
///                   dropped (the next prepare() creates a new one)
/// ```
///
/// A processor is dropped on a host thread, never on the audio thread. What
/// has to outlive a processor belongs in the model.
///
/// # Thread Safety
///
/// Implementors must be `Send` because the plugin may be moved between threads.
/// The `process` method is called on the audio thread and must be real-time safe:
/// - No allocations
/// - No locks (use lock-free structures)
/// - No syscalls
/// - No unbounded loops
pub trait Processor: Send + 'static {
    /// Process an audio buffer with transport context.
    ///
    /// This is the main DSP entry point, called on the audio thread for each
    /// block of audio. The buffer provides input samples and mutable output
    /// buffers for the main bus.
    ///
    /// # Arguments
    ///
    /// * `buffer` - Main audio bus (stereo/surround input and output)
    /// * `aux` - Auxiliary buses (sidechain, aux sends) - ignore if not needed
    /// * `context` - Processing context with sample rate, buffer size and transport info
    ///
    /// # Real-Time Safety
    ///
    /// This method must be real-time safe. Do not allocate, lock mutexes,
    /// or perform any operation with unbounded execution time.
    ///
    /// # Example: Simple Gain
    ///
    /// ```ignore
    /// fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
    ///     let gain = self.model.gain();
    ///     for (input, output) in buffer.zip_channels() {
    ///         for (i, o) in input.iter().zip(output.iter_mut()) {
    ///             *o = *i * gain;
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Example: Tempo-Synced LFO
    ///
    /// ```ignore
    /// fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
    ///     // Calculate LFO rate synced to host tempo
    ///     let lfo_hz = context.transport.tempo
    ///         .map(|tempo| tempo / 60.0 / 4.0)  // 1 cycle per 4 beats
    ///         .unwrap_or(2.0);                   // Fallback: 2 Hz
    ///
    ///     let increment = (lfo_hz * 2.0 * std::f32::consts::PI) / context.sample_rate as f32;
    ///
    ///     for (input, output) in buffer.zip_channels() {
    ///         for (i, o) in input.iter().zip(output.iter_mut()) {
    ///             let lfo = self.phase.sin();
    ///             *o = *i * (1.0 + lfo * 0.5);
    ///             self.phase += increment;
    ///         }
    ///     }
    /// }
    /// ```
    ///
    /// # Example: Sidechain Ducker
    ///
    /// ```ignore
    /// fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
    ///     let duck = aux.sidechain()
    ///         .map(|sc| (sc.rms(0) * 4.0).min(1.0))
    ///         .unwrap_or(0.0);
    ///
    ///     buffer.copy_to_output();
    ///     buffer.apply_output_gain(1.0 - duck * 0.8);
    /// }
    /// ```
    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext);

    // =========================================================================
    // Activation State
    // =========================================================================

    /// Called when the plugin is activated or deactivated.
    ///
    /// Activation typically happens when the user inserts the plugin into a
    /// track or opens a project. Deactivation happens when removed or project
    /// is closed.
    ///
    /// **Important:** When `active == true`, you should reset your DSP state
    /// (clear delay lines, reset filter histories, zero envelopes, etc.).
    /// Hosts call `setActive(false)` followed by `setActive(true)` to request
    /// a full state reset.
    ///
    /// Must be realtime-safe (no allocation, no locks): when an Audio Unit
    /// host asks for a reset while audio runs, the wrapper calls this at the
    /// start of the next render call, on the audio thread.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn set_active(&mut self, active: bool) {
    ///     if active {
    ///         // Reset DSP state on activation
    ///         self.delay_line.clear();
    ///         self.envelope.reset();
    ///         self.filter_state = FilterState::default();
    ///     }
    /// }
    /// ```
    ///
    /// Default implementation does nothing.
    fn set_active(&mut self, _active: bool) {}

    /// Get the tail length in samples.
    ///
    /// This indicates how many samples of audio "tail" the plugin produces
    /// after input stops (e.g., reverb decay). Return 0 for no tail, or
    /// `u32::MAX` for infinite tail.
    ///
    /// Called on the audio thread for every block, so it must be
    /// realtime-safe. A host that asks for the tail gets the value of the
    /// most recent block.
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
    /// Called on the audio thread for every block, so it must be
    /// realtime-safe. A host that asks for the latency gets the value of the
    /// most recent block.
    ///
    /// Default returns 0 (no latency).
    fn latency_samples(&self) -> u32 {
        0
    }

    /// Get the bypass ramp length in samples.
    ///
    /// When bypass is engaged or disengaged, this defines the crossfade
    /// duration to avoid clicks. The host uses this value (combined with
    /// `tail_samples()`) to determine how long to continue calling `process()`
    /// after input stops.
    ///
    /// Return 0 for instant bypass (no crossfade), or a sample count for
    /// smooth crossfading. Typical values:
    /// - 64 samples (~1.3ms at 48kHz) - fast, suitable for most effects
    /// - 256 samples (~5.3ms at 48kHz) - smoother, for sensitive material
    /// - 512+ samples - very smooth, for reverbs/delays with long tails
    ///
    /// Default returns 64 samples.
    ///
    /// Called on the audio thread for every block, so it must be
    /// realtime-safe.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn bypass_ramp_samples(&self) -> u32 {
    ///     // Use 10ms crossfade based on current sample rate
    ///     (self.sample_rate * 0.01) as u32
    /// }
    /// ```
    fn bypass_ramp_samples(&self) -> u32 {
        64
    }

    // =========================================================================
    // 64-bit Processing Support
    // =========================================================================

    /// Returns true if the plugin supports native 64-bit (double precision) processing.
    ///
    /// Override this to return `true` if your plugin implements `process_f64()` natively.
    /// When false (default), the framework will automatically convert 64-bit host buffers
    /// to 32-bit, call `process()` and convert back.
    ///
    /// # Performance Considerations
    ///
    /// - For most plugins, f32 is sufficient and the default conversion is fine
    /// - Implement native f64 only if your DSP algorithm benefits from double precision
    ///   (e.g., IIR filters with long decay, precision-sensitive synthesis)
    /// - The conversion overhead is minimal (~few microseconds per buffer)
    ///
    /// Default returns `false`.
    fn supports_double_precision(&self) -> bool {
        false
    }

    /// Process an audio buffer at 64-bit (double) precision.
    ///
    /// This is the f64 equivalent of `process()`. Override this method AND
    /// return `true` from `supports_double_precision()` to enable native
    /// 64-bit processing.
    ///
    /// If `supports_double_precision()` returns `false`, this method is never
    /// called - the framework converts to f32 and calls `process()` instead.
    ///
    /// # Default Implementation
    ///
    /// The default implementation converts f64→f32, calls `process()`, then
    /// converts f32→f64. This allows any plugin to work in a 64-bit host
    /// without modification.
    ///
    /// # Example: Native f64 Plugin
    ///
    /// ```ignore
    /// fn supports_double_precision(&self) -> bool {
    ///     true
    /// }
    ///
    /// fn process_f64(
    ///     &mut self,
    ///     buffer: &mut Buffer<f64>,
    ///     aux: &mut AuxiliaryBuffers<f64>,
    ///     context: &ProcessContext,
    /// ) {
    ///     let gain = self.model.gain_linear() as f64;
    ///     for (input, output) in buffer.zip_channels() {
    ///         for (i, o) in input.iter().zip(output.iter_mut()) {
    ///             *o = *i * gain;
    ///         }
    ///     }
    /// }
    /// ```
    fn process_f64(
        &mut self,
        buffer: &mut Buffer<f64>,
        _aux: &mut AuxiliaryBuffers<f64>,
        context: &ProcessContext,
    ) {
        // Default implementation: convert f64 → f32, process, convert back
        //
        // NOTE: This is a fallback implementation that allocates memory.
        // In practice, this method is rarely called because:
        // - Format wrappers (AU, VST3) handle conversion with pre-allocated buffers
        //   (see respective processor implementations)
        //
        // If you're implementing a custom wrapper, ensure you handle
        // f64→f32 conversion with pre-allocated buffers for real-time safety.

        let num_samples = buffer.num_samples();
        let num_input_channels = buffer.num_input_channels();
        let num_output_channels = buffer.num_output_channels();

        // Allocate conversion buffers (VST3 wrapper uses pre-allocated buffers,
        // this is only for the fallback default implementation)
        let input_f32: Vec<Vec<f32>> = (0..num_input_channels)
            .map(|ch| buffer.input(ch).iter().map(|&s| s as f32).collect())
            .collect();
        let mut output_f32: Vec<Vec<f32>> = (0..num_output_channels)
            .map(|_| vec![0.0f32; num_samples])
            .collect();

        // Build f32 buffer slices
        let input_slices: Vec<&[f32]> = input_f32.iter().map(|v| v.as_slice()).collect();
        let output_slices: Vec<&mut [f32]> = output_f32
            .iter_mut()
            .map(|v| v.as_mut_slice())
            .collect();

        let mut buffer_f32 = Buffer::new(input_slices, output_slices, num_samples);

        // For aux buffers, we use empty for now (full aux conversion is complex)
        // The VST3 wrapper handles proper aux conversion
        let mut aux_f32: AuxiliaryBuffers<f32> = AuxiliaryBuffers::empty();

        // Process at f32
        self.process(&mut buffer_f32, &mut aux_f32, context);

        // Convert output back to f64
        for (ch, output_samples) in output_f32.iter().enumerate().take(num_output_channels) {
            let output_ch = buffer.output(ch);
            for (i, sample) in output_samples.iter().enumerate() {
                output_ch[i] = *sample as f64;
            }
        }
    }

    // =========================================================================
    // MIDI Processing
    // =========================================================================

    /// Process MIDI events.
    ///
    /// Called right before every `process()` call, with the MIDI events of
    /// that call. Their `sample_offset` counts from the first sample of that
    /// call, and the events in `output` are timed the same way. A format
    /// wrapper may split a host buffer into several `process()` calls, for
    /// example at automation points, and then calls this before each of
    /// them. Events of a host call without samples (a VST3 host flushes
    /// parameter changes that way) come with the next call, at offset 0.
    /// Plugins can transform events and add them to the output buffer, pass
    /// them through unchanged or consume them entirely.
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
    /// **Note:** Cloning a `SysEx` event allocates due to `Box<SysEx>`. SysEx
    /// events are rare in typical use cases. If strict real-time safety is
    /// required, override this method to handle SysEx specially.
    ///
    /// # Default Implementation
    ///
    /// The default implementation passes all events through unchanged.
    fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
        for event in input {
            output.push(event.clone());
        }
    }

}

// =============================================================================
// Model Trait
// =============================================================================

/// The model of a plugin: what every thread shares, and the plugin as the
/// host sees it.
///
/// This is the primary trait that plugin authors implement. The struct is
/// marked with `#[beamer::model]`, which reads its parameters, `State`
/// fields and event queue, and the `impl Model` adds how the plugin behaves:
/// it creates a [`Processor`] via [`Model::prepare()`] when the audio
/// configuration is known, describes buses and MIDI behavior and answers the
/// web UI.
///
/// The framework creates the model with the plugin instance and keeps it in
/// one [`Arc`] for as long as the instance lives. The host, the UI and the
/// processors share that one value.
///
/// # Lifecycle
///
/// ```text
/// Model::default() -> Model (in an Arc, lives as long as the plugin instance)
///                      |
///                      v  Model::prepare(self: Arc<Self>, setup)
///                      |
///                      v
///                Processor (ready for audio)
///                      |
///                      v  the host releases the audio resources or
///                      |  changes the configuration
///                      v
///                   dropped (the next prepare() creates a new one)
/// ```
///
/// # Example: Simple Gain (no setup)
///
/// ```ignore
/// #[beamer::export]
/// #[beamer::model]
/// pub struct GainModel {
///     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
///     pub gain: FloatParameter,
/// }
///
/// impl Model for GainModel {
///     type Setup = ();
///     type Processor = GainProcessor;
///
///     fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
///         GainProcessor { model: self }
///     }
/// }
///
/// pub struct GainProcessor {
///     model: Arc<GainModel>,
/// }
///
/// impl Processor for GainProcessor {
///     fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
///         let gain = self.model.gain.as_linear() as f32;
///         for (input, output) in buffer.zip_channels() {
///             for (i, o) in input.iter().zip(output.iter_mut()) {
///                 *o = *i * gain;
///             }
///         }
///     }
/// }
/// ```
///
/// # Example: Delay (SampleRate)
///
/// ```ignore
/// impl Model for DelayModel {
///     type Setup = SampleRate;
///     type Processor = DelayProcessor;
///
///     fn prepare(self: Arc<Self>, setup: SampleRate) -> DelayProcessor {
///         let buffer_size = (MAX_DELAY_SECONDS * setup.hz()) as usize;
///         DelayProcessor {
///             model: self,
///             sample_rate: setup.hz(),  // Real value from start!
///             buffer: vec![0.0; buffer_size],   // Correct allocation!
///         }
///     }
/// }
/// ```
///
/// # Thread Safety
///
/// A host asks the model about buses and MIDI from its own threads, also
/// while audio is processed and from more than one thread at the same time.
/// A model is therefore `Sync`, and every method takes `&self` and must be
/// callable from several threads at once. A method that changes something,
/// such as [`on_midi_learn()`](Model::on_midi_learn), keeps what it changes
/// in a value that is safe to share: a `State` field (which is also saved
/// with the project), an atomic or a `Mutex`. No method of this trait is
/// called on the audio thread.
pub trait Model: ModelFields + ParameterStore + Default + 'static {
    /// The setup information this plugin needs to prepare.
    ///
    /// Request exactly what you need:
    /// - `()`: No setup needed (gain, pan)
    /// - [`SampleRate`]: Time-based DSP (delay, filter, envelope) - most common
    /// - `(SampleRate, MaxBufferSize)`: FFT, lookahead
    /// - `(SampleRate, MainOutputChannels)`: Surround, per-channel state
    ///
    /// See [`PluginSetup`] for all available types.
    type Setup: PluginSetup;

    /// The prepared processor type created by [`Model::prepare()`].
    type Processor: Processor;

    /// Create a processor for an audio configuration.
    ///
    /// This is called when audio configuration becomes available (in VST3,
    /// during `setupProcessing()`), and again when it changes: the previous
    /// processor is dropped and a new one is created with the new setup.
    ///
    /// The processor keeps `self`, the model's `Arc`, or only the readers and
    /// values it needs. Take readers before `self` moves into the processor:
    /// `Processor { pan_law: self.pan_law.reader(), model: self }`, in that
    /// order. The framework has set the host's sample rate on the parameter
    /// smoothers before this call.
    ///
    /// This runs on a host thread, never on the audio thread, so it may
    /// allocate.
    ///
    /// # Arguments
    ///
    /// * `setup` - The setup information (sample rate, buffer size, layout)
    ///
    /// # Returns
    ///
    /// A prepared processor ready for audio processing.
    fn prepare(self: Arc<Self>, setup: Self::Setup) -> Self::Processor;

    // =========================================================================
    // Bus Configuration (static, known before prepare)
    // =========================================================================
    //
    // The host and the framework size their buffers from these answers, so
    // they must stay the same for the life of a plugin instance.

    /// Returns the number of audio input buses.
    ///
    /// Default returns 1 (single stereo input). A plugin without audio input,
    /// such as most instruments and generators, overrides this to return 0.
    ///
    /// The bus methods must give the same answers for the life of a plugin
    /// instance: the host and the framework size their buffers from them.
    fn input_bus_count(&self) -> usize {
        1
    }

    /// Returns the number of audio output buses.
    ///
    /// Default returns 1 (single stereo output). An Audio Unit always has an
    /// output bus: a plugin that declares no bus at all gets one in the AU
    /// formats, as wide as `output_bus_info(0)` says or stereo.
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

    /// Returns whether this plugin processes MIDI events.
    ///
    /// Override to return `true` if your plugin needs MIDI input/output.
    /// This is used by the host to determine event bus configuration.
    ///
    /// Default returns `false`.
    fn wants_midi(&self) -> bool {
        false
    }

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
    /// `Some(parameter_id)` if this CC is mapped to a parameter, `None` otherwise.
    ///
    /// # Example
    /// ```ignore
    /// fn midi_cc_to_parameter(&self, _bus: i32, _channel: i16, cc: u8) -> Option<u32> {
    ///     match cc {
    ///         cc::MOD_WHEEL => Some(PARAM_VIBRATO_DEPTH),
    ///         cc::EXPRESSION => Some(PARAM_VOLUME),
    ///         _ => None,
    ///     }
    /// }
    /// ```
    fn midi_cc_to_parameter(&self, bus_index: i32, channel: i16, cc: u8) -> Option<u32> {
        let _ = (bus_index, channel, cc);
        None
    }

    // =========================================================================
    // MIDI CC Configuration (VST3 IMidiMapping hidden parameters)
    // =========================================================================

    /// Returns MIDI CC configuration for automatic host mapping.
    ///
    /// Override to enable MIDI CC/pitch bend/aftertouch reception via IMidiMapping.
    /// The framework will:
    /// 1. Create hidden parameters for each enabled controller
    /// 2. Handle IMidiMapping queries from the DAW
    /// 3. Convert parameter changes to MidiEvents in process_midi()
    /// 4. Provide direct CC value access via ProcessContext::midi_cc()
    ///
    /// This solves the VST3 MIDI input problem where most DAWs don't send
    /// `kLegacyMIDICCOutEvent` for input. Instead, they use the `IMidiMapping`
    /// interface to map MIDI controllers to parameters.
    ///
    /// # Example
    ///
    /// ```ignore
    /// impl Model for MySynth {
    ///     fn midi_cc_config(&self) -> Option<MidiCcConfig> {
    ///         Some(MidiCcConfig::new()
    ///             .with_pitch_bend()
    ///             .with_mod_wheel()
    ///             .with_aftertouch()
    ///             .with_ccs(&[7, 10, 11, 64]))  // Volume, Pan, Expression, Sustain
    ///     }
    /// }
    ///
    /// // Access CC values during processing:
    /// fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
    ///     if let Some(cc) = context.midi_cc() {
    ///         let pitch_bend = cc.pitch_bend();  // -1.0 to 1.0
    ///         let mod_wheel = cc.mod_wheel();    // 0.0 to 1.0
    ///     }
    /// }
    /// ```
    fn midi_cc_config(&self) -> Option<MidiCcConfig> {
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
    /// Keep the learned mapping in a `State` field of the model, so
    /// that it is saved with the project, or in another value that is safe
    /// to share between threads.
    ///
    /// # Arguments
    /// * `bus_index` - MIDI bus index (usually 0)
    /// * `channel` - MIDI channel (0-15)
    /// * `cc` - MIDI CC number that was moved
    ///
    /// # Returns
    /// `true` if the input was handled (learned), `false` otherwise.
    fn on_midi_learn(&self, bus_index: i32, channel: i16, cc: u8) -> bool {
        let _ = (bus_index, channel, cc);
        false
    }

    // =========================================================================
    // MIDI 2.0 Mapping (IMidiMapping2)
    // =========================================================================

    /// Get all MIDI 1.0 CC assignments for bulk query.
    ///
    /// Override to provide mappings for DAW queries. This is more efficient
    /// than individual `midi_cc_to_parameter` queries when there are many mappings.
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
    fn on_midi1_learn(&self, bus_index: i32, channel: u8, cc: u8) -> bool {
        let _ = (bus_index, channel, cc);
        false
    }

    /// Called when MIDI 2.0 controller input is received during learn mode.
    ///
    /// Override to implement MIDI 2.0 controller learning.
    ///
    /// Default returns `false` (not handled).
    fn on_midi2_learn(&self, bus_index: i32, channel: u8, controller: Midi2Controller) -> bool {
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
    fn enable_mpe_input_processing(&self, enabled: bool) -> bool {
        let _ = enabled;
        true
    }

    /// Called when the MPE input device settings change.
    ///
    /// Override to receive MPE zone configuration from wrappers.
    ///
    /// Default does nothing and returns true.
    fn set_mpe_input_device_settings(&self, settings: MpeInputDeviceSettings) -> bool {
        let _ = settings;
        true
    }

    // =========================================================================
    // WebView (custom JS invoke/event handling)
    // =========================================================================

    /// Answer an invoke call from JavaScript.
    ///
    /// Called on the main thread when the web UI calls
    /// `__BEAMER__.invoke("method", args...)`. Return `Ok(value)` to
    /// resolve the JavaScript promise with `value`, `Err(message)` to reject
    /// it. Parameter synchronization is automatic and does not go through
    /// here.
    ///
    /// The default rejects every call as an unknown method. An override
    /// ends its `match` the same way for the methods it does not know.
    fn on_invoke(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        let _ = args;
        Err(format!("unknown method: {method}"))
    }

    /// Handle a custom event from JavaScript.
    ///
    /// Called on the main thread when the web UI calls
    /// `__BEAMER__.emit("name", data)`. The default does nothing.
    fn on_event(&self, name: &str, data: &serde_json::Value) {
        let _ = (name, data);
    }
}

// =============================================================================
// MIDI Mapping Types
// =============================================================================

/// Base assignment info for MIDI controller → parameter mapping.
#[derive(Debug, Clone, Copy)]
pub struct MidiControllerAssignment {
    /// Parameter ID this controller maps to.
    pub parameter_id: u32,
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
    /// Base assignment info (parameter_id, bus, channel).
    pub assignment: MidiControllerAssignment,
    /// CC number (0-127).
    pub controller: u8,
}

impl Midi1Assignment {
    /// Create a new MIDI 1.0 CC assignment.
    pub const fn new(parameter_id: u32, bus_index: i32, channel: u8, controller: u8) -> Self {
        Self {
            assignment: MidiControllerAssignment {
                parameter_id,
                bus_index,
                channel,
            },
            controller,
        }
    }

    /// Create an assignment for the default bus and all channels.
    pub const fn simple(parameter_id: u32, controller: u8) -> Self {
        Self::new(parameter_id, 0, 0, controller)
    }
}

/// MIDI 2.0 controller assignment.
///
/// Maps a MIDI 2.0 Registered/Assignable Controller to a parameter.
#[derive(Debug, Clone, Copy)]
pub struct Midi2Assignment {
    /// Base assignment info (parameter_id, bus, channel).
    pub assignment: MidiControllerAssignment,
    /// MIDI 2.0 controller identifier.
    pub controller: Midi2Controller,
}

impl Midi2Assignment {
    /// Create a new MIDI 2.0 controller assignment.
    pub const fn new(
        parameter_id: u32,
        bus_index: i32,
        channel: u8,
        controller: Midi2Controller,
    ) -> Self {
        Self {
            assignment: MidiControllerAssignment {
                parameter_id,
                bus_index,
                channel,
            },
            controller,
        }
    }

    /// Create an assignment for the default bus and all channels.
    pub const fn simple(parameter_id: u32, controller: Midi2Controller) -> Self {
        Self::new(parameter_id, 0, 0, controller)
    }
}
