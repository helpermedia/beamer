//! Generic VST3 processor wrapper.
//!
//! This module provides [`Vst3Processor`], a generic wrapper that bridges any
//! [`beamer_core::Plugin`] implementation to VST3 COM interfaces.
//!
//! # Architecture
//!
//! Uses the **combined component** pattern where processor and controller are
//! implemented by the same object. This is the modern approach used by most
//! audio plugin frameworks.
//!
//! ```text
//! User Plugin (implements Plugin trait)
//!        ↓
//! Vst3Processor<P> (this wrapper)
//!        ↓
//! VST3 COM interfaces (IComponent, IAudioProcessor, IEditController)
//! ```

use std::cell::UnsafeCell;
use std::ffi::{c_char, c_void};
use std::marker::PhantomData;
use std::slice;

use log::warn;
use vst3::{Class, ComRef, Steinberg::Vst::*, Steinberg::*};

use beamer_core::{
    AudioProcessor, AudioSetup, AuxiliaryBuffers, Buffer, BusInfo as CoreBusInfo, BusLayout,
    BusType as CoreBusType, ChordInfo, FrameRate as CoreFrameRate, FullAudioSetup, HasParams,
    MidiBuffer, MidiCcParams, MidiEvent, MidiEventKind, NoConfig, NoteExpressionInt,
    NoteExpressionText, NoteExpressionValue as CoreNoteExpressionValue, Parameters, Plugin,
    ProcessContext as CoreProcessContext, ProcessorConfig, ScaleInfo, SysEx, Transport, MAX_BUSES,
    MAX_CHANNELS, MAX_CHORD_NAME_SIZE, MAX_EXPRESSION_TEXT_SIZE, MAX_SCALE_NAME_SIZE,
    MAX_SYSEX_SIZE,
};

use crate::factory::ComponentFactory;
use crate::util::{copy_wstring, len_wstring};
use crate::wrapper::PluginConfig;

// VST3 event type constants
const K_NOTE_ON_EVENT: u16 = 0;
const K_NOTE_OFF_EVENT: u16 = 1;
const K_DATA_EVENT: u16 = 2;
const K_POLY_PRESSURE_EVENT: u16 = 3;
const K_NOTE_EXPRESSION_VALUE_EVENT: u16 = 4;
const K_NOTE_EXPRESSION_TEXT_EVENT: u16 = 5;
const K_CHORD_EVENT: u16 = 6;
const K_SCALE_EVENT: u16 = 7;
const K_NOTE_EXPRESSION_INT_VALUE_EVENT: u16 = 8;
const K_LEGACY_MIDI_CC_OUT_EVENT: u16 = 65535;

// LegacyMIDICCOutEvent controlNumber special values
const LEGACY_CC_CHANNEL_PRESSURE: u8 = 128;
const LEGACY_CC_PITCH_BEND: u8 = 129;
const LEGACY_CC_PROGRAM_CHANGE: u8 = 130;

// DataEvent type for SysEx
const DATA_TYPE_MIDI_SYSEX: u32 = 0;

// =============================================================================
// SysEx Output Buffer Pool
// =============================================================================

/// Pool of buffers for SysEx output events.
///
/// VST3's DataEvent requires a pointer to data that must remain valid until
/// the host processes the event. This pool provides stable storage for SysEx
/// data during each process() call.
///
/// The pool is pre-allocated at construction time based on plugin configuration,
/// ensuring no heap allocations occur during audio processing (unless the
/// `sysex-heap-fallback` feature is enabled and the pool overflows).
struct SysExOutputPool {
    /// Pre-allocated buffer slots for SysEx data (Vec of Vecs, but fixed capacity)
    buffers: Vec<Vec<u8>>,
    /// Length of valid data in each slot
    lengths: Vec<usize>,
    /// Maximum number of slots
    max_slots: usize,
    /// Maximum buffer size per slot
    max_buffer_size: usize,
    /// Next available slot index
    next_slot: usize,
    /// Set to true when an allocation fails due to pool exhaustion
    overflowed: bool,
    /// Heap-backed fallback buffer for overflow (only when feature enabled).
    /// Messages stored here are emitted at the start of the next process block.
    #[cfg(feature = "sysex-heap-fallback")]
    fallback: Vec<Vec<u8>>,
}

impl SysExOutputPool {
    /// Create a new pool with the specified capacity.
    ///
    /// Pre-allocates all buffers to avoid heap allocation during process().
    fn with_capacity(slots: usize, buffer_size: usize) -> Self {
        let mut buffers = Vec::with_capacity(slots);
        for _ in 0..slots {
            let buf = vec![0u8; buffer_size];
            buffers.push(buf);
        }
        let lengths = vec![0usize; slots];

        Self {
            buffers,
            lengths,
            max_slots: slots,
            max_buffer_size: buffer_size,
            next_slot: 0,
            overflowed: false,
            #[cfg(feature = "sysex-heap-fallback")]
            fallback: Vec::new(),
        }
    }

    /// Clear the pool for reuse.
    ///
    /// Note: This does NOT clear the fallback buffer, which is drained separately
    /// at the start of the next process block.
    #[inline]
    fn clear(&mut self) {
        self.next_slot = 0;
        self.overflowed = false;
    }

    /// Returns true if any SysEx allocation failed since the last clear.
    #[inline]
    fn has_overflowed(&self) -> bool {
        self.overflowed
    }

    /// Returns the maximum number of slots in this pool.
    #[inline]
    fn capacity(&self) -> usize {
        self.max_slots
    }

    /// Returns true if there are pending fallback messages from a previous overflow.
    #[cfg(feature = "sysex-heap-fallback")]
    #[inline]
    fn has_fallback(&self) -> bool {
        !self.fallback.is_empty()
    }

    /// Take all pending fallback messages, leaving the fallback buffer empty.
    ///
    /// These messages should be emitted at the start of the current process block.
    #[cfg(feature = "sysex-heap-fallback")]
    #[inline]
    fn take_fallback(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.fallback)
    }

    /// Allocate a slot and copy SysEx data into it.
    ///
    /// Returns a pointer to the data and its length, or None if the pool is full.
    /// Sets the overflow flag when the pool is exhausted.
    ///
    /// With `sysex-heap-fallback` feature: overflow messages are stored in a
    /// heap-backed fallback buffer instead of being dropped.
    fn allocate(&mut self, data: &[u8]) -> Option<(*const u8, usize)> {
        if self.next_slot >= self.max_slots {
            self.overflowed = true;

            // With heap fallback enabled, store overflow in fallback buffer
            #[cfg(feature = "sysex-heap-fallback")]
            {
                let copy_len = data.len().min(self.max_buffer_size);
                self.fallback.push(data[..copy_len].to_vec());
            }

            return None;
        }

        let slot = self.next_slot;
        self.next_slot += 1;

        let copy_len = data.len().min(self.max_buffer_size);
        self.buffers[slot][..copy_len].copy_from_slice(&data[..copy_len]);
        self.lengths[slot] = copy_len;

        Some((self.buffers[slot].as_ptr(), copy_len))
    }
}

// =============================================================================
// Transport Extraction
// =============================================================================

/// Helper macro for extracting optional fields based on validity flags.
/// Reduces repetitive `if state & FLAG != 0 { Some(value) } else { None }` patterns.
macro_rules! valid_if {
    ($state:expr, $flag:expr, $value:expr) => {
        if $state & $flag != 0 {
            Some($value)
        } else {
            None
        }
    };
}

/// Extract transport information from VST3 ProcessContext.
///
/// Converts VST3's validity flags to Rust's Option<T> idiom.
/// Returns a default Transport if the context pointer is null.
///
/// # Safety
///
/// The caller must ensure `ctx_ptr` is either null or points to a valid
/// ProcessContext struct for the duration of this call.
unsafe fn extract_transport(ctx_ptr: *const ProcessContext) -> Transport {
    if ctx_ptr.is_null() {
        return Transport::default();
    }

    let ctx = &*ctx_ptr;
    let state = ctx.state;

    // VST3 ProcessContext state flags
    const K_PLAYING: u32 = 1 << 1;
    const K_CYCLE_ACTIVE: u32 = 1 << 2;
    const K_RECORDING: u32 = 1 << 3;
    const K_SYSTEM_TIME_VALID: u32 = 1 << 8;
    const K_PROJECT_TIME_MUSIC_VALID: u32 = 1 << 9;
    const K_TEMPO_VALID: u32 = 1 << 10;
    const K_BAR_POSITION_VALID: u32 = 1 << 11;
    const K_CYCLE_VALID: u32 = 1 << 12;
    const K_TIME_SIG_VALID: u32 = 1 << 13;
    const K_SMPTE_VALID: u32 = 1 << 14;
    const K_CLOCK_VALID: u32 = 1 << 15;
    const K_CONT_TIME_VALID: u32 = 1 << 17;

    Transport {
        // Tempo and time signature
        tempo: valid_if!(state, K_TEMPO_VALID, ctx.tempo),
        time_sig_numerator: valid_if!(state, K_TIME_SIG_VALID, ctx.timeSigNumerator),
        time_sig_denominator: valid_if!(state, K_TIME_SIG_VALID, ctx.timeSigDenominator),

        // Position
        project_time_samples: Some(ctx.projectTimeSamples),
        project_time_beats: valid_if!(state, K_PROJECT_TIME_MUSIC_VALID, ctx.projectTimeMusic),
        bar_position_beats: valid_if!(state, K_BAR_POSITION_VALID, ctx.barPositionMusic),

        // Cycle/loop
        cycle_start_beats: valid_if!(state, K_CYCLE_VALID, ctx.cycleStartMusic),
        cycle_end_beats: valid_if!(state, K_CYCLE_VALID, ctx.cycleEndMusic),

        // Transport state (always valid)
        is_playing: state & K_PLAYING != 0,
        is_recording: state & K_RECORDING != 0,
        is_cycle_active: state & K_CYCLE_ACTIVE != 0,

        // Advanced timing
        system_time_ns: valid_if!(state, K_SYSTEM_TIME_VALID, ctx.systemTime),
        continuous_time_samples: valid_if!(state, K_CONT_TIME_VALID, ctx.continousTimeSamples), // Note: VST3 SDK typo
        samples_to_next_clock: valid_if!(state, K_CLOCK_VALID, ctx.samplesToNextClock),

        // SMPTE - use FrameRate::from_raw() for conversion
        smpte_offset_subframes: valid_if!(state, K_SMPTE_VALID, ctx.smpteOffsetSubframes),
        frame_rate: if state & K_SMPTE_VALID != 0 {
            let is_drop = ctx.frameRate.flags & 1 != 0;
            CoreFrameRate::from_raw(ctx.frameRate.framesPerSecond, is_drop)
        } else {
            None
        },
    }
}

/// Conversion buffers for f64→f32 processing when plugin doesn't support native f64.
///
/// Pre-allocated in `setupProcessing()` to avoid heap allocations on the audio thread.
struct ConversionBuffers {
    /// Input conversion buffers: f64 → f32
    /// Outer Vec: per bus, Inner Vec: per channel
    main_input_f32: Vec<Vec<f32>>,
    /// Output conversion buffers: f32 → f64
    main_output_f32: Vec<Vec<f32>>,
    /// Auxiliary input conversion buffers
    aux_input_f32: Vec<Vec<Vec<f32>>>,
    /// Auxiliary output conversion buffers
    aux_output_f32: Vec<Vec<Vec<f32>>>,
}

impl ConversionBuffers {
    fn new() -> Self {
        Self {
            main_input_f32: Vec::new(),
            main_output_f32: Vec::new(),
            aux_input_f32: Vec::new(),
            aux_output_f32: Vec::new(),
        }
    }

    /// Pre-allocate buffers based on cached bus configuration and max block size.
    ///
    /// This is used during setupProcessing() when we don't have a plugin reference
    /// anymore (it was consumed by prepare()).
    fn allocate_from_config(bus_config: &CachedBusConfig, max_block_size: usize) -> Self {
        // Main bus (bus 0) channels
        let main_in_channels = bus_config.input_bus_info(0).map(|b| b.channel_count as usize).unwrap_or(0);
        let main_out_channels = bus_config.output_bus_info(0).map(|b| b.channel_count as usize).unwrap_or(0);

        let main_input_f32: Vec<Vec<f32>> = (0..main_in_channels)
            .map(|_| vec![0.0f32; max_block_size])
            .collect();

        let main_output_f32: Vec<Vec<f32>> = (0..main_out_channels)
            .map(|_| vec![0.0f32; max_block_size])
            .collect();

        // Auxiliary buses (bus 1+)
        let mut aux_input_f32 = Vec::new();
        for bus_idx in 1..bus_config.input_bus_count {
            if let Some(info) = bus_config.input_bus_info(bus_idx) {
                let channels: Vec<Vec<f32>> = (0..info.channel_count)
                    .map(|_| vec![0.0f32; max_block_size])
                    .collect();
                aux_input_f32.push(channels);
            }
        }

        let mut aux_output_f32 = Vec::new();
        for bus_idx in 1..bus_config.output_bus_count {
            if let Some(info) = bus_config.output_bus_info(bus_idx) {
                let channels: Vec<Vec<f32>> = (0..info.channel_count)
                    .map(|_| vec![0.0f32; max_block_size])
                    .collect();
                aux_output_f32.push(channels);
            }
        }

        Self {
            main_input_f32,
            main_output_f32,
            aux_input_f32,
            aux_output_f32,
        }
    }
}

// =============================================================================
// Bus Limit Validation
// =============================================================================

