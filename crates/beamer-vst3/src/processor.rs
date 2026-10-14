//! Generic VST3 processor wrapper.
//!
//! This module provides [`Vst3Processor`], a generic wrapper that bridges any
//! [`beamer_core::Descriptor`] implementation to VST3 COM interfaces.
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
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use log::warn;
use vst3::{Class, ComRef, Steinberg::Vst::*, Steinberg::*};

use beamer_core::{
    AuxiliaryBuffers, Buffer, BusInfo as CoreBusInfo, BusLayout,
    BusType as CoreBusType, CachedBusConfig, CachedBusInfo, ChordInfo, ConversionBuffers,
    Descriptor, FactoryPresets, FrameRate as CoreFrameRate, HasParameters, MidiBuffer, MidiCcState,
    MidiEvent, MidiEventKind, NoPresets, NoteExpressionInt, NoteExpressionText,
    NoteExpressionValue as CoreNoteExpressionValue, ParameterStore, Config, PluginSetup,
    ProcessBufferStorage, ProcessContext as CoreProcessContext, Processor, ScaleInfo, SysEx,
    SysExOutputPool, Transport, WebViewEvents, WebViewHandler, MAX_BUSES, MAX_CHANNELS,
    MAX_CHORD_NAME_SIZE,
    MAX_EXPRESSION_TEXT_SIZE, MAX_SCALE_NAME_SIZE, MAX_SYSEX_SIZE,
};

use crate::factory::ComponentFactory;
use crate::util::{copy_wstring, len_wstring};

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

// Program change parameter ID (for preset selection, distinct from MIDI CC range)
const PROGRAM_CHANGE_PARAM_ID: u32 = 0x20000000;

/// Check that no parameter of the plugin has an id this wrapper uses for
/// its own parameters: the MIDI CC parameters and the program change.
///
/// The id of a parameter is a hash of its path, so it can land on one of
/// them. The wrapper would then answer for its own parameter and the
/// plugin's would be unreachable. The error names the parameter.
fn check_reserved_ids(parameters: &dyn beamer_core::parameter_types::Parameters) -> Result<(), String> {
    for parameter in parameters.iter() {
        let id = parameter.id();
        let reserved_for = if MidiCcState::is_midi_cc_parameter(id) {
            "the MIDI CC parameters"
        } else if id == PROGRAM_CHANGE_PARAM_ID {
            "the program change parameter"
        } else {
            continue;
        };
        return Err(format!(
            "parameter \"{}\" has the host id 0x{:08x}, which the VST3 wrapper uses for {}; give it another id",
            parameter.info().string_id,
            id,
            reserved_for
        ));
    }
    Ok(())
}

// Program list ID for factory presets
const FACTORY_PRESETS_LIST_ID: i32 = 0;

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
/// Converts VST3's validity flags to Rust's `Option<T>` idiom.
/// Returns a default Transport if the context pointer is null.
///
/// # Safety
///
/// The caller must ensure `context_ptr` is either null or points to a valid
/// ProcessContext struct for the duration of this call.
unsafe fn extract_transport(context_ptr: *const ProcessContext) -> Transport {
    if context_ptr.is_null() {
        return Transport::default();
    }

    // SAFETY: Validated context_ptr is non-null above. Caller guarantees it points to
    // a valid ProcessContext for the duration of this call.
    let context = unsafe { &*context_ptr };
    let state = context.state;

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
        tempo: valid_if!(state, K_TEMPO_VALID, context.tempo),
        time_sig_numerator: valid_if!(state, K_TIME_SIG_VALID, context.timeSigNumerator),
        time_sig_denominator: valid_if!(state, K_TIME_SIG_VALID, context.timeSigDenominator),

        // Position
        project_time_samples: Some(context.projectTimeSamples),
        project_time_beats: valid_if!(state, K_PROJECT_TIME_MUSIC_VALID, context.projectTimeMusic),
        bar_position_beats: valid_if!(state, K_BAR_POSITION_VALID, context.barPositionMusic),

        // Cycle/loop
        cycle_start_beats: valid_if!(state, K_CYCLE_VALID, context.cycleStartMusic),
        cycle_end_beats: valid_if!(state, K_CYCLE_VALID, context.cycleEndMusic),

        // Transport state (always valid)
        is_playing: state & K_PLAYING != 0,
        is_recording: state & K_RECORDING != 0,
        is_cycle_active: state & K_CYCLE_ACTIVE != 0,

        // Advanced timing
        system_time_ns: valid_if!(state, K_SYSTEM_TIME_VALID, context.systemTime),
        continuous_time_samples: valid_if!(state, K_CONT_TIME_VALID, context.continousTimeSamples), // Note: VST3 SDK typo
        samples_to_next_clock: valid_if!(state, K_CLOCK_VALID, context.samplesToNextClock),

        // SMPTE - use FrameRate::from_raw() for conversion
        smpte_offset_subframes: valid_if!(state, K_SMPTE_VALID, context.smpteOffsetSubframes),
        frame_rate: if state & K_SMPTE_VALID != 0 {
            let is_drop = context.frameRate.flags & 1 != 0;
            CoreFrameRate::from_raw(context.frameRate.framesPerSecond, is_drop)
        } else {
            None
        },
    }
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
// Setup Extraction
// =============================================================================

/// Build plugin setup from VST3 ProcessSetup.
///
/// Creates a HostSetup with all available information, then uses the
/// `PluginSetup::extract` method to extract only what the plugin needs.
fn build_setup<S: PluginSetup>(setup: &ProcessSetup, bus_layout: &BusLayout) -> S {
    use beamer_core::{HostSetup, ProcessMode};

    // Convert VST3 process mode to our ProcessMode.
    let process_mode = match setup.processMode as ProcessModes {
        ProcessModes_::kOffline => ProcessMode::Offline,
        ProcessModes_::kPrefetch => ProcessMode::Prefetch,
        _ => ProcessMode::Realtime,
    };

    let host_setup = HostSetup::new(
        setup.sampleRate,
        setup.maxSamplesPerBlock as usize,
        bus_layout.clone(),
        process_mode,
    );

    S::extract(&host_setup)
}

// =============================================================================
// Plugin State Machine
// =============================================================================

// Note: beamer_core::BusInfo is imported as CoreBusInfo in the main imports
// to avoid collision with vst3::Steinberg::Vst::BusInfo used in COM interfaces.
// We use CoreBusInfo throughout this module for the beamer type.

// =============================================================================
// Vst3Processor Wrapper
// =============================================================================

/// Generic VST3 processor wrapping any [`Descriptor`] implementation.
///
/// This struct implements the VST3 combined component pattern, providing
/// `IComponent`, `IAudioProcessor` and `IEditController` interfaces that
/// delegate to the wrapped plugin.
///
/// # Lifecycle
///
/// The wrapper keeps the plugin's descriptor and its parameters for its
/// whole life, and a processor while the plugin is prepared:
///
/// ```text
/// Vst3Processor::new()
///     ↓ creates Descriptor::default()
/// descriptor, no processor
///     ↓ setupProcessing() calls descriptor.prepare(setup)
/// descriptor and processor
///     ↓ setupProcessing() with a new setup: drops the processor, then prepare()
/// descriptor and a new processor
/// ```
///
/// # Usage
///
/// ```ignore
/// use beamer_vst3::{export_vst3, Vst3Processor, Config};
/// use beamer_core::config::Category;
///
/// #[derive(Default)]
/// struct MyPlugin { parameters: Arc<MyParameters> }
/// impl Descriptor for MyPlugin { /* ... */ }
///
/// struct MyProcessor { parameters: Arc<MyParameters>, sample_rate: f64 }
/// impl Processor for MyProcessor { /* ... */ }
///
/// static CONFIG: Config = Config::new("MyPlugin", Category::Effect, "Mfgr", "plgn");
/// export_vst3!(CONFIG, MyPlugin);
/// ```
///
/// # Thread Safety
///
/// VST3 guarantees that `process()` is called from a single thread at a time.
/// We use `UnsafeCell` for interior mutability in `process()` since the COM
/// interface only provides `&self`.
///
/// A host calls the other interfaces from its own threads while `process()`
/// runs. What those calls use is kept apart from what `process()` uses: the
/// descriptor sits behind a lock that `process()` never takes, the parameters
/// are shared in an `Arc` and work through a shared reference, and latency,
/// tail and the smoother reset are exchanged through atomics.
pub struct Vst3Processor<P, Presets = NoPresets<<P as HasParameters>::Parameters>>
where
    P: Descriptor,
    Presets: FactoryPresets<Parameters = <P as HasParameters>::Parameters>,
{
    /// The plugin as the host sees it. It lives as long as the wrapper and
    /// is only used by host threads, never by `process()`. Host threads
    /// read it at the same time, also while `setupProcessing()` lets it
    /// create a processor. The write side is for its few methods that
    /// change it (MIDI learn, MPE settings).
    descriptor: RwLock<P>,
    /// The parameters of the plugin, shared with the descriptor and with
    /// every processor it creates. `process()` and the host threads use
    /// them through this handle at the same time.
    parameters: Arc<P::Parameters>,
    /// The processor, once `setupProcessing()` has created one. It belongs
    /// to `process()`. Host calls only touch it when the host guarantees
    /// that `process()` does not run (`setupProcessing()`, `setActive()`).
    processor: UnsafeCell<Option<P::Processor>>,
    /// Plugin configuration reference
    config: &'static Config,
    /// Current sample rate
    sample_rate: UnsafeCell<f64>,
    /// Maximum block size
    max_block_size: UnsafeCell<usize>,
    /// Current symbolic sample size (kSample32 or kSample64)
    symbolic_sample_size: UnsafeCell<i32>,
    /// Process mode of the current setup (a `ProcessModes` value)
    process_mode: UnsafeCell<int32>,
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
    /// MIDI CC state (created from Plugin's midi_cc_config())
    /// Framework owns this - plugin authors don't touch it
    midi_cc_state: Option<MidiCcState>,
    /// Current factory preset index (0-based, or -1 for no preset / custom state)
    /// Used for the program change parameter exposed to the host
    current_preset_index: UnsafeCell<i32>,
    /// Component handler for notifying host of parameter changes
    /// Stored as raw pointer - host manages lifetime, we just AddRef/Release
    component_handler: UnsafeCell<*mut IComponentHandler>,
    /// Custom WebView message handler (invoke/event routing).
    webview_handler: Option<Arc<dyn WebViewHandler>>,
    /// The plugin's queue of events for the web UI, if it has one.
    webview_events: Option<WebViewEvents>,
    /// Latency in samples as the processor reported it last, 0 while unprepared.
    /// The host asks for it from its own threads while `process()` runs, so
    /// the answer must not need the processor (see `publish_timing`).
    latency_samples: AtomicU32,
    /// Tail in samples, with the bypass ramp, as the processor reported it last.
    tail_samples: AtomicU32,
    /// A state was loaded: `process()` resets the parameter smoothers before
    /// its next block. They belong to the audio thread.
    smoothing_reset_requested: AtomicBool,
    /// Marker for the plugin type and preset collection
    _marker: PhantomData<(P, Presets)>,
}

// Safety: Vst3Processor is Send because:
// - Descriptor: Send is required by the Descriptor trait
// - Processor: Send is required by the Processor trait
// - The parameters are Send + Sync
unsafe impl<P: Descriptor, Presets> Send for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
}

// Safety: Vst3Processor is Sync because:
// - VST3 guarantees process() is called from one thread at a time
// - The parameters are Sync, and process() reaches them through its own
//   `Arc`, not through the descriptor
// - The descriptor is Sync and behind an RwLock that only host threads take
// - current_preset_index and component_handler are used by the controller
//   thread alone
// - What a host thread uses while process() runs is atomic: the parameter
//   values (also for saving and loading the state), latency, tail and the
//   request to reset the smoothers
unsafe impl<P: Descriptor, Presets> Sync for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
}