/// Validate that a cached bus configuration doesn't exceed compile-time limits.
///
/// Returns `Ok(())` if valid, or `Err` with a descriptive message if limits are exceeded.
/// Used during setupProcessing() to validate the cached config.
fn validate_bus_limits_from_config(bus_config: &CachedBusConfig) -> Result<(), String> {
    // Validate bus counts
    if bus_config.input_bus_count > MAX_BUSES {
        return Err(format!(
            "Plugin declares {} input buses, but MAX_BUSES is {}",
            bus_config.input_bus_count, MAX_BUSES
        ));
    }
    if bus_config.output_bus_count > MAX_BUSES {
        return Err(format!(
            "Plugin declares {} output buses, but MAX_BUSES is {}",
            bus_config.output_bus_count, MAX_BUSES
        ));
    }

    // Validate channel counts for each input bus
    for (i, info) in bus_config.input_buses.iter().enumerate() {
        let channels = info.channel_count as usize;
        if channels > MAX_CHANNELS {
            return Err(format!(
                "Input bus {} declares {} channels, but MAX_CHANNELS is {}",
                i, channels, MAX_CHANNELS
            ));
        }
    }

    // Validate channel counts for each output bus
    for (i, info) in bus_config.output_buses.iter().enumerate() {
        let channels = info.channel_count as usize;
        if channels > MAX_CHANNELS {
            return Err(format!(
                "Output bus {} declares {} channels, but MAX_CHANNELS is {}",
                i, channels, MAX_CHANNELS
            ));
        }
    }

    Ok(())
}

/// Validate that a speaker arrangement doesn't exceed MAX_CHANNELS.
///
/// Returns `Ok(())` if valid, or `Err` with a descriptive message if exceeded.
fn validate_speaker_arrangement(arrangement: SpeakerArrangement) -> Result<(), String> {
    let channel_count = arrangement.count_ones() as usize;
    if channel_count > MAX_CHANNELS {
        return Err(format!(
            "Speaker arrangement has {} channels, but MAX_CHANNELS is {}",
            channel_count, MAX_CHANNELS
        ));
    }
    Ok(())
}

// =============================================================================
// ProcessBufferStorage - Pre-allocated channel pointer storage
// =============================================================================

use beamer_core::sample::Sample;

/// Pre-allocated storage for channel pointers during audio processing.
///
/// This struct holds Vec storage with pre-reserved capacity based on the
/// plugin's BusInfo declarations. During process(), we use clear()+push()
/// which is O(1) and never allocates since capacity is pre-reserved.
///
/// Generic over sample type S (f32 or f64) to avoid code duplication.
///
/// # Real-Time Safety
///
/// After `allocate()` is called in `setupProcessing()`:
/// - `clear()` is O(1), sets len=0 without deallocating
/// - `push()` never allocates because capacity is sufficient
/// - No heap operations occur in the audio processing path
struct ProcessBufferStorage<S: Sample> {
    /// Main bus input channel pointers
    main_inputs: Vec<*const S>,
    /// Main bus output channel pointers
    main_outputs: Vec<*mut S>,
    /// Auxiliary bus input channel pointers (per-bus Vec)
    aux_inputs: Vec<Vec<*const S>>,
    /// Auxiliary bus output channel pointers (per-bus Vec)
    aux_outputs: Vec<Vec<*mut S>>,
}

// Safety: ProcessBufferStorage is Send because:
// - Raw pointers point to host-owned audio buffers valid only during process()
// - The struct is only accessed from the audio thread
// - Pointers are never dereferenced outside the process() call scope
unsafe impl<S: Sample> Send for ProcessBufferStorage<S> {}

// Safety: ProcessBufferStorage is Sync because:
// - Raw pointers are only populated and cleared during process()
// - VST3 guarantees process() is called from one thread at a time
// - No shared mutable state is accessed across threads
unsafe impl<S: Sample> Sync for ProcessBufferStorage<S> {}

impl<S: Sample> ProcessBufferStorage<S> {
    /// Create empty storage (no capacity reserved).
    fn new() -> Self {
        Self {
            main_inputs: Vec::new(),
            main_outputs: Vec::new(),
            aux_inputs: Vec::new(),
            aux_outputs: Vec::new(),
        }
    }

    /// Pre-allocate storage based on cached bus configuration.
    ///
    /// Reserves Vec capacity for the exact channel counts declared by the plugin.
    /// This ensures that subsequent push() calls in process() never allocate.
    fn allocate_from_config(bus_config: &CachedBusConfig) -> Self {
        // Get main bus channel counts
        let main_in_channels = bus_config
            .input_bus_info(0)
            .map(|b| b.channel_count as usize)
            .unwrap_or(0);
        let main_out_channels = bus_config
            .output_bus_info(0)
            .map(|b| b.channel_count as usize)
            .unwrap_or(0);

        // Pre-allocate main bus storage
        let main_inputs = Vec::with_capacity(main_in_channels);
        let main_outputs = Vec::with_capacity(main_out_channels);

        // Pre-allocate auxiliary bus storage
        let aux_input_bus_count = bus_config.input_bus_count.saturating_sub(1);
        let aux_output_bus_count = bus_config.output_bus_count.saturating_sub(1);

        let mut aux_inputs = Vec::with_capacity(aux_input_bus_count);
        for bus_idx in 1..bus_config.input_bus_count {
            if let Some(info) = bus_config.input_bus_info(bus_idx) {
                aux_inputs.push(Vec::with_capacity(info.channel_count as usize));
            } else {
                aux_inputs.push(Vec::new());
            }
        }

        let mut aux_outputs = Vec::with_capacity(aux_output_bus_count);
        for bus_idx in 1..bus_config.output_bus_count {
            if let Some(info) = bus_config.output_bus_info(bus_idx) {
                aux_outputs.push(Vec::with_capacity(info.channel_count as usize));
            } else {
                aux_outputs.push(Vec::new());
            }
        }

        Self {
            main_inputs,
            main_outputs,
            aux_inputs,
            aux_outputs,
        }
    }

    /// Clear all pointer storage for reuse.
    ///
    /// O(1) operation - does not deallocate, just sets len to 0.
    #[inline]
    fn clear(&mut self) {
        self.main_inputs.clear();
        self.main_outputs.clear();
        for bus in &mut self.aux_inputs {
            bus.clear();
        }
        for bus in &mut self.aux_outputs {
            bus.clear();
        }
    }
}

// =============================================================================
// Config Building
// =============================================================================

/// Internal trait for building plugin configs from VST3 ProcessSetup.
///
/// Each ProcessorConfig type implements this to construct itself from the
/// VST3 setup information and optional plugin reference.
///
/// This trait is `pub(crate)` to satisfy the bound in `Vst3Processor<P>` where
/// `P::Config: BuildConfig`. External users don't need to implement this -
/// all standard ProcessorConfig types (NoConfig, AudioSetup, FullAudioSetup)
/// have built-in implementations.
pub(crate) trait BuildConfig: ProcessorConfig {
    fn build<P: Plugin>(setup: &ProcessSetup, plugin: &P, bus_layout: &BusLayout) -> Self;
}

impl BuildConfig for NoConfig {
    fn build<P: Plugin>(_setup: &ProcessSetup, _plugin: &P, _bus_layout: &BusLayout) -> Self {
        NoConfig
    }
}

impl BuildConfig for AudioSetup {
    fn build<P: Plugin>(setup: &ProcessSetup, _plugin: &P, _bus_layout: &BusLayout) -> Self {
        AudioSetup {
            sample_rate: setup.sampleRate,
            max_buffer_size: setup.maxSamplesPerBlock as usize,
        }
    }
}

impl BuildConfig for FullAudioSetup {
    fn build<P: Plugin>(setup: &ProcessSetup, _plugin: &P, bus_layout: &BusLayout) -> Self {
        FullAudioSetup {
            sample_rate: setup.sampleRate,
            max_buffer_size: setup.maxSamplesPerBlock as usize,
            layout: bus_layout.clone(),
        }
    }
}

// =============================================================================
// Plugin State Machine
// =============================================================================

// Note: beamer_core::BusInfo is imported as CoreBusInfo in the main imports
// to avoid collision with vst3::Steinberg::Vst::BusInfo used in COM interfaces.
// We use CoreBusInfo throughout this module for the beamer type.

/// Cached bus configuration for the Prepared state.
///
/// VST3 can query bus info at any time, including after setupProcessing().
/// Since the Plugin is consumed during prepare(), we cache the bus config.
#[derive(Clone)]
struct CachedBusConfig {
    input_bus_count: usize,
    output_bus_count: usize,
    input_buses: Vec<CoreBusInfo>,
    output_buses: Vec<CoreBusInfo>,
}

impl CachedBusConfig {
    /// Create from a plugin's bus configuration.
    fn from_plugin<P: Plugin>(plugin: &P) -> Self {
        let input_bus_count = plugin.input_bus_count();
        let output_bus_count = plugin.output_bus_count();

        let input_buses: Vec<CoreBusInfo> = (0..input_bus_count)
            .filter_map(|i| plugin.input_bus_info(i))
            .collect();

        let output_buses: Vec<CoreBusInfo> = (0..output_bus_count)
            .filter_map(|i| plugin.output_bus_info(i))
            .collect();

        Self {
            input_bus_count,
            output_bus_count,
            input_buses,
            output_buses,
        }
    }

    fn input_bus_info(&self, index: usize) -> Option<&CoreBusInfo> {
        self.input_buses.get(index)
    }

    fn output_bus_info(&self, index: usize) -> Option<&CoreBusInfo> {
        self.output_buses.get(index)
    }
}

/// Internal state machine for plugin lifecycle.
///
/// The wrapper manages two states:
/// - **Unprepared**: Plugin exists, but audio config (sample rate) is unknown
/// - **Prepared**: Processor exists with valid audio config, ready for processing
///
/// This enables the type-safe prepare/unprepare cycle where processors cannot
/// be used until they have valid configuration.
enum PluginState<P: Plugin> {
    /// Before setupProcessing() - plugin exists but no audio config yet.
    Unprepared {
        /// The unprepared plugin (holds parameters)
        plugin: P,
        /// State data received before prepare (deferred loading)
        pending_state: Option<Vec<u8>>,
    },
    /// After setupProcessing() - processor is ready for audio.
    Prepared {
        /// The prepared processor (ready for audio)
        processor: P::Processor,
        /// Cached bus configuration (since Plugin is consumed)
        bus_config: CachedBusConfig,
    },
}

// =============================================================================
// Vst3Processor Wrapper
// =============================================================================

/// Generic VST3 processor wrapping any [`Plugin`] implementation.
///
/// This struct implements the VST3 combined component pattern, providing
/// `IComponent`, `IAudioProcessor`, and `IEditController` interfaces that
/// delegate to the wrapped plugin.
///
/// # Two-Phase Lifecycle
///
/// The wrapper manages the plugin's two-phase lifecycle:
///
/// ```text
/// Vst3Processor::new()
///     ↓ creates Plugin::default()
/// PluginState::Unprepared { plugin }
///     ↓ setupProcessing() calls plugin.prepare(config)
/// PluginState::Prepared { processor }
///     ↓ sample rate change: processor.unprepare()
/// PluginState::Unprepared { plugin }
///     ↓ setupProcessing() again
/// PluginState::Prepared { processor }
/// ```
///
/// # Usage
///
/// ```ignore
/// use beamer_vst3::{export_vst3, Vst3Processor, PluginConfig};
///
/// #[derive(Default)]
/// struct MyPlugin { params: MyParams }
/// impl Plugin for MyPlugin { /* ... */ }
///
/// struct MyProcessor { params: MyParams, sample_rate: f64 }
/// impl AudioProcessor for MyProcessor { /* ... */ }
///
/// static CONFIG: PluginConfig = PluginConfig::new("MyPlugin", MY_UID);
/// export_vst3!(CONFIG, Vst3Processor<MyPlugin>);
/// ```
///
/// # Thread Safety
///
/// VST3 guarantees that `process()` is called from a single thread at a time.
/// We use `UnsafeCell` for interior mutability in `process()` since the COM
/// interface only provides `&self`.
pub struct Vst3Processor<P: Plugin> {
    /// The plugin state machine (Unprepared or Prepared)
    state: UnsafeCell<PluginState<P>>,
    /// Plugin configuration reference
    config: &'static PluginConfig,
    /// Current sample rate
    sample_rate: UnsafeCell<f64>,
    /// Maximum block size
    max_block_size: UnsafeCell<usize>,
    /// Current symbolic sample size (kSample32 or kSample64)
    symbolic_sample_size: UnsafeCell<i32>,
    /// MIDI input buffer (reused each process call to avoid stack overflow)
    midi_input: UnsafeCell<MidiBuffer>,
    /// MIDI output buffer (reused each process call)
    midi_output: UnsafeCell<MidiBuffer>,
    /// SysEx output buffer pool (for VST3 DataEvent pointer stability)
    sysex_output_pool: UnsafeCell<SysExOutputPool>,
    /// Conversion buffers for f64→f32 processing
    conversion_buffers: UnsafeCell<ConversionBuffers>,
    /// Pre-allocated channel pointer storage for f32 processing
    buffer_storage_f32: UnsafeCell<ProcessBufferStorage<f32>>,
    /// Pre-allocated channel pointer storage for f64 processing
    buffer_storage_f64: UnsafeCell<ProcessBufferStorage<f64>>,
    /// Marker for the plugin type
    _marker: PhantomData<P>,
}

// Safety: Vst3Processor is Send because:
// - Plugin: Send is required by the Plugin trait
// - AudioProcessor: Send is required by the AudioProcessor trait
// - UnsafeCell contents are only accessed from VST3's guaranteed single-threaded contexts
unsafe impl<P: Plugin> Send for Vst3Processor<P> {}

// Safety: Vst3Processor is Sync because:
// - VST3 guarantees process() is called from one thread at a time
// - Parameter access through Parameters trait requires Sync
unsafe impl<P: Plugin> Sync for Vst3Processor<P> {}

// Allow private_bounds: BuildConfig is intentionally private (sealed pattern).
// External users use standard ProcessorConfig types (NoConfig, AudioSetup, FullAudioSetup)
// which already implement BuildConfig. Custom ProcessorConfig types are internal only.
#[allow(private_bounds)]
impl<P: Plugin + 'static> Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    /// Create a new VST3 processor wrapping the given plugin configuration.
    ///
    /// The wrapper starts in the Unprepared state with a default plugin instance.
    /// The processor will be created when `setupProcessing()` is called.
    pub fn new(config: &'static PluginConfig) -> Self {
        Self {
            state: UnsafeCell::new(PluginState::Unprepared {
                plugin: P::default(),
                pending_state: None,
            }),
            config,
            sample_rate: UnsafeCell::new(44100.0),
            max_block_size: UnsafeCell::new(1024),
            symbolic_sample_size: UnsafeCell::new(SymbolicSampleSizes_::kSample32 as i32),
            midi_input: UnsafeCell::new(MidiBuffer::new()),
            midi_output: UnsafeCell::new(MidiBuffer::new()),
            sysex_output_pool: UnsafeCell::new(SysExOutputPool::with_capacity(
                config.sysex_slots,
                config.sysex_buffer_size,
            )),
            conversion_buffers: UnsafeCell::new(ConversionBuffers::new()),
            buffer_storage_f32: UnsafeCell::new(ProcessBufferStorage::new()),
            buffer_storage_f64: UnsafeCell::new(ProcessBufferStorage::new()),
            _marker: PhantomData,
        }
    }

    /// Get a reference to the prepared processor.
    ///
    /// # Safety
    /// - Must only be called when no mutable reference exists.
    /// - Must only be called when in Prepared state.
    ///
    /// # Panics
    /// Panics if called when in Unprepared state (VST3 host violation).
    #[inline]
    #[allow(dead_code)] // API method for potential future use
    unsafe fn processor(&self) -> &P::Processor {
        match &*self.state.get() {
            PluginState::Prepared { processor, .. } => processor,
            PluginState::Unprepared { .. } => {
                panic!("Attempted to access processor before setupProcessing()")
            }
        }
    }

    /// Get a mutable reference to the prepared processor.
    ///
    /// # Safety
    /// - Must only be called from contexts where VST3 guarantees single-threaded access
    ///   (e.g., process(), setupProcessing()).
    /// - Must only be called when in Prepared state.
    ///
    /// # Panics
    /// Panics if called when in Unprepared state (VST3 host violation).
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn processor_mut(&self) -> &mut P::Processor {
        match &mut *self.state.get() {
            PluginState::Prepared { processor, .. } => processor,
            PluginState::Unprepared { .. } => {
                panic!("Attempted to access processor before setupProcessing()")
            }
        }
    }

    /// Get a reference to the unprepared plugin.
    ///
    /// # Safety
    /// Must only be called when no mutable reference exists.
    ///
    /// # Panics
    /// Panics if called when in Prepared state.
    #[inline]
    #[allow(dead_code)] // API method for potential future use
    unsafe fn unprepared_plugin(&self) -> &P {
        match &*self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin,
            PluginState::Prepared { .. } => {
                panic!("Attempted to access unprepared plugin after setupProcessing()")
            }
        }
    }

    /// Get a mutable reference to the unprepared plugin.
    ///
    /// # Safety
    /// Must only be called from contexts where VST3 guarantees single-threaded access.
    ///
    /// # Panics
    /// Panics if called when in Prepared state.
    #[inline]
    #[allow(dead_code)] // API method for potential future use
    #[allow(clippy::mut_from_ref)]
    unsafe fn unprepared_plugin_mut(&self) -> &mut P {
        match &mut *self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin,
            PluginState::Prepared { .. } => {
                panic!("Attempted to access unprepared plugin after setupProcessing()")
            }
        }
    }

    /// Check if the wrapper is in prepared state.
    #[inline]
    #[allow(dead_code)] // API method for potential future use
    unsafe fn is_prepared(&self) -> bool {
        matches!(&*self.state.get(), PluginState::Prepared { .. })
    }

    /// Try to get a reference to the unprepared plugin.
    ///
    /// Returns Some(&P) when in unprepared state, None when prepared.
    /// Use this for Plugin methods that might be called in either state.
    #[inline]
    unsafe fn try_plugin(&self) -> Option<&P> {
        match &*self.state.get() {
            PluginState::Unprepared { plugin, .. } => Some(plugin),
            PluginState::Prepared { .. } => None,
        }
    }

    /// Try to get a mutable reference to the unprepared plugin.
    ///
    /// Returns Some(&mut P) when in unprepared state, None when prepared.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn try_plugin_mut(&self) -> Option<&mut P> {
        match &mut *self.state.get() {
            PluginState::Unprepared { plugin, .. } => Some(plugin),
            PluginState::Prepared { .. } => None,
        }
    }

    // =========================================================================
    // Bus Info Access (works in both states)
    // =========================================================================

    /// Get input bus count (works in both states).
    #[inline]
    unsafe fn input_bus_count(&self) -> usize {
        match &*self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin.input_bus_count(),
            PluginState::Prepared { bus_config, .. } => bus_config.input_bus_count,
        }
    }

    /// Get output bus count (works in both states).
    #[inline]
    unsafe fn output_bus_count(&self) -> usize {
        match &*self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin.output_bus_count(),
            PluginState::Prepared { bus_config, .. } => bus_config.output_bus_count,
        }
    }

    /// Get input bus info (works in both states).
    /// Returns beamer_core::BusInfo (not vst3::BusInfo).
    #[inline]
    unsafe fn core_input_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
        match &*self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin.input_bus_info(index),
            PluginState::Prepared { bus_config, .. } => bus_config.input_bus_info(index).cloned(),
        }
    }

    /// Get output bus info (works in both states).
    /// Returns beamer_core::BusInfo (not vst3::BusInfo).
    #[inline]
    unsafe fn core_output_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
        match &*self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin.output_bus_info(index),
            PluginState::Prepared { bus_config, .. } => bus_config.output_bus_info(index).cloned(),
        }
    }

    // =========================================================================
    // Parameter Access (works in both states)
    // =========================================================================

    /// Get parameters (works in both states).
    ///
    /// # Safety
    /// Must only be called when no mutable reference exists.
    #[inline]
    unsafe fn params(&self) -> &P::Params {
        match &*self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin.params(),
            PluginState::Prepared { processor, .. } => {
                // SAFETY: Trait bounds guarantee P::Processor::Params == P::Params.
                // Pointer cast through *const _ lets compiler verify type equality.
                &*(processor.params() as *const _)
            }
        }
    }

    /// Get mutable parameters (works in both states).
    ///
    /// # Safety
    /// Must only be called from contexts where VST3 guarantees single-threaded access.
    #[inline]
    #[allow(dead_code)] // API method for potential future use
    #[allow(clippy::mut_from_ref)]
    unsafe fn params_mut(&self) -> &mut P::Params {
        match &mut *self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin.params_mut(),
            PluginState::Prepared { processor, .. } => {
                // SAFETY: Trait bounds guarantee P::Processor::Params == P::Params.
                // Pointer cast through *mut _ lets compiler verify type equality.
                &mut *(processor.params_mut() as *mut _)
            }
        }
    }

    // =========================================================================
    // AudioProcessor Method Access (works in both states)
    // =========================================================================

    /// Check if plugin wants MIDI (works in both states).
    ///
    /// Queries both Plugin (unprepared) and AudioProcessor (prepared) for MIDI support.
    #[inline]
    unsafe fn wants_midi(&self) -> bool {
        match &*self.state.get() {
            PluginState::Unprepared { plugin, .. } => plugin.wants_midi(),
            PluginState::Prepared { processor, .. } => processor.wants_midi(),
        }
    }

    /// Get latency samples (works in both states).
    ///
    /// Returns 0 when unprepared (conservative default), processor's value when prepared.
    #[inline]
    unsafe fn latency_samples(&self) -> u32 {
        match &*self.state.get() {
            PluginState::Unprepared { .. } => 0,
            PluginState::Prepared { processor, .. } => processor.latency_samples(),
        }
    }

    /// Get tail samples (works in both states).
    ///
    /// Returns 0 when unprepared (conservative default), processor's value when prepared.
    #[inline]
    #[allow(dead_code)] // API method for potential future use
    unsafe fn tail_samples(&self) -> u32 {
        match &*self.state.get() {
            PluginState::Unprepared { .. } => 0,
            PluginState::Prepared { processor, .. } => processor.tail_samples(),
        }
    }

    /// Check if processor supports double precision (works in both states).
    ///
    /// Returns false when unprepared (conservative default), processor's value when prepared.
    #[inline]
    #[allow(dead_code)] // API method for potential future use
    unsafe fn supports_double_precision(&self) -> bool {
        match &*self.state.get() {
            PluginState::Unprepared { .. } => false,
            PluginState::Prepared { processor, .. } => processor.supports_double_precision(),
        }
    }

    // =========================================================================
    // Audio Processing Helpers
    // =========================================================================
    //
    // NOTE: process_audio_f32() and process_audio_f64_native() have similar
    // structure but cannot be deduplicated because:
    //
    // 1. VST3's ProcessData uses a C union: channelBuffers32 and channelBuffers64
    //    are separate pointer fields, not generic over sample type
    //
    // 2. Rust's type system can't easily abstract over C FFI union field access
    //    (ProcessData.__field0.channelBuffers32 vs .channelBuffers64)
    //
    // 3. A macro or trait-based abstraction would add complexity for just two
    //    concrete implementations; explicit code is clearer for maintainability
    //
    // If adding a third sample type (e.g., i32 for fixed-point), consider
    // refactoring to a macro-based approach.
    // =========================================================================

    /// Process audio at 32-bit (f32) precision.
    ///
    /// This is the standard processing path used when the host uses kSample32.
    /// Uses pre-allocated ProcessBufferStorage - no heap allocations.
    #[inline]
    unsafe fn process_audio_f32(
        &self,
        process_data: &ProcessData,
        num_samples: usize,
        processor: &mut P::Processor,
        context: &CoreProcessContext,
    ) {
        // Get pre-allocated storage and clear for reuse (O(1), no deallocation)
        let storage = &mut *self.buffer_storage_f32.get();
        storage.clear();

        // Collect main input channel pointers (bounded by pre-allocated capacity)
        if process_data.numInputs > 0 && !process_data.inputs.is_null() {
            let bus = &*process_data.inputs;
            let num_channels = bus.numChannels as usize;
            let max_channels = storage.main_inputs.capacity();
            if num_channels > 0 && !bus.__field0.channelBuffers32.is_null() {
                let channel_ptrs =
                    slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels);
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    if !ptr.is_null() {
                        storage.main_inputs.push(ptr);
                    }
                }
            }
        }

        // Collect main output channel pointers (bounded by pre-allocated capacity)
        if process_data.numOutputs > 0 && !process_data.outputs.is_null() {
            let bus = &*process_data.outputs;
            let num_channels = bus.numChannels as usize;
            let max_channels = storage.main_outputs.capacity();
            if num_channels > 0 && !bus.__field0.channelBuffers32.is_null() {
                let channel_ptrs =
                    slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels);
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    if !ptr.is_null() {
                        storage.main_outputs.push(ptr);
                    }
                }
            }
        }

        // Collect auxiliary input channel pointers (bounded by pre-allocated capacity)
        if process_data.numInputs > 1 && !process_data.inputs.is_null() {
            let input_buses =
                slice::from_raw_parts(process_data.inputs, process_data.numInputs as usize);
            for (aux_idx, bus) in input_buses[1..].iter().enumerate() {
                if aux_idx < storage.aux_inputs.len() {
                    let num_channels = bus.numChannels as usize;
                    let max_channels = storage.aux_inputs[aux_idx].capacity();
                    if num_channels > 0 && !bus.__field0.channelBuffers32.is_null() {
                        let channel_ptrs =
                            slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels);
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            if !ptr.is_null() {
                                storage.aux_inputs[aux_idx].push(ptr);
                            }
                        }
                    }
                }
            }
        }

        // Collect auxiliary output channel pointers (bounded by pre-allocated capacity)
        if process_data.numOutputs > 1 && !process_data.outputs.is_null() {
            let output_buses =
                slice::from_raw_parts(process_data.outputs, process_data.numOutputs as usize);
            for (aux_idx, bus) in output_buses[1..].iter().enumerate() {
                if aux_idx < storage.aux_outputs.len() {
                    let num_channels = bus.numChannels as usize;
                    let max_channels = storage.aux_outputs[aux_idx].capacity();
                    if num_channels > 0 && !bus.__field0.channelBuffers32.is_null() {
                        let channel_ptrs =
                            slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels);
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            if !ptr.is_null() {
                                storage.aux_outputs[aux_idx].push(ptr);
                            }
                        }
                    }
                }
            }
        }

        // Create slices from pointers (safe: ProcessData lifetime covers this scope)
        let main_in_iter = storage
            .main_inputs
            .iter()
            .map(|&ptr| slice::from_raw_parts(ptr, num_samples));
        let main_out_iter = storage
            .main_outputs
            .iter()
            .map(|&ptr| slice::from_raw_parts_mut(ptr, num_samples));

        let aux_in_iter = storage.aux_inputs.iter().map(|bus| {
            bus.iter()
                .map(|&ptr| slice::from_raw_parts(ptr, num_samples))
        });
        let aux_out_iter = storage.aux_outputs.iter().map(|bus| {
            bus.iter()
                .map(|&ptr| slice::from_raw_parts_mut(ptr, num_samples))
        });

        // Construct buffers and process
        let mut buffer = Buffer::new(main_in_iter, main_out_iter, num_samples);
        let mut aux = AuxiliaryBuffers::new(aux_in_iter, aux_out_iter, num_samples);

        processor.process(&mut buffer, &mut aux, context);
    }

    /// Process audio at 64-bit (f64) precision with native plugin support.
    ///
    /// Used when host uses kSample64 and processor.supports_double_precision() is true.
    /// Uses pre-allocated ProcessBufferStorage - no heap allocations.
    #[inline]
    unsafe fn process_audio_f64_native(
        &self,
        process_data: &ProcessData,
        num_samples: usize,
        processor: &mut P::Processor,
        context: &CoreProcessContext,
    ) {
        // Get pre-allocated storage and clear for reuse (O(1), no deallocation)
        let storage = &mut *self.buffer_storage_f64.get();
        storage.clear();

        // Collect main input channel pointers (bounded by pre-allocated capacity)
        if process_data.numInputs > 0 && !process_data.inputs.is_null() {
            let bus = &*process_data.inputs;
            let num_channels = bus.numChannels as usize;
            let max_channels = storage.main_inputs.capacity();
            if num_channels > 0 && !bus.__field0.channelBuffers64.is_null() {
                let channel_ptrs =
                    slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels);
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    if !ptr.is_null() {
                        storage.main_inputs.push(ptr);
                    }
                }
            }
        }

        // Collect main output channel pointers (bounded by pre-allocated capacity)
        if process_data.numOutputs > 0 && !process_data.outputs.is_null() {
            let bus = &*process_data.outputs;
            let num_channels = bus.numChannels as usize;
            let max_channels = storage.main_outputs.capacity();
            if num_channels > 0 && !bus.__field0.channelBuffers64.is_null() {
                let channel_ptrs =
                    slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels);
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    if !ptr.is_null() {
                        storage.main_outputs.push(ptr);
                    }
                }
            }
        }

        // Collect auxiliary input channel pointers (bounded by pre-allocated capacity)
        if process_data.numInputs > 1 && !process_data.inputs.is_null() {
            let input_buses =
                slice::from_raw_parts(process_data.inputs, process_data.numInputs as usize);
            for (aux_idx, bus) in input_buses[1..].iter().enumerate() {
                if aux_idx < storage.aux_inputs.len() {
                    let num_channels = bus.numChannels as usize;
                    let max_channels = storage.aux_inputs[aux_idx].capacity();
                    if num_channels > 0 && !bus.__field0.channelBuffers64.is_null() {
                        let channel_ptrs =
                            slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels);
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            if !ptr.is_null() {
                                storage.aux_inputs[aux_idx].push(ptr);
                            }
                        }
                    }
                }
            }
        }

        // Collect auxiliary output channel pointers (bounded by pre-allocated capacity)
        if process_data.numOutputs > 1 && !process_data.outputs.is_null() {
            let output_buses =
                slice::from_raw_parts(process_data.outputs, process_data.numOutputs as usize);
            for (aux_idx, bus) in output_buses[1..].iter().enumerate() {
                if aux_idx < storage.aux_outputs.len() {
                    let num_channels = bus.numChannels as usize;
                    let max_channels = storage.aux_outputs[aux_idx].capacity();
                    if num_channels > 0 && !bus.__field0.channelBuffers64.is_null() {
                        let channel_ptrs =
                            slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels);
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            if !ptr.is_null() {
                                storage.aux_outputs[aux_idx].push(ptr);
                            }
                        }
                    }
                }
            }
        }

        // Create slices from pointers (safe: ProcessData lifetime covers this scope)
        let main_in_iter = storage
            .main_inputs
            .iter()
            .map(|&ptr| slice::from_raw_parts(ptr, num_samples));
        let main_out_iter = storage
            .main_outputs
            .iter()
            .map(|&ptr| slice::from_raw_parts_mut(ptr, num_samples));

        let aux_in_iter = storage.aux_inputs.iter().map(|bus| {
            bus.iter()
                .map(|&ptr| slice::from_raw_parts(ptr, num_samples))
        });
        let aux_out_iter = storage.aux_outputs.iter().map(|bus| {
            bus.iter()
                .map(|&ptr| slice::from_raw_parts_mut(ptr, num_samples))
        });

        // Construct buffers and process
        let mut buffer: Buffer<f64> = Buffer::new(main_in_iter, main_out_iter, num_samples);
        let mut aux: AuxiliaryBuffers<f64> =
            AuxiliaryBuffers::new(aux_in_iter, aux_out_iter, num_samples);

        processor.process_f64(&mut buffer, &mut aux, context);
    }

    /// Process audio at 64-bit (f64) with conversion to/from f32.
    ///
    /// Used when host uses kSample64 but processor.supports_double_precision() is false.
    /// Converts f64→f32, calls process(), converts f32→f64.
    #[inline]
    unsafe fn process_audio_f64_converted(
        &self,
        process_data: &ProcessData,
        num_samples: usize,
        processor: &mut P::Processor,
        context: &CoreProcessContext,
    ) {
        let conv = &mut *self.conversion_buffers.get();

        // Convert main input f64 → f32
        if process_data.numInputs > 0 && !process_data.inputs.is_null() {
            let input_buses = slice::from_raw_parts(process_data.inputs, 1);
            let bus = &input_buses[0];
            let num_channels = (bus.numChannels as usize).min(conv.main_input_f32.len());
            if num_channels > 0 && !bus.__field0.channelBuffers64.is_null() {
                let channel_ptrs = slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels);
                for (ch, &ptr) in channel_ptrs.iter().enumerate() {
                    if !ptr.is_null() && ch < conv.main_input_f32.len() {
                        let src = slice::from_raw_parts(ptr, num_samples);
                        for (i, &s) in src.iter().enumerate() {
                            conv.main_input_f32[ch][i] = s as f32;
                        }
                    }
                }
            }
        }

        // Convert aux input f64 → f32
        for (bus_idx, aux_bus) in conv.aux_input_f32.iter_mut().enumerate() {
            let vst_bus_idx = bus_idx + 1; // aux buses start at index 1
            if process_data.numInputs as usize > vst_bus_idx && !process_data.inputs.is_null() {
                let input_buses = slice::from_raw_parts(
                    process_data.inputs,
                    process_data.numInputs as usize,
                );
                let bus = &input_buses[vst_bus_idx];
                let num_channels = (bus.numChannels as usize).min(aux_bus.len());
                if num_channels > 0 && !bus.__field0.channelBuffers64.is_null() {
                    let channel_ptrs = slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels);
                    for (ch, &ptr) in channel_ptrs.iter().enumerate() {
                        if !ptr.is_null() && ch < aux_bus.len() {
                            let src = slice::from_raw_parts(ptr, num_samples);
                            for (i, &s) in src.iter().enumerate() {
                                aux_bus[ch][i] = s as f32;
                            }
                        }
                    }
                }
            }
        }

        // Build f32 buffer slices using iterators (no allocation)
        let main_input_iter = conv.main_input_f32
            .iter()
            .map(|v| &v[..num_samples]);
        let main_output_iter = conv.main_output_f32
            .iter_mut()
            .map(|v| &mut v[..num_samples]);

        let aux_input_iter = conv.aux_input_f32
            .iter()
            .map(|bus| bus.iter().map(|v| &v[..num_samples]));
        let aux_output_iter = conv.aux_output_f32
            .iter_mut()
            .map(|bus| bus.iter_mut().map(|v| &mut v[..num_samples]));

        // Construct f32 buffers and process
        let mut buffer = Buffer::new(main_input_iter, main_output_iter, num_samples);
        let mut aux = AuxiliaryBuffers::new(aux_input_iter, aux_output_iter, num_samples);

        processor.process(&mut buffer, &mut aux, context);

        // Convert main output f32 → f64
        if process_data.numOutputs > 0 && !process_data.outputs.is_null() {
            let output_buses = slice::from_raw_parts(process_data.outputs, 1);
            let bus = &output_buses[0];
            let num_channels = (bus.numChannels as usize).min(conv.main_output_f32.len());
            if num_channels > 0 && !bus.__field0.channelBuffers64.is_null() {
                let channel_ptrs = slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels);
                for (ch, &ptr) in channel_ptrs.iter().enumerate() {
                    if !ptr.is_null() && ch < conv.main_output_f32.len() {
                        let dst = slice::from_raw_parts_mut(ptr, num_samples);
                        for (i, sample) in conv.main_output_f32[ch][..num_samples].iter().enumerate() {
                            dst[i] = *sample as f64;
                        }
                    }
                }
            }
        }

        // Convert aux output f32 → f64
        for (bus_idx, aux_bus) in conv.aux_output_f32.iter().enumerate() {
            let vst_bus_idx = bus_idx + 1;
            if process_data.numOutputs as usize > vst_bus_idx && !process_data.outputs.is_null() {
                let output_buses = slice::from_raw_parts(
                    process_data.outputs,
                    process_data.numOutputs as usize,
                );
                let bus = &output_buses[vst_bus_idx];
                let num_channels = (bus.numChannels as usize).min(aux_bus.len());
                if num_channels > 0 && !bus.__field0.channelBuffers64.is_null() {
                    let channel_ptrs = slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels);
                    for (ch, &ptr) in channel_ptrs.iter().enumerate() {
                        if !ptr.is_null() && ch < aux_bus.len() {
                            let dst = slice::from_raw_parts_mut(ptr, num_samples);
                            for (i, sample) in aux_bus[ch][..num_samples].iter().enumerate() {
                                dst[i] = *sample as f64;
                            }
                        }
                    }
                }
            }
        }
    }
}