impl<P: Descriptor + 'static, Presets> Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    /// Create a new VST3 processor wrapping the given plugin configuration.
    ///
    /// The wrapper starts unprepared, with a default plugin instance.
    /// The processor will be created when `setupProcessing()` is called.
    pub fn new(config: &'static Config) -> Self {
        let plugin = P::default();

        // Create MidiCcState from plugin's config (framework-managed)
        let midi_cc_state = plugin.midi_cc_config().map(|cfg| MidiCcState::from_config(&cfg));

        // The WebView handler (if any) of the plugin.
        let webview_handler = plugin.webview_handler();

        // The plugin's queue of events for the web UI. Taking it here
        // marks it as delivered by this instance.
        let webview_events = plugin.webview_events().cloned();
        if let Some(events) = &webview_events {
            events.claim();
        }

        let parameters = Arc::clone(plugin.parameters());

        Self {
            descriptor: RwLock::new(plugin),
            parameters,
            processor: UnsafeCell::new(None),
            config,
            sample_rate: UnsafeCell::new(44100.0),
            max_block_size: UnsafeCell::new(1024),
            symbolic_sample_size: UnsafeCell::new(SymbolicSampleSizes_::kSample32 as i32),
            process_mode: UnsafeCell::new(ProcessModes_::kRealtime as int32),
            midi_input: UnsafeCell::new(MidiBuffer::new()),
            midi_output: UnsafeCell::new(MidiBuffer::new()),
            sysex_output_pool: UnsafeCell::new(SysExOutputPool::with_capacity(
                config.sysex_slots,
                config.sysex_buffer_size,
            )),
            conversion_buffers: UnsafeCell::new(ConversionBuffers::new()),
            buffer_storage_f32: UnsafeCell::new(ProcessBufferStorage::new()),
            buffer_storage_f64: UnsafeCell::new(ProcessBufferStorage::new()),
            midi_cc_state,
            current_preset_index: UnsafeCell::new(0), // Default to first preset
            component_handler: UnsafeCell::new(std::ptr::null_mut()),
            webview_handler,
            webview_events,
            latency_samples: AtomicU32::new(0),
            tail_samples: AtomicU32::new(0),
            smoothing_reset_requested: AtomicBool::new(false),
            _marker: PhantomData,
        }
    }

    /// Publish the latency and tail of the processor for the host.
    ///
    /// The host calls `getLatencySamples()` and `getTailSamples()` from its
    /// own threads while `process()` runs. Whoever has the processor to
    /// itself stores the values here: `setupProcessing()`, `setActive()` and
    /// each `process()` call.
    fn publish_timing(&self, processor: &P::Processor) {
        self.latency_samples
            .store(processor.latency_samples(), Ordering::Relaxed);
        self.tail_samples.store(
            processor
                .tail_samples()
                .saturating_add(processor.bypass_ramp_samples()),
            Ordering::Relaxed,
        );
    }

    /// Get a mutable reference to the prepared processor.
    ///
    /// # Safety
    /// - Must only be called from contexts where VST3 guarantees single-threaded access
    ///   to the processor (`process()`).
    /// - Must only be called when prepared.
    ///
    /// # Panics
    /// Panics if called when unprepared (VST3 host violation).
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn processor_mut(&self) -> &mut P::Processor {
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        match unsafe { &mut *self.processor.get() } {
            Some(processor) => processor,
            None => panic!("Attempted to access processor before setupProcessing()"),
        }
    }

    /// Read the descriptor for a call of the host.
    ///
    /// Not for `process()`: a host thread may hold the write side.
    fn descriptor(&self) -> RwLockReadGuard<'_, P> {
        // A poisoned lock only means a panic in an earlier call. The
        // descriptor answers the host all the same.
        self.descriptor
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Lock the descriptor for a call of the host that changes it.
    ///
    /// Not for `process()`.
    fn descriptor_mut(&self) -> RwLockWriteGuard<'_, P> {
        self.descriptor
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    // =========================================================================
    // Descriptor Access (works in both states, not for process())
    // =========================================================================

    /// Get input bus count.
    #[inline]
    fn input_bus_count(&self) -> usize {
        self.descriptor().input_bus_count()
    }

    /// Get output bus count.
    #[inline]
    fn output_bus_count(&self) -> usize {
        self.descriptor().output_bus_count()
    }

    /// Get input bus info.
    /// Returns beamer_core::BusInfo (not vst3::BusInfo).
    #[inline]
    fn core_input_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
        self.descriptor().input_bus_info(index)
    }

    /// Get output bus info.
    /// Returns beamer_core::BusInfo (not vst3::BusInfo).
    #[inline]
    fn core_output_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
        self.descriptor().output_bus_info(index)
    }

    /// Check if plugin wants MIDI.
    #[inline]
    fn wants_midi(&self) -> bool {
        self.descriptor().wants_midi()
    }

    // =========================================================================
    // Parameter Access (works in both states)
    // =========================================================================

    /// Get the parameters (works in both states, from every thread).
    #[inline]
    fn parameters(&self) -> &P::Parameters {
        &self.parameters
    }

    /// Get a handle to the parameters for an editor view. The view may
    /// live longer than a processor and uses the parameters from its own
    /// thread.
    #[cfg_attr(not(feature = "webview"), allow(dead_code))]
    fn shared_parameter_store(&self) -> Arc<dyn ParameterStore> {
        let parameters: Arc<P::Parameters> = Arc::clone(&self.parameters);
        parameters
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
    //
    // TODO: Null buffer handling - Currently we skip null channel pointers.
    // This is correct for VST3's parameter flushing (numSamples=0). Some hosts
    // may send null buffers with non-zero numSamples. Consider adding internal
    // buffer fallback like beamer-au does for instruments if this becomes an
    // issue. For now, VST3 hosts are generally compliant.
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
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let storage = unsafe { &mut *self.buffer_storage_f32.get() };
        storage.clear();

        // Collect main input channel pointers (bounded by pre-allocated capacity)
        if process_data.numInputs > 0 && !process_data.inputs.is_null() {
            // SAFETY: inputs is non-null and host guarantees validity for numInputs elements.
            let bus = unsafe { &*process_data.inputs };
            let num_channels = bus.numChannels as usize;
            let max_channels = storage.main_inputs.capacity();
            // SAFETY: symbolic_sample_size == kSample32, so channelBuffers32 is valid variant.
            if num_channels > 0 && !unsafe { bus.__field0.channelBuffers32 }.is_null() {
                // SAFETY: Host guarantees channelBuffers32 valid for numChannels elements.
                let channel_ptrs = unsafe {
                    slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels)
                };
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    if !ptr.is_null() {
                        storage.main_inputs.push(ptr);
                    }
                }
            }
        }

        // Collect main output channel pointers (bounded by pre-allocated capacity)
        if process_data.numOutputs > 0 && !process_data.outputs.is_null() {
            // SAFETY: outputs is non-null and host guarantees validity for numOutputs elements.
            let bus = unsafe { &*process_data.outputs };
            let num_channels = bus.numChannels as usize;
            let max_channels = storage.main_outputs.capacity();
            // SAFETY: symbolic_sample_size == kSample32, so channelBuffers32 is valid variant.
            if num_channels > 0 && !unsafe { bus.__field0.channelBuffers32 }.is_null() {
                // SAFETY: Host guarantees channelBuffers32 valid for numChannels elements.
                let channel_ptrs = unsafe {
                    slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels)
                };
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    if !ptr.is_null() {
                        storage.main_outputs.push(ptr);
                    }
                }
            }
        }

        // Collect auxiliary input channel pointers (bounded by pre-allocated capacity)
        if process_data.numInputs > 1 && !process_data.inputs.is_null() {
            // SAFETY: inputs is non-null and host guarantees validity for numInputs elements.
            let input_buses = unsafe {
                slice::from_raw_parts(process_data.inputs, process_data.numInputs as usize)
            };
            for (aux_idx, bus) in input_buses[1..].iter().enumerate() {
                if aux_idx < storage.aux_inputs.len() {
                    let num_channels = bus.numChannels as usize;
                    let max_channels = storage.aux_inputs[aux_idx].capacity();
                    // SAFETY: symbolic_sample_size == kSample32, so channelBuffers32 is valid.
                    if num_channels > 0 && !unsafe { bus.__field0.channelBuffers32 }.is_null() {
                        // SAFETY: Host guarantees channelBuffers32 valid for numChannels elements.
                        let channel_ptrs = unsafe {
                            slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels)
                        };
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
            // SAFETY: outputs is non-null and host guarantees validity for numOutputs elements.
            let output_buses = unsafe {
                slice::from_raw_parts(process_data.outputs, process_data.numOutputs as usize)
            };
            for (aux_idx, bus) in output_buses[1..].iter().enumerate() {
                if aux_idx < storage.aux_outputs.len() {
                    let num_channels = bus.numChannels as usize;
                    let max_channels = storage.aux_outputs[aux_idx].capacity();
                    // SAFETY: symbolic_sample_size == kSample32, so channelBuffers32 is valid.
                    if num_channels > 0 && !unsafe { bus.__field0.channelBuffers32 }.is_null() {
                        // SAFETY: Host guarantees channelBuffers32 valid for numChannels elements.
                        let channel_ptrs = unsafe {
                            slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels)
                        };
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            if !ptr.is_null() {
                                storage.aux_outputs[aux_idx].push(ptr);
                            }
                        }
                    }
                }
            }
        }

        // Create slices from pointers
        // SAFETY: Host guarantees channel pointers valid for num_samples elements
        // for the duration of process().
        let main_in_iter = storage.main_inputs.iter().map(|&ptr| {
            // SAFETY: Host guarantees buffer pointer valid for num_samples elements.
            unsafe { slice::from_raw_parts(ptr, num_samples) }
        });
        let main_out_iter = storage.main_outputs.iter().map(|&ptr| {
            // SAFETY: Host guarantees buffer pointer valid for num_samples elements.
            unsafe { slice::from_raw_parts_mut(ptr, num_samples) }
        });

        let aux_in_iter = storage.aux_inputs.iter().map(|bus| {
            bus.iter().map(|&ptr| {
                // SAFETY: Host guarantees buffer pointer valid for num_samples elements.
                unsafe { slice::from_raw_parts(ptr, num_samples) }
            })
        });
        let aux_out_iter = storage.aux_outputs.iter().map(|bus| {
            bus.iter().map(|&ptr| {
                // SAFETY: Host guarantees buffer pointer valid for num_samples elements.
                unsafe { slice::from_raw_parts_mut(ptr, num_samples) }
            })
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
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let storage = unsafe { &mut *self.buffer_storage_f64.get() };
        storage.clear();

        // Collect main input channel pointers (bounded by pre-allocated capacity)
        if process_data.numInputs > 0 && !process_data.inputs.is_null() {
            // SAFETY: inputs is non-null and host guarantees validity for numInputs elements.
            let bus = unsafe { &*process_data.inputs };
            let num_channels = bus.numChannels as usize;
            let max_channels = storage.main_inputs.capacity();
            // SAFETY: symbolic_sample_size == kSample64, so channelBuffers64 is valid variant.
            if num_channels > 0 && !unsafe { bus.__field0.channelBuffers64 }.is_null() {
                // SAFETY: Host guarantees channelBuffers64 valid for numChannels elements.
                let channel_ptrs = unsafe {
                    slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels)
                };
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    if !ptr.is_null() {
                        storage.main_inputs.push(ptr);
                    }
                }
            }
        }

        // Collect main output channel pointers (bounded by pre-allocated capacity)
        if process_data.numOutputs > 0 && !process_data.outputs.is_null() {
            // SAFETY: outputs is non-null and host guarantees validity for numOutputs elements.
            let bus = unsafe { &*process_data.outputs };
            let num_channels = bus.numChannels as usize;
            let max_channels = storage.main_outputs.capacity();
            // SAFETY: symbolic_sample_size == kSample64, so channelBuffers64 is valid variant.
            if num_channels > 0 && !unsafe { bus.__field0.channelBuffers64 }.is_null() {
                // SAFETY: Host guarantees channelBuffers64 valid for numChannels elements.
                let channel_ptrs = unsafe {
                    slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels)
                };
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    if !ptr.is_null() {
                        storage.main_outputs.push(ptr);
                    }
                }
            }
        }

        // Collect auxiliary input channel pointers (bounded by pre-allocated capacity)
        if process_data.numInputs > 1 && !process_data.inputs.is_null() {
            // SAFETY: inputs is non-null and host guarantees validity for numInputs elements.
            let input_buses = unsafe {
                slice::from_raw_parts(process_data.inputs, process_data.numInputs as usize)
            };
            for (aux_idx, bus) in input_buses[1..].iter().enumerate() {
                if aux_idx < storage.aux_inputs.len() {
                    let num_channels = bus.numChannels as usize;
                    let max_channels = storage.aux_inputs[aux_idx].capacity();
                    // SAFETY: symbolic_sample_size == kSample64, so channelBuffers64 is valid.
                    if num_channels > 0 && !unsafe { bus.__field0.channelBuffers64 }.is_null() {
                        // SAFETY: Host guarantees channelBuffers64 valid for numChannels elements.
                        let channel_ptrs = unsafe {
                            slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels)
                        };
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
            // SAFETY: outputs is non-null and host guarantees validity for numOutputs elements.
            let output_buses = unsafe {
                slice::from_raw_parts(process_data.outputs, process_data.numOutputs as usize)
            };
            for (aux_idx, bus) in output_buses[1..].iter().enumerate() {
                if aux_idx < storage.aux_outputs.len() {
                    let num_channels = bus.numChannels as usize;
                    let max_channels = storage.aux_outputs[aux_idx].capacity();
                    // SAFETY: symbolic_sample_size == kSample64, so channelBuffers64 is valid.
                    if num_channels > 0 && !unsafe { bus.__field0.channelBuffers64 }.is_null() {
                        // SAFETY: Host guarantees channelBuffers64 valid for numChannels elements.
                        let channel_ptrs = unsafe {
                            slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels)
                        };
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            if !ptr.is_null() {
                                storage.aux_outputs[aux_idx].push(ptr);
                            }
                        }
                    }
                }
            }
        }

        // Create slices from pointers
        // SAFETY: Host guarantees channel pointers valid for num_samples elements
        // for the duration of process().
        let main_in_iter = storage.main_inputs.iter().map(|&ptr| {
            // SAFETY: Host guarantees buffer pointer valid for num_samples elements.
            unsafe { slice::from_raw_parts(ptr, num_samples) }
        });
        let main_out_iter = storage.main_outputs.iter().map(|&ptr| {
            // SAFETY: Host guarantees buffer pointer valid for num_samples elements.
            unsafe { slice::from_raw_parts_mut(ptr, num_samples) }
        });

        let aux_in_iter = storage.aux_inputs.iter().map(|bus| {
            bus.iter().map(|&ptr| {
                // SAFETY: Host guarantees buffer pointer valid for num_samples elements.
                unsafe { slice::from_raw_parts(ptr, num_samples) }
            })
        });
        let aux_out_iter = storage.aux_outputs.iter().map(|bus| {
            bus.iter().map(|&ptr| {
                // SAFETY: Host guarantees buffer pointer valid for num_samples elements.
                unsafe { slice::from_raw_parts_mut(ptr, num_samples) }
            })
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
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let conv = unsafe { &mut *self.conversion_buffers.get() };

        // Convert main input f64 → f32
        if process_data.numInputs > 0 && !process_data.inputs.is_null() {
            // SAFETY: inputs is non-null and host guarantees validity.
            let input_buses = unsafe { slice::from_raw_parts(process_data.inputs, 1) };
            let bus = &input_buses[0];
            let num_channels = (bus.numChannels as usize).min(conv.main_input_f32.len());
            // SAFETY: symbolic_sample_size == kSample64, so channelBuffers64 is valid variant.
            if num_channels > 0 && !unsafe { bus.__field0.channelBuffers64 }.is_null() {
                // SAFETY: Host guarantees channelBuffers64 valid for numChannels elements.
                let channel_ptrs = unsafe { slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels) };
                for (ch, &ptr) in channel_ptrs.iter().enumerate() {
                    if !ptr.is_null() && ch < conv.main_input_f32.len() {
                        // SAFETY: Host guarantees channel pointer valid for num_samples elements.
                        let src = unsafe { slice::from_raw_parts(ptr, num_samples) };
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
                // SAFETY: inputs is non-null and host guarantees validity for numInputs elements.
                let input_buses = unsafe {
                    slice::from_raw_parts(
                        process_data.inputs,
                        process_data.numInputs as usize,
                    )
                };
                let bus = &input_buses[vst_bus_idx];
                let num_channels = (bus.numChannels as usize).min(aux_bus.len());
                // SAFETY: symbolic_sample_size == kSample64, so channelBuffers64 is valid variant.
                if num_channels > 0 && !unsafe { bus.__field0.channelBuffers64 }.is_null() {
                    // SAFETY: Host guarantees channelBuffers64 valid for numChannels elements.
                    let channel_ptrs = unsafe { slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels) };
                    for (ch, &ptr) in channel_ptrs.iter().enumerate() {
                        if !ptr.is_null() && ch < aux_bus.len() {
                            // SAFETY: Host guarantees channel pointer valid for num_samples elements.
                            let src = unsafe { slice::from_raw_parts(ptr, num_samples) };
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
            // SAFETY: outputs is non-null and host guarantees validity.
            let output_buses = unsafe { slice::from_raw_parts(process_data.outputs, 1) };
            let bus = &output_buses[0];
            let num_channels = (bus.numChannels as usize).min(conv.main_output_f32.len());
            // SAFETY: symbolic_sample_size == kSample64, so channelBuffers64 is valid variant.
            if num_channels > 0 && !unsafe { bus.__field0.channelBuffers64 }.is_null() {
                // SAFETY: Host guarantees channelBuffers64 valid for numChannels elements.
                let channel_ptrs = unsafe { slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels) };
                for (ch, &ptr) in channel_ptrs.iter().enumerate() {
                    if !ptr.is_null() && ch < conv.main_output_f32.len() {
                        // SAFETY: Host guarantees channel pointer valid for num_samples elements.
                        let dst = unsafe { slice::from_raw_parts_mut(ptr, num_samples) };
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
                // SAFETY: outputs is non-null and host guarantees validity for numOutputs elements.
                let output_buses = unsafe {
                    slice::from_raw_parts(
                        process_data.outputs,
                        process_data.numOutputs as usize,
                    )
                };
                let bus = &output_buses[vst_bus_idx];
                let num_channels = (bus.numChannels as usize).min(aux_bus.len());
                // SAFETY: symbolic_sample_size == kSample64, so channelBuffers64 is valid variant.
                if num_channels > 0 && !unsafe { bus.__field0.channelBuffers64 }.is_null() {
                    // SAFETY: Host guarantees channelBuffers64 valid for numChannels elements.
                    let channel_ptrs = unsafe { slice::from_raw_parts(bus.__field0.channelBuffers64, num_channels) };
                    for (ch, &ptr) in channel_ptrs.iter().enumerate() {
                        if !ptr.is_null() && ch < aux_bus.len() {
                            // SAFETY: Host guarantees channel pointer valid for num_samples elements.
                            let dst = unsafe { slice::from_raw_parts_mut(ptr, num_samples) };
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

impl<P: Descriptor + 'static, Presets> ComponentFactory for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    fn create(config: &'static Config) -> Option<Self> {
        let processor = Self::new(config);

        // Two parameters with one id would be one parameter to the host,
        // and so would a parameter with an id of the wrapper's own. The
        // reason also goes to standard error: a plugin has no logger unless
        // its author installs one.
        let parameters: &dyn beamer_core::parameter_types::Parameters = &*processor.parameters;
        let checked = beamer_core::parameter_types::check_unique_ids(parameters)
            .and_then(|()| check_reserved_ids(parameters));
        if let Err(error) = checked {
            log::error!("{}: {}", config.name, error);
            eprintln!("{}: {}", config.name, error);
            return None;
        }

        Some(processor)
    }
}

impl<P: Descriptor + 'static, Presets> Class for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
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

impl<P: Descriptor + 'static, Presets> IPluginBaseTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
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

impl<P: Descriptor + 'static, Presets> IComponentTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    unsafe fn getControllerClassId(&self, class_id: *mut TUID) -> tresult {
        if class_id.is_null() {
            return kInvalidArgument;
        }

        // For combined component, return the controller UID if set, otherwise kNotImplemented
        if let Some(parts) = self.config.vst3_controller_uid_parts() {
            let controller = vst3::uid(parts[0], parts[1], parts[2], parts[3]);
            // SAFETY: Validated class_id is non-null above. Host guarantees validity.
            unsafe { *class_id = controller };
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
                    // SAFETY: Validated bus is non-null above. Host guarantees validity.
                    let bus = unsafe { &mut *bus };
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
                // Only index 0 for event bus and only if plugin wants MIDI
                if index != 0 || !self.wants_midi() {
                    return kInvalidArgument;
                }

                // SAFETY: Validated bus is non-null above. Host guarantees validity.
                let bus = unsafe { &mut *bus };
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
        // SAFETY: VST3 guarantees single-threaded access. No aliasing.
        if let Some(processor) = unsafe { &mut *self.processor.get() } {
            processor.set_active(state != 0);
            // A host asks for the latency after activating.
            self.publish_timing(processor);
        }
        // When unprepared, silently succeed (host may call this before setupProcessing)
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        if state.is_null() {
            return kInvalidArgument;
        }

        // SAFETY: state is non-null and host guarantees it points to valid IBStream.
        let stream = match unsafe { ComRef::from_raw(state) } {
            Some(s) => s,
            None => return kInvalidArgument,
        };

        // Read all bytes from stream
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let mut bytes_read: i32 = 0;
            // SAFETY: stream is valid ComRef, chunk is valid buffer.
            let result = unsafe {
                stream.read(
                    chunk.as_mut_ptr() as *mut c_void,
                    chunk.len() as i32,
                    &mut bytes_read,
                )
            };

            if result != kResultOk || bytes_read <= 0 {
                break;
            }

            buffer.extend_from_slice(&chunk[..bytes_read as usize]);
        }

        if buffer.is_empty() {
            return kResultOk;
        }

        // The state is the parameters. Their values are atomic, so they are
        // loaded through a shared reference: in both states, and also while
        // process() runs, which is when a host loads a preset during playback.
        use beamer_core::Parameters;
        match self.parameters().load_state(&buffer) {
            Ok(()) => {
                // The smoothers belong to the audio thread: the next process()
                // call makes them jump to the loaded values, so that those do
                // not ramp in.
                self.smoothing_reset_requested.store(true, Ordering::Release);
                kResultOk
            }
            Err(_) => kResultFalse,
        }
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        if state.is_null() {
            return kInvalidArgument;
        }

        // The state is the parameters. Their values are atomic, so they are
        // saved through a shared reference: in both states, and also while
        // process() runs, which is when a host saves during playback.
        use beamer_core::Parameters;
        let data: Vec<u8> = self.parameters().save_state();

        if data.is_empty() {
            return kResultOk;
        }

        // Write to IBStream
        // SAFETY: state is non-null and host guarantees it points to valid IBStream.
        let stream = match unsafe { ComRef::from_raw(state) } {
            Some(s) => s,
            None => return kInvalidArgument,
        };
        let mut bytes_written: i32 = 0;
        // SAFETY: stream is valid ComRef, data is valid slice.
        let result = unsafe {
            stream.write(
                data.as_ptr() as *mut c_void,
                data.len() as i32,
                &mut bytes_written,
            )
        };

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

impl<P: Descriptor + 'static, Presets> IAudioProcessorTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
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
        let input_count = self.input_bus_count();
        let output_count = self.output_bus_count();
        if num_ins as usize != input_count || num_outs as usize != output_count {
            return kResultFalse;
        }

        // Validate each input bus
        for i in 0..num_ins as usize {
            // Early rejection: channel count exceeds compile-time limits
            // SAFETY: inputs is non-null (checked above) and host guarantees validity for num_ins.
            let requested = unsafe { *inputs.add(i) };
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
            // SAFETY: outputs is non-null (checked above) and host guarantees validity for num_outs.
            let requested = unsafe { *outputs.add(i) };
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
            // SAFETY: arr is non-null (checked above) and host guarantees validity.
            unsafe { *arr = channel_count_to_speaker_arrangement(info.channel_count) };
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
        self.latency_samples.load(Ordering::Relaxed)
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        if setup.is_null() {
            return kInvalidArgument;
        }

        // SAFETY: setup is non-null and host guarantees it points to valid ProcessSetup.
        let setup = unsafe { &*setup };

        // Compare with the setup of the previous call before it is replaced.
        // SAFETY: VST3 guarantees single-threaded access during setupProcessing(). No aliasing.
        let setup_changed = unsafe {
            *self.sample_rate.get() != setup.sampleRate
                || *self.max_block_size.get() != setup.maxSamplesPerBlock as usize
                || *self.symbolic_sample_size.get() != setup.symbolicSampleSize
                || *self.process_mode.get() != setup.processMode
        };

        // Host threads keep reading the descriptor while it creates the
        // processor below.
        let plugin = self.descriptor();
        let plugin = &*plugin;
        // SAFETY: VST3 guarantees single-threaded access during setupProcessing(). No aliasing.
        let slot = unsafe { &mut *self.processor.get() };

        // A host may repeat a setup. Preparing again would then only throw
        // away what the processor holds. With a new sample rate, block size,
        // sample size or process mode the plugin is prepared again: it
        // learns those when a processor is created.
        if slot.is_none() || setup_changed {
            let input_bus_count = plugin.input_bus_count();
            let output_bus_count = plugin.output_bus_count();
            let input_buses: Vec<CoreBusInfo> = (0..input_bus_count)
                .filter_map(|i| plugin.input_bus_info(i))
                .collect();
            let output_buses: Vec<CoreBusInfo> = (0..output_bus_count)
                .filter_map(|i| plugin.output_bus_info(i))
                .collect();

            let bus_layout = BusLayout::from_plugin(plugin);

            // Validate plugin's bus configuration against compile-time limits
            if let Err(msg) = CachedBusConfig::from_plugin(plugin).validate() {
                log::error!("Plugin bus configuration exceeds limits: {}", msg);
                return kResultFalse;
            }

            // Store the setup. A setup that was refused above is not kept,
            // so that it does not count as the current one when it comes
            // again.
            // SAFETY: VST3 guarantees single-threaded access during setupProcessing(). No aliasing.
            unsafe {
                *self.sample_rate.get() = setup.sampleRate;
                *self.max_block_size.get() = setup.maxSamplesPerBlock as usize;
                *self.symbolic_sample_size.get() = setup.symbolicSampleSize;
                *self.process_mode.get() = setup.processMode;
            }

            // Build the plugin setup
            let plugin_setup = build_setup::<P::Setup>(setup, &bus_layout);

            // A processor of an earlier setup goes first, so that its
            // buffers are free before the new ones are allocated.
            *slot = None;

            // The smoothers ramp at the host's rate unless the plugin
            // sets another one in prepare.
            beamer_core::Parameters::set_sample_rate(self.parameters(), setup.sampleRate);

            // Create the processor. A state that was loaded before is in
            // the parameters it shares with the descriptor.
            let processor = plugin.prepare(plugin_setup);

            // Pre-allocate buffer storage based on bus config
            let bus_config = CachedBusConfig::new(
                input_buses.iter().map(CachedBusInfo::from_bus_info).collect(),
                output_buses.iter().map(CachedBusInfo::from_bus_info).collect(),
            );
            let max_frames = setup.maxSamplesPerBlock as usize;
            // SAFETY: VST3 guarantees single-threaded access. No aliasing.
            unsafe {
                *self.buffer_storage_f32.get() =
                    ProcessBufferStorage::allocate_from_config(&bus_config, max_frames);
                *self.buffer_storage_f64.get() =
                    ProcessBufferStorage::allocate_from_config(&bus_config, max_frames);
            }

            // Pre-allocate conversion buffers for f64→f32 processing
            if setup.symbolicSampleSize == SymbolicSampleSizes_::kSample64 as i32
                && !processor.supports_double_precision()
            {
                // SAFETY: VST3 guarantees single-threaded access. No aliasing.
                unsafe {
                    *self.conversion_buffers.get() =
                        ConversionBuffers::allocate_from_buses(&input_buses, &output_buses, max_frames);
                }
            }

            *slot = Some(processor);
        }

        // The host may ask for latency and tail before the first process() call.
        if let Some(processor) = slot {
            self.publish_timing(processor);
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

        // SAFETY: data is non-null and host guarantees it points to valid ProcessData.
        let process_data = unsafe { &*data };
        let num_samples = process_data.numSamples as usize;

        // A state was loaded since the last block: the smoothers jump to the
        // loaded values before anything else happens to the parameters. They
        // belong to this thread.
        if self.smoothing_reset_requested.swap(false, Ordering::AcqRel) {
            use beamer_core::Parameters;
            self.parameters().reset_smoothing();
        }

        // 1. Handle incoming parameter changes from host. This needs the
        // parameters only, so it also works for a call without samples.
        // SAFETY: inputParameterChanges may be null; ComRef::from_raw handles this.
        if let Some(parameter_changes) = unsafe { ComRef::from_raw(process_data.inputParameterChanges) } {
            let parameters = self.parameters();
            // SAFETY: parameter_changes is valid ComRef.
            let parameter_count = unsafe { parameter_changes.getParameterCount() };

            for i in 0..parameter_count {
                // SAFETY: getParameterData may return null; ComRef::from_raw handles this.
                if let Some(queue) = unsafe { ComRef::from_raw(parameter_changes.getParameterData(i)) } {
                    // SAFETY: queue is valid ComRef.
                    let parameter_id = unsafe { queue.getParameterId() };
                    // SAFETY: queue is valid ComRef.
                    let point_count = unsafe { queue.getPointCount() };

                    if point_count > 0 {
                        let mut sample_offset = 0;
                        let mut value = 0.0;
                        // Get the last value in the queue (simplest approach)
                        // SAFETY: queue is valid, sample_offset and value are valid pointers.
                        if unsafe { queue.getPoint(point_count - 1, &mut sample_offset, &mut value) }
                            == kResultTrue
                        {
                            parameters.set_normalized(parameter_id, value);
                        }
                    }
                }
            }
        }

        // Host threads do not touch the processor while audio runs. What they
        // want to know about it is published for them with each call, after
        // the parameter changes that may have changed it.
        // SAFETY: process() has the processor to itself.
        if let Some(processor) = unsafe { &*self.processor.get() } {
            self.publish_timing(processor);
        }

        // A host sends a block without samples to flush parameter changes
        // while it does not process audio. The parameter values are applied
        // above. Controller changes that arrive as parameter changes become
        // MIDI events only in a block with samples.
        if num_samples == 0 {
            return kResultOk;
        }

        // 2. Handle MIDI events (reuse pre-allocated buffer to avoid stack overflow)
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let midi_input = unsafe { &mut *self.midi_input.get() };
        midi_input.clear();

        // SAFETY: inputEvents may be null; ComRef::from_raw handles this.
        if let Some(event_list) = unsafe { ComRef::from_raw(process_data.inputEvents) } {
            // SAFETY: event_list is valid ComRef.
            let event_count = unsafe { event_list.getEventCount() };

            for i in 0..event_count {
                // SAFETY: zeroed Event is valid for VST3 Event union type.
                let mut event: Event = unsafe { std::mem::zeroed() };
                // SAFETY: event_list is valid, event is valid mutable pointer.
                if unsafe { event_list.getEvent(i, &mut event) } == kResultOk {
                    // SAFETY: event is valid Event populated by getEvent.
                    if let Some(midi_event) = unsafe { convert_vst3_to_midi(&event) } {
                        midi_input.push(midi_event);
                    }
                }
            }
        }

        // 2.5. Convert MIDI CC parameter changes to MIDI events
        // This handles the VST3 IMidiMapping flow where DAWs send CC/pitch bend
        // as parameter changes instead of raw MIDI events.
        // Uses framework-owned MidiCcState.
        // SAFETY: inputParameterChanges may be null; ComRef::from_raw handles this.
        if let Some(parameter_changes) = unsafe { ComRef::from_raw(process_data.inputParameterChanges) } {
            if let Some(cc_state) = self.midi_cc_state.as_ref() {
                // SAFETY: parameter_changes is valid ComRef.
                let parameter_count = unsafe { parameter_changes.getParameterCount() };

                for i in 0..parameter_count {
                    // SAFETY: getParameterData may return null; ComRef::from_raw handles this.
                    if let Some(queue) = unsafe { ComRef::from_raw(parameter_changes.getParameterData(i)) } {
                        // SAFETY: queue is valid ComRef.
                        let parameter_id = unsafe { queue.getParameterId() };

                        // Check if this is a MIDI CC parameter
                        if let Some(controller) = MidiCcState::parameter_id_to_controller(parameter_id) {
                            if cc_state.has_controller(controller) {
                                // SAFETY: queue is valid ComRef.
                                let point_count = unsafe { queue.getPointCount() };

                                // Process all points for sample-accurate timing
                                for j in 0..point_count {
                                    let mut sample_offset: i32 = 0;
                                    let mut value: f64 = 0.0;

                                    // SAFETY: queue is valid, sample_offset and value are valid pointers.
                                    if unsafe { queue.getPoint(j, &mut sample_offset, &mut value) } == kResultOk {
                                        let midi_event = convert_cc_parameter_to_midi(
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
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let midi_output = unsafe { &mut *self.midi_output.get() };
        midi_output.clear();
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let sysex_pool = unsafe { &mut *self.sysex_output_pool.get() };

        // Clear pool FIRST so next_slot is reset to 0 before draining fallback
        sysex_pool.clear();

        // With heap fallback enabled, emit any overflow messages from previous block first.
        // These allocate slots starting from 0; new plugin output will append after them.
        #[cfg(feature = "sysex-heap-fallback")]
        if sysex_pool.has_fallback() {
            // SAFETY: outputEvents may be null; ComRef::from_raw handles this.
            if let Some(event_list) = unsafe { ComRef::from_raw(process_data.outputEvents) } {
                for sysex_data in sysex_pool.take_fallback() {
                    // Allocate from pool (succeeds since we just cleared it)
                    if let Some((ptr, len)) = sysex_pool.allocate(&sysex_data) {
                        // SAFETY: zeroed Event is valid for VST3 Event union type.
                        let mut event: Event = unsafe { std::mem::zeroed() };
                        event.busIndex = 0;
                        event.sampleOffset = 0; // Delayed message, emit at start of block
                        event.ppqPosition = 0.0;
                        event.flags = 0;
                        event.r#type = K_DATA_EVENT;
                        event.__field0.data.r#type = DATA_TYPE_MIDI_SYSEX;
                        event.__field0.data.size = len as u32;
                        event.__field0.data.bytes = ptr;
                        // SAFETY: event_list is valid ComRef, event is valid mutable pointer.
                        let _ = unsafe { event_list.addEvent(&mut event) };
                    }
                }
            }
            // Log that we recovered from overflow
            warn!(
                "SysEx fallback: emitted delayed messages from previous block overflow"
            );
        }
        // NOTE: Don't clear again - fallback events occupy slots 0..N, new events append after

        // Process MIDI events (process_midi is on Processor)
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let processor = unsafe { self.processor_mut() };
        processor.process_midi(midi_input.as_slice(), midi_output);

        // Write output MIDI events
        // SAFETY: outputEvents may be null; ComRef::from_raw handles this.
        if let Some(event_list) = unsafe { ComRef::from_raw(process_data.outputEvents) } {
            for midi_event in midi_output.iter() {
                if let Some(mut vst3_event) = convert_midi_to_vst3(midi_event, sysex_pool) {
                    // SAFETY: event_list is valid ComRef, vst3_event is valid mutable pointer.
                    let _ = unsafe { event_list.addEvent(&mut vst3_event) };
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
        // SAFETY: processContext may be null; extract_transport handles this.
        let transport = unsafe { extract_transport(process_data.processContext) };
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let sample_rate = unsafe { *self.sample_rate.get() };
        let context = if let Some(cc_state) = self.midi_cc_state.as_ref() {
            CoreProcessContext::with_midi_cc(sample_rate, num_samples, transport, cc_state)
        } else {
            CoreProcessContext::new(sample_rate, num_samples, transport)
        };

        // 4. Process audio based on sample size
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let symbolic_sample_size = unsafe { *self.symbolic_sample_size.get() };
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let processor = unsafe { self.processor_mut() };

        if symbolic_sample_size == SymbolicSampleSizes_::kSample64 as i32 {
            // 64-bit processing path
            if processor.supports_double_precision() {
                // Native f64: extract f64 buffers and call process_f64()
                // SAFETY: process_data is valid, processor is valid mutable reference.
                unsafe { self.process_audio_f64_native(process_data, num_samples, processor, &context) };
            } else {
                // Conversion: f64→f32, process, f32→f64
                // SAFETY: process_data is valid, processor is valid mutable reference.
                unsafe { self.process_audio_f64_converted(process_data, num_samples, processor, &context) };
            }
        } else {
            // 32-bit processing path (default)
            // SAFETY: process_data is valid, processor is valid mutable reference.
            unsafe { self.process_audio_f32(process_data, num_samples, processor, &context) };
        }

        kResultOk
    }

    unsafe fn getTailSamples(&self) -> u32 {
        // The tail and the bypass ramp of the processor (see `publish_timing`).
        self.tail_samples.load(Ordering::Relaxed)
    }
}

impl<P: Descriptor + 'static, Presets> IProcessContextRequirementsTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
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

impl<P: Descriptor + 'static, Presets> IEditControllerTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
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
        let user_parameters = self.parameters().count();
        // MIDI CC state is framework-owned, always available
        let cc_parameters = self
            .midi_cc_state
            .as_ref()
            .map(|s| s.enabled_count())
            .unwrap_or(0);
        // Add program change parameter if we have factory presets
        let preset_parameter = if Presets::count() > 0 { 1 } else { 0 };
        (user_parameters + cc_parameters + preset_parameter) as i32
    }

    unsafe fn getParameterInfo(&self, parameter_index: i32, info: *mut ParameterInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let parameters = self.parameters();
        let user_parameter_count = parameters.count();

        // User-defined parameters first
        if (parameter_index as usize) < user_parameter_count {
            if let Some(parameter_info) = parameters.info(parameter_index as usize) {
                // SAFETY: info is non-null (checked above) and host guarantees validity.
                let info = unsafe { &mut *info };
                info.id = parameter_info.id;
                copy_wstring(parameter_info.name, &mut info.title);
                copy_wstring(parameter_info.short_name, &mut info.shortTitle);
                copy_wstring(parameter_info.units, &mut info.units);
                info.stepCount = parameter_info.step_count;
                info.defaultNormalizedValue = parameter_info.default_normalized;
                info.unitId = parameter_info.group_id;
                info.flags = {
                    let mut flags = 0;
                    if parameter_info.flags.can_automate {
                        flags |= ParameterInfo_::ParameterFlags_::kCanAutomate;
                    }
                    if parameter_info.flags.is_bypass {
                        flags |= ParameterInfo_::ParameterFlags_::kIsBypass;
                    }
                    // List parameters (enums) - display as dropdown with text labels
                    if parameter_info.flags.is_list {
                        flags |= ParameterInfo_::ParameterFlags_::kIsList;
                    }
                    // Hidden parameters (MIDI CC emulation)
                    if parameter_info.flags.is_hidden {
                        flags |= ParameterInfo_::ParameterFlags_::kIsHidden;
                    }
                    flags
                };
                return kResultOk;
            }
            return kInvalidArgument;
        }

        // Hidden MIDI CC parameters (framework-owned state)
        let cc_parameter_count = self
            .midi_cc_state
            .as_ref()
            .map(|s| s.enabled_count())
            .unwrap_or(0);

        if let Some(cc_state) = self.midi_cc_state.as_ref() {
            let cc_index = (parameter_index as usize) - user_parameter_count;
            if cc_index < cc_parameter_count {
                if let Some(parameter_info) = cc_state.info(cc_index) {
                    // SAFETY: info is non-null (checked above) and host guarantees validity.
                    let info = unsafe { &mut *info };
                    info.id = parameter_info.id;
                    copy_wstring(parameter_info.name, &mut info.title);
                    copy_wstring(parameter_info.short_name, &mut info.shortTitle);
                    copy_wstring(parameter_info.units, &mut info.units);
                    info.stepCount = parameter_info.step_count;
                    info.defaultNormalizedValue = parameter_info.default_normalized;
                    info.unitId = parameter_info.group_id;
                    // Hidden + automatable
                    info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate
                        | ParameterInfo_::ParameterFlags_::kIsHidden;
                    return kResultOk;
                }
            }
        }

        // Program change parameter for factory presets (after all other parameters)
        let preset_count = Presets::count();
        if preset_count > 0 {
            let preset_param_index = user_parameter_count + cc_parameter_count;
            if parameter_index as usize == preset_param_index {
                // SAFETY: info is non-null (checked above) and host guarantees validity.
                let info = unsafe { &mut *info };
                info.id = PROGRAM_CHANGE_PARAM_ID;
                copy_wstring("Program", &mut info.title);
                copy_wstring("Prg", &mut info.shortTitle);
                copy_wstring("", &mut info.units);
                info.stepCount = (preset_count - 1) as i32; // Discrete steps
                info.defaultNormalizedValue = 0.0; // First preset is default
                info.unitId = 0; // Root unit
                // Program change + list (shows preset names in host)
                info.flags = ParameterInfo_::ParameterFlags_::kIsProgramChange
                    | ParameterInfo_::ParameterFlags_::kIsList;
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

        // Handle program change parameter (preset names)
        if id == PROGRAM_CHANGE_PARAM_ID {
            let preset_count = Presets::count();
            if preset_count > 0 {
                // Convert normalized value to preset index
                let step_count = (preset_count - 1).max(1) as f64;
                let preset_index = (value_normalized * step_count).round() as usize;
                let preset_index = preset_index.min(preset_count - 1);

                if let Some(preset_info) = Presets::info(preset_index) {
                    // SAFETY: string is non-null (checked above) and host guarantees validity.
                    copy_wstring(preset_info.name, unsafe { &mut *string });
                    return kResultOk;
                }
            }
            // SAFETY: string is non-null (checked above) and host guarantees validity.
            copy_wstring("", unsafe { &mut *string });
            return kResultOk;
        }

        let parameters = self.parameters();
        let display = parameters.normalized_to_string(id, value_normalized);
        // SAFETY: string is non-null (checked above) and host guarantees validity.
        copy_wstring(&display, unsafe { &mut *string });
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

        // SAFETY: string is non-null (checked above) and is null-terminated.
        let len = unsafe { len_wstring(string as *const TChar) };
        // SAFETY: string is valid for len elements (len_wstring counts to null terminator).
        if let Ok(s) = String::from_utf16(unsafe { slice::from_raw_parts(string as *const u16, len) }) {
            // Handle program change parameter (preset name to value)
            if id == PROGRAM_CHANGE_PARAM_ID {
                let preset_count = Presets::count();
                // Find preset by name
                for i in 0..preset_count {
                    if let Some(preset_info) = Presets::info(i) {
                        if preset_info.name == s {
                            let step_count = (preset_count - 1).max(1) as f64;
                            // SAFETY: value_normalized is non-null (checked above).
                            unsafe { *value_normalized = (i as f64) / step_count };
                            return kResultOk;
                        }
                    }
                }
                return kInvalidArgument;
            }

            let parameters = self.parameters();
            if let Some(value) = parameters.string_to_normalized(id, &s) {
                // SAFETY: value_normalized is non-null (checked above).
                unsafe { *value_normalized = value };
                return kResultOk;
            }
        }
        kInvalidArgument
    }

    unsafe fn normalizedParamToPlain(&self, id: u32, value_normalized: f64) -> f64 {
        // Handle program change parameter (normalized to index)
        if id == PROGRAM_CHANGE_PARAM_ID {
            let preset_count = Presets::count();
            if preset_count > 0 {
                let step_count = (preset_count - 1).max(1) as f64;
                return (value_normalized * step_count).round();
            }
            return 0.0;
        }
        self.parameters().normalized_to_plain(id, value_normalized)
    }

    unsafe fn plainParamToNormalized(&self, id: u32, plain_value: f64) -> f64 {
        // Handle program change parameter (index to normalized)
        if id == PROGRAM_CHANGE_PARAM_ID {
            let preset_count = Presets::count();
            if preset_count > 1 {
                let step_count = (preset_count - 1) as f64;
                return plain_value / step_count;
            }
            return 0.0;
        }
        self.parameters().plain_to_normalized(id, plain_value)
    }

    unsafe fn getParamNormalized(&self, id: u32) -> f64 {
        // Check if this is a MIDI CC parameter
        if MidiCcState::is_midi_cc_parameter(id) {
            if let Some(cc_state) = self.midi_cc_state.as_ref() {
                return cc_state.get_normalized(id);
            }
        }

        // Check if this is the program change parameter
        if id == PROGRAM_CHANGE_PARAM_ID {
            let preset_count = Presets::count();
            if preset_count > 1 {
                // SAFETY: VST3 guarantees single-threaded access. No aliasing.
                let current_index = unsafe { *self.current_preset_index.get() };
                let step_count = (preset_count - 1) as f64;
                return (current_index as f64) / step_count;
            } else if preset_count == 1 {
                return 0.0;
            }
            return 0.0;
        }

        self.parameters().get_normalized(id)
    }

    unsafe fn setParamNormalized(&self, id: u32, value: f64) -> tresult {
        // Check if this is a MIDI CC parameter
        if MidiCcState::is_midi_cc_parameter(id) {
            if let Some(cc_state) = self.midi_cc_state.as_ref() {
                cc_state.set_normalized(id, value);
                return kResultOk;
            }
        }

        // Check if this is the program change parameter (preset selection)
        if id == PROGRAM_CHANGE_PARAM_ID {
            let preset_count = Presets::count();
            if preset_count > 0 {
                // Convert normalized value to preset index
                // stepCount = preset_count - 1, so index = round(value * stepCount)
                let step_count = (preset_count - 1) as f64;
                let preset_index = (value * step_count).round() as usize;
                let preset_index = preset_index.min(preset_count - 1);

                // Always apply unconditionally - never skip with "if changed" guard.
                // Hosts may re-send the same preset index (e.g., user clicks preset 0
                // when it's already selected) and skipping would break preset 0 on
                // fresh load when current_preset_index is initialized to 0.
                Presets::apply(preset_index, self.parameters());

                // Store the current preset index
                // SAFETY: VST3 guarantees single-threaded access. No aliasing.
                unsafe { *self.current_preset_index.get() = preset_index as i32 };

                // Notify host that parameter values changed so UI refreshes
                // SAFETY: VST3 guarantees single-threaded access. No aliasing.
                let handler = unsafe { *self.component_handler.get() };
                if !handler.is_null() {
                    // SAFETY: handler is non-null and is valid COM pointer with valid vtbl.
                    unsafe {
                        ((*(*handler).vtbl).restartComponent)(
                            handler,
                            RestartFlags_::kParamValuesChanged,
                        );
                    }
                }

                return kResultOk;
            }
            return kInvalidArgument;
        }

        self.parameters().set_normalized(id, value);
        kResultOk
    }

    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        let handler_ptr = self.component_handler.get();
        // SAFETY: VST3 guarantees single-threaded access. No aliasing.
        let old_handler = unsafe { *handler_ptr };

        // Release old handler if present
        if !old_handler.is_null() {
            let unknown = old_handler as *mut FUnknown;
            // SAFETY: old_handler is non-null and is valid COM pointer with valid vtbl.
            unsafe { ((*(*unknown).vtbl).release)(unknown) };
        }

        // Store and AddRef new handler if present
        if !handler.is_null() {
            let unknown = handler as *mut FUnknown;
            // SAFETY: handler is non-null and is valid COM pointer with valid vtbl.
            unsafe { ((*(*unknown).vtbl).addRef)(unknown) };
        }

        // SAFETY: VST3 guarantees single-threaded access. No aliasing.
        unsafe { *handler_ptr = handler };
        kResultOk
    }

    unsafe fn createView(&self, name: *const c_char) -> *mut IPlugView {
        if name.is_null() {
            return std::ptr::null_mut();
        }

        // SAFETY: name is non-null (checked above) and is null-terminated C string.
        let name_str = unsafe { std::ffi::CStr::from_ptr(name) }.to_str().unwrap_or("");
        if name_str != "editor" || !self.config.has_gui {
            return std::ptr::null_mut();
        }

        #[cfg(feature = "webview")]
        {
            use beamer_webview::WebViewConfig;

            if self.config.gui_url.is_none() && self.config.gui_assets.is_none() {
                return std::ptr::null_mut();
            }

            let config = WebViewConfig {
                assets: self.config.gui_assets,
                url: self.config.gui_url,
                dev_tools: cfg!(debug_assertions),
                background_color: self.config.gui_background_color,
                message_callback: None,
                loaded_callback: None,
                callback_context: std::ptr::null_mut(),
            };
            debug_assert!(
                self.config.gui_width > 0 && self.config.gui_height > 0,
                "gui_size must be set when has_gui is true"
            );
            let size = beamer_core::Size::new(self.config.gui_width, self.config.gui_height);
            let constraints = beamer_core::GuiConstraints {
                min: size,
                ..beamer_core::GuiConstraints::default()
            };
            let delegate = Box::new(crate::webview::StaticGuiDelegate::new(size, constraints));

            // SAFETY: VST3 guarantees single-threaded access. No aliasing.
            let component_handler = unsafe { *self.component_handler.get() };

            // SAFETY: component_handler is null or the handler the host set.
            let view = unsafe {
                crate::webview::WebViewPlugView::new(
                    config,
                    delegate,
                    self.shared_parameter_store(),
                    component_handler,
                    self.webview_handler.clone(),
                    self.webview_events.clone(),
                )
            };
            let wrapper = vst3::ComWrapper::new(view);
            match wrapper.to_com_ptr::<IPlugView>() {
                Some(ptr) => ptr.into_raw(),
                None => std::ptr::null_mut(),
            }
        }

        #[cfg(not(feature = "webview"))]
        {
            std::ptr::null_mut()
        }
    }
}

// =============================================================================
// IUnitInfo implementation (VST3 Unit/Group hierarchy)
// =============================================================================

impl<P: Descriptor + 'static, Presets> IUnitInfoTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    unsafe fn getUnitCount(&self) -> i32 {
        use beamer_core::parameter_groups::ParameterGroups;
        self.parameters().group_count() as i32
    }

    unsafe fn getUnitInfo(&self, unit_index: i32, info: *mut UnitInfo) -> tresult {
        if info.is_null() || unit_index < 0 {
            return kInvalidArgument;
        }

        use beamer_core::parameter_groups::ParameterGroups;
        let parameters = self.parameters();

        if let Some(group_info) = parameters.group_info(unit_index as usize) {
            // SAFETY: info is non-null (checked above) and host guarantees validity.
            let info = unsafe { &mut *info };
            info.id = group_info.id;
            info.parentUnitId = group_info.parent_id;
            // Assign program list to root unit if we have presets
            info.programListId = if group_info.id == 0 && Presets::count() > 0 {
                FACTORY_PRESETS_LIST_ID
            } else {
                kNoProgramListId
            };
            copy_wstring(group_info.name, &mut info.name);
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn getProgramListCount(&self) -> i32 {
        if Presets::count() > 0 {
            1 // One program list for factory presets
        } else {
            0
        }
    }

    unsafe fn getProgramListInfo(
        &self,
        list_index: i32,
        info: *mut ProgramListInfo,
    ) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        // Only support our single factory presets list
        if list_index != 0 || Presets::count() == 0 {
            return kInvalidArgument;
        }

        // SAFETY: info is non-null (checked above) and host guarantees validity.
        let info = unsafe { &mut *info };
        info.id = FACTORY_PRESETS_LIST_ID;
        info.programCount = Presets::count() as i32;
        copy_wstring("Factory Presets", &mut info.name);

        kResultOk
    }

    unsafe fn getProgramName(&self, list_id: i32, program_index: i32, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }

        // Only support our factory presets list
        if list_id != FACTORY_PRESETS_LIST_ID {
            return kInvalidArgument;
        }

        if let Some(preset_info) = Presets::info(program_index as usize) {
            // SAFETY: name is non-null (checked above) and host guarantees validity.
            copy_wstring(preset_info.name, unsafe { &mut *name });
            kResultOk
        } else {
            kInvalidArgument
        }
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

impl<P: Descriptor + 'static, Presets> IMidiMappingTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
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

        // 1. First check plugin's custom mappings.
        {
            let plugin = self.descriptor();
            if let Some(parameter_id) = plugin.midi_cc_to_parameter(bus_index, channel, controller) {
                // SAFETY: id is non-null (checked above) and host guarantees validity.
                unsafe { *id = parameter_id };
                return kResultOk;
            }
        }

        // 2. Check framework-owned MIDI CC state (omni channel - ignore channel parameter)
        if let Some(cc_state) = self.midi_cc_state.as_ref() {
            if cc_state.has_controller(controller) {
                // SAFETY: id is non-null (checked above) and host guarantees validity.
                unsafe { *id = MidiCcState::parameter_id(controller) };
                return kResultOk;
            }
        }

        kResultFalse
    }
}

// =============================================================================
// IMidiLearn implementation (VST3 SDK 3.8.0)
// =============================================================================

impl<P: Descriptor + 'static, Presets> IMidiLearnTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    unsafe fn onLiveMIDIControllerInput(
        &self,
        bus_index: i32,
        channel: i16,
        midi_cc: i16,
    ) -> tresult {
        {
            let mut plugin = self.descriptor_mut();
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

impl<P: Descriptor + 'static, Presets> IMidiMapping2Trait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    unsafe fn getNumMidi1ControllerAssignments(&self, direction: BusDirections) -> u32 {
        // Only support input direction
        if direction != BusDirections_::kInput {
            return 0;
        }
        self.descriptor().midi1_assignments().len() as u32
    }

    unsafe fn getMidi1ControllerAssignments(
        &self,
        direction: BusDirections,
        list: *const Midi1ControllerParamIDAssignmentList,
    ) -> tresult {
        if list.is_null() || direction != BusDirections_::kInput {
            return kInvalidArgument;
        }

        let plugin = self.descriptor();
        let assignments = plugin.midi1_assignments();
        // SAFETY: list is non-null (checked above) and host guarantees validity.
        let list_ref = unsafe { &*list };

        if (list_ref.count as usize) < assignments.len() {
            return kResultFalse;
        }

        if assignments.is_empty() {
            return kResultOk;
        }

        // SAFETY: list_ref.map is valid for list_ref.count elements per host contract.
        let map = unsafe { slice::from_raw_parts_mut(list_ref.map, assignments.len()) };
        for (i, a) in assignments.iter().enumerate() {
            map[i] = Midi1ControllerParamIDAssignment {
                pId: a.assignment.parameter_id,
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
        self.descriptor().midi2_assignments().len() as u32
    }

    unsafe fn getMidi2ControllerAssignments(
        &self,
        direction: BusDirections,
        list: *const Midi2ControllerParamIDAssignmentList,
    ) -> tresult {
        if list.is_null() || direction != BusDirections_::kInput {
            return kInvalidArgument;
        }

        let plugin = self.descriptor();
        let assignments = plugin.midi2_assignments();
        // SAFETY: list is non-null (checked above) and host guarantees validity.
        let list_ref = unsafe { &*list };

        if (list_ref.count as usize) < assignments.len() {
            return kResultFalse;
        }

        if assignments.is_empty() {
            return kResultOk;
        }

        // SAFETY: list_ref.map is valid for list_ref.count elements per host contract.
        let map = unsafe { slice::from_raw_parts_mut(list_ref.map, assignments.len()) };
        for (i, a) in assignments.iter().enumerate() {
            map[i] = Midi2ControllerParamIDAssignment {
                pId: a.assignment.parameter_id,
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

impl<P: Descriptor + 'static, Presets> IMidiLearn2Trait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    unsafe fn onLiveMidi1ControllerInput(
        &self,
        bus_index: i32,
        channel: u8,
        midi_cc: i16,
    ) -> tresult {
        {
            let mut plugin = self.descriptor_mut();
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
        {
            let mut plugin = self.descriptor_mut();
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

impl<P: Descriptor + 'static, Presets> INoteExpressionControllerTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    unsafe fn getNoteExpressionCount(&self, bus_index: i32, channel: i16) -> i32 {
        self.descriptor().note_expression_count(bus_index, channel) as i32
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

        let plugin = self.descriptor();
        if let Some(expr_info) =
            plugin.note_expression_info(bus_index, channel, note_expression_index as usize)
        {
            // SAFETY: info is non-null (checked above) and host guarantees validity.
            let vst_info = unsafe { &mut *info };
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

        let plugin = self.descriptor();
        let display = plugin.note_expression_value_to_string(bus_index, channel, id, value_normalized);
        // SAFETY: string is non-null (checked above) and host guarantees validity.
        copy_wstring(&display, unsafe { &mut *string });
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

        // SAFETY: string is non-null (checked above) and is null-terminated.
        let len = unsafe { len_wstring(string) };
        // SAFETY: string is valid for len elements.
        if let Ok(s) = String::from_utf16(unsafe { slice::from_raw_parts(string, len) }) {
            {
                let plugin = self.descriptor();
                if let Some(value) = plugin.note_expression_string_to_value(bus_index, channel, id, &s) {
                    // SAFETY: value_normalized is non-null (checked above).
                    unsafe { *value_normalized = value };
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

impl<P: Descriptor + 'static, Presets> IKeyswitchControllerTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    unsafe fn getKeyswitchCount(&self, bus_index: i32, channel: i16) -> i32 {
        self.descriptor().keyswitch_count(bus_index, channel) as i32
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

        let plugin = self.descriptor();
        if let Some(ks_info) =
            plugin.keyswitch_info(bus_index, channel, keyswitch_index as usize)
        {
            // SAFETY: info is non-null (checked above) and host guarantees validity.
            let vst_info = unsafe { &mut *info };
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

impl<P: Descriptor + 'static, Presets> INoteExpressionPhysicalUIMappingTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
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

        let plugin = self.descriptor();
        let mappings = plugin.physical_ui_mappings(bus_index, channel);
        // SAFETY: list is non-null (checked above) and host guarantees validity.
        let list_ref = unsafe { &mut *list };

        // Fill in the mappings up to the provided count
        let fill_count = (list_ref.count as usize).min(mappings.len());
        if fill_count > 0 && !list_ref.map.is_null() {
            // SAFETY: list_ref.map is valid for list_ref.count elements per host contract.
            let map_slice = unsafe { slice::from_raw_parts_mut(list_ref.map, fill_count) };
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

impl<P: Descriptor + 'static, Presets> IVst3WrapperMPESupportTrait for Vst3Processor<P, Presets>
where
    Presets: FactoryPresets<Parameters = P::Parameters>,
{
    unsafe fn enableMPEInputProcessing(&self, state: TBool) -> tresult {
        {
            let mut plugin = self.descriptor_mut();
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
        {
            let mut plugin = self.descriptor_mut();
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
fn convert_cc_parameter_to_midi(controller: u8, normalized_value: f32, sample_offset: u32) -> MidiEvent {
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
            // SAFETY: event.type == K_NOTE_ON_EVENT, so noteOn is active variant.
            let note_on = unsafe { &event.__field0.noteOn };
            // Use pitch as note_id when host sends -1
            let note_id = if note_on.noteId == -1 {
                note_on.pitch as i32
            } else {
                note_on.noteId
            };
            Some(MidiEvent::note_on(
                sample_offset,
                note_on.channel as u8,
                note_on.pitch as u8,
                note_on.velocity,
                note_id,
                note_on.tuning,
                note_on.length,
            ))
        }
        K_NOTE_OFF_EVENT => {
            // SAFETY: event.type == K_NOTE_OFF_EVENT, so noteOff is active variant.
            let note_off = unsafe { &event.__field0.noteOff };
            // Use pitch as note_id when host sends -1
            let note_id = if note_off.noteId == -1 {
                note_off.pitch as i32
            } else {
                note_off.noteId
            };
            Some(MidiEvent::note_off(
                sample_offset,
                note_off.channel as u8,
                note_off.pitch as u8,
                note_off.velocity,
                note_id,
                note_off.tuning,
            ))
        }
        K_POLY_PRESSURE_EVENT => {
            // SAFETY: event.type == K_POLY_PRESSURE_EVENT, so polyPressure is active variant.
            let poly = unsafe { &event.__field0.polyPressure };
            // Use pitch as note_id when host sends -1
            let note_id = if poly.noteId == -1 {
                poly.pitch as i32
            } else {
                poly.noteId
            };
            Some(MidiEvent::poly_pressure(
                sample_offset,
                poly.channel as u8,
                poly.pitch as u8,
                poly.pressure,
                note_id,
            ))
        }
        K_DATA_EVENT => {
            // SAFETY: event.type == K_DATA_EVENT, so data is active variant.
            let data_event = unsafe { &event.__field0.data };
            // Only handle SysEx data type.
            // NOTE: Box::new() allocates here. This runs once per SysEx event per
            // process block (not per clone), and SysEx input from hosts is rare.
            // A Box recycling pool could eliminate this, but the complexity is not
            // justified given the low frequency.
            if data_event.r#type == DATA_TYPE_MIDI_SYSEX {
                let mut sysex = SysEx::new();
                let copy_len = (data_event.size as usize).min(MAX_SYSEX_SIZE);
                if copy_len > 0 && !data_event.bytes.is_null() {
                    // SAFETY: bytes is non-null and host guarantees validity for size bytes.
                    let src = unsafe { std::slice::from_raw_parts(data_event.bytes, copy_len) };
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
            // SAFETY: event.type == K_NOTE_EXPRESSION_VALUE_EVENT, so noteExpressionValue is active.
            let expr = unsafe { &event.__field0.noteExpressionValue };
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
            // SAFETY: event.type == K_NOTE_EXPRESSION_INT_VALUE_EVENT, so noteExpressionIntValue is active.
            let expr = unsafe { &event.__field0.noteExpressionIntValue };
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
            // SAFETY: event.type == K_NOTE_EXPRESSION_TEXT_EVENT, so noteExpressionText is active.
            let expr = unsafe { &event.__field0.noteExpressionText };
            let mut text_event = NoteExpressionText {
                note_id: expr.noteId,
                expression_type: expr.typeId,
                text: [0u8; MAX_EXPRESSION_TEXT_SIZE],
                text_len: 0,
            };
            // Convert UTF-16 to UTF-8
            let text_len = expr.textLen as usize;
            if !expr.text.is_null() && text_len > 0 {
                // SAFETY: text is non-null and host guarantees validity for textLen elements.
                let text_slice = unsafe { std::slice::from_raw_parts(expr.text, text_len) };
                let utf8_len = utf16_to_utf8(text_slice, &mut text_event.text);
                text_event.text_len = utf8_len as u8;
            }
            Some(MidiEvent {
                sample_offset,
                event: MidiEventKind::NoteExpressionText(text_event),
            })
        }
        K_CHORD_EVENT => {
            // SAFETY: event.type == K_CHORD_EVENT, so chord is active variant.
            let chord = unsafe { &event.__field0.chord };
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
                // SAFETY: text is non-null and host guarantees validity for textLen elements.
                let text_slice = unsafe { std::slice::from_raw_parts(chord.text, text_len) };
                let utf8_len = utf16_to_utf8(text_slice, &mut info.name);
                info.name_len = utf8_len as u8;
            }
            Some(MidiEvent {
                sample_offset,
                event: MidiEventKind::ChordInfo(info),
            })
        }
        K_SCALE_EVENT => {
            // SAFETY: event.type == K_SCALE_EVENT, so scale is active variant.
            let scale = unsafe { &event.__field0.scale };
            let mut info = ScaleInfo {
                root: scale.root as i8,
                mask: scale.mask as u16,
                name: [0u8; MAX_SCALE_NAME_SIZE],
                name_len: 0,
            };
            // Convert UTF-16 to UTF-8
            let text_len = scale.textLen as usize;
            if !scale.text.is_null() && text_len > 0 {
                // SAFETY: text is non-null and host guarantees validity for textLen elements.
                let text_slice = unsafe { std::slice::from_raw_parts(scale.text, text_len) };
                let utf8_len = utf16_to_utf8(text_slice, &mut info.name);
                info.name_len = utf8_len as u8;
            }
            Some(MidiEvent {
                sample_offset,
                event: MidiEventKind::ScaleInfo(info),
            })
        }
        K_LEGACY_MIDI_CC_OUT_EVENT => {
            // SAFETY: event.type == K_LEGACY_MIDI_CC_OUT_EVENT, so midiCCOut is active variant.
            let cc_event = unsafe { &event.__field0.midiCCOut };
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
/// Note: ChordInfo, ScaleInfo and NoteExpressionText are primarily input events
/// (DAW → plugin) and are not output.
fn convert_midi_to_vst3(midi: &MidiEvent, sysex_pool: &mut SysExOutputPool) -> Option<Event> {
    // SAFETY: Event is a C struct with no invalid bit patterns; zeroed is a valid state.
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

#[cfg(test)]
mod tests {
    use super::*;
    use beamer_core::config::Category;
    use beamer_core::{NoParameters, ProcessMode};

    fn process_mode_for(process_mode: int32) -> ProcessMode {
        let setup = ProcessSetup {
            processMode: process_mode,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 512,
            sampleRate: 48000.0,
        };
        build_setup::<ProcessMode>(&setup, &BusLayout::default())
    }

    #[test]
    fn maps_vst3_process_modes() {
        assert_eq!(process_mode_for(ProcessModes_::kRealtime as int32), ProcessMode::Realtime);
        assert_eq!(process_mode_for(ProcessModes_::kPrefetch as int32), ProcessMode::Prefetch);
        assert_eq!(process_mode_for(ProcessModes_::kOffline as int32), ProcessMode::Offline);
        // Unknown modes fall back to realtime.
        assert_eq!(process_mode_for(-1), ProcessMode::Realtime);
    }

    // A plugin that keeps the setup it was prepared with.
    type RecordedSetup = (beamer_core::SampleRate, beamer_core::MaxBufferSize, ProcessMode);

    #[derive(Default)]
    struct SetupPlugin {
        parameters: Arc<NoParameters>,
    }

    struct SetupProcessor {
        setup: RecordedSetup,
    }

    impl HasParameters for SetupPlugin {
        type Parameters = NoParameters;
        fn parameters(&self) -> &Arc<Self::Parameters> {
            &self.parameters
        }
    }

    impl Descriptor for SetupPlugin {
        type Setup = RecordedSetup;
        type Processor = SetupProcessor;

        fn prepare(&self, setup: RecordedSetup) -> SetupProcessor {
            PREPARE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            SetupProcessor { setup }
        }
    }

    impl Processor for SetupProcessor {
        fn process(
            &mut self,
            _buffer: &mut beamer_core::Buffer,
            _aux: &mut beamer_core::AuxiliaryBuffers,
            _context: &CoreProcessContext,
        ) {
        }
    }

    /// Number of `prepare` calls on `SetupPlugin`. Only one test uses the plugin.
    static PREPARE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    static TEST_CONFIG: Config = Config::new("Setup Test", Category::Effect, "Test", "stup");

    /// Call setupProcessing like a host and return the setup the plugin holds.
    fn setup_processing(
        wrapper: &Vst3Processor<SetupPlugin>,
        sample_rate: f64,
        max_block: int32,
        mode: int32,
    ) -> (f64, usize, ProcessMode) {
        let mut setup = ProcessSetup {
            processMode: mode,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: max_block,
            sampleRate: sample_rate,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);

        // SAFETY: No other reference to the processor exists in this test.
        match unsafe { &*wrapper.processor.get() } {
            Some(processor) => {
                let (rate, block, mode) = processor.setup;
                (rate.0, block.0, mode)
            }
            None => panic!("the plugin is not prepared"),
        }
    }

    #[test]
    fn test_setup_processing_prepares_again_for_a_new_setup() {
        let wrapper = Vst3Processor::<SetupPlugin>::new(&TEST_CONFIG);
        let realtime = ProcessModes_::kRealtime as int32;
        let offline = ProcessModes_::kOffline as int32;

        assert_eq!(
            setup_processing(&wrapper, 44100.0, 512, realtime),
            (44100.0, 512, ProcessMode::Realtime)
        );
        // Each part of the setup can change on its own.
        assert_eq!(
            setup_processing(&wrapper, 96000.0, 512, realtime),
            (96000.0, 512, ProcessMode::Realtime)
        );
        assert_eq!(
            setup_processing(&wrapper, 96000.0, 2048, realtime),
            (96000.0, 2048, ProcessMode::Realtime)
        );
        assert_eq!(
            setup_processing(&wrapper, 96000.0, 2048, offline),
            (96000.0, 2048, ProcessMode::Offline)
        );
        assert_eq!(PREPARE_COUNT.load(std::sync::atomic::Ordering::Relaxed), 4);

        // A repeated setup leaves the processor alone.
        assert_eq!(
            setup_processing(&wrapper, 96000.0, 2048, offline),
            (96000.0, 2048, ProcessMode::Offline)
        );
        assert_eq!(PREPARE_COUNT.load(std::sync::atomic::Ordering::Relaxed), 4);
    }

    // =========================================================================
    // State, latency and tail while process() runs
    // =========================================================================
    //
    // A host saves and loads the state, and asks for latency and tail, from
    // its own threads while process() runs. None of that may use the
    // processor.

    use beamer_core::parameter_types::{FloatParameter, ParameterRef, Parameters};
    use beamer_core::{ParameterGroups, ParameterInfo, ParameterStore};
    use std::cell::Cell;
    use std::sync::Mutex;

    const GAIN_ID: u32 = 1;
    const STATE_TAIL_SAMPLES: u32 = 4410;
    const STATE_BYPASS_RAMP_SAMPLES: u32 = 128;

    thread_local! {
        /// How often the smoothers were reset on this thread.
        static SMOOTHING_RESETS: Cell<usize> = const { Cell::new(0) };
        /// The sample rate the smoothers were last given on this thread.
        static SMOOTHING_SAMPLE_RATE: Cell<f64> = const { Cell::new(0.0) };
        /// What `SMOOTHING_SAMPLE_RATE` was when a `StatePlugin` was last
        /// prepared on this thread.
        static SMOOTHING_SAMPLE_RATE_AT_PREPARE: Cell<f64> = const { Cell::new(0.0) };
        /// How many `StateProcessor`s exist on this thread.
        static LIVE_PROCESSORS: Cell<usize> = const { Cell::new(0) };
        /// What `LIVE_PROCESSORS` was when a `StatePlugin` was last prepared
        /// on this thread.
        static LIVE_PROCESSORS_AT_PREPARE: Cell<usize> = const { Cell::new(0) };
    }

    fn smoothing_resets() -> usize {
        SMOOTHING_RESETS.with(Cell::get)
    }

    /// Latency the `StateProcessor` reports for a gain value.
    fn latency_for(gain: f64) -> u32 {
        (gain * 100.0).round() as u32
    }

    struct StateParameters {
        gain: FloatParameter,
    }

    impl Default for StateParameters {
        fn default() -> Self {
            Self {
                gain: FloatParameter::new("Gain", 0.5, 0.0..=1.0).with_id(GAIN_ID),
            }
        }
    }

    impl ParameterGroups for StateParameters {}

    impl Parameters for StateParameters {
        fn count(&self) -> usize {
            1
        }
        fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
            Box::new(std::iter::once(&self.gain as &dyn ParameterRef))
        }
        fn by_id(&self, id: u32) -> Option<&dyn ParameterRef> {
            (id == GAIN_ID).then_some(&self.gain as &dyn ParameterRef)
        }
        fn set_sample_rate(&self, sample_rate: f64) {
            SMOOTHING_SAMPLE_RATE.with(|rate| rate.set(sample_rate));
        }
        fn reset_smoothing(&self) {
            SMOOTHING_RESETS.with(|resets| resets.set(resets.get() + 1));
        }
    }

    impl ParameterStore for StateParameters {
        fn count(&self) -> usize {
            1
        }
        fn info(&self, index: usize) -> Option<&ParameterInfo> {
            (index == 0).then(|| ParameterRef::info(&self.gain))
        }
        fn get_normalized(&self, id: u32) -> f64 {
            Parameters::by_id(self, id).map_or(0.0, |parameter| parameter.get_normalized())
        }
        fn set_normalized(&self, id: u32, value: f64) {
            if let Some(parameter) = Parameters::by_id(self, id) {
                parameter.set_normalized(value);
            }
        }
        fn normalized_to_string(&self, id: u32, normalized: f64) -> String {
            Parameters::by_id(self, id)
                .map_or_else(String::new, |parameter| parameter.display_normalized(normalized))
        }
        fn string_to_normalized(&self, id: u32, string: &str) -> Option<f64> {
            Parameters::by_id(self, id)?.parse(string)
        }
        fn normalized_to_plain(&self, id: u32, normalized: f64) -> f64 {
            Parameters::by_id(self, id)
                .map_or(0.0, |parameter| parameter.normalized_to_plain(normalized))
        }
        fn plain_to_normalized(&self, id: u32, plain: f64) -> f64 {
            Parameters::by_id(self, id).map_or(0.0, |parameter| parameter.plain_to_normalized(plain))
        }
    }

    #[derive(Default)]
    struct StatePlugin {
        parameters: Arc<StateParameters>,
    }

    struct StateProcessor {
        parameters: Arc<StateParameters>,
    }

    impl Drop for StateProcessor {
        fn drop(&mut self) {
            // A thread that ends may have dropped its counter already.
            let _ = LIVE_PROCESSORS.try_with(|live| live.set(live.get().saturating_sub(1)));
        }
    }

    impl HasParameters for StatePlugin {
        type Parameters = StateParameters;
        fn parameters(&self) -> &Arc<Self::Parameters> {
            &self.parameters
        }
    }

    impl Descriptor for StatePlugin {
        type Setup = ();
        type Processor = StateProcessor;

        fn prepare(&self, _setup: ()) -> StateProcessor {
            SMOOTHING_SAMPLE_RATE_AT_PREPARE
                .with(|rate| rate.set(SMOOTHING_SAMPLE_RATE.with(Cell::get)));
            LIVE_PROCESSORS_AT_PREPARE.with(|live| live.set(LIVE_PROCESSORS.with(Cell::get)));
            LIVE_PROCESSORS.with(|live| live.set(live.get() + 1));
            StateProcessor {
                parameters: Arc::clone(&self.parameters),
            }
        }
    }

    impl Processor for StateProcessor {
        fn process(
            &mut self,
            _buffer: &mut beamer_core::Buffer,
            _aux: &mut beamer_core::AuxiliaryBuffers,
            _context: &CoreProcessContext,
        ) {
        }

        fn latency_samples(&self) -> u32 {
            latency_for(self.parameters.gain.get_normalized())
        }

        fn tail_samples(&self) -> u32 {
            STATE_TAIL_SAMPLES
        }

        fn bypass_ramp_samples(&self) -> u32 {
            STATE_BYPASS_RAMP_SAMPLES
        }
    }

    static STATE_CONFIG: Config = Config::new("State Test", Category::Effect, "Test", "stat");

    /// A plugin that wants MIDI. Only its descriptor can say so.
    #[derive(Default)]
    struct MidiPlugin {
        parameters: Arc<NoParameters>,
        /// The controller of the last MIDI learn call.
        learned: Option<u8>,
    }

    struct MidiProcessor;

    impl HasParameters for MidiPlugin {
        type Parameters = NoParameters;
        fn parameters(&self) -> &Arc<Self::Parameters> {
            &self.parameters
        }
    }

    impl Descriptor for MidiPlugin {
        type Setup = ();
        type Processor = MidiProcessor;

        fn prepare(&self, _setup: ()) -> MidiProcessor {
            MidiProcessor
        }

        fn wants_midi(&self) -> bool {
            true
        }

        fn input_bus_count(&self) -> usize {
            2
        }

        fn input_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
            match index {
                0 => Some(CoreBusInfo::stereo("Input")),
                1 => Some(CoreBusInfo::aux("Sidechain", 2)),
                _ => None,
            }
        }

        fn midi_cc_to_parameter(&self, _bus_index: i32, _channel: i16, cc: u8) -> Option<u32> {
            (cc == MAPPED_CC).then_some(MAPPED_PARAMETER)
        }

        fn on_midi_learn(&mut self, _bus_index: i32, _channel: i16, cc: u8) -> bool {
            assert_ne!(cc, PANICKING_CC, "a plugin that panics in a callback");
            self.learned = Some(cc);
            true
        }
    }

    const MAPPED_CC: u8 = 74;
    const PANICKING_CC: u8 = 99;
    const MAPPED_PARAMETER: u32 = 1234;

    impl Processor for MidiProcessor {
        fn process(
            &mut self,
            _buffer: &mut beamer_core::Buffer,
            _aux: &mut beamer_core::AuxiliaryBuffers,
            _context: &CoreProcessContext,
        ) {
        }
    }

    static MIDI_CONFIG: Config = Config::new("Midi Test", Category::Instrument, "Test", "midi");

    /// The bus counts a host sees: audio in, audio out, event in, event out.
    fn bus_counts(wrapper: &Vst3Processor<MidiPlugin>) -> [i32; 4] {
        let audio = MediaTypes_::kAudio as MediaType;
        let event = MediaTypes_::kEvent as MediaType;
        let input = BusDirections_::kInput as BusDirection;
        let output = BusDirections_::kOutput as BusDirection;
        // SAFETY: The wrapper is alive for the calls.
        unsafe {
            [
                wrapper.getBusCount(audio, input),
                wrapper.getBusCount(audio, output),
                wrapper.getBusCount(event, input),
                wrapper.getBusCount(event, output),
            ]
        }
    }

    /// The channel count of an audio input bus as a host sees it.
    fn input_channels(wrapper: &Vst3Processor<MidiPlugin>, index: i32) -> Option<i32> {
        // SAFETY: A zeroed BusInfo is valid, and it is alive for the call.
        let mut info: BusInfo = unsafe { std::mem::zeroed() };
        // SAFETY: The wrapper and the info are alive for the call.
        let result = unsafe {
            wrapper.getBusInfo(
                MediaTypes_::kAudio as MediaType,
                BusDirections_::kInput as BusDirection,
                index,
                &mut info,
            )
        };
        (result == kResultOk).then_some(info.channelCount)
    }

    #[test]
    fn test_buses_are_reported_prepared_or_not() {
        let wrapper = Vst3Processor::<MidiPlugin>::new(&MIDI_CONFIG);
        let event_buses = |wrapper: &Vst3Processor<MidiPlugin>| {
            let [.., event_in, event_out] = bus_counts(wrapper);
            [event_in, event_out]
        };
        assert_eq!(bus_counts(&wrapper), [2, 1, 1, 1]);
        assert_eq!(input_channels(&wrapper, 1), Some(2));
        assert_eq!(input_channels(&wrapper, 2), None);

        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 44100.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        assert_eq!(event_buses(&wrapper), [1, 1]);
        assert_eq!(bus_counts(&wrapper), [2, 1, 1, 1]);
        assert_eq!(input_channels(&wrapper, 1), Some(2));
        assert_eq!(input_channels(&wrapper, 2), None);
    }

    #[test]
    fn test_descriptor_answers_after_a_panic_in_one_of_its_calls() {
        let wrapper = Vst3Processor::<MidiPlugin>::new(&MIDI_CONFIG);

        // The plugin panics while the wrapper holds its descriptor for
        // writing.
        let learn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: The wrapper is alive for the call.
            unsafe { wrapper.onLiveMIDIControllerInput(0, 0, PANICKING_CC as i16) }
        }));
        assert!(learn.is_err());

        // The host still gets its answers.
        assert_eq!(bus_counts(&wrapper), [2, 1, 1, 1]);
        // SAFETY: The wrapper is alive for the call.
        assert_eq!(unsafe { wrapper.onLiveMIDIControllerInput(0, 0, 1) }, kResultOk);
        assert_eq!(wrapper.descriptor().learned, Some(1));
    }

    #[test]
    fn test_descriptor_answers_midi_mapping_prepared_or_not() {
        let wrapper = Vst3Processor::<MidiPlugin>::new(&MIDI_CONFIG);
        let mapped = |wrapper: &Vst3Processor<MidiPlugin>| {
            let mut id = 0u32;
            // SAFETY: The wrapper and the id are alive for the call.
            let result = unsafe {
                wrapper.getMidiControllerAssignment(0, 0, MAPPED_CC as i16, &mut id)
            };
            (result, id)
        };
        let learn = |wrapper: &Vst3Processor<MidiPlugin>, cc: u8| {
            // SAFETY: The wrapper is alive for the call.
            let result = unsafe { wrapper.onLiveMIDIControllerInput(0, 0, cc as i16) };
            (result, wrapper.descriptor().learned)
        };
        assert_eq!(mapped(&wrapper), (kResultOk, MAPPED_PARAMETER));
        assert_eq!(learn(&wrapper, 1), (kResultOk, Some(1)));

        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 44100.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);

        // A prepared plugin still has its descriptor to answer.
        assert_eq!(mapped(&wrapper), (kResultOk, MAPPED_PARAMETER));
        assert_eq!(learn(&wrapper, 7), (kResultOk, Some(7)));
    }

    fn prepared_state_wrapper() -> Vst3Processor<StatePlugin> {
        let wrapper = Vst3Processor::<StatePlugin>::new(&STATE_CONFIG);
        prepare_state_wrapper(&wrapper);
        wrapper
    }

    fn prepare_state_wrapper(wrapper: &Vst3Processor<StatePlugin>) {
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 44100.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
    }

    /// Process one stereo block like a host.
    fn process_block(wrapper: &Vst3Processor<StatePlugin>) {
        process_with(wrapper, 64, std::ptr::null_mut());
    }

    /// Call process like a host, with `frames` of the 64 frames the buffers
    /// hold and with the parameter changes of the block, if any.
    fn process_with(
        wrapper: &Vst3Processor<StatePlugin>,
        frames: int32,
        changes: *mut IParameterChanges,
    ) {
        const FRAMES: usize = 64;
        let mut input = [[0.5f32; FRAMES]; 2];
        let mut output = [[0.0f32; FRAMES]; 2];
        let mut input_channels = [input[0].as_mut_ptr(), input[1].as_mut_ptr()];
        let mut output_channels = [output[0].as_mut_ptr(), output[1].as_mut_ptr()];
        let mut input_bus = AudioBusBuffers {
            numChannels: 2,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: input_channels.as_mut_ptr(),
            },
        };
        let mut output_bus = AudioBusBuffers {
            numChannels: 2,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: output_channels.as_mut_ptr(),
            },
        };
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            numSamples: frames,
            numInputs: 1,
            numOutputs: 1,
            inputs: &mut input_bus,
            outputs: &mut output_bus,
            inputParameterChanges: changes,
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: std::ptr::null_mut(),
        };
        // SAFETY: The data and its buffers are valid for the call.
        assert_eq!(unsafe { wrapper.process(&mut data) }, kResultOk);
    }

    /// The change of one parameter to one value at the start of a block.
    struct OneChange {
        id: u32,
        value: f64,
    }

    impl Class for OneChange {
        type Interfaces = (IParamValueQueue,);
    }

    impl IParamValueQueueTrait for OneChange {
        unsafe fn getParameterId(&self) -> ParamID {
            self.id
        }

        unsafe fn getPointCount(&self) -> int32 {
            1
        }

        unsafe fn getPoint(
            &self,
            index: int32,
            sample_offset: *mut int32,
            value: *mut ParamValue,
        ) -> tresult {
            if index != 0 {
                return kResultFalse;
            }
            // SAFETY: The caller provides valid pointers.
            unsafe {
                *sample_offset = 0;
                *value = self.value;
            }
            kResultOk
        }

        unsafe fn addPoint(&self, _sample_offset: int32, _value: ParamValue, _index: *mut int32) -> tresult {
            kNotImplemented
        }
    }

    /// The parameter changes a host sends with a process call: one here.
    struct Changes {
        queue: vst3::ComPtr<IParamValueQueue>,
    }

    impl Class for Changes {
        type Interfaces = (IParameterChanges,);
    }

    impl IParameterChangesTrait for Changes {
        unsafe fn getParameterCount(&self) -> int32 {
            1
        }

        unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
            if index == 0 {
                self.queue.as_ptr()
            } else {
                std::ptr::null_mut()
            }
        }

        unsafe fn addParameterData(&self, _id: *const ParamID, _index: *mut int32) -> *mut IParamValueQueue {
            std::ptr::null_mut()
        }
    }

    /// Call process with one parameter change and `frames` frames.
    fn process_change(wrapper: &Vst3Processor<StatePlugin>, frames: int32, id: u32, value: f64) {
        let queue = vst3::ComWrapper::new(OneChange { id, value })
            .to_com_ptr::<IParamValueQueue>()
            .expect("the queue is an IParamValueQueue");
        let changes = vst3::ComWrapper::new(Changes { queue })
            .to_com_ptr::<IParameterChanges>()
            .expect("the changes are IParameterChanges");
        process_with(wrapper, frames, changes.as_ptr());
    }

    /// A stream in memory, like the one a host hands to getState and setState.
    #[derive(Default)]
    struct MemoryStream {
        /// The bytes and the read position.
        content: Mutex<(Vec<u8>, usize)>,
    }

    impl Class for MemoryStream {
        type Interfaces = (IBStream,);
    }

    impl IBStreamTrait for MemoryStream {
        unsafe fn read(
            &self,
            buffer: *mut c_void,
            num_bytes: int32,
            num_bytes_read: *mut int32,
        ) -> tresult {
            let mut content = self.content.lock().unwrap();
            let (bytes, position) = &mut *content;
            let count = (num_bytes.max(0) as usize).min(bytes.len() - *position);
            // SAFETY: The caller provides a buffer of at least num_bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes[*position..].as_ptr(), buffer.cast::<u8>(), count);
            }
            *position += count;
            if !num_bytes_read.is_null() {
                // SAFETY: Checked for null above.
                unsafe { *num_bytes_read = count as int32 };
            }
            kResultOk
        }

        unsafe fn write(
            &self,
            buffer: *mut c_void,
            num_bytes: int32,
            num_bytes_written: *mut int32,
        ) -> tresult {
            // SAFETY: The caller provides a buffer of num_bytes.
            let written = unsafe { slice::from_raw_parts(buffer.cast::<u8>(), num_bytes.max(0) as usize) };
            self.content.lock().unwrap().0.extend_from_slice(written);
            if !num_bytes_written.is_null() {
                // SAFETY: Checked for null above.
                unsafe { *num_bytes_written = num_bytes.max(0) };
            }
            kResultOk
        }

        unsafe fn seek(&self, _pos: int64, _mode: int32, _result: *mut int64) -> tresult {
            kNotImplemented
        }

        unsafe fn tell(&self, _pos: *mut int64) -> tresult {
            kNotImplemented
        }
    }

    /// Save the state of a wrapper like a host and return the bytes.
    fn saved_state(wrapper: &Vst3Processor<StatePlugin>) -> Vec<u8> {
        let stream = vst3::ComWrapper::new(MemoryStream::default());
        let pointer = stream.to_com_ptr::<IBStream>().expect("the stream is an IBStream");
        // SAFETY: The stream is alive for the call.
        assert_eq!(unsafe { IComponentTrait::getState(wrapper, pointer.as_ptr()) }, kResultOk);
        let bytes = stream.content.lock().unwrap().0.clone();
        bytes
    }

    /// Load a state into a wrapper like a host.
    fn load_state(wrapper: &Vst3Processor<StatePlugin>, bytes: &[u8]) -> tresult {
        let stream = vst3::ComWrapper::new(MemoryStream {
            content: Mutex::new((bytes.to_vec(), 0)),
        });
        let pointer = stream.to_com_ptr::<IBStream>().expect("the stream is an IBStream");
        // SAFETY: The stream is alive for the call.
        unsafe { IComponentTrait::setState(wrapper, pointer.as_ptr()) }
    }

    fn gain(wrapper: &Vst3Processor<StatePlugin>) -> f64 {
        // SAFETY: The wrapper is alive.
        unsafe { wrapper.getParamNormalized(GAIN_ID) }
    }

    fn set_gain(wrapper: &Vst3Processor<StatePlugin>, value: f64) {
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.setParamNormalized(GAIN_ID, value) }, kResultOk);
    }

    #[test]
    fn test_state_is_the_same_prepared_or_not() {
        let wrapper = Vst3Processor::<StatePlugin>::new(&STATE_CONFIG);

        // Before preparing: the state is saved and a loaded one applies.
        set_gain(&wrapper, 0.25);
        let quiet = saved_state(&wrapper);
        assert!(!quiet.is_empty());
        set_gain(&wrapper, 0.75);
        assert_eq!(load_state(&wrapper, &quiet), kResultOk);
        assert_eq!(gain(&wrapper), 0.25);

        // The processor shares it: it is saved to the same bytes when prepared.
        prepare_state_wrapper(&wrapper);
        assert_eq!(saved_state(&wrapper), quiet);
        set_gain(&wrapper, 0.75);
        assert_eq!(load_state(&wrapper, &quiet), kResultOk);
        assert_eq!(gain(&wrapper), 0.25);

        // An empty state changes nothing.
        assert_eq!(load_state(&wrapper, &[]), kResultOk);
        assert_eq!(gain(&wrapper), 0.25);
    }

    #[test]
    fn test_loaded_state_resets_the_smoothers_in_the_next_block() {
        let wrapper = prepared_state_wrapper();
        process_block(&wrapper);
        let state = saved_state(&wrapper);
        let before = smoothing_resets();

        // The smoothers belong to the audio thread: loading leaves them alone.
        assert_eq!(load_state(&wrapper, &state), kResultOk);
        assert_eq!(smoothing_resets(), before);

        // The next block resets them, once.
        process_block(&wrapper);
        assert_eq!(smoothing_resets(), before + 1);
        process_block(&wrapper);
        assert_eq!(smoothing_resets(), before + 1);
    }

    #[test]
    fn test_state_loaded_before_preparing_resets_the_smoothers_in_the_first_block() {
        let wrapper = Vst3Processor::<StatePlugin>::new(&STATE_CONFIG);
        let state = saved_state(&wrapper);
        assert_eq!(load_state(&wrapper, &state), kResultOk);
        let before = smoothing_resets();

        prepare_state_wrapper(&wrapper);
        process_block(&wrapper);
        assert_eq!(smoothing_resets(), before + 1);
    }

    #[test]
    fn test_setup_processing_gives_the_smoothers_the_host_sample_rate() {
        let wrapper = Vst3Processor::<StatePlugin>::new(&STATE_CONFIG);

        // The rate is there when the plugin creates its processor.
        prepare_state_wrapper(&wrapper);
        assert_eq!(SMOOTHING_SAMPLE_RATE_AT_PREPARE.with(Cell::get), 44100.0);

        // A new setup of a prepared plugin does it again.
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 96000.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        assert_eq!(SMOOTHING_SAMPLE_RATE_AT_PREPARE.with(Cell::get), 96000.0);
    }

    #[test]
    fn test_new_setup_replaces_the_processor_and_keeps_the_parameters() {
        let wrapper = Vst3Processor::<StatePlugin>::new(&STATE_CONFIG);
        set_gain(&wrapper, 0.25);

        prepare_state_wrapper(&wrapper);
        assert_eq!(LIVE_PROCESSORS_AT_PREPARE.with(Cell::get), 0);
        assert_eq!(LIVE_PROCESSORS.with(Cell::get), 1);

        // A new sample rate: the old processor is gone before the new one
        // is created.
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 96000.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        assert_eq!(LIVE_PROCESSORS_AT_PREPARE.with(Cell::get), 0);
        assert_eq!(LIVE_PROCESSORS.with(Cell::get), 1);

        // The value is still there, and the new processor shares it: the
        // latency it reports follows the parameter.
        assert_eq!(gain(&wrapper), 0.25);
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.getLatencySamples() }, latency_for(0.25));
        set_gain(&wrapper, 0.5);
        process_block(&wrapper);
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.getLatencySamples() }, latency_for(0.5));
    }

    #[test]
    fn test_process_call_without_samples_applies_its_parameter_changes() {
        let wrapper = prepared_state_wrapper();
        set_gain(&wrapper, 0.25);

        // A host flushes parameter changes this way while it does not
        // process audio.
        process_change(&wrapper, 0, GAIN_ID, 0.75);
        assert_eq!(gain(&wrapper), 0.75);

        // A block with samples applies its changes as well.
        process_change(&wrapper, 64, GAIN_ID, 0.5);
        assert_eq!(gain(&wrapper), 0.5);
    }

    #[test]
    fn test_process_call_without_samples_on_an_unprepared_plugin() {
        let wrapper = Vst3Processor::<StatePlugin>::new(&STATE_CONFIG);
        set_gain(&wrapper, 0.25);

        // There is no processor yet. The values are applied all the same.
        process_change(&wrapper, 0, GAIN_ID, 0.75);
        assert_eq!(gain(&wrapper), 0.75);
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.getLatencySamples() }, 0);
    }

    #[test]
    fn test_process_call_without_samples_publishes_the_latency() {
        let wrapper = prepared_state_wrapper();

        process_change(&wrapper, 0, GAIN_ID, 0.75);
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.getLatencySamples() }, latency_for(0.75));
    }

    #[test]
    fn test_process_does_not_wait_for_the_descriptor() {
        let wrapper = prepared_state_wrapper();

        // A host thread holds the descriptor for a call that changes it,
        // while the audio thread processes.
        let host_call = wrapper.descriptor_mut();
        let (done, processed) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let wrapper = &wrapper;
            scope.spawn(move || {
                process_block(wrapper);
                done.send(()).expect("the test thread waits");
            });
            let in_time = processed.recv_timeout(std::time::Duration::from_secs(10));
            // Release the descriptor first, so that a process call that did
            // wait can end and the scope can close.
            drop(host_call);
            assert!(in_time.is_ok(), "process() waited for the descriptor");
        });
    }

    #[test]
    fn test_process_call_without_samples_resets_the_smoothers_after_a_state_load() {
        let wrapper = prepared_state_wrapper();
        let state = saved_state(&wrapper);
        assert_eq!(load_state(&wrapper, &state), kResultOk);
        let before = smoothing_resets();

        process_with(&wrapper, 0, std::ptr::null_mut());
        assert_eq!(smoothing_resets(), before + 1);
        process_block(&wrapper);
        assert_eq!(smoothing_resets(), before + 1);
    }

    #[test]
    fn test_editor_shares_the_parameters_in_both_states() {
        let wrapper = Vst3Processor::<StatePlugin>::new(&STATE_CONFIG);

        // An editor that is opened before audio is set up.
        let editor = wrapper.shared_parameter_store();
        editor.set_normalized(GAIN_ID, 0.25);
        assert_eq!(gain(&wrapper), 0.25);

        // Setting up and changing the setup creates processors. The editor
        // keeps reaching the parameters they use.
        prepare_state_wrapper(&wrapper);
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 96000.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        editor.set_normalized(GAIN_ID, 0.5);
        process_block(&wrapper);
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.getLatencySamples() }, latency_for(0.5));
        assert_eq!(editor.get_normalized(GAIN_ID), 0.5);

        // An editor that is opened now gets the same parameters.
        let later = wrapper.shared_parameter_store();
        assert!(std::ptr::addr_eq(Arc::as_ptr(&editor), Arc::as_ptr(&later)));

        // An editor that outlives the plugin still has its parameters.
        drop(later);
        drop(wrapper);
        assert_eq!(editor.get_normalized(GAIN_ID), 0.5);
        editor.set_normalized(GAIN_ID, 0.75);
        assert_eq!(editor.get_normalized(GAIN_ID), 0.75);
    }

    #[test]
    fn test_latency_and_tail_are_published_for_the_host() {
        let wrapper = Vst3Processor::<StatePlugin>::new(&STATE_CONFIG);
        let tail = STATE_TAIL_SAMPLES + STATE_BYPASS_RAMP_SAMPLES;

        // SAFETY: The wrapper is alive for all calls below.
        unsafe {
            // Unprepared: neither.
            assert_eq!(wrapper.getLatencySamples(), 0);
            assert_eq!(wrapper.getTailSamples(), 0);

            // Prepared: known before the first process call.
            prepare_state_wrapper(&wrapper);
            assert_eq!(wrapper.getLatencySamples(), latency_for(0.5));
            assert_eq!(wrapper.getTailSamples(), tail);

            // A change shows with the next block.
            set_gain(&wrapper, 0.25);
            assert_eq!(wrapper.getLatencySamples(), latency_for(0.5));
            process_block(&wrapper);
            assert_eq!(wrapper.getLatencySamples(), latency_for(0.25));

            // And after activating, which is when a host asks again.
            set_gain(&wrapper, 0.75);
            assert_eq!(wrapper.setActive(1), kResultOk);
            assert_eq!(wrapper.getLatencySamples(), latency_for(0.75));
            assert_eq!(wrapper.getTailSamples(), tail);
        }
    }
}