impl<P: Plugin + 'static> ComponentFactory for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    fn create(config: &'static PluginConfig) -> Self {
        Self::new(config)
    }
}

impl<P: Plugin + 'static> Class for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    type Interfaces = (
        IComponent,
        IAudioProcessor,
        IProcessContextRequirements,
        IEditController,
        IUnitInfo,
        IMidiMapping,
        IMidiLearn,
        IMidiMapping2,
        IMidiLearn2,
        INoteExpressionController,
        IKeyswitchController,
        INoteExpressionPhysicalUIMapping,
        IVst3WrapperMPESupport,
    );
}

// =============================================================================
// IPluginBase implementation
// =============================================================================

impl<P: Plugin + 'static> IPluginBaseTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        kResultOk
    }
}

// =============================================================================
// IComponent implementation
// =============================================================================

impl<P: Plugin + 'static> IComponentTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        if class_id.is_null() {
            return kInvalidArgument;
        }

        // For combined component, return the controller UID if set, otherwise kNotImplemented
        if let Some(controller) = self.config.controller_uid {
            *class_id = controller;
            kResultOk
        } else {
            kNotImplemented
        }
    }

    unsafe fn setIoMode(&self, _mode: IoMode) -> tresult {
        kResultOk
    }

    unsafe fn getBusCount(&self, media_type: MediaType, dir: BusDirection) -> i32 {
        match media_type as MediaTypes {
            MediaTypes_::kAudio => match dir as BusDirections {
                BusDirections_::kInput => self.input_bus_count() as i32,
                BusDirections_::kOutput => self.output_bus_count() as i32,
                _ => 0,
            },
            MediaTypes_::kEvent => {
                // Return 1 event bus in each direction if plugin wants MIDI
                if self.wants_midi() {
                    1
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    unsafe fn getBusInfo(
        &self,
        media_type: MediaType,
        dir: BusDirection,
        index: i32,
        bus: *mut BusInfo,
    ) -> tresult {
        if bus.is_null() {
            return kInvalidArgument;
        }

        match media_type as MediaTypes {
            MediaTypes_::kAudio => {
                let info = match dir as BusDirections {
                    BusDirections_::kInput => self.core_input_bus_info(index as usize),
                    BusDirections_::kOutput => self.core_output_bus_info(index as usize),
                    _ => None,
                };

                if let Some(info) = info {
                    let bus = &mut *bus;
                    bus.mediaType = MediaTypes_::kAudio as MediaType;
                    bus.direction = dir;
                    bus.channelCount = info.channel_count as i32;
                    copy_wstring(info.name, &mut bus.name);
                    bus.busType = match info.bus_type {
                        CoreBusType::Main => BusTypes_::kMain,
                        CoreBusType::Aux => BusTypes_::kAux,
                    } as BusType;
                    bus.flags = if info.is_default_active {
                        BusInfo_::BusFlags_::kDefaultActive
                    } else {
                        0
                    };
                    kResultOk
                } else {
                    kInvalidArgument
                }
            }
            MediaTypes_::kEvent => {
                // Only index 0 for event bus, and only if plugin wants MIDI
                if index != 0 || !self.wants_midi() {
                    return kInvalidArgument;
                }

                let bus = &mut *bus;
                bus.mediaType = MediaTypes_::kEvent as MediaType;
                bus.direction = dir;
                bus.channelCount = 1; // Single event channel
                let name = match dir as BusDirections {
                    BusDirections_::kInput => "MIDI In",
                    BusDirections_::kOutput => "MIDI Out",
                    _ => "MIDI",
                };
                copy_wstring(name, &mut bus.name);
                bus.busType = BusTypes_::kMain as BusType;
                bus.flags = BusInfo_::BusFlags_::kDefaultActive;
                kResultOk
            }
            _ => kInvalidArgument,
        }
    }

    unsafe fn getRoutingInfo(
        &self,
        _in_info: *mut RoutingInfo,
        _out_info: *mut RoutingInfo,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn activateBus(
        &self,
        _media_type: MediaType,
        _dir: BusDirection,
        _index: i32,
        _state: TBool,
    ) -> tresult {
        kResultOk
    }

    unsafe fn setActive(&self, state: TBool) -> tresult {
        // set_active is only meaningful when prepared (processor exists)
        if let PluginState::Prepared { processor, .. } = &mut *self.state.get() {
            processor.set_active(state != 0);
        }
        // When unprepared, silently succeed (host may call this before setupProcessing)
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        if state.is_null() {
            return kInvalidArgument;
        }

        let stream = match ComRef::from_raw(state) {
            Some(s) => s,
            None => return kInvalidArgument,
        };

        // Read all bytes from stream
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let mut bytes_read: i32 = 0;
            let result = stream.read(
                chunk.as_mut_ptr() as *mut c_void,
                chunk.len() as i32,
                &mut bytes_read,
            );

            if result != kResultOk || bytes_read <= 0 {
                break;
            }

            buffer.extend_from_slice(&chunk[..bytes_read as usize]);
        }

        if buffer.is_empty() {
            return kResultOk;
        }

        // Load state based on current state
        match &mut *self.state.get() {
            PluginState::Unprepared { pending_state, .. } => {
                // Store for deferred loading when prepare() is called
                *pending_state = Some(buffer);
                kResultOk
            }
            PluginState::Prepared { processor, .. } => {
                match processor.load_state(&buffer) {
                    Ok(()) => {
                        // Apply current sample rate and reset smoothers
                        use beamer_core::param_types::Params;
                        let sample_rate = *self.sample_rate.get();
                        if sample_rate > 0.0 {
                            processor.params_mut().set_sample_rate(sample_rate);
                        }
                        processor.params_mut().reset_smoothing();
                        kResultOk
                    }
                    Err(_) => kResultFalse,
                }
            }
        }
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        if state.is_null() {
            return kInvalidArgument;
        }

        // Get state from processor (only available when prepared)
        let data: Vec<u8> = match &*self.state.get() {
            PluginState::Unprepared { .. } => {
                // When unprepared, we can't save processor state
                // Return empty success (some hosts call this before prepare)
                return kResultOk;
            }
            PluginState::Prepared { processor, .. } => {
                match processor.save_state() {
                    Ok(d) => d,
                    Err(_) => return kResultFalse,
                }
            }
        };

        if data.is_empty() {
            return kResultOk;
        }

        // Write to IBStream
        let stream = match ComRef::from_raw(state) {
            Some(s) => s,
            None => return kInvalidArgument,
        };
        let mut bytes_written: i32 = 0;
        let result = stream.write(
            data.as_ptr() as *mut c_void,
            data.len() as i32,
            &mut bytes_written,
        );

        if result == kResultOk && bytes_written == data.len() as i32 {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

// =============================================================================
// IAudioProcessor implementation
// =============================================================================

impl<P: Plugin + 'static> IAudioProcessorTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: i32,
        outputs: *mut SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        // Early rejection: negative counts or bus count exceeds compile-time limits
        if num_ins < 0
            || num_outs < 0
            || num_ins as usize > MAX_BUSES
            || num_outs as usize > MAX_BUSES
        {
            return kResultFalse;
        }

        // Early rejection: null pointers with non-zero counts
        if (num_ins > 0 && inputs.is_null()) || (num_outs > 0 && outputs.is_null()) {
            return kInvalidArgument;
        }

        // Check if the requested arrangement matches our bus configuration
        if num_ins as usize != self.input_bus_count()
            || num_outs as usize != self.output_bus_count()
        {
            return kResultFalse;
        }

        // Validate each input bus
        for i in 0..num_ins as usize {
            // Early rejection: channel count exceeds compile-time limits
            let requested = *inputs.add(i);
            if validate_speaker_arrangement(requested).is_err() {
                return kResultFalse;
            }

            if let Some(info) = self.core_input_bus_info(i) {
                let expected = channel_count_to_speaker_arrangement(info.channel_count);
                if requested != expected {
                    return kResultFalse;
                }
            }
        }

        // Validate each output bus
        for i in 0..num_outs as usize {
            // Early rejection: channel count exceeds compile-time limits
            let requested = *outputs.add(i);
            if validate_speaker_arrangement(requested).is_err() {
                return kResultFalse;
            }

            if let Some(info) = self.core_output_bus_info(i) {
                let expected = channel_count_to_speaker_arrangement(info.channel_count);
                if requested != expected {
                    return kResultFalse;
                }
            }
        }

        kResultTrue
    }

    unsafe fn getBusArrangement(
        &self,
        dir: BusDirection,
        index: i32,
        arr: *mut SpeakerArrangement,
    ) -> tresult {
        if arr.is_null() {
            return kInvalidArgument;
        }

        let info = match dir as BusDirections {
            BusDirections_::kInput => self.core_input_bus_info(index as usize),
            BusDirections_::kOutput => self.core_output_bus_info(index as usize),
            _ => None,
        };

        if let Some(info) = info {
            *arr = channel_count_to_speaker_arrangement(info.channel_count);
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn canProcessSampleSize(&self, symbolic_sample_size: i32) -> tresult {
        match symbolic_sample_size as SymbolicSampleSizes {
            SymbolicSampleSizes_::kSample32 => kResultOk,
            SymbolicSampleSizes_::kSample64 => kResultOk, // Support 64-bit via native or conversion
            _ => kNotImplemented,
        }
    }

    unsafe fn getLatencySamples(&self) -> u32 {
        self.latency_samples()
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        if setup.is_null() {
            return kInvalidArgument;
        }

        let setup = &*setup;

        // Store setup parameters
        *self.sample_rate.get() = setup.sampleRate;
        *self.max_block_size.get() = setup.maxSamplesPerBlock as usize;
        *self.symbolic_sample_size.get() = setup.symbolicSampleSize;

        // Handle state transition
        let state = &mut *self.state.get();
        match state {
            PluginState::Unprepared { plugin, pending_state } => {
                // Cache bus config before consuming the plugin
                let bus_config = CachedBusConfig::from_plugin(plugin);
                let bus_layout = BusLayout::from_plugin(plugin);

                // Validate plugin's bus configuration against compile-time limits
                if let Err(msg) = validate_bus_limits_from_config(&bus_config) {
                    log::error!("Plugin bus configuration exceeds limits: {}", msg);
                    return kResultFalse;
                }

                // Build the processor config
                let config = P::Config::build(setup, plugin, &bus_layout);

                // Take ownership of the plugin and any pending state
                let plugin = std::mem::take(plugin);
                let pending = pending_state.take();

                // Prepare the processor
                let mut processor = plugin.prepare(config);

                // Apply any pending state that was set before preparation
                if let Some(data) = pending {
                    let _ = processor.load_state(&data);
                    // Update params sample rate after loading
                    use beamer_core::Params;
                    processor.params_mut().set_sample_rate(setup.sampleRate);
                }

                // Pre-allocate buffer storage based on bus config
                *self.buffer_storage_f32.get() =
                    ProcessBufferStorage::allocate_from_config(&bus_config);
                *self.buffer_storage_f64.get() =
                    ProcessBufferStorage::allocate_from_config(&bus_config);

                // Pre-allocate conversion buffers for f64→f32 processing
                if setup.symbolicSampleSize == SymbolicSampleSizes_::kSample64 as i32
                    && !processor.supports_double_precision()
                {
                    *self.conversion_buffers.get() =
                        ConversionBuffers::allocate_from_config(&bus_config, setup.maxSamplesPerBlock as usize);
                }

                // Update state to Prepared
                *state = PluginState::Prepared {
                    processor,
                    bus_config,
                };
            }
            PluginState::Prepared { processor, bus_config } => {
                // Already prepared - check if sample rate changed
                let current_sample_rate = *self.sample_rate.get();
                if (current_sample_rate - setup.sampleRate).abs() > 0.001 {
                    // Sample rate changed - unprepare and re-prepare
                    let bus_layout = BusLayout {
                        main_input_channels: bus_config
                            .input_bus_info(0)
                            .map(|b| b.channel_count)
                            .unwrap_or(2),
                        main_output_channels: bus_config
                            .output_bus_info(0)
                            .map(|b| b.channel_count)
                            .unwrap_or(2),
                        aux_input_count: bus_config.input_bus_count.saturating_sub(1),
                        aux_output_count: bus_config.output_bus_count.saturating_sub(1),
                    };

                    // Take ownership of the processor
                    let old_processor = std::mem::replace(
                        processor,
                        // This placeholder will be overwritten
                        unsafe { std::mem::zeroed() },
                    );

                    // Unprepare to get the plugin back
                    let plugin = old_processor.unprepare();

                    // Build new config and re-prepare
                    let config = P::Config::build(setup, &plugin, &bus_layout);
                    let new_processor = plugin.prepare(config);

                    // Pre-allocate conversion buffers if needed
                    if setup.symbolicSampleSize == SymbolicSampleSizes_::kSample64 as i32
                        && !new_processor.supports_double_precision()
                    {
                        *self.conversion_buffers.get() =
                            ConversionBuffers::allocate_from_config(bus_config, setup.maxSamplesPerBlock as usize);
                    }

                    *processor = new_processor;
                }
                // If sample rate hasn't changed, nothing to do
            }
        }

        kResultOk
    }

    unsafe fn setProcessing(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        if data.is_null() {
            return kInvalidArgument;
        }

        let process_data = &*data;
        let num_samples = process_data.numSamples as usize;

        if num_samples == 0 {
            return kResultOk;
        }

        // 1. Handle incoming parameter changes from host
        if let Some(param_changes) = ComRef::from_raw(process_data.inputParameterChanges) {
            let params = self.params();
            let param_count = param_changes.getParameterCount();

            for i in 0..param_count {
                if let Some(queue) = ComRef::from_raw(param_changes.getParameterData(i)) {
                    let param_id = queue.getParameterId();
                    let point_count = queue.getPointCount();

                    if point_count > 0 {
                        let mut sample_offset = 0;
                        let mut value = 0.0;
                        // Get the last value in the queue (simplest approach)
                        if queue.getPoint(point_count - 1, &mut sample_offset, &mut value)
                            == kResultTrue
                        {
                            params.set_normalized(param_id, value);
                        }
                    }
                }
            }
        }

        // 2. Handle MIDI events (reuse pre-allocated buffer to avoid stack overflow)
        let midi_input = &mut *self.midi_input.get();
        midi_input.clear();

        if let Some(event_list) = ComRef::from_raw(process_data.inputEvents) {
            let event_count = event_list.getEventCount();

            for i in 0..event_count {
                let mut event: Event = std::mem::zeroed();
                if event_list.getEvent(i, &mut event) == kResultOk {
                    if let Some(midi_event) = convert_vst3_to_midi(&event) {
                        midi_input.push(midi_event);
                    }
                }
            }
        }

        // 2.5. Convert MIDI CC parameter changes to MIDI events
        // This handles the VST3 IMidiMapping flow where DAWs send CC/pitch bend
        // as parameter changes instead of raw MIDI events.
        // Gets MIDI CC config directly from the prepared processor.
        if let Some(param_changes) = ComRef::from_raw(process_data.inputParameterChanges) {
            if let Some(cc_params) = self.processor_mut().midi_cc_params() {
                let param_count = param_changes.getParameterCount();

                for i in 0..param_count {
                    if let Some(queue) = ComRef::from_raw(param_changes.getParameterData(i)) {
                        let param_id = queue.getParameterId();

                        // Check if this is a MIDI CC parameter
                        if let Some(controller) = MidiCcParams::param_id_to_controller(param_id) {
                            if cc_params.has_controller(controller) {
                                let point_count = queue.getPointCount();

                                // Process all points for sample-accurate timing
                                for j in 0..point_count {
                                    let mut sample_offset: i32 = 0;
                                    let mut value: f64 = 0.0;

                                    if queue.getPoint(j, &mut sample_offset, &mut value) == kResultOk {
                                        let midi_event = convert_cc_param_to_midi(
                                            controller,
                                            value as f32,
                                            sample_offset as u32,
                                        );
                                        midi_input.push(midi_event);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Check for MIDI input buffer overflow (once per block)
        if midi_input.has_overflowed() {
            warn!(
                "MIDI input buffer overflow: {} events max, some events were dropped",
                beamer_core::midi::MAX_MIDI_EVENTS
            );
        }

        // Clear and prepare MIDI output buffer and SysEx pool
        let midi_output = &mut *self.midi_output.get();
        midi_output.clear();
        let sysex_pool = &mut *self.sysex_output_pool.get();

        // Clear pool FIRST so next_slot is reset to 0 before draining fallback
        sysex_pool.clear();

        // With heap fallback enabled, emit any overflow messages from previous block first.
        // These allocate slots starting from 0; new plugin output will append after them.
        #[cfg(feature = "sysex-heap-fallback")]
        if sysex_pool.has_fallback() {
            if let Some(event_list) = ComRef::from_raw(process_data.outputEvents) {
                for sysex_data in sysex_pool.take_fallback() {
                    // Allocate from pool (succeeds since we just cleared it)
                    if let Some((ptr, len)) = sysex_pool.allocate(&sysex_data) {
                        let mut event: Event = std::mem::zeroed();
                        event.busIndex = 0;
                        event.sampleOffset = 0; // Delayed message, emit at start of block
                        event.ppqPosition = 0.0;
                        event.flags = 0;
                        event.r#type = K_DATA_EVENT;
                        event.__field0.data.r#type = DATA_TYPE_MIDI_SYSEX;
                        event.__field0.data.size = len as u32;
                        event.__field0.data.bytes = ptr;
                        let _ = event_list.addEvent(&mut event);
                    }
                }
            }
            // Log that we recovered from overflow
            warn!(
                "SysEx fallback: emitted delayed messages from previous block overflow"
            );
        }
        // NOTE: Don't clear again - fallback events occupy slots 0..N, new events append after

        // Process MIDI events (process_midi is on AudioProcessor)
        let processor = self.processor_mut();
        processor.process_midi(midi_input.as_slice(), midi_output);

        // Write output MIDI events
        if let Some(event_list) = ComRef::from_raw(process_data.outputEvents) {
            for midi_event in midi_output.iter() {
                if let Some(mut vst3_event) = convert_midi_to_vst3(midi_event, sysex_pool) {
                    let _ = event_list.addEvent(&mut vst3_event);
                }
            }
        }

        // Check for MIDI buffer overflow (once per block)
        if midi_output.has_overflowed() {
            warn!(
                "MIDI output buffer overflow: {} events reached capacity, some events were dropped",
                midi_output.len()
            );
        }

        // Check for SysEx pool overflow (once per block)
        if sysex_pool.has_overflowed() {
            warn!(
                "SysEx output pool overflow: {} slots exhausted, some SysEx messages were dropped",
                sysex_pool.capacity()
            );
        }

        // 3. Extract transport info from VST3 ProcessContext
        let transport = extract_transport(process_data.processContext);
        let sample_rate = *self.sample_rate.get();
        let context = CoreProcessContext::new(sample_rate, num_samples, transport);

        // 4. Process audio based on sample size
        let symbolic_sample_size = *self.symbolic_sample_size.get();
        let processor = self.processor_mut();

        if symbolic_sample_size == SymbolicSampleSizes_::kSample64 as i32 {
            // 64-bit processing path
            if processor.supports_double_precision() {
                // Native f64: extract f64 buffers and call process_f64()
                self.process_audio_f64_native(process_data, num_samples, processor, &context);
            } else {
                // Conversion: f64→f32, process, f32→f64
                self.process_audio_f64_converted(process_data, num_samples, processor, &context);
            }
        } else {
            // 32-bit processing path (default)
            self.process_audio_f32(process_data, num_samples, processor, &context);
        }

        kResultOk
    }

    unsafe fn getTailSamples(&self) -> u32 {
        // tail_samples and bypass_ramp_samples are on AudioProcessor
        match &*self.state.get() {
            PluginState::Unprepared { .. } => 0,
            PluginState::Prepared { processor, .. } => {
                processor.tail_samples().saturating_add(processor.bypass_ramp_samples())
            }
        }
    }
}

impl<P: Plugin + 'static> IProcessContextRequirementsTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn getProcessContextRequirements(&self) -> u32 {
        // Request all available transport information from host.
        // These flags tell the host which ProcessContext fields we need.
        // See VST3 SDK: IProcessContextRequirements interface
        const K_NEED_SYSTEM_TIME: u32 = 1 << 0;
        const K_NEED_CONTINUOUS_TIME_SAMPLES: u32 = 1 << 1;
        const K_NEED_PROJECT_TIME_MUSIC: u32 = 1 << 2;
        const K_NEED_BAR_POSITION_MUSIC: u32 = 1 << 3;
        const K_NEED_CYCLE_MUSIC: u32 = 1 << 4;
        const K_NEED_SAMPLES_TO_NEXT_CLOCK: u32 = 1 << 5;
        const K_NEED_TEMPO: u32 = 1 << 6;
        const K_NEED_TIME_SIGNATURE: u32 = 1 << 7;
        const K_NEED_FRAME_RATE: u32 = 1 << 9;
        const K_NEED_TRANSPORT_STATE: u32 = 1 << 10;

        K_NEED_SYSTEM_TIME
            | K_NEED_CONTINUOUS_TIME_SAMPLES
            | K_NEED_PROJECT_TIME_MUSIC
            | K_NEED_BAR_POSITION_MUSIC
            | K_NEED_CYCLE_MUSIC
            | K_NEED_SAMPLES_TO_NEXT_CLOCK
            | K_NEED_TEMPO
            | K_NEED_TIME_SIGNATURE
            | K_NEED_FRAME_RATE
            | K_NEED_TRANSPORT_STATE
    }
}

// =============================================================================
// IEditController implementation
// =============================================================================

impl<P: Plugin + 'static> IEditControllerTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        // For combined component, state is handled by IComponent::setState
        kResultOk
    }

    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }

    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        kResultOk
    }

    unsafe fn getParameterCount(&self) -> i32 {
        let user_params = self.params().count();
        // midi_cc_params is on Plugin, only available in unprepared state
        let cc_params = self.try_plugin()
            .and_then(|p| p.midi_cc_params())
            .map(|p| p.enabled_count())
            .unwrap_or(0);
        (user_params + cc_params) as i32
    }

    unsafe fn getParameterInfo(&self, param_index: i32, info: *mut ParameterInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let params = self.params();
        let user_param_count = params.count();

        // User-defined parameters first
        if (param_index as usize) < user_param_count {
            if let Some(param_info) = params.info(param_index as usize) {
                let info = &mut *info;
                info.id = param_info.id;
                copy_wstring(param_info.name, &mut info.title);
                copy_wstring(param_info.short_name, &mut info.shortTitle);
                copy_wstring(param_info.units, &mut info.units);
                info.stepCount = param_info.step_count;
                info.defaultNormalizedValue = param_info.default_normalized;
                info.unitId = param_info.unit_id;
                info.flags = {
                    let mut flags = 0;
                    if param_info.flags.can_automate {
                        flags |= ParameterInfo_::ParameterFlags_::kCanAutomate;
                    }
                    if param_info.flags.is_bypass {
                        flags |= ParameterInfo_::ParameterFlags_::kIsBypass;
                    }
                    // List parameters (enums) - display as dropdown with text labels
                    if param_info.flags.is_list {
                        flags |= ParameterInfo_::ParameterFlags_::kIsList;
                    }
                    // Hidden parameters (MIDI CC emulation)
                    if param_info.flags.is_hidden {
                        flags |= ParameterInfo_::ParameterFlags_::kIsHidden;
                    }
                    flags
                };
                return kResultOk;
            }
            return kInvalidArgument;
        }

        // Hidden MIDI CC parameters (only available in unprepared state)
        if let Some(cc_params) = self.try_plugin().and_then(|p| p.midi_cc_params()) {
            let cc_index = (param_index as usize) - user_param_count;
            if let Some(param_info) = cc_params.info(cc_index) {
                let info = &mut *info;
                info.id = param_info.id;
                copy_wstring(param_info.name, &mut info.title);
                copy_wstring(param_info.short_name, &mut info.shortTitle);
                copy_wstring(param_info.units, &mut info.units);
                info.stepCount = param_info.step_count;
                info.defaultNormalizedValue = param_info.default_normalized;
                info.unitId = param_info.unit_id;
                // Hidden + automatable
                info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate
                    | ParameterInfo_::ParameterFlags_::kIsHidden;
                return kResultOk;
            }
        }

        kInvalidArgument
    }

    unsafe fn getParamStringByValue(
        &self,
        id: u32,
        value_normalized: f64,
        string: *mut String128,
    ) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }

        let params = self.params();
        let display = params.normalized_to_string(id, value_normalized);
        copy_wstring(&display, &mut *string);
        kResultOk
    }

    unsafe fn getParamValueByString(
        &self,
        id: u32,
        string: *mut TChar,
        value_normalized: *mut f64,
    ) -> tresult {
        if string.is_null() || value_normalized.is_null() {
            return kInvalidArgument;
        }

        let len = len_wstring(string as *const TChar);
        if let Ok(s) = String::from_utf16(slice::from_raw_parts(string as *const u16, len)) {
            let params = self.params();
            if let Some(value) = params.string_to_normalized(id, &s) {
                *value_normalized = value;
                return kResultOk;
            }
        }
        kInvalidArgument
    }

    unsafe fn normalizedParamToPlain(&self, id: u32, value_normalized: f64) -> f64 {
        self.params().normalized_to_plain(id, value_normalized)
    }

    unsafe fn plainParamToNormalized(&self, id: u32, plain_value: f64) -> f64 {
        self.params().plain_to_normalized(id, plain_value)
    }

    unsafe fn getParamNormalized(&self, id: u32) -> f64 {
        // Check if this is a MIDI CC parameter
        if MidiCcParams::is_midi_cc_param(id) {
            if let Some(cc_params) = self.try_plugin().and_then(|p| p.midi_cc_params()) {
                return cc_params.get_normalized(id);
            }
        }

        self.params().get_normalized(id)
    }

    unsafe fn setParamNormalized(&self, id: u32, value: f64) -> tresult {
        // Check if this is a MIDI CC parameter
        if MidiCcParams::is_midi_cc_param(id) {
            if let Some(cc_params) = self.try_plugin().and_then(|p| p.midi_cc_params()) {
                cc_params.set_normalized(id, value);
                return kResultOk;
            }
        }

        self.params().set_normalized(id, value);
        kResultOk
    }

    unsafe fn setComponentHandler(&self, _handler: *mut IComponentHandler) -> tresult {
        // TODO: Store handler for notifying host of parameter changes from GUI
        kResultOk
    }

    unsafe fn createView(&self, name: *const c_char) -> *mut IPlugView {
        if name.is_null() {
            return std::ptr::null_mut();
        }

        // Check if this is an "editor" view request
        let name_str = std::ffi::CStr::from_ptr(name).to_str().unwrap_or("");
        if name_str != "editor" {
            return std::ptr::null_mut();
        }

        // TODO: Integrate WebView editor via EditorDelegate in Phase 2
        // For now, return null (no editor)
        std::ptr::null_mut()
    }
}

// =============================================================================
// IUnitInfo implementation (VST3 Unit/Group hierarchy)
// =============================================================================

impl<P: Plugin + 'static> IUnitInfoTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn getUnitCount(&self) -> i32 {
        use beamer_core::params::Units;
        self.params().unit_count() as i32
    }

    unsafe fn getUnitInfo(&self, unit_index: i32, info: *mut UnitInfo) -> tresult {
        if info.is_null() || unit_index < 0 {
            return kInvalidArgument;
        }

        use beamer_core::params::Units;
        let params = self.params();

        if let Some(unit_info) = params.unit_info(unit_index as usize) {
            let info = &mut *info;
            info.id = unit_info.id;
            info.parentUnitId = unit_info.parent_id;
            info.programListId = kNoProgramListId;
            copy_wstring(unit_info.name, &mut info.name);
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn getProgramListCount(&self) -> i32 {
        0 // No program lists (presets) for now
    }

    unsafe fn getProgramListInfo(
        &self,
        _list_index: i32,
        _info: *mut ProgramListInfo,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn getProgramName(&self, _list_id: i32, _program_index: i32, _name: *mut String128) -> tresult {
        kNotImplemented
    }

    unsafe fn getProgramInfo(
        &self,
        _list_id: i32,
        _program_index: i32,
        _attribute_id: *const c_char,
        _attribute_value: *mut String128,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn hasProgramPitchNames(&self, _list_id: i32, _program_index: i32) -> tresult {
        kResultFalse
    }

    unsafe fn getProgramPitchName(
        &self,
        _list_id: i32,
        _program_index: i32,
        _midi_pitch: i16,
        _name: *mut String128,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn getSelectedUnit(&self) -> i32 {
        0 // Return root unit
    }

    unsafe fn selectUnit(&self, _unit_id: i32) -> tresult {
        kResultOk // Accept but ignore unit selection
    }

    unsafe fn getUnitByBus(
        &self,
        _media_type: MediaType,
        _dir: BusDirection,
        _bus_index: i32,
        _channel: i32,
        _unit_id: *mut i32,
    ) -> tresult {
        kNotImplemented
    }

    unsafe fn setUnitProgramData(
        &self,
        _list_or_unit_id: i32,
        _program_index: i32,
        _data: *mut IBStream,
    ) -> tresult {
        kNotImplemented
    }
}

// =============================================================================
// IMidiMapping implementation (VST3 SDK 3.8.0)
// =============================================================================

impl<P: Plugin + 'static> IMidiMappingTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn getMidiControllerAssignment(
        &self,
        bus_index: i32,
        channel: i16,
        midi_controller_number: i16,
        id: *mut u32,
    ) -> tresult {
        if id.is_null() {
            return kInvalidArgument;
        }

        let controller = midi_controller_number as u8;

        // These methods are on Plugin, only available in unprepared state
        if let Some(plugin) = self.try_plugin() {
            // 1. First check plugin's custom mappings
            if let Some(param_id) = plugin.midi_cc_to_param(bus_index, channel, controller) {
                *id = param_id;
                return kResultOk;
            }

            // 2. Check hidden MIDI CC parameters (omni channel - ignore channel param)
            if let Some(cc_params) = plugin.midi_cc_params() {
                if cc_params.has_controller(controller) {
                    *id = MidiCcParams::param_id(controller);
                    return kResultOk;
                }
            }
        }

        kResultFalse
    }
}

// =============================================================================
// IMidiLearn implementation (VST3 SDK 3.8.0)
// =============================================================================

impl<P: Plugin + 'static> IMidiLearnTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn onLiveMIDIControllerInput(
        &self,
        bus_index: i32,
        channel: i16,
        midi_cc: i16,
    ) -> tresult {
        // on_midi_learn is on Plugin, only available in unprepared state
        if let Some(plugin) = self.try_plugin_mut() {
            if plugin.on_midi_learn(bus_index, channel, midi_cc as u8) {
                return kResultOk;
            }
        }
        kResultFalse
    }
}

// =============================================================================
// IMidiMapping2 implementation (VST3 SDK 3.8.0 - MIDI 2.0)
// =============================================================================

impl<P: Plugin + 'static> IMidiMapping2Trait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn getNumMidi1ControllerAssignments(&self, direction: BusDirections) -> u32 {
        // Only support input direction
        if direction != BusDirections_::kInput {
            return 0;
        }
        // midi1_assignments is on Plugin, only available in unprepared state
        self.try_plugin()
            .map(|p| p.midi1_assignments().len() as u32)
            .unwrap_or(0)
    }

    unsafe fn getMidi1ControllerAssignments(
        &self,
        direction: BusDirections,
        list: *const Midi1ControllerParamIDAssignmentList,
    ) -> tresult {
        if list.is_null() || direction != BusDirections_::kInput {
            return kInvalidArgument;
        }

        // midi1_assignments is on Plugin, only available in unprepared state
        let Some(plugin) = self.try_plugin() else {
            return kResultFalse;
        };
        let assignments = plugin.midi1_assignments();
        let list_ref = &*list;

        if (list_ref.count as usize) < assignments.len() {
            return kResultFalse;
        }

        if assignments.is_empty() {
            return kResultOk;
        }

        let map = slice::from_raw_parts_mut(list_ref.map, assignments.len());
        for (i, a) in assignments.iter().enumerate() {
            map[i] = Midi1ControllerParamIDAssignment {
                pId: a.assignment.param_id,
                busIndex: a.assignment.bus_index,
                channel: a.assignment.channel,
                controller: a.controller as i16,
            };
        }

        kResultOk
    }

    unsafe fn getNumMidi2ControllerAssignments(&self, direction: BusDirections) -> u32 {
        // Only support input direction
        if direction != BusDirections_::kInput {
            return 0;
        }
        // midi2_assignments is on Plugin, only available in unprepared state
        self.try_plugin()
            .map(|p| p.midi2_assignments().len() as u32)
            .unwrap_or(0)
    }

    unsafe fn getMidi2ControllerAssignments(
        &self,
        direction: BusDirections,
        list: *const Midi2ControllerParamIDAssignmentList,
    ) -> tresult {
        if list.is_null() || direction != BusDirections_::kInput {
            return kInvalidArgument;
        }

        // midi2_assignments is on Plugin, only available in unprepared state
        let Some(plugin) = self.try_plugin() else {
            return kResultFalse;
        };
        let assignments = plugin.midi2_assignments();
        let list_ref = &*list;

        if (list_ref.count as usize) < assignments.len() {
            return kResultFalse;
        }

        if assignments.is_empty() {
            return kResultOk;
        }

        let map = slice::from_raw_parts_mut(list_ref.map, assignments.len());
        for (i, a) in assignments.iter().enumerate() {
            map[i] = Midi2ControllerParamIDAssignment {
                pId: a.assignment.param_id,
                busIndex: a.assignment.bus_index,
                channel: a.assignment.channel,
                controller: Midi2Controller {
                    bank: a.controller.bank,
                    registered: if a.controller.registered { 1 } else { 0 },
                    index: a.controller.index,
                    reserved: 0,
                },
            };
        }

        kResultOk
    }
}

// =============================================================================
// IMidiLearn2 implementation (VST3 SDK 3.8.0 - MIDI 2.0)
// =============================================================================

impl<P: Plugin + 'static> IMidiLearn2Trait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn onLiveMidi1ControllerInput(
        &self,
        bus_index: i32,
        channel: u8,
        midi_cc: i16,
    ) -> tresult {
        if let Some(plugin) = self.try_plugin_mut() {
            if plugin.on_midi1_learn(bus_index, channel, midi_cc as u8) {
                return kResultOk;
            }
        }
        kResultFalse
    }

    unsafe fn onLiveMidi2ControllerInput(
        &self,
        bus_index: i32,
        channel: u8,
        midi_cc: Midi2Controller,
    ) -> tresult {
        if let Some(plugin) = self.try_plugin_mut() {
            let controller = beamer_core::Midi2Controller {
                bank: midi_cc.bank,
                registered: midi_cc.registered != 0,
                index: midi_cc.index,
            };
            if plugin.on_midi2_learn(bus_index, channel, controller) {
                return kResultOk;
            }
        }
        kResultFalse
    }
}

// =============================================================================
// INoteExpressionController implementation (VST3 SDK 3.5.0)
// =============================================================================

impl<P: Plugin + 'static> INoteExpressionControllerTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn getNoteExpressionCount(&self, bus_index: i32, channel: i16) -> i32 {
        self.try_plugin()
            .map(|p| p.note_expression_count(bus_index, channel) as i32)
            .unwrap_or(0)
    }

    unsafe fn getNoteExpressionInfo(
        &self,
        bus_index: i32,
        channel: i16,
        note_expression_index: i32,
        info: *mut NoteExpressionTypeInfo,
    ) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let Some(plugin) = self.try_plugin() else {
            return kInvalidArgument;
        };
        if let Some(expr_info) =
            plugin.note_expression_info(bus_index, channel, note_expression_index as usize)
        {
            let vst_info = &mut *info;
            vst_info.typeId = expr_info.type_id;
            copy_wstring(expr_info.title_str(), &mut vst_info.title);
            copy_wstring(expr_info.short_title_str(), &mut vst_info.shortTitle);
            copy_wstring(expr_info.units_str(), &mut vst_info.units);
            vst_info.unitId = expr_info.unit_id;
            vst_info.valueDesc.minimum = expr_info.value_desc.minimum;
            vst_info.valueDesc.maximum = expr_info.value_desc.maximum;
            vst_info.valueDesc.defaultValue = expr_info.value_desc.default_value;
            vst_info.valueDesc.stepCount = expr_info.value_desc.step_count;
            vst_info.associatedParameterId = expr_info.associated_parameter_id as u32;
            vst_info.flags = expr_info.flags.0;
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn getNoteExpressionStringByValue(
        &self,
        bus_index: i32,
        channel: i16,
        id: NoteExpressionTypeID,
        value_normalized: NoteExpressionValue,
        string: *mut String128,
    ) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }

        let Some(plugin) = self.try_plugin() else {
            return kInvalidArgument;
        };
        let display = plugin.note_expression_value_to_string(bus_index, channel, id, value_normalized);
        copy_wstring(&display, &mut *string);
        kResultOk
    }

    unsafe fn getNoteExpressionValueByString(
        &self,
        bus_index: i32,
        channel: i16,
        id: NoteExpressionTypeID,
        string: *const TChar,
        value_normalized: *mut NoteExpressionValue,
    ) -> tresult {
        if string.is_null() || value_normalized.is_null() {
            return kInvalidArgument;
        }

        let len = len_wstring(string);
        if let Ok(s) = String::from_utf16(slice::from_raw_parts(string, len)) {
            if let Some(plugin) = self.try_plugin() {
                if let Some(value) = plugin.note_expression_string_to_value(bus_index, channel, id, &s) {
                    *value_normalized = value;
                    return kResultOk;
                }
            }
        }
        kResultFalse
    }
}

// =============================================================================
// IKeyswitchController implementation (VST3 SDK 3.5.0)
// =============================================================================

impl<P: Plugin + 'static> IKeyswitchControllerTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn getKeyswitchCount(&self, bus_index: i32, channel: i16) -> i32 {
        self.try_plugin()
            .map(|p| p.keyswitch_count(bus_index, channel) as i32)
            .unwrap_or(0)
    }

    unsafe fn getKeyswitchInfo(
        &self,
        bus_index: i32,
        channel: i16,
        keyswitch_index: i32,
        info: *mut KeyswitchInfo,
    ) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let Some(plugin) = self.try_plugin() else {
            return kInvalidArgument;
        };
        if let Some(ks_info) =
            plugin.keyswitch_info(bus_index, channel, keyswitch_index as usize)
        {
            let vst_info = &mut *info;
            vst_info.typeId = ks_info.type_id;
            copy_wstring(ks_info.title_str(), &mut vst_info.title);
            copy_wstring(ks_info.short_title_str(), &mut vst_info.shortTitle);
            vst_info.keyswitchMin = ks_info.keyswitch_min;
            vst_info.keyswitchMax = ks_info.keyswitch_max;
            vst_info.keyRemapped = ks_info.key_remapped;
            vst_info.unitId = ks_info.unit_id;
            vst_info.flags = ks_info.flags;
            kResultOk
        } else {
            kInvalidArgument
        }
    }
}

// =============================================================================
// INoteExpressionPhysicalUIMapping implementation (VST3 SDK 3.6.11)
// =============================================================================

impl<P: Plugin + 'static> INoteExpressionPhysicalUIMappingTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn getPhysicalUIMapping(
        &self,
        bus_index: i32,
        channel: i16,
        list: *mut PhysicalUIMapList,
    ) -> tresult {
        if list.is_null() {
            return kInvalidArgument;
        }

        let Some(plugin) = self.try_plugin() else {
            return kInvalidArgument;
        };
        let mappings = plugin.physical_ui_mappings(bus_index, channel);
        let list_ref = &mut *list;

        // Fill in the mappings up to the provided count
        let fill_count = (list_ref.count as usize).min(mappings.len());
        if fill_count > 0 && !list_ref.map.is_null() {
            let map_slice = slice::from_raw_parts_mut(list_ref.map, fill_count);
            for (i, mapping) in mappings.iter().take(fill_count).enumerate() {
                map_slice[i].physicalUITypeID = mapping.physical_ui_type_id;
                map_slice[i].noteExpressionTypeID = mapping.note_expression_type_id;
            }
        }

        kResultOk
    }
}

// =============================================================================
// IVst3WrapperMPESupport implementation (VST3 SDK 3.6.12)
// =============================================================================

impl<P: Plugin + 'static> IVst3WrapperMPESupportTrait for Vst3Processor<P>
where
    P::Config: BuildConfig,
{
    unsafe fn enableMPEInputProcessing(&self, state: TBool) -> tresult {
        if let Some(plugin) = self.try_plugin_mut() {
            if plugin.enable_mpe_input_processing(state != 0) {
                return kResultOk;
            }
        }
        kResultFalse
    }

    unsafe fn setMPEInputDeviceSettings(
        &self,
        master_channel: i32,
        member_begin_channel: i32,
        member_end_channel: i32,
    ) -> tresult {
        if let Some(plugin) = self.try_plugin_mut() {
            let settings = beamer_core::MpeInputDeviceSettings {
                master_channel,
                member_begin_channel,
                member_end_channel,
            };
            if plugin.set_mpe_input_device_settings(settings) {
                return kResultOk;
            }
        }
        kResultFalse
    }
}

// =============================================================================
// Helper functions
// =============================================================================

/// Convert UTF-16 slice to UTF-8, writing into a fixed-size buffer.
///
/// Returns the number of bytes written. Handles BMP characters (most common)
/// and replaces non-BMP surrogates with replacement character.
fn utf16_to_utf8(utf16: &[u16], utf8_buf: &mut [u8]) -> usize {
    let mut utf8_pos = 0;

    for &code_unit in utf16 {
        if code_unit == 0 {
            // Null terminator
            break;
        }

        // Calculate how many UTF-8 bytes this character needs
        let (bytes_needed, char_value) = if code_unit < 0x80 {
            // ASCII (1 byte)
            (1, code_unit as u32)
        } else if code_unit < 0x800 {
            // 2-byte UTF-8
            (2, code_unit as u32)
        } else if (0xD800..=0xDFFF).contains(&code_unit) {
            // Surrogate pair (non-BMP) - simplified: use replacement char
            // Full implementation would need to look ahead for the low surrogate
            (3, 0xFFFD) // Unicode replacement character
        } else {
            // 3-byte UTF-8 (most non-ASCII BMP characters)
            (3, code_unit as u32)
        };

        // Check if we have room
        if utf8_pos + bytes_needed > utf8_buf.len() {
            break;
        }

        // Encode to UTF-8
        match bytes_needed {
            1 => {
                utf8_buf[utf8_pos] = char_value as u8;
            }
            2 => {
                utf8_buf[utf8_pos] = (0xC0 | (char_value >> 6)) as u8;
                utf8_buf[utf8_pos + 1] = (0x80 | (char_value & 0x3F)) as u8;
            }
            3 => {
                utf8_buf[utf8_pos] = (0xE0 | (char_value >> 12)) as u8;
                utf8_buf[utf8_pos + 1] = (0x80 | ((char_value >> 6) & 0x3F)) as u8;
                utf8_buf[utf8_pos + 2] = (0x80 | (char_value & 0x3F)) as u8;
            }
            _ => unreachable!(),
        }

        utf8_pos += bytes_needed;
    }

    utf8_pos
}

/// Convert a channel count to the corresponding VST3 speaker arrangement.
fn channel_count_to_speaker_arrangement(channel_count: u32) -> SpeakerArrangement {
    match channel_count {
        1 => SpeakerArr::kMono,
        2 => SpeakerArr::kStereo,
        // For other channel counts, create a bitmask with that many speakers
        n => (1u64 << n) - 1,
    }
}

/// Convert a MIDI CC parameter value to a MidiEvent.
///
/// This is used to convert parameter changes from IMidiMapping back to MIDI events.
/// The controller number determines the event type:
/// - 0-127: Standard MIDI CC (ControlChange)
/// - 128: Channel Aftertouch (ChannelPressure)
/// - 129: Pitch Bend (PitchBend)
fn convert_cc_param_to_midi(controller: u8, normalized_value: f32, sample_offset: u32) -> MidiEvent {
    match controller {
        LEGACY_CC_PITCH_BEND => {
            // Pitch bend: 0.0-1.0 normalized → -1.0 to 1.0
            let bend = normalized_value * 2.0 - 1.0;
            MidiEvent::pitch_bend(sample_offset, 0, bend)
        }
        LEGACY_CC_CHANNEL_PRESSURE => {
            // Channel aftertouch: 0.0-1.0
            MidiEvent::channel_pressure(sample_offset, 0, normalized_value)
        }
        cc => {
            // Standard CC: 0.0-1.0
            MidiEvent::control_change(sample_offset, 0, cc, normalized_value)
        }
    }
}

/// Convert a VST3 Event to a MIDI event.
///
/// Returns None for unsupported event types.
unsafe fn convert_vst3_to_midi(event: &Event) -> Option<MidiEvent> {
    let sample_offset = event.sampleOffset as u32;

    match event.r#type {
        K_NOTE_ON_EVENT => {
            let note_on = &event.__field0.noteOn;
            Some(MidiEvent::note_on(
                sample_offset,
                note_on.channel as u8,
                note_on.pitch as u8,
                note_on.velocity,
                note_on.noteId,
                note_on.tuning,
                note_on.length,
            ))
        }
        K_NOTE_OFF_EVENT => {
            let note_off = &event.__field0.noteOff;
            Some(MidiEvent::note_off(
                sample_offset,
                note_off.channel as u8,
                note_off.pitch as u8,
                note_off.velocity,
                note_off.noteId,
                note_off.tuning,
            ))
        }
        K_POLY_PRESSURE_EVENT => {
            let poly = &event.__field0.polyPressure;
            Some(MidiEvent::poly_pressure(
                sample_offset,
                poly.channel as u8,
                poly.pitch as u8,
                poly.pressure,
                poly.noteId,
            ))
        }
        K_DATA_EVENT => {
            let data_event = &event.__field0.data;
            // Only handle SysEx data type
            if data_event.r#type == DATA_TYPE_MIDI_SYSEX {
                let mut sysex = SysEx::new();
                let copy_len = (data_event.size as usize).min(MAX_SYSEX_SIZE);
                if copy_len > 0 && !data_event.bytes.is_null() {
                    let src = std::slice::from_raw_parts(data_event.bytes, copy_len);
                    sysex.data[..copy_len].copy_from_slice(src);
                    sysex.len = copy_len as u16;
                }
                Some(MidiEvent {
                    sample_offset,
                    event: MidiEventKind::SysEx(Box::new(sysex)),
                })
            } else {
                None
            }
        }
        K_NOTE_EXPRESSION_VALUE_EVENT => {
            let expr = &event.__field0.noteExpressionValue;
            Some(MidiEvent {
                sample_offset,
                event: MidiEventKind::NoteExpressionValue(CoreNoteExpressionValue {
                    note_id: expr.noteId,
                    expression_type: expr.typeId,
                    value: expr.value,
                }),
            })
        }
        K_NOTE_EXPRESSION_INT_VALUE_EVENT => {
            let expr = &event.__field0.noteExpressionIntValue;
            Some(MidiEvent {
                sample_offset,
                event: MidiEventKind::NoteExpressionInt(NoteExpressionInt {
                    note_id: expr.noteId,
                    expression_type: expr.typeId,
                    value: expr.value,
                }),
            })
        }
        K_NOTE_EXPRESSION_TEXT_EVENT => {
            let expr = &event.__field0.noteExpressionText;
            let mut text_event = NoteExpressionText {
                note_id: expr.noteId,
                expression_type: expr.typeId,
                text: [0u8; MAX_EXPRESSION_TEXT_SIZE],
                text_len: 0,
            };
            // Convert UTF-16 to UTF-8
            let text_len = expr.textLen as usize;
            if !expr.text.is_null() && text_len > 0 {
                let text_slice = std::slice::from_raw_parts(expr.text, text_len);
                let utf8_len = utf16_to_utf8(text_slice, &mut text_event.text);
                text_event.text_len = utf8_len as u8;
            }
            Some(MidiEvent {
                sample_offset,
                event: MidiEventKind::NoteExpressionText(text_event),
            })
        }
        K_CHORD_EVENT => {
            let chord = &event.__field0.chord;
            let mut info = ChordInfo {
                root: chord.root as i8,
                bass_note: chord.bassNote as i8,
                mask: chord.mask as u16,
                name: [0u8; MAX_CHORD_NAME_SIZE],
                name_len: 0,
            };
            // Convert UTF-16 to UTF-8
            let text_len = chord.textLen as usize;
            if !chord.text.is_null() && text_len > 0 {
                let text_slice = std::slice::from_raw_parts(chord.text, text_len);
                let utf8_len = utf16_to_utf8(text_slice, &mut info.name);
                info.name_len = utf8_len as u8;
            }
            Some(MidiEvent {
                sample_offset,
                event: MidiEventKind::ChordInfo(info),
            })
        }
        K_SCALE_EVENT => {
            let scale = &event.__field0.scale;
            let mut info = ScaleInfo {
                root: scale.root as i8,
                mask: scale.mask as u16,
                name: [0u8; MAX_SCALE_NAME_SIZE],
                name_len: 0,
            };
            // Convert UTF-16 to UTF-8
            let text_len = scale.textLen as usize;
            if !scale.text.is_null() && text_len > 0 {
                let text_slice = std::slice::from_raw_parts(scale.text, text_len);
                let utf8_len = utf16_to_utf8(text_slice, &mut info.name);
                info.name_len = utf8_len as u8;
            }
            Some(MidiEvent {
                sample_offset,
                event: MidiEventKind::ScaleInfo(info),
            })
        }
        K_LEGACY_MIDI_CC_OUT_EVENT => {
            let cc_event = &event.__field0.midiCCOut;
            let channel = cc_event.channel as u8;

            match cc_event.controlNumber {
                0..=127 => {
                    // Standard Control Change
                    Some(MidiEvent::control_change(
                        sample_offset,
                        channel,
                        cc_event.controlNumber,
                        cc_event.value as f32 / 127.0,
                    ))
                }
                LEGACY_CC_CHANNEL_PRESSURE => Some(MidiEvent::channel_pressure(
                    sample_offset,
                    channel,
                    cc_event.value as f32 / 127.0,
                )),
                LEGACY_CC_PITCH_BEND => {
                    // Pitch bend: 14-bit value split across value (LSB) and value2 (MSB)
                    // Cast to u8 first to avoid sign extension issues
                    let lsb = (cc_event.value as u8) as u16;
                    let msb = (cc_event.value2 as u8) as u16;
                    let raw = (msb << 7) | (lsb & 0x7F);
                    let normalized = (raw as f32 - 8192.0) / 8192.0;
                    Some(MidiEvent::pitch_bend(sample_offset, channel, normalized))
                }
                LEGACY_CC_PROGRAM_CHANGE => Some(MidiEvent::program_change(
                    sample_offset,
                    channel,
                    cc_event.value as u8,
                )),
                _ => None, // Unknown control number
            }
        }
        _ => None, // Unsupported event type
    }
}

/// Convert a MIDI event to a VST3 Event.
///
/// The `sysex_pool` parameter provides stable storage for SysEx data during the
/// process() call, ensuring the pointers remain valid until the host processes them.
///
/// Note: ChordInfo, ScaleInfo, and NoteExpressionText are primarily input events
/// (DAW → plugin) and are not output.
fn convert_midi_to_vst3(midi: &MidiEvent, sysex_pool: &mut SysExOutputPool) -> Option<Event> {
    let mut event: Event = unsafe { std::mem::zeroed() };
    event.busIndex = 0;
    event.sampleOffset = midi.sample_offset as i32;
    event.ppqPosition = 0.0;
    event.flags = 0;

    // Note: Writing to union fields is safe in Rust, only reading requires unsafe
    match &midi.event {
        MidiEventKind::NoteOn(note_on) => {
            event.r#type = K_NOTE_ON_EVENT;
            event.__field0.noteOn.channel = note_on.channel as i16;
            event.__field0.noteOn.pitch = note_on.pitch as i16;
            event.__field0.noteOn.velocity = note_on.velocity;
            event.__field0.noteOn.noteId = note_on.note_id;
            event.__field0.noteOn.tuning = note_on.tuning;
            event.__field0.noteOn.length = note_on.length;
        }
        MidiEventKind::NoteOff(note_off) => {
            event.r#type = K_NOTE_OFF_EVENT;
            event.__field0.noteOff.channel = note_off.channel as i16;
            event.__field0.noteOff.pitch = note_off.pitch as i16;
            event.__field0.noteOff.velocity = note_off.velocity;
            event.__field0.noteOff.noteId = note_off.note_id;
            event.__field0.noteOff.tuning = note_off.tuning;
        }
        MidiEventKind::PolyPressure(poly) => {
            event.r#type = K_POLY_PRESSURE_EVENT;
            event.__field0.polyPressure.channel = poly.channel as i16;
            event.__field0.polyPressure.pitch = poly.pitch as i16;
            event.__field0.polyPressure.pressure = poly.pressure;
            event.__field0.polyPressure.noteId = poly.note_id;
        }
        MidiEventKind::ControlChange(cc) => {
            event.r#type = K_LEGACY_MIDI_CC_OUT_EVENT;
            event.__field0.midiCCOut.controlNumber = cc.controller;
            event.__field0.midiCCOut.channel = cc.channel as i8;
            event.__field0.midiCCOut.value = (cc.value * 127.0) as i8;
            event.__field0.midiCCOut.value2 = 0;
        }
        MidiEventKind::PitchBend(pb) => {
            event.r#type = K_LEGACY_MIDI_CC_OUT_EVENT;
            event.__field0.midiCCOut.controlNumber = LEGACY_CC_PITCH_BEND;
            event.__field0.midiCCOut.channel = pb.channel as i8;
            // Convert -1.0..1.0 to 14-bit value (0-16383, center at 8192)
            let raw = ((pb.value * 8192.0) + 8192.0).clamp(0.0, 16383.0) as i16;
            event.__field0.midiCCOut.value = (raw & 0x7F) as i8;
            event.__field0.midiCCOut.value2 = ((raw >> 7) & 0x7F) as i8;
        }
        MidiEventKind::ChannelPressure(cp) => {
            event.r#type = K_LEGACY_MIDI_CC_OUT_EVENT;
            event.__field0.midiCCOut.controlNumber = LEGACY_CC_CHANNEL_PRESSURE;
            event.__field0.midiCCOut.channel = cp.channel as i8;
            event.__field0.midiCCOut.value = (cp.pressure * 127.0) as i8;
            event.__field0.midiCCOut.value2 = 0;
        }
        MidiEventKind::ProgramChange(pc) => {
            event.r#type = K_LEGACY_MIDI_CC_OUT_EVENT;
            event.__field0.midiCCOut.controlNumber = LEGACY_CC_PROGRAM_CHANGE;
            event.__field0.midiCCOut.channel = pc.channel as i8;
            event.__field0.midiCCOut.value = pc.program as i8;
            event.__field0.midiCCOut.value2 = 0;
        }
        MidiEventKind::NoteExpressionValue(expr) => {
            event.r#type = K_NOTE_EXPRESSION_VALUE_EVENT;
            event.__field0.noteExpressionValue.noteId = expr.note_id;
            event.__field0.noteExpressionValue.typeId = expr.expression_type;
            event.__field0.noteExpressionValue.value = expr.value;
        }
        MidiEventKind::NoteExpressionInt(expr) => {
            event.r#type = K_NOTE_EXPRESSION_INT_VALUE_EVENT;
            event.__field0.noteExpressionIntValue.noteId = expr.note_id;
            event.__field0.noteExpressionIntValue.typeId = expr.expression_type;
            event.__field0.noteExpressionIntValue.value = expr.value;
        }
        MidiEventKind::SysEx(sysex) => {
            // Allocate a slot in the pool for stable pointer storage
            if let Some((ptr, len)) = sysex_pool.allocate(sysex.as_slice()) {
                event.r#type = K_DATA_EVENT;
                event.__field0.data.r#type = DATA_TYPE_MIDI_SYSEX;
                event.__field0.data.size = len as u32;
                event.__field0.data.bytes = ptr;
            } else {
                // Pool is full, drop this SysEx
                return None;
            }
        }
        // ChordInfo/ScaleInfo are DAW → plugin only (chord track metadata).
        // Plugins receive these from the DAW but don't generate them.
        MidiEventKind::ChordInfo(_) => return None,
        MidiEventKind::ScaleInfo(_) => return None,

        // TODO: NoteExpressionText output not yet implemented.
        // Some vocal/granular synths emit phoneme or waveform text data.
        // Implementation would require a UTF-8→UTF-16 buffer pool (like SysEx)
        // to provide stable pointers for the host. Low priority but valid use case.
        MidiEventKind::NoteExpressionText(_) => return None,
    }

    Some(event)
}
