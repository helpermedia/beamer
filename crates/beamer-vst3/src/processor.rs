//! Generic VST3 processor wrapper.
//!
//! This module provides [`Vst3Processor`], a generic wrapper that bridges any
//! [`beamer_core::Model`] implementation to VST3 COM interfaces.
//!
//! # Architecture
//!
//! Uses the **combined component** pattern where processor and controller are
//! implemented by the same object. This is the modern approach used by most
//! audio plugin frameworks.
//!
//! ```text
//! User Plugin (implements beamer_core::Model)
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
#[cfg(feature = "webview")]
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use vst3::{Class, ComRef, Steinberg::Vst::*, Steinberg::*};

use beamer_core::{
    AuxiliaryBuffers, Buffer, BusInfo as CoreBusInfo, BusLayout,
    BusType as CoreBusType, CachedBusConfig, CachedBusInfo, ChordInfo, ConversionBuffers,
    FrameRate as CoreFrameRate, MidiBuffer, MidiCcState, Model,
    MidiEvent, MidiEventKind, NoteExpressionInt, NoteExpressionText, Preset,
    NoteExpressionValue as CoreNoteExpressionValue, ParameterId, ParameterStore, Config, PluginSetup,
    ProcessBufferStorage, ProcessContext as CoreProcessContext, ProcessWarning,
    ProcessWarnings, Processor, Sample, ScaleInfo, SysEx,
    SharedModel, SysExOutputPool, Transport, MAX_BUSES, MAX_CHANNELS,
    MAX_CHORD_NAME_SIZE,
    MAX_EXPRESSION_TEXT_SIZE, MAX_SCALE_NAME_SIZE, MAX_SYSEX_SIZE,
};

use beamer_core::state_format::{StateEntries, StateWriter};
use beamer_core::sub_blocks::{offset_from_sub_block, run_sub_blocks, sort_by_offset};

use crate::component_handler::ComponentHandler;
#[cfg(feature = "webview")]
use beamer_core::{WebViewDispatch, WebViewEventQueue};
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

/// How many parameter points one process call keeps for the sub-block loop.
const MAX_PARAMETER_POINTS: usize = 2048;

/// A point of a host parameter queue, applied by the sub-block loop: a
/// parameter takes `value` at `sample_offset`.
///
/// A queue describes a curve through its points, made of segments from one
/// point to the next (the first from the start of the block). A segment of
/// one sample is a jump, which a host sends as the old value and the new
/// value one sample later: the new value applies at its point. A longer
/// segment is a ramp: its end value applies at its start, so the
/// parameter's smoother moves towards it during the segment, as for an
/// Audio Unit ramp. A discrete parameter has no smoother, so a ramp of one
/// changes it at each sample where the line reaches the next step.
#[derive(Clone, Copy, Debug)]
struct ParameterPoint {
    sample_offset: u32,
    id: u32,
    value: f64,
}

/// What `take_parameter_changes` found besides the parameter points.
struct TakenChanges {
    /// A program change applied a factory preset.
    preset_applied: bool,
    /// Points did not fit and were left out.
    overflowed: bool,
}

/// What the sub-block loop of a process call needs besides its buffers.
struct BlockInput<'a> {
    /// The parameter points, sorted by offset.
    points: &'a [ParameterPoint],
    /// The MIDI input, sorted by offset.
    midi_input: &'a mut [MidiEvent],
    /// Where `process_midi` writes, with the offsets of the whole block.
    midi_output: &'a mut MidiBuffer,
    /// The host's transport at the start of the block.
    transport: Transport,
    sample_rate: f64,
}

/// Collect the points of the queue of one parameter into `points`, each at
/// the sample it applies at (see [`ParameterPoint`]). Returns `false` when
/// they do not all fit, without growing `points`.
fn collect_points(
    parameter: &dyn beamer_core::parameter_types::ParameterRef,
    id: u32,
    queue: ComRef<'_, IParamValueQueue>,
    points: &mut Vec<ParameterPoint>,
) -> bool {
    let mut push = |sample_offset: u32, value: f64| {
        let fits = points.len() < points.capacity();
        if fits {
            points.push(ParameterPoint { sample_offset, id, value });
        }
        fits
    };

    // The curve starts at the start of the block, at the value the
    // parameter holds. `held` is what the parameter holds after the points
    // so far, and the segment towards the next point starts at the last
    // point, also one that changed nothing.
    let mut held = parameter.get_normalized();
    let mut segment_start = 0;
    let mut segment_from = held;
    // SAFETY: queue is a valid ComRef.
    let point_count = unsafe { queue.getPointCount() };
    for index in 0..point_count {
        let mut sample_offset = 0;
        let mut value = 0.0;
        // SAFETY: queue is valid, sample_offset and value are valid pointers.
        if unsafe { queue.getPoint(index, &mut sample_offset, &mut value) } != kResultTrue {
            continue;
        }
        let end = sample_offset.max(0) as u32;
        let start = std::mem::replace(&mut segment_start, end);
        let from = std::mem::replace(&mut segment_from, value);
        let stored = parameter.stored_normalized(value);
        if stored == held {
            continue;
        }

        if end <= start + 1 {
            // A segment of one sample is a jump, which takes effect at its
            // point (as do points out of order).
            if !push(end, value) {
                return false;
            }
            held = stored;
        } else if parameter.is_discrete() {
            // A ramp of a discrete parameter: each step applies at the first
            // sample where the line reaches it.
            let value_at = |sample: u32| from + (value - from) * f64::from(sample - start) / f64::from(end - start);
            let mut reached = start;
            while held != stored {
                // The line moves one way and `end` is at another step, so a
                // binary search finds the first sample after `reached` that
                // is.
                let (mut low, mut high) = (reached + 1, end);
                while low < high {
                    let middle = low + (high - low) / 2;
                    if parameter.stored_normalized(value_at(middle)) != held {
                        high = middle;
                    } else {
                        low = middle + 1;
                    }
                }
                let step_value = if low == end { value } else { value_at(low) };
                if !push(low, step_value) {
                    return false;
                }
                held = parameter.stored_normalized(step_value);
                reached = low;
            }
        } else {
            // A ramp: its end value applies at its start, for the smoother
            // to move towards.
            if !push(start, value) {
                return false;
            }
            held = stored;
        }
    }
    true
}

/// The last value in a parameter queue, which is the value at the end of
/// the block.
fn last_value(queue: ComRef<'_, IParamValueQueue>) -> Option<f64> {
    // SAFETY: queue is a valid ComRef.
    let point_count = unsafe { queue.getPointCount() };
    if point_count <= 0 {
        return None;
    }
    let mut sample_offset = 0;
    let mut value = 0.0;
    // SAFETY: queue is valid, sample_offset and value are valid pointers.
    let result = unsafe { queue.getPoint(point_count - 1, &mut sample_offset, &mut value) };
    (result == kResultTrue).then_some(value)
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

/// Generic VST3 processor wrapping any [`Model`] implementation.
///
/// This struct implements the VST3 combined component pattern, providing
/// `IComponent`, `IAudioProcessor` and `IEditController` interfaces that
/// delegate to the wrapped plugin.
///
/// # Lifecycle
///
/// The wrapper keeps the plugin's model for its whole life, and a
/// processor while the plugin is prepared:
///
/// ```text
/// Vst3Processor::new()
///     ↓ creates Model::default() in an Arc
/// model, no processor
///     ↓ setupProcessing() calls model.prepare(setup) with a clone of the Arc
/// model and processor
///     ↓ setupProcessing() with a new setup: drops the processor, then prepare()
/// model and a new processor
/// ```
///
/// # Usage
///
/// ```ignore
/// use beamer_vst3::{export_vst3, Vst3Processor, Config};
/// use beamer_core::config::Category;
///
/// #[beamer::model]
/// struct MyModel { /* parameters and State fields */ }
/// impl Model for MyModel { /* ... */ }
///
/// struct MyProcessor { model: Arc<MyModel>, sample_rate: f64 }
/// impl Processor for MyProcessor { /* ... */ }
///
/// static CONFIG: Config = Config::new("MyPlugin", Category::Effect, "Mfgr", "plgn");
/// export_vst3!(CONFIG, MyModel);
/// ```
///
/// # Thread Safety
///
/// VST3 guarantees that `process()` is called from a single thread at a time.
/// We use `UnsafeCell` for interior mutability in `process()` since the COM
/// interface only provides `&self`.
///
/// The SDK headers note the thread and state of each call. `process()` runs
/// on the processing thread. `setupProcessing()` and `setActive()` run on
/// the UI thread while the plugin does not process. `getState()`,
/// `setState()` and the controller calls run on the UI thread, also while
/// `process()` runs. The headers list `getLatencySamples()` and
/// `getTailSamples()` for the UI thread before activation, but hosts ask
/// for them at any time. What all those calls use is kept apart from what
/// `process()` uses.
/// The model is shared in an `Arc` and works through a shared reference.
/// Latency, tail and the smoother reset are exchanged through atomics.
pub struct Vst3Processor<P>
where
    P: Model,
{
    /// The model of the plugin, shared with every processor it creates and
    /// with the editor. Every method of it takes `&self`: `process()` reads
    /// its parameters through the processor's own `Arc`, and host threads
    /// use it at the same time, also while `setupProcessing()` lets it
    /// create a processor.
    model: Arc<P>,
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
    /// Whether the MIDI input buffer holds the events of calls without
    /// samples, which wait for the next call with samples.
    midi_input_carried: UnsafeCell<bool>,
    /// MIDI output buffer (reused each process call)
    midi_output: UnsafeCell<MidiBuffer>,
    /// The parameter points of a process call, for the sub-block loop
    /// (pre-allocated, never grows in `process()`).
    parameter_points: UnsafeCell<Vec<ParameterPoint>>,
    /// SysEx output buffer pool (for VST3 DataEvent pointer stability)
    sysex_output_pool: UnsafeCell<SysExOutputPool>,
    /// Conversion buffers for f64→f32 processing
    conversion_buffers: UnsafeCell<ConversionBuffers>,
    /// Pre-allocated channel pointer storage for f32 processing
    buffer_storage_f32: UnsafeCell<ProcessBufferStorage<f32>>,
    /// Pre-allocated channel pointer storage for f64 processing
    buffer_storage_f64: UnsafeCell<ProcessBufferStorage<f64>>,
    /// MIDI CC state (created from the model's midi_cc_config())
    /// Framework owns this - plugin authors don't touch it
    midi_cc_state: Option<MidiCcState>,
    /// Current factory preset index (0-based).
    /// Used for the program change parameter exposed to the host. Set by
    /// host threads and by process(), read by host threads. Saved with the
    /// state and taken back from it.
    current_preset_index: AtomicU32,
    /// The parameters the factory preset of the last program change in
    /// `process()` changed, with their values before it, for
    /// [`report_preset_changes`](Self::report_preset_changes). Before the
    /// preset is applied it holds every parameter. Allocated for all
    /// parameters when the plugin has presets, so `process()` never grows
    /// it. Belongs to `process()`.
    preset_changes: UnsafeCell<Vec<(ParameterId, f64)>>,
    /// The ids of all parameters of the model, for the comparison around a
    /// factory preset. Empty when the plugin has no presets.
    parameter_ids: Box<[ParameterId]>,
    /// The component handler the host set, shared with the editors.
    component_handler: Arc<ComponentHandler>,
    /// The plugin's queue of events for the web UI, if it has one.
    #[cfg(feature = "webview")]
    webview_events: Option<WebViewEventQueue>,
    /// Latency in samples as the processor reported it last, 0 while unprepared.
    /// The host asks for it from its own threads while `process()` runs, so
    /// the answer must not need the processor (see `publish_timing`).
    latency_samples: AtomicU32,
    /// Tail in samples, with the bypass ramp, as the processor reported it last.
    tail_samples: AtomicU32,
    /// A state was loaded: `process()` resets the parameter smoothers before
    /// its next block. They belong to the audio thread.
    smoothing_reset_requested: AtomicBool,
    /// What `process()` had to drop since a host thread last looked. The
    /// audio thread does not log: `setupProcessing()`, `setActive()` and
    /// `terminate()` log them.
    process_warnings: ProcessWarnings,
    /// The `state_generation()` of the model that the host knows
    /// about: at creation and after a load. Shared with the editor, which
    /// tells the host when it changed.
    #[cfg(feature = "webview")]
    known_state_generation: Arc<AtomicU64>,
    /// Marker for the plugin type
    _marker: PhantomData<P>,
}

// Safety: Vst3Processor is Send because:
// - The model is Send + Sync, required by the Model trait
// - Processor: Send is required by the Processor trait
unsafe impl<P: Model> Send for Vst3Processor<P> {}

// Safety: Vst3Processor is Sync because:
// - VST3 guarantees process() is called from one thread at a time
// - The model is Sync, and process() reaches it through the processor's
//   own `Arc`
// - component_handler is used by controller calls, which the host never
//   makes concurrently
// - What a host thread uses while process() runs is atomic: the parameter
//   values (also for saving and loading the state), the current preset,
//   latency, tail and the request to reset the smoothers
unsafe impl<P: Model> Sync for Vst3Processor<P> {}

impl<P: Model> Vst3Processor<P> {
    /// Create a new VST3 processor wrapping the given plugin configuration.
    ///
    /// The wrapper starts unprepared, with a default plugin instance.
    /// The processor will be created when `setupProcessing()` is called.
    pub fn new(config: &'static Config) -> Self {
        let model = Arc::new(P::default());

        // The ids for the comparison around a factory preset in `process()`.
        let parameter_ids: Box<[ParameterId]> = if !P::PRESETS.is_empty() {
            beamer_core::parameter_types::Parameters::iter(&*model)
                .map(|parameter| parameter.id())
                .collect()
        } else {
            Box::default()
        };

        // Create MidiCcState from the model's config (framework-managed). The
        // controllers are MIDI input, so a plugin without it has none: no
        // hidden parameters, no mapping and no events from them.
        let midi_cc_state = config
            .midi_input
            .then(|| model.midi_cc_config())
            .flatten()
            .map(|cfg| MidiCcState::from_config(&cfg));

        // The plugin's queue of events for the web UI. Claiming it marks it
        // as delivered by this instance; without an editor its events are
        // dropped, as while no editor is open.
        let events = beamer_core::WebViewEvents::webview_events(&*model);
        if let Some(events) = events {
            events.claim();
        }
        #[cfg(feature = "webview")]
        let webview_events = events.cloned();

        #[cfg(feature = "webview")]
        let known_state_generation =
            Arc::new(AtomicU64::new(beamer_core::Saved::state_generation(&*model)));

        Self {
            model,
            processor: UnsafeCell::new(None),
            config,
            sample_rate: UnsafeCell::new(44100.0),
            max_block_size: UnsafeCell::new(1024),
            symbolic_sample_size: UnsafeCell::new(SymbolicSampleSizes_::kSample32 as i32),
            process_mode: UnsafeCell::new(ProcessModes_::kRealtime as int32),
            midi_input: UnsafeCell::new(MidiBuffer::new()),
            midi_input_carried: UnsafeCell::new(false),
            midi_output: UnsafeCell::new(MidiBuffer::new()),
            parameter_points: UnsafeCell::new(Vec::with_capacity(MAX_PARAMETER_POINTS)),
            // Each slot holds a message with its F0 and F7.
            sysex_output_pool: UnsafeCell::new(SysExOutputPool::with_capacity(
                config.sysex_slots,
                config.sysex_buffer_size + 2,
            )),
            conversion_buffers: UnsafeCell::new(ConversionBuffers::new()),
            buffer_storage_f32: UnsafeCell::new(ProcessBufferStorage::new()),
            buffer_storage_f64: UnsafeCell::new(ProcessBufferStorage::new()),
            midi_cc_state,
            // Default to first preset.
            current_preset_index: AtomicU32::new(0),
            preset_changes: UnsafeCell::new(Vec::with_capacity(parameter_ids.len())),
            parameter_ids,
            component_handler: Arc::new(ComponentHandler::new()),
            #[cfg(feature = "webview")]
            webview_events,
            latency_samples: AtomicU32::new(0),
            tail_samples: AtomicU32::new(0),
            smoothing_reset_requested: AtomicBool::new(false),
            process_warnings: ProcessWarnings::default(),
            #[cfg(feature = "webview")]
            known_state_generation,
            _marker: PhantomData,
        }
    }

    /// The factory preset that a normalized value of the program change
    /// parameter selects, or `None` when the plugin has no presets.
    fn preset_index_for(value: f64) -> Option<usize> {
        let preset_count = P::PRESETS.len();
        if preset_count == 0 {
            return None;
        }
        // stepCount = preset_count - 1, so index = round(value * stepCount).
        let step_count = (preset_count - 1) as f64;
        Some(((value * step_count).round() as usize).min(preset_count - 1))
    }

    /// Take the parameter changes a host sends with a process call.
    ///
    /// A change of the program change parameter selects a factory preset.
    /// It is applied at once, before the changes of single parameters in the
    /// same block, and the parameters it changed are kept for
    /// [`report_preset_changes`](Self::report_preset_changes).
    ///
    /// The points of the other queues go into `points`, to be applied by the
    /// sub-block loop (see [`ParameterPoint`] for where each one applies). A
    /// point that leaves the parameter at the value it holds is left out,
    /// such as the first of the two points a host sends for a jump, or a
    /// value between two steps of a discrete parameter that rounds to the
    /// step it is at. A queue whose points (with the steps of its discrete
    /// ramps) do not all fit keeps only its last point, at the start of the
    /// block, which `overflowed` reports.
    fn take_parameter_changes(
        &self,
        input: ComRef<'_, IParameterChanges>,
        points: &mut Vec<ParameterPoint>,
    ) -> TakenChanges {
        let model = self.model();
        // SAFETY: input is a valid ComRef.
        let parameter_count = unsafe { input.getParameterCount() };
        let queues = || {
            (0..parameter_count).filter_map(|i| {
                // SAFETY: getParameterData may return null; ComRef::from_raw handles this.
                unsafe { ComRef::from_raw(input.getParameterData(i)) }
            })
        };

        let mut preset_applied = false;
        for queue in queues() {
            // SAFETY: queue is a valid ComRef.
            if unsafe { queue.getParameterId() } != PROGRAM_CHANGE_PARAM_ID {
                continue;
            }
            if let Some(index) = last_value(queue).and_then(Self::preset_index_for) {
                // A preset is a function, so what it changed is found by
                // comparing the values before and after it.
                // SAFETY: process() has the preset changes to itself.
                let changes = unsafe { &mut *self.preset_changes.get() };
                changes.clear();
                changes.extend(self.parameter_ids.iter().map(|&id| (id, model.get_normalized(id))));
                P::PRESETS[index].apply(model);
                changes.retain(|&(id, before)| model.get_normalized(id) != before);
                self.current_preset_index.store(index as u32, Ordering::Relaxed);
                preset_applied = true;
            }
        }

        let mut overflowed = false;
        for queue in queues() {
            // SAFETY: queue is a valid ComRef.
            let parameter_id = unsafe { queue.getParameterId() };
            // The program change is applied above. MIDI controller queues
            // and ids of no parameter are not parameters of the model.
            if parameter_id == PROGRAM_CHANGE_PARAM_ID {
                continue;
            }
            let Some(parameter) = beamer_core::parameter_types::Parameters::by_id(model, parameter_id) else {
                continue;
            };
            let queue_start = points.len();
            if !collect_points(parameter, parameter_id, queue, points) {
                overflowed = true;
                points.truncate(queue_start);
                if let Some(value) = last_value(queue) {
                    if points.len() < points.capacity() {
                        points.push(ParameterPoint { sample_offset: 0, id: parameter_id, value });
                    } else {
                        parameter.set_normalized(value);
                    }
                }
            }
        }

        TakenChanges { preset_applied, overflowed }
    }

    /// Report the parameters that the factory preset of this process call
    /// changed in `output`, so that the host and the edit controller learn
    /// them. Called at the end of the process call that applied the preset,
    /// so these are the values after its block, with the parameter changes
    /// of the block.
    fn report_preset_changes(&self, output: *mut IParameterChanges) {
        let model = self.model();
        // SAFETY: outputParameterChanges may be null; ComRef::from_raw handles this.
        let Some(output) = (unsafe { ComRef::from_raw(output) }) else {
            return;
        };
        // SAFETY: process() has the preset changes to itself.
        let changes = unsafe { &*self.preset_changes.get() };
        for &(id, _) in changes {
            let mut queue_index = 0;
            // SAFETY: output is a valid ComRef, the pointers are valid for the call.
            let queue = unsafe { ComRef::from_raw(output.addParameterData(&id, &mut queue_index)) };
            if let Some(queue) = queue {
                let mut point_index = 0;
                // SAFETY: queue is a valid ComRef, the pointer is valid for the call.
                unsafe { queue.addPoint(0, model.get_normalized(id), &mut point_index) };
            }
        }
    }

    /// Publish the timing of the processor, if there is one, at the end of
    /// a process call, after the parameter changes that may have changed it.
    ///
    /// Host threads do not touch the processor while audio runs. What they
    /// want to know about it is published for them with each call.
    fn publish_timing_if_prepared(&self) {
        // SAFETY: process() has the processor to itself.
        if let Some(processor) = unsafe { &*self.processor.get() } {
            self.publish_timing(processor);
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

    // =========================================================================
    // Bus and MIDI Answers (works in both states, not for process())
    // =========================================================================

    /// Get input bus count.
    #[inline]
    fn input_bus_count(&self) -> usize {
        self.model().input_bus_count()
    }

    /// Get output bus count.
    #[inline]
    fn output_bus_count(&self) -> usize {
        self.model().output_bus_count()
    }

    /// Get input bus info.
    /// Returns beamer_core::BusInfo (not vst3::BusInfo).
    #[inline]
    fn core_input_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
        self.model().input_bus_info(index)
    }

    /// Get output bus info.
    /// Returns beamer_core::BusInfo (not vst3::BusInfo).
    #[inline]
    fn core_output_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
        self.model().output_bus_info(index)
    }

    /// Whether the plugin has an event bus in this direction: input for
    /// MIDI it receives, output for MIDI it sends.
    #[inline]
    fn has_event_bus(&self, dir: BusDirection) -> bool {
        match dir as BusDirections {
            BusDirections_::kInput => self.config.midi_input,
            BusDirections_::kOutput => self.config.midi_output,
            _ => false,
        }
    }

    // =========================================================================
    // Model Access (works in both states)
    // =========================================================================

    /// Get the model (works in both states, from every thread).
    #[inline]
    fn model(&self) -> &P {
        &self.model
    }

    /// Get a handle to the model for an editor view. The view may live
    /// longer than a processor and uses the model from its own thread.
    #[cfg_attr(not(feature = "webview"), allow(dead_code))]
    fn shared_model(&self) -> Arc<dyn SharedModel> {
        let model: Arc<P> = Arc::clone(&self.model);
        model
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
    // Null buffers: a bus without a channel array has no channels. A null
    // channel pointer in it keeps its index; prepare_in_place gives it
    // memory of our own (silence for an input, discarded scratch for an
    // output).
    // =========================================================================

    /// Run a block of `num_samples` samples through the processor.
    ///
    /// With sample-accurate automation the block runs as sub-blocks that
    /// start at its parameter points; without it every point counts as due
    /// at the start, so the block runs as one after all of them, in their
    /// order. Before each sub-block its points are applied and
    /// `process_midi` gets its MIDI, relative to its start.
    /// `process_range` then processes the samples `start..start + len`.
    /// Points at or after the end of the block are applied after the last
    /// sub-block. MIDI there reaches the last sub-block at its last sample,
    /// since `num_samples` is not 0 and no sub-block stops the run.
    fn run_blocks(
        &self,
        num_samples: usize,
        processor: &mut P::Processor,
        input: BlockInput<'_>,
        mut process_range: impl FnMut(&mut P::Processor, usize, usize, &CoreProcessContext),
    ) {
        let model = self.model();
        let split = self.config.sample_accurate_automation;
        let BlockInput { points, midi_input, midi_output, transport, sample_rate } = input;

        let unreached = run_sub_blocks(
            num_samples,
            points,
            |point| if split { point.sample_offset } else { 0 },
            midi_input,
            |block| {
                for point in block.changes {
                    model.set_normalized(point.id, point.value);
                }

                // The plugin's MIDI output gets the offsets of the whole block.
                let output_start = midi_output.len();
                processor.process_midi(block.midi, midi_output);
                offset_from_sub_block(&mut midi_output.as_mut_slice()[output_start..], block.start);

                let block_transport = transport.advanced_by(block.start, sample_rate);
                let context = if let Some(cc_state) = self.midi_cc_state.as_ref() {
                    CoreProcessContext::with_midi_cc(sample_rate, block.len, block_transport, cc_state)
                } else {
                    CoreProcessContext::new(sample_rate, block.len, block_transport)
                };
                process_range(processor, block.start, block.len, &context);
                true
            },
        );

        for point in unreached.changes {
            model.set_normalized(point.id, point.value);
        }
    }

    /// Prepare the collected channels of this call for in-place processing
    /// and return the number of samples to process. A block the channels
    /// cannot be prepared for (longer than set up, with overlapping buffers)
    /// is rendered as silence, and 0 is returned.
    ///
    /// # Safety
    ///
    /// The collected pointers must be valid for `num_samples` samples, and no
    /// reference to their memory may be alive.
    unsafe fn prepare_channels<S: Sample>(
        &self,
        storage: &mut ProcessBufferStorage<S>,
        num_samples: usize,
    ) -> usize {
        // SAFETY: See the function's safety section.
        if unsafe { storage.prepare_in_place(num_samples) } {
            return num_samples;
        }
        // SAFETY: See the function's safety section.
        unsafe { storage.silence_outputs(num_samples) };
        self.process_warnings.note(ProcessWarning::BlockTooLong);
        0
    }

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
        input: BlockInput<'_>,
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
                // A null channel keeps its index (see prepare_in_place).
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    storage.main_inputs.push(ptr);
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
                // A null channel keeps its index (see prepare_in_place).
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    storage.main_outputs.push(ptr);
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
                        // A null channel keeps its index (see prepare_in_place).
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            storage.aux_inputs[aux_idx].push(ptr);
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
                        // A null channel keeps its index (see prepare_in_place).
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            storage.aux_outputs[aux_idx].push(ptr);
                        }
                    }
                }
            }
        }

        // SAFETY: The host guarantees the channel pointers valid for
        // num_samples elements for the duration of process().
        let render_samples = unsafe { self.prepare_channels(storage, num_samples) };
        let storage = &*storage;
        self.run_blocks(render_samples, processor, input, |processor, start, len, context| {
            // SAFETY: The channels were prepared for this call and the
            // sub-block lies within them.
            let (mut buffer, mut aux) = unsafe { storage.sub_block_buffers(start, len) };
            processor.process(&mut buffer, &mut aux, context);
        });
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
        input: BlockInput<'_>,
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
                // A null channel keeps its index (see prepare_in_place).
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    storage.main_inputs.push(ptr);
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
                // A null channel keeps its index (see prepare_in_place).
                for &ptr in channel_ptrs.iter().take(max_channels) {
                    storage.main_outputs.push(ptr);
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
                        // A null channel keeps its index (see prepare_in_place).
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            storage.aux_inputs[aux_idx].push(ptr);
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
                        // A null channel keeps its index (see prepare_in_place).
                        for &ptr in channel_ptrs.iter().take(max_channels) {
                            storage.aux_outputs[aux_idx].push(ptr);
                        }
                    }
                }
            }
        }

        // SAFETY: The host guarantees the channel pointers valid for
        // num_samples elements for the duration of process().
        let render_samples = unsafe { self.prepare_channels(storage, num_samples) };
        let storage = &*storage;
        self.run_blocks(render_samples, processor, input, |processor, start, len, context| {
            // SAFETY: The channels were prepared for this call and the
            // sub-block lies within them.
            let (mut buffer, mut aux) = unsafe { storage.sub_block_buffers(start, len) };
            processor.process_f64(&mut buffer, &mut aux, context);
        });
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
        input: BlockInput<'_>,
    ) {
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let conv = unsafe { &mut *self.conversion_buffers.get() };

        // The plugin processes the f32 output channels in place. They start
        // silent, and a channel with a host input gets it below.
        for channel in &mut conv.main_output_f32 {
            channel[..num_samples].fill(0.0);
        }
        for channel in conv.aux_output_f32.iter_mut().flatten() {
            channel[..num_samples].fill(0.0);
        }

        // Convert main input f64 → f32: into the output channel of the same
        // index, or into the input buffer for a channel without an output.
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
                    if !ptr.is_null() {
                        // SAFETY: Host guarantees channel pointer valid for num_samples elements.
                        let src = unsafe { slice::from_raw_parts(ptr, num_samples) };
                        let converted = match conv.main_output_f32.get_mut(ch) {
                            Some(channel) => channel,
                            None => &mut conv.main_input_f32[ch],
                        };
                        for (to, &from) in converted.iter_mut().zip(src) {
                            *to = from as f32;
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

        self.run_blocks(num_samples, processor, input, |processor, start, len, context| {
            // The f32 slices of this sub-block (no allocation).
            let range = start..start + len;
            let num_channels = conv.main_output_f32.len();
            let num_input_channels = conv.main_input_f32.len();
            let channel_iter = conv.main_output_f32.iter_mut().map(|v| &mut v[range.clone()]);
            let unpaired_input_iter =
                conv.main_input_f32.iter().skip(num_channels).map(|v| &v[range.clone()]);

            let aux_input_iter = conv
                .aux_input_f32
                .iter()
                .map(|bus| bus.iter().map(|v| &v[range.clone()]));
            let aux_output_iter = conv
                .aux_output_f32
                .iter_mut()
                .map(|bus| bus.iter_mut().map(|v| &mut v[range.clone()]));

            let mut buffer = Buffer::new(channel_iter, unpaired_input_iter, num_input_channels, len);
            let mut aux = AuxiliaryBuffers::new(aux_input_iter, aux_output_iter, len);
            processor.process(&mut buffer, &mut aux, context);
        });

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

impl<P: Model> ComponentFactory for Vst3Processor<P> {
    fn create(config: &'static Config) -> Option<Self> {
        let processor = Self::new(config);

        // Two parameters with one id would be one parameter to the host,
        // and so would a parameter with an id of the wrapper's own. The
        // reason also goes to standard error: a plugin has no logger unless
        // it sets `log_level` in its Config.toml.
        let parameters: &dyn beamer_core::parameter_types::Parameters = &*processor.model;
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

impl<P: Model> Class for Vst3Processor<P> {
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

impl<P: Model> IPluginBaseTrait for Vst3Processor<P> {
    unsafe fn initialize(&self, _context: *mut FUnknown) -> tresult {
        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        self.process_warnings.log();
        // The host is done with this instance: let go of its handler.
        // SAFETY: Null is allowed, and the host makes no other controller
        // call at the same time.
        unsafe { self.component_handler.set(std::ptr::null_mut()) };
        kResultOk
    }
}

// =============================================================================
// IComponent implementation
// =============================================================================

impl<P: Model> IComponentTrait for Vst3Processor<P> {
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
            // One event bus for MIDI input and one for MIDI output.
            MediaTypes_::kEvent if self.has_event_bus(dir) => 1,
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
                // Only index 0, and only in a direction with an event bus.
                if index != 0 || !self.has_event_bus(dir) {
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
        self.process_warnings.log();
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

        // The state is the model: its parameters and `State` fields. Their
        // values are atomic or shared, so they are loaded through a shared
        // reference: in both states, and also while process() runs, which is
        // when a host loads a preset during playback.
        match StateEntries::parse(&buffer) {
            Ok(Some(entries)) => {
                beamer_core::Saved::load_entries(self.model(), &entries, "");
                entries.log_unused();
                // The program the host showed when it saved. Only the
                // program change parameter changes: the preset is not
                // applied again over the loaded values. A state without
                // one, or with one this plugin does not have, shows the
                // first, like a new instance.
                let preset_index = entries
                    .factory_preset()
                    .filter(|&index| (index as usize) < P::PRESETS.len())
                    .unwrap_or(0);
                self.current_preset_index.store(preset_index, Ordering::Relaxed);
                // The smoothers belong to the audio thread: the next process()
                // call makes them jump to the loaded values, so that those do
                // not ramp in.
                self.smoothing_reset_requested.store(true, Ordering::Release);
                // A loaded state is one the host knows.
                #[cfg(feature = "webview")]
                self.known_state_generation
                    .store(beamer_core::Saved::state_generation(self.model()), Ordering::Relaxed);
                kResultOk
            }
            // The buffer is not empty, so there is a state.
            Ok(None) => kResultOk,
            Err(_) => kResultFalse,
        }
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        if state.is_null() {
            return kInvalidArgument;
        }

        // The state is the model: its parameters and `State` fields. Their
        // values are atomic or shared, so they are saved through a shared
        // reference: in both states, and also while process() runs, which is
        // when a host saves during playback.
        let mut writer = StateWriter::new();
        beamer_core::Saved::save_entries(self.model(), &mut writer, "");
        // With factory presets, also the program the host shows: a host
        // keeps no value of the program change parameter itself.
        if !P::PRESETS.is_empty() {
            writer.factory_preset(self.current_preset_index.load(Ordering::Relaxed));
        }
        let data: Vec<u8> = writer.finish();

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

impl<P: Model> IAudioProcessorTrait for Vst3Processor<P> {
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
        self.process_warnings.log();
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

        // Host threads keep using the model while it creates the processor
        // below.
        let model = self.model();
        // SAFETY: VST3 guarantees single-threaded access during setupProcessing(). No aliasing.
        let slot = unsafe { &mut *self.processor.get() };

        // A host may repeat a setup. Preparing again would then only throw
        // away what the processor holds. With a new sample rate, block size,
        // sample size or process mode the model is prepared again: it
        // learns those when a processor is created.
        if slot.is_none() || setup_changed {
            let input_bus_count = model.input_bus_count();
            let output_bus_count = model.output_bus_count();
            let input_buses: Vec<CoreBusInfo> = (0..input_bus_count)
                .filter_map(|i| model.input_bus_info(i))
                .collect();
            let output_buses: Vec<CoreBusInfo> = (0..output_bus_count)
                .filter_map(|i| model.output_bus_info(i))
                .collect();

            let bus_layout = BusLayout::from_model(model);

            // Validate model's bus configuration against compile-time limits
            if let Err(msg) = CachedBusConfig::from_model(model).validate() {
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

            // Build the model setup
            let plugin_setup = build_setup::<P::Setup>(setup, &bus_layout);

            // A processor of an earlier setup goes first, so that its
            // buffers are free before the new ones are allocated.
            *slot = None;

            // The smoothers ramp at the host's rate unless the model
            // sets another one in prepare.
            beamer_core::Parameters::set_sample_rate(self.model(), setup.sampleRate);

            // Create the processor, with its own handle to the model. A state
            // that was loaded before is in the model already.
            let processor = Arc::clone(&self.model).prepare(plugin_setup);

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
        // In debug builds, calls for host threads panic from here on.
        let _process = beamer_core::ProcessScope::enter();

        // SAFETY: data is non-null and host guarantees it points to valid ProcessData.
        let process_data = unsafe { &*data };
        let num_samples = process_data.numSamples as usize;

        // A state was loaded since the last block: the smoothers jump to the
        // loaded values before anything else happens to the parameters. They
        // belong to this thread.
        if self.smoothing_reset_requested.swap(false, Ordering::AcqRel) {
            self.model().reset_smoothing();
        }

        // 1. Take the parameter changes from the host: a preset change is
        // applied now, the points of the other parameters are applied at
        // their offsets while the block runs (see `run_blocks`).
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let points = unsafe { &mut *self.parameter_points.get() };
        points.clear();
        let mut preset_applied = false;
        // SAFETY: inputParameterChanges may be null; ComRef::from_raw handles this.
        if let Some(parameter_changes) = unsafe { ComRef::from_raw(process_data.inputParameterChanges) } {
            let taken = self.take_parameter_changes(parameter_changes, points);
            if taken.overflowed {
                self.process_warnings.note(ProcessWarning::ParameterEventOverflow);
            }
            preset_applied = taken.preset_applied;
        }
        sort_by_offset(points, |point| point.sample_offset);

        // 2. Handle MIDI events (reuse pre-allocated buffer to avoid stack overflow)
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let midi_input = unsafe { &mut *self.midi_input.get() };
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let carried = unsafe { &mut *self.midi_input_carried.get() };
        // Events of calls without samples wait in the buffer (see below).
        if !std::mem::take(carried) {
            midi_input.clear();
        }

        // A plugin without MIDI input gets no events, also from a host that
        // sends them without an event bus.
        // SAFETY: inputEvents may be null; ComRef::from_raw handles this.
        let input_events = unsafe { ComRef::from_raw(process_data.inputEvents) };
        if let Some(event_list) = input_events.filter(|_| self.config.midi_input) {
            // SAFETY: event_list is valid ComRef.
            let event_count = unsafe { event_list.getEventCount() };

            for i in 0..event_count {
                // SAFETY: zeroed Event is valid for VST3 Event union type.
                let mut event: Event = unsafe { std::mem::zeroed() };
                // SAFETY: event_list is valid, event is valid mutable pointer.
                if unsafe { event_list.getEvent(i, &mut event) } == kResultOk {
                    // SAFETY: event is valid Event populated by getEvent.
                    if let Some(midi_event) = unsafe { convert_vst3_to_midi(&event) } {
                        push_midi_input(midi_input, midi_event, num_samples == 0);
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
                                        push_midi_input(midi_input, midi_event, num_samples == 0);
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
            self.process_warnings.note(ProcessWarning::MidiInputOverflow);
        }

        // A host sends a block without samples to flush parameter changes
        // while it does not process audio. The parameter values are applied
        // here, in their order. Its MIDI events, including the controller
        // changes that came as parameter changes, stay in the buffer at the
        // start of the next block with samples: `process_midi` runs right
        // before a `process()` call, with the events of that call.
        if num_samples == 0 {
            for point in points.iter() {
                self.model().set_normalized(point.id, point.value);
            }
            if preset_applied {
                self.report_preset_changes(process_data.outputParameterChanges);
            }
            self.publish_timing_if_prepared();
            *carried = true;
            return kResultOk;
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
            self.process_warnings.note(ProcessWarning::SysExOutputDelayed);
        }
        // NOTE: Don't clear again - fallback events occupy slots 0..N, new events append after

        // 3. Extract transport info from VST3 ProcessContext
        // SAFETY: processContext may be null; extract_transport handles this.
        let transport = unsafe { extract_transport(process_data.processContext) };
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let sample_rate = unsafe { *self.sample_rate.get() };

        // The sub-blocks take the MIDI in order of its offsets. The
        // controller changes that came as parameter changes follow the
        // host's events in the buffer.
        sort_by_offset(midi_input.as_mut_slice(), |event| event.sample_offset);
        let input = BlockInput {
            points,
            midi_input: midi_input.as_mut_slice(),
            midi_output: &mut *midi_output,
            transport,
            sample_rate,
        };

        // 4. Process audio based on sample size: the parameter points, MIDI
        // and audio of the block run through `run_blocks`.
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let symbolic_sample_size = unsafe { *self.symbolic_sample_size.get() };
        // SAFETY: VST3 guarantees single-threaded access during process(). No aliasing.
        let processor = unsafe { self.processor_mut() };

        if symbolic_sample_size == SymbolicSampleSizes_::kSample64 as i32 {
            // 64-bit processing path
            if processor.supports_double_precision() {
                // Native f64: extract f64 buffers and call process_f64()
                // SAFETY: process_data is valid, processor is valid mutable reference.
                unsafe { self.process_audio_f64_native(process_data, num_samples, processor, input) };
            } else {
                // Conversion: f64→f32, process, f32→f64
                // SAFETY: process_data is valid, processor is valid mutable reference.
                unsafe { self.process_audio_f64_converted(process_data, num_samples, processor, input) };
            }
        } else {
            // 32-bit processing path (default)
            // SAFETY: process_data is valid, processor is valid mutable reference.
            unsafe { self.process_audio_f32(process_data, num_samples, processor, input) };
        }

        // Write output MIDI events. A plugin without MIDI output sends none,
        // also when the host passes an event list without an event bus.
        // SAFETY: outputEvents may be null; ComRef::from_raw handles this.
        let output_events = unsafe { ComRef::from_raw(process_data.outputEvents) };
        if let Some(event_list) = output_events.filter(|_| self.config.midi_output) {
            for midi_event in midi_output.iter() {
                if let Some(mut vst3_event) = convert_midi_to_vst3(midi_event, sysex_pool) {
                    // SAFETY: event_list is valid ComRef, vst3_event is valid mutable pointer.
                    let _ = unsafe { event_list.addEvent(&mut vst3_event) };
                }
            }
        }

        // Check for MIDI buffer overflow (once per block)
        if midi_output.has_overflowed() {
            self.process_warnings.note(ProcessWarning::MidiOutputOverflow);
        }

        // Check for SysEx pool overflow (once per block)
        if sysex_pool.has_overflowed() {
            self.process_warnings.note(ProcessWarning::SysExOutputOverflow);
        }

        if preset_applied {
            self.report_preset_changes(process_data.outputParameterChanges);
        }
        self.publish_timing_if_prepared();
        kResultOk
    }

    unsafe fn getTailSamples(&self) -> u32 {
        // The tail and the bypass ramp of the processor (see `publish_timing`).
        self.tail_samples.load(Ordering::Relaxed)
    }
}

impl<P: Model> IProcessContextRequirementsTrait for Vst3Processor<P> {
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

impl<P: Model> IEditControllerTrait for Vst3Processor<P> {
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
        let user_parameters = ParameterStore::count(self.model());
        // MIDI CC state is framework-owned, always available
        let cc_parameters = self
            .midi_cc_state
            .as_ref()
            .map(|s| s.enabled_count())
            .unwrap_or(0);
        // Add program change parameter if we have factory presets
        let preset_parameter = if !P::PRESETS.is_empty() { 1 } else { 0 };
        (user_parameters + cc_parameters + preset_parameter) as i32
    }

    unsafe fn getParameterInfo(&self, parameter_index: i32, info: *mut ParameterInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let model = self.model();
        let user_parameter_count = ParameterStore::count(model);

        // User-defined parameters first
        if (parameter_index as usize) < user_parameter_count {
            if let Some(parameter_info) = model.info(parameter_index as usize) {
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
        let preset_count = P::PRESETS.len();
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
            let preset_count = P::PRESETS.len();
            if preset_count > 0 {
                // Convert normalized value to preset index
                let step_count = (preset_count - 1).max(1) as f64;
                let preset_index = (value_normalized * step_count).round() as usize;
                let preset_index = preset_index.min(preset_count - 1);

                if let Some(name) = P::PRESETS.get(preset_index).map(Preset::name) {
                    // SAFETY: string is non-null (checked above) and host guarantees validity.
                    copy_wstring(name, unsafe { &mut *string });
                    return kResultOk;
                }
            }
            // SAFETY: string is non-null (checked above) and host guarantees validity.
            copy_wstring("", unsafe { &mut *string });
            return kResultOk;
        }

        let model = self.model();
        let display = model.normalized_to_string(id, value_normalized);
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
                let preset_count = P::PRESETS.len();
                // Find preset by name
                for i in 0..preset_count {
                    if let Some(name) = P::PRESETS.get(i).map(Preset::name) {
                        if name == s {
                            let step_count = (preset_count - 1).max(1) as f64;
                            // SAFETY: value_normalized is non-null (checked above).
                            unsafe { *value_normalized = (i as f64) / step_count };
                            return kResultOk;
                        }
                    }
                }
                return kInvalidArgument;
            }

            let model = self.model();
            if let Some(value) = model.string_to_normalized(id, &s) {
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
            let preset_count = P::PRESETS.len();
            if preset_count > 0 {
                let step_count = (preset_count - 1).max(1) as f64;
                return (value_normalized * step_count).round();
            }
            return 0.0;
        }
        self.model().normalized_to_plain(id, value_normalized)
    }

    unsafe fn plainParamToNormalized(&self, id: u32, plain_value: f64) -> f64 {
        // Handle program change parameter (index to normalized)
        if id == PROGRAM_CHANGE_PARAM_ID {
            let preset_count = P::PRESETS.len();
            if preset_count > 1 {
                let step_count = (preset_count - 1) as f64;
                return plain_value / step_count;
            }
            return 0.0;
        }
        self.model().plain_to_normalized(id, plain_value)
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
            let preset_count = P::PRESETS.len();
            if preset_count > 1 {
                let current_index = self.current_preset_index.load(Ordering::Relaxed);
                let step_count = (preset_count - 1) as f64;
                return (current_index as f64) / step_count;
            } else if preset_count == 1 {
                return 0.0;
            }
            return 0.0;
        }

        self.model().get_normalized(id)
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
            if let Some(preset_index) = Self::preset_index_for(value) {
                // Always apply unconditionally - never skip with "if changed" guard.
                // Hosts may re-send the same preset index (e.g., user clicks preset 0
                // when it's already selected) and skipping would break preset 0 on
                // fresh load when current_preset_index is initialized to 0.
                P::PRESETS[preset_index].apply(self.model());
                self.current_preset_index.store(preset_index as u32, Ordering::Relaxed);

                // Notify host that parameter values changed so UI refreshes.
                let handler = self.component_handler.get();
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

        self.model().set_normalized(id, value);
        kResultOk
    }

    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        // SAFETY: handler is a valid COM pointer or null, and the host makes
        // no other controller call at the same time.
        unsafe { self.component_handler.set(handler) };
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

            let view = crate::webview::WebViewPlugView::new(
                config,
                delegate,
                self.shared_model(),
                Arc::clone(&self.component_handler),
                // The web UI's invoke calls and events go to the model.
                Arc::clone(&self.model) as Arc<dyn WebViewDispatch>,
                self.webview_events.clone(),
                Arc::clone(&self.known_state_generation),
            );
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

impl<P: Model> IUnitInfoTrait for Vst3Processor<P> {
    unsafe fn getUnitCount(&self) -> i32 {
        self.model().group_count() as i32
    }

    unsafe fn getUnitInfo(&self, unit_index: i32, info: *mut UnitInfo) -> tresult {
        if info.is_null() || unit_index < 0 {
            return kInvalidArgument;
        }

        let model = self.model();

        if let Some(group_info) = model.group_info(unit_index as usize) {
            // SAFETY: info is non-null (checked above) and host guarantees validity.
            let info = unsafe { &mut *info };
            info.id = group_info.id;
            info.parentUnitId = group_info.parent_id;
            // Assign program list to root unit if we have presets
            info.programListId = if group_info.id == 0 && !P::PRESETS.is_empty() {
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
        if !P::PRESETS.is_empty() {
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
        if list_index != 0 || P::PRESETS.is_empty() {
            return kInvalidArgument;
        }

        // SAFETY: info is non-null (checked above) and host guarantees validity.
        let info = unsafe { &mut *info };
        info.id = FACTORY_PRESETS_LIST_ID;
        info.programCount = P::PRESETS.len() as i32;
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

        if let Some(preset_name) = P::PRESETS.get(program_index as usize).map(Preset::name) {
            // SAFETY: name is non-null (checked above) and host guarantees validity.
            copy_wstring(preset_name, unsafe { &mut *name });
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

impl<P: Model> IMidiMappingTrait for Vst3Processor<P> {
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
            let model = self.model();
            if let Some(parameter_id) = model.midi_cc_to_parameter(bus_index, channel, controller) {
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

impl<P: Model> IMidiLearnTrait for Vst3Processor<P> {
    unsafe fn onLiveMIDIControllerInput(
        &self,
        bus_index: i32,
        channel: i16,
        midi_cc: i16,
    ) -> tresult {
        if self.model().on_midi_learn(bus_index, channel, midi_cc as u8) {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

// =============================================================================
// IMidiMapping2 implementation (VST3 SDK 3.8.0 - MIDI 2.0)
// =============================================================================

impl<P: Model> IMidiMapping2Trait for Vst3Processor<P> {
    unsafe fn getNumMidi1ControllerAssignments(&self, direction: BusDirections) -> u32 {
        // Only support input direction
        if direction != BusDirections_::kInput {
            return 0;
        }
        self.model().midi1_assignments().len() as u32
    }

    unsafe fn getMidi1ControllerAssignments(
        &self,
        direction: BusDirections,
        list: *const Midi1ControllerParamIDAssignmentList,
    ) -> tresult {
        if list.is_null() || direction != BusDirections_::kInput {
            return kInvalidArgument;
        }

        let model = self.model();
        let assignments = model.midi1_assignments();
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
        self.model().midi2_assignments().len() as u32
    }

    unsafe fn getMidi2ControllerAssignments(
        &self,
        direction: BusDirections,
        list: *const Midi2ControllerParamIDAssignmentList,
    ) -> tresult {
        if list.is_null() || direction != BusDirections_::kInput {
            return kInvalidArgument;
        }

        let model = self.model();
        let assignments = model.midi2_assignments();
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

impl<P: Model> IMidiLearn2Trait for Vst3Processor<P> {
    unsafe fn onLiveMidi1ControllerInput(
        &self,
        bus_index: i32,
        channel: u8,
        midi_cc: i16,
    ) -> tresult {
        if self.model().on_midi1_learn(bus_index, channel, midi_cc as u8) {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn onLiveMidi2ControllerInput(
        &self,
        bus_index: i32,
        channel: u8,
        midi_cc: Midi2Controller,
    ) -> tresult {
        let controller = beamer_core::Midi2Controller {
            bank: midi_cc.bank,
            registered: midi_cc.registered != 0,
            index: midi_cc.index,
        };
        if self.model().on_midi2_learn(bus_index, channel, controller) {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

// =============================================================================
// INoteExpressionController implementation (VST3 SDK 3.5.0)
// =============================================================================

impl<P: Model> INoteExpressionControllerTrait for Vst3Processor<P> {
    unsafe fn getNoteExpressionCount(&self, bus_index: i32, channel: i16) -> i32 {
        self.model().note_expression_count(bus_index, channel) as i32
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

        let model = self.model();
        if let Some(expr_info) =
            model.note_expression_info(bus_index, channel, note_expression_index as usize)
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

        let model = self.model();
        let display = model.note_expression_value_to_string(bus_index, channel, id, value_normalized);
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
                let model = self.model();
                if let Some(value) = model.note_expression_string_to_value(bus_index, channel, id, &s) {
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

impl<P: Model> IKeyswitchControllerTrait for Vst3Processor<P> {
    unsafe fn getKeyswitchCount(&self, bus_index: i32, channel: i16) -> i32 {
        self.model().keyswitch_count(bus_index, channel) as i32
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

        let model = self.model();
        if let Some(ks_info) =
            model.keyswitch_info(bus_index, channel, keyswitch_index as usize)
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

impl<P: Model> INoteExpressionPhysicalUIMappingTrait for Vst3Processor<P> {
    unsafe fn getPhysicalUIMapping(
        &self,
        bus_index: i32,
        channel: i16,
        list: *mut PhysicalUIMapList,
    ) -> tresult {
        if list.is_null() {
            return kInvalidArgument;
        }

        let model = self.model();
        let mappings = model.physical_ui_mappings(bus_index, channel);
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

impl<P: Model> IVst3WrapperMPESupportTrait for Vst3Processor<P> {
    unsafe fn enableMPEInputProcessing(&self, state: TBool) -> tresult {
        if self.model().enable_mpe_input_processing(state != 0) {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn setMPEInputDeviceSettings(
        &self,
        master_channel: i32,
        member_begin_channel: i32,
        member_end_channel: i32,
    ) -> tresult {
        let settings = beamer_core::MpeInputDeviceSettings {
            master_channel,
            member_begin_channel,
            member_end_channel,
        };
        if self.model().set_mpe_input_device_settings(settings) {
            kResultOk
        } else {
            kResultFalse
        }
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

/// Add an event to the MIDI input of a process call.
///
/// The events of a call without samples wait for the next call with
/// samples, at offset 0. A host may send many such calls in a row, for
/// example while a controller moves on a suspended track. A controller event
/// then replaces the waiting one of the same controller and moves to the
/// end, so that only the latest value waits and the buffer does not fill up.
fn push_midi_input(input: &mut MidiBuffer, mut event: MidiEvent, waiting: bool) {
    if waiting {
        event.sample_offset = 0;
        if let Some(key) = controller_key(&event) {
            let events = input.as_mut_slice();
            if let Some(index) = events.iter().position(|old| controller_key(old) == Some(key)) {
                events[index..].rotate_left(1);
                if let Some(last) = events.last_mut() {
                    *last = event;
                }
                return;
            }
        }
    }
    input.push(event);
}

/// The controller an event sets the value of, if it is a controller event:
/// its kind, channel and controller number or note.
///
/// Of two events with one key, the later one makes the earlier one obsolete.
fn controller_key(event: &MidiEvent) -> Option<(u8, u8, u8)> {
    match &event.event {
        MidiEventKind::ControlChange(cc) => Some((0, cc.channel, cc.controller)),
        MidiEventKind::PitchBend(bend) => Some((1, bend.channel, 0)),
        MidiEventKind::ChannelPressure(pressure) => Some((2, pressure.channel, 0)),
        MidiEventKind::PolyPressure(pressure) => Some((3, pressure.channel, pressure.pitch)),
        MidiEventKind::ProgramChange(program) => Some((4, program.channel, 0)),
        _ => None,
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
            // A note-on with velocity 0 ends the note, as in MIDI 1.0 and
            // as the Audio Unit wrapper reads it.
            if note_on.velocity <= 0.0 {
                return Some(MidiEvent::note_off(
                    sample_offset,
                    note_on.channel as u8,
                    note_on.pitch as u8,
                    0.0,
                    note_id,
                    note_on.tuning,
                ));
            }
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
                // Hosts send the message framed by F0 and F7; the event holds
                // the payload.
                let sysex = if data_event.size > 0 && !data_event.bytes.is_null() {
                    // SAFETY: bytes is non-null and host guarantees validity for size bytes.
                    let message = unsafe { std::slice::from_raw_parts(data_event.bytes, data_event.size as usize) };
                    SysEx::from_message(message)
                } else {
                    SysEx::new()
                };
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
            // Hosts expect the message framed by F0 and F7. It goes into a
            // slot of the pool for stable pointer storage, so it is framed to
            // fit one.
            let mut buffer = [0u8; MAX_SYSEX_SIZE + 2];
            let slot = sysex_pool.buffer_size().min(buffer.len());
            if let Some((ptr, len)) = sysex_pool.allocate(sysex.write_framed(&mut buffer[..slot])) {
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
    struct SetupModel {
        parameters: NoParameters,
    }

    struct SetupProcessor {
        setup: RecordedSetup,
    }

    beamer_core::__forward_model!(SetupModel, parameters);

    impl Model for SetupModel {
        type Setup = RecordedSetup;
        type Processor = SetupProcessor;

        fn prepare(self: Arc<Self>, setup: RecordedSetup) -> SetupProcessor {
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

    /// Number of `prepare` calls on `SetupModel`. Only one test uses the plugin.
    static PREPARE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    static TEST_CONFIG: Config = Config::new("Setup Test", Category::Effect, "Test", "stup");

    /// Call setupProcessing like a host and return the setup the plugin holds.
    fn setup_processing(
        wrapper: &Vst3Processor<SetupModel>,
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
        let wrapper = Vst3Processor::<SetupModel>::new(&TEST_CONFIG);
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
    use beamer_core::preset::Preset;
    use beamer_core::ParameterGroups;
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
        /// What `SMOOTHING_SAMPLE_RATE` was when a `StateModel` was last
        /// prepared on this thread.
        static SMOOTHING_SAMPLE_RATE_AT_PREPARE: Cell<f64> = const { Cell::new(0.0) };
        /// How many `StateProcessor`s exist on this thread.
        static LIVE_PROCESSORS: Cell<usize> = const { Cell::new(0) };
        /// What `LIVE_PROCESSORS` was when a `StateModel` was last prepared
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

    impl beamer_core::Saved for StateParameters {}

    impl beamer_core::WebViewEvents for StateParameters {}

    #[derive(Default)]
    struct StateModel {
        parameters: StateParameters,
    }

    struct StateProcessor {
        model: Arc<StateModel>,
    }

    impl Drop for StateProcessor {
        fn drop(&mut self) {
            // A thread that ends may have dropped its counter already.
            let _ = LIVE_PROCESSORS.try_with(|live| live.set(live.get().saturating_sub(1)));
        }
    }

    beamer_core::__forward_model!(StateModel, parameters);

    impl Model for StateModel {
        type Setup = ();
        type Processor = StateProcessor;

        fn prepare(self: Arc<Self>, _setup: ()) -> StateProcessor {
            SMOOTHING_SAMPLE_RATE_AT_PREPARE
                .with(|rate| rate.set(SMOOTHING_SAMPLE_RATE.with(Cell::get)));
            LIVE_PROCESSORS_AT_PREPARE.with(|live| live.set(LIVE_PROCESSORS.with(Cell::get)));
            LIVE_PROCESSORS.with(|live| live.set(live.get() + 1));
            StateProcessor { model: self }
        }
    }

    impl Processor for StateProcessor {
        fn process(
            &mut self,
            _buffer: &mut beamer_core::Buffer,
            _aux: &mut beamer_core::AuxiliaryBuffers,
            _context: &CoreProcessContext,
        ) {
            // The wrapper marks the time a plugin processes.
            assert_eq!(beamer_core::audio_thread::in_process(), cfg!(debug_assertions));
        }

        fn latency_samples(&self) -> u32 {
            latency_for(self.model.parameters.gain.get_normalized())
        }

        fn tail_samples(&self) -> u32 {
            STATE_TAIL_SAMPLES
        }

        fn bypass_ramp_samples(&self) -> u32 {
            STATE_BYPASS_RAMP_SAMPLES
        }
    }

    static STATE_CONFIG: Config = Config::new("State Test", Category::Effect, "Test", "stat");

    /// A plugin with a sidechain, which maps and learns MIDI controllers.
    /// Its config says which event buses it has.
    #[derive(Default)]
    struct MidiModel {
        parameters: NoParameters,
        /// The controller of the last MIDI learn call.
        learned: Mutex<Option<u8>>,
    }

    struct MidiProcessor;

    beamer_core::__forward_model!(MidiModel, parameters);

    impl Model for MidiModel {
        type Setup = ();
        type Processor = MidiProcessor;

        fn prepare(self: Arc<Self>, _setup: ()) -> MidiProcessor {
            MidiProcessor
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

        fn on_midi_learn(&self, _bus_index: i32, _channel: i16, cc: u8) -> bool {
            assert_ne!(cc, PANICKING_CC, "a plugin that panics in a callback");
            *self.learned.lock().expect("no test panics while holding it") = Some(cc);
            true
        }
    }

    impl MidiModel {
        /// The controller of the last MIDI learn call.
        fn learned(&self) -> Option<u8> {
            *self.learned.lock().expect("no test panics while holding it")
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

    static MIDI_CONFIG: Config =
        Config::new("Midi Test", Category::Instrument, "Test", "midi").with_midi_output(true);

    thread_local! {
        /// The input of each `process_midi` call of a `CcProcessor` on this
        /// thread.
        static CC_PROCESSOR_MIDI: std::cell::RefCell<Vec<Vec<beamer_core::MidiEvent>>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// A plugin that gets the mod wheel as a parameter change, and sends
    /// the MIDI it gets back.
    #[derive(Default)]
    struct CcModel {
        parameters: NoParameters,
    }

    struct CcProcessor;

    beamer_core::__forward_model!(CcModel, parameters);

    impl Model for CcModel {
        type Setup = ();
        type Processor = CcProcessor;

        fn prepare(self: Arc<Self>, _setup: ()) -> CcProcessor {
            CcProcessor
        }

        fn midi_cc_config(&self) -> Option<beamer_core::MidiCcConfig> {
            Some(beamer_core::MidiCcConfig::new().with_mod_wheel())
        }
    }

    impl Processor for CcProcessor {
        fn process(
            &mut self,
            _buffer: &mut beamer_core::Buffer,
            _aux: &mut beamer_core::AuxiliaryBuffers,
            _context: &CoreProcessContext,
        ) {
        }

        fn process_midi(&mut self, input: &[beamer_core::MidiEvent], output: &mut beamer_core::MidiBuffer) {
            CC_PROCESSOR_MIDI.with(|calls| calls.borrow_mut().push(input.to_vec()));
            for event in input {
                output.push(event.clone());
            }
        }
    }

    static CC_CONFIG: Config = Config::new("Cc Test", Category::Instrument, "Test", "ccts");

    #[test]
    fn test_midi_of_a_process_call_without_samples_reaches_the_next_block() {
        let wrapper = Vst3Processor::<CcModel>::new(&CC_CONFIG);
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 44100.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        CC_PROCESSOR_MIDI.with(|calls| calls.borrow_mut().clear());
        let calls = || CC_PROCESSOR_MIDI.with(|calls| calls.borrow().clone());

        // A host flushes the mod wheel while it does not process audio. No
        // process() call follows, so the plugin gets nothing yet.
        let mod_wheel = changes_of(&[(MidiCcState::parameter_id(1), 0.5)]);
        process_with(&wrapper, 0, mod_wheel.as_ptr(), std::ptr::null_mut());
        assert!(calls().is_empty());

        // The next block with samples starts with it.
        process_with(&wrapper, 64, std::ptr::null_mut(), std::ptr::null_mut());
        let received = calls();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].len(), 1);
        assert_eq!(received[0][0].sample_offset, 0);
        assert!(matches!(
            &received[0][0].event,
            beamer_core::MidiEventKind::ControlChange(cc) if cc.controller == 1
        ));

        // And the block after that has no events left.
        process_with(&wrapper, 64, std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(calls().get(1).map(Vec::len), Some(0));
    }

    #[test]
    fn test_midi_follows_the_config_whatever_the_host_sends() {
        static WITHOUT_INPUT: Config = Config::new("No Input", Category::Instrument, "Test", "noin")
            .with_midi_input(false)
            .with_midi_output(true);
        static WITHOUT_OUTPUT: Config = Config::new("No Output", Category::Instrument, "Test", "noou");
        static WITH_BOTH: Config =
            Config::new("Both", Category::Instrument, "Test", "both").with_midi_output(true);

        // A host that passes event lists both ways, whatever buses the
        // plugin has, and sends a note. Returns how many events the plugin
        // got and how many the host got back.
        let run = |config: &'static Config| {
            let wrapper = Vst3Processor::<CcModel>::new(config);
            let mut setup = ProcessSetup {
                processMode: ProcessModes_::kRealtime as int32,
                symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
                maxSamplesPerBlock: 64,
                sampleRate: 44100.0,
            };
            // SAFETY: The setup is valid for the call.
            assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
            CC_PROCESSOR_MIDI.with(|calls| calls.borrow_mut().clear());

            let mut pool = SysExOutputPool::with_capacity(1, 16);
            let note = convert_midi_to_vst3(&beamer_core::MidiEvent::note_on(0, 0, 60, 1.0, 0, 0.0, 0), &mut pool)
                .expect("a note-on converts");
            let input = vst3::ComWrapper::new(Events { events: std::cell::RefCell::new(vec![note]) });
            let output = vst3::ComWrapper::new(Events::default());
            let input_list = input.to_com_ptr::<IEventList>().expect("the events are an IEventList");
            let output_list = output.to_com_ptr::<IEventList>().expect("the events are an IEventList");
            let no_changes = std::ptr::null_mut();
            process_with_events(&wrapper, 64, no_changes, no_changes, input_list.as_ptr(), output_list.as_ptr());

            let received = CC_PROCESSOR_MIDI.with(|calls| calls.borrow().iter().map(Vec::len).sum::<usize>());
            let sent = output.events.borrow().len();
            (received, sent)
        };

        assert_eq!(run(&WITH_BOTH), (1, 1));
        assert_eq!(run(&WITHOUT_INPUT), (0, 0));
        assert_eq!(run(&WITHOUT_OUTPUT), (1, 0));
    }

    #[test]
    fn test_midi_controllers_need_midi_input() {
        static WITHOUT_INPUT: Config = Config::new("No Input", Category::Effect, "Test", "nocc");
        let wrapper = Vst3Processor::<CcModel>::new(&WITHOUT_INPUT);

        // The host finds no mapping for the mod wheel.
        let mut id = 0u32;
        // SAFETY: The wrapper and the id are alive for the call.
        let mapped = unsafe { wrapper.getMidiControllerAssignment(0, 0, 1, &mut id) };
        assert_ne!(mapped, kResultOk);

        // And a change of the parameter it would have mapped to reaches the
        // plugin as no event.
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 44100.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        CC_PROCESSOR_MIDI.with(|calls| calls.borrow_mut().clear());
        let mod_wheel = changes_of(&[(MidiCcState::parameter_id(1), 0.5)]);
        process_with(&wrapper, 64, mod_wheel.as_ptr(), std::ptr::null_mut());
        let received = CC_PROCESSOR_MIDI.with(|calls| calls.borrow().clone());
        assert_eq!(received, [vec![]]);
    }

    #[test]
    fn test_only_the_latest_controller_value_waits_for_a_block() {
        let wrapper = Vst3Processor::<CcModel>::new(&CC_CONFIG);
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 44100.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        CC_PROCESSOR_MIDI.with(|calls| calls.borrow_mut().clear());

        // More calls without samples than the buffer holds events, while
        // the mod wheel moves.
        let calls = 2 * beamer_core::midi::MAX_MIDI_EVENTS;
        for index in 0..calls {
            let value = index as f64 / calls as f64;
            let mod_wheel = changes_of(&[(MidiCcState::parameter_id(1), value)]);
            process_with(&wrapper, 0, mod_wheel.as_ptr(), std::ptr::null_mut());
        }

        process_with(&wrapper, 64, std::ptr::null_mut(), std::ptr::null_mut());
        let received = CC_PROCESSOR_MIDI.with(|calls| calls.borrow().clone());
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].len(), 1);
        let beamer_core::MidiEventKind::ControlChange(cc) = &received[0][0].event else {
            panic!("a controller change");
        };
        let last = (calls - 1) as f32 / calls as f32;
        assert!((cc.value - last).abs() < 1e-6, "{} is not the last value {}", cc.value, last);
    }

    #[test]
    fn test_note_on_with_velocity_zero_is_a_note_off() {
        // SAFETY: zeroed Event is valid for VST3 Event union type.
        let mut event: Event = unsafe { std::mem::zeroed() };
        event.r#type = K_NOTE_ON_EVENT;
        event.sampleOffset = 12;
        event.__field0.noteOn.channel = 2;
        event.__field0.noteOn.pitch = 64;
        event.__field0.noteOn.noteId = -1;

        // SAFETY: The event is a valid note-on event.
        let received = unsafe { convert_vst3_to_midi(&event) }.expect("a note event");
        let MidiEventKind::NoteOff(note_off) = &received.event else {
            panic!("a note-off, not {:?}", received.event);
        };
        assert_eq!(received.sample_offset, 12);
        assert_eq!((note_off.channel, note_off.pitch, note_off.note_id), (2, 64, 64));

        // Any velocity above 0 starts a note.
        event.__field0.noteOn.velocity = 0.01;
        // SAFETY: The event is a valid note-on event.
        let received = unsafe { convert_vst3_to_midi(&event) }.expect("a note event");
        assert!(matches!(received.event, MidiEventKind::NoteOn(_)));
    }

    #[test]
    fn test_sysex_is_framed_for_the_host_and_not_for_the_plugin() {
        let message = [0xF0u8, 0x7E, 0x00, 0x06, 0x01, 0xF7];

        // From the host: the event holds the payload.
        // SAFETY: zeroed Event is valid for VST3 Event union type.
        let mut event: Event = unsafe { std::mem::zeroed() };
        event.r#type = K_DATA_EVENT;
        event.__field0.data.r#type = DATA_TYPE_MIDI_SYSEX;
        event.__field0.data.size = message.len() as u32;
        event.__field0.data.bytes = message.as_ptr();
        // SAFETY: The event and its bytes are valid for the call.
        let received = unsafe { convert_vst3_to_midi(&event) }.expect("a SysEx event");
        let MidiEventKind::SysEx(sysex) = &received.event else {
            panic!("a SysEx event");
        };
        assert_eq!(sysex.as_slice(), &message[1..5]);

        // To the host: framed, also at the largest payload, which is cut to
        // the slot size before its F7.
        let slot_payload = beamer_core::config::DEFAULT_SYSEX_BUFFER_SIZE;
        let mut pool = SysExOutputPool::with_capacity(2, slot_payload + 2);
        let largest = MidiEvent::sysex(0, &[0x01; MAX_SYSEX_SIZE]);
        let largest_len = MAX_SYSEX_SIZE.min(slot_payload) + 2;
        for (event, expected_len) in [(received, message.len()), (largest, largest_len)] {
            let sent = convert_midi_to_vst3(&event, &mut pool).expect("a data event");
            // SAFETY: The event is a data event with bytes in the pool.
            let bytes = unsafe { std::slice::from_raw_parts(sent.__field0.data.bytes, sent.__field0.data.size as usize) };
            assert_eq!(bytes.len(), expected_len);
            assert_eq!((bytes[0], bytes[bytes.len() - 1]), (0xF0, 0xF7));
        }
    }

    #[test]
    fn test_a_waiting_controller_event_moves_behind_later_events() {
        let mut input = MidiBuffer::new();
        push_midi_input(&mut input, MidiEvent::control_change(0, 0, 64, 1.0), true);
        push_midi_input(&mut input, MidiEvent::note_on(0, 0, 60, 1.0, 0, 0.0, 0), true);
        push_midi_input(&mut input, MidiEvent::control_change(0, 0, 64, 0.0), true);
        // Another controller and a block with samples are left alone.
        push_midi_input(&mut input, MidiEvent::control_change(0, 0, 1, 0.5), true);
        push_midi_input(&mut input, MidiEvent::control_change(5, 0, 1, 0.7), false);

        let kinds: Vec<_> = input
            .as_slice()
            .iter()
            .map(|event| match &event.event {
                MidiEventKind::ControlChange(cc) => (event.sample_offset, cc.controller, cc.value),
                _ => (event.sample_offset, 0, -1.0),
            })
            .collect();
        assert_eq!(kinds, [(0, 0, -1.0), (0, 64, 0.0), (0, 1, 0.5), (5, 1, 0.7)]);
    }

    /// The bus counts a host sees: audio in, audio out, event in, event out.
    fn bus_counts(wrapper: &Vst3Processor<MidiModel>) -> [i32; 4] {
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
    fn input_channels(wrapper: &Vst3Processor<MidiModel>, index: i32) -> Option<i32> {
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
        let wrapper = Vst3Processor::<MidiModel>::new(&MIDI_CONFIG);
        let event_buses = |wrapper: &Vst3Processor<MidiModel>| {
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
    fn test_event_buses_follow_the_config() {
        static INSTRUMENT: Config = Config::new("Instrument", Category::Instrument, "Test", "inst");
        static EFFECT: Config = Config::new("Effect", Category::Effect, "Test", "efct");
        static MUSIC_EFFECT: Config =
            Config::new("Music Effect", Category::Effect, "Test", "mfx_").with_midi_input(true);
        static MIDI_EFFECT: Config = Config::new("Midi Effect", Category::MidiEffect, "Test", "mfx2");
        static TRACKER: Config =
            Config::new("Tracker", Category::Effect, "Test", "trck").with_midi_output(true);

        // The event buses in and out.
        let event_buses = |config: &'static Config| {
            let [.., event_in, event_out] = bus_counts(&Vst3Processor::<MidiModel>::new(config));
            [event_in, event_out]
        };
        assert_eq!(event_buses(&INSTRUMENT), [1, 0]);
        assert_eq!(event_buses(&EFFECT), [0, 0]);
        assert_eq!(event_buses(&MUSIC_EFFECT), [1, 0]);
        assert_eq!(event_buses(&MIDI_EFFECT), [1, 1]);
        assert_eq!(event_buses(&TRACKER), [0, 1]);

        // A host gets no info for a bus that is not there.
        let wrapper = Vst3Processor::<MidiModel>::new(&INSTRUMENT);
        let event_info = |dir: BusDirections| {
            // SAFETY: A zeroed BusInfo is valid, and it is alive for the call.
            let mut info: BusInfo = unsafe { std::mem::zeroed() };
            // SAFETY: The wrapper and the info are alive for the call.
            unsafe { wrapper.getBusInfo(MediaTypes_::kEvent as MediaType, dir as BusDirection, 0, &mut info) }
        };
        assert_eq!(event_info(BusDirections_::kInput), kResultOk);
        assert_eq!(event_info(BusDirections_::kOutput), kInvalidArgument);
    }

    #[test]
    fn test_model_answers_after_a_panic_in_one_of_its_calls() {
        let wrapper = Vst3Processor::<MidiModel>::new(&MIDI_CONFIG);

        // Nothing of the wrapper stands between a host call and the model,
        // so a panic in one call leaves nothing behind for the next. The plugin panics in a call of the host.
        let learn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: The wrapper is alive for the call.
            unsafe { wrapper.onLiveMIDIControllerInput(0, 0, PANICKING_CC as i16) }
        }));
        assert!(learn.is_err());

        // The host still gets its answers.
        assert_eq!(bus_counts(&wrapper), [2, 1, 1, 1]);
        // SAFETY: The wrapper is alive for the call.
        assert_eq!(unsafe { wrapper.onLiveMIDIControllerInput(0, 0, 1) }, kResultOk);
        assert_eq!(wrapper.model().learned(), Some(1));
    }

    #[test]
    fn test_model_answers_midi_mapping_prepared_or_not() {
        let wrapper = Vst3Processor::<MidiModel>::new(&MIDI_CONFIG);
        let mapped = |wrapper: &Vst3Processor<MidiModel>| {
            let mut id = 0u32;
            // SAFETY: The wrapper and the id are alive for the call.
            let result = unsafe {
                wrapper.getMidiControllerAssignment(0, 0, MAPPED_CC as i16, &mut id)
            };
            (result, id)
        };
        let learn = |wrapper: &Vst3Processor<MidiModel>, cc: u8| {
            // SAFETY: The wrapper is alive for the call.
            let result = unsafe { wrapper.onLiveMIDIControllerInput(0, 0, cc as i16) };
            (result, wrapper.model().learned())
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

        // A prepared plugin still has its model to answer.
        assert_eq!(mapped(&wrapper), (kResultOk, MAPPED_PARAMETER));
        assert_eq!(learn(&wrapper, 7), (kResultOk, Some(7)));
    }

    fn prepared_state_wrapper() -> Vst3Processor<StateModel> {
        let wrapper = Vst3Processor::<StateModel>::new(&STATE_CONFIG);
        prepare_state_wrapper(&wrapper);
        wrapper
    }

    fn prepare_state_wrapper<P: Model>(wrapper: &Vst3Processor<P>) {
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
    fn process_block(wrapper: &Vst3Processor<StateModel>) {
        process_with(wrapper, 64, std::ptr::null_mut(), std::ptr::null_mut());
    }

    /// Call process like a host, with `frames` of the 64 frames the buffers
    /// hold, with the parameter changes of the block and a list for the
    /// changes the plugin reports, if any.
    fn process_with<P>(
        wrapper: &Vst3Processor<P>,
        frames: int32,
        changes: *mut IParameterChanges,
        output_changes: *mut IParameterChanges,
    ) where
        P: Model,
    {
        let no_events = std::ptr::null_mut();
        process_with_events(wrapper, frames, changes, output_changes, no_events, no_events);
    }

    /// Like `process_with`, with event lists for the MIDI in and out.
    fn process_with_events<P>(
        wrapper: &Vst3Processor<P>,
        frames: int32,
        changes: *mut IParameterChanges,
        output_changes: *mut IParameterChanges,
        input_events: *mut IEventList,
        output_events: *mut IEventList,
    ) where
        P: Model,
    {
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
            outputParameterChanges: output_changes,
            inputEvents: input_events,
            outputEvents: output_events,
            processContext: std::ptr::null_mut(),
        };
        // SAFETY: The data and its buffers are valid for the call.
        assert_eq!(unsafe { wrapper.process(&mut data) }, kResultOk);
    }

    /// The events a host sends with a process call, or gets back from one.
    #[derive(Default)]
    struct Events {
        events: std::cell::RefCell<Vec<Event>>,
    }

    impl Class for Events {
        type Interfaces = (IEventList,);
    }

    impl IEventListTrait for Events {
        unsafe fn getEventCount(&self) -> int32 {
            self.events.borrow().len() as int32
        }

        unsafe fn getEvent(&self, index: int32, event: *mut Event) -> tresult {
            match self.events.borrow().get(index as usize) {
                Some(stored) => {
                    // SAFETY: The wrapper passes a valid event to fill.
                    unsafe { *event = *stored };
                    kResultOk
                }
                None => kInvalidArgument,
            }
        }

        unsafe fn addEvent(&self, event: *mut Event) -> tresult {
            // SAFETY: The wrapper passes a valid event.
            self.events.borrow_mut().push(unsafe { *event });
            kResultOk
        }
    }

    /// The points of one parameter in a block, as (sample offset, value).
    struct PointQueue {
        id: u32,
        points: Vec<(int32, f64)>,
    }

    impl Class for PointQueue {
        type Interfaces = (IParamValueQueue,);
    }

    impl IParamValueQueueTrait for PointQueue {
        unsafe fn getParameterId(&self) -> ParamID {
            self.id
        }

        unsafe fn getPointCount(&self) -> int32 {
            self.points.len() as int32
        }

        unsafe fn getPoint(
            &self,
            index: int32,
            sample_offset: *mut int32,
            value: *mut ParamValue,
        ) -> tresult {
            let Some(&(offset, point_value)) = usize::try_from(index).ok().and_then(|i| self.points.get(i))
            else {
                return kResultFalse;
            };
            // SAFETY: The caller provides valid pointers.
            unsafe {
                *sample_offset = offset;
                *value = point_value;
            }
            kResultOk
        }

        unsafe fn addPoint(&self, _sample_offset: int32, _value: ParamValue, _index: *mut int32) -> tresult {
            kNotImplemented
        }
    }

    /// The parameter changes a host sends with a process call.
    struct Changes {
        queues: Vec<vst3::ComPtr<IParamValueQueue>>,
    }

    impl Class for Changes {
        type Interfaces = (IParameterChanges,);
    }

    impl IParameterChangesTrait for Changes {
        unsafe fn getParameterCount(&self) -> int32 {
            self.queues.len() as int32
        }

        unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
            usize::try_from(index)
                .ok()
                .and_then(|index| self.queues.get(index))
                .map_or(std::ptr::null_mut(), |queue| queue.as_ptr())
        }

        unsafe fn addParameterData(&self, _id: *const ParamID, _index: *mut int32) -> *mut IParamValueQueue {
            std::ptr::null_mut()
        }
    }

    /// Call process with one parameter change and `frames` frames.
    fn process_change(wrapper: &Vst3Processor<StateModel>, frames: int32, id: u32, value: f64) {
        let changes = changes_of(&[(id, value)]);
        process_with(wrapper, frames, changes.as_ptr(), std::ptr::null_mut());
    }

    /// The parameter changes of a block, one value per parameter at its
    /// start.
    fn changes_of(values: &[(u32, f64)]) -> vst3::ComPtr<IParameterChanges> {
        let queues: Vec<_> = values.iter().map(|&(id, value)| (id, vec![(0, value)])).collect();
        changes_with_points(&queues)
    }

    /// The parameter changes of a block, with the points of each parameter.
    fn changes_with_points(queues: &[(u32, Vec<(int32, f64)>)]) -> vst3::ComPtr<IParameterChanges> {
        let queues = queues
            .iter()
            .map(|(id, points)| {
                vst3::ComWrapper::new(PointQueue { id: *id, points: points.clone() })
                    .to_com_ptr::<IParamValueQueue>()
                    .expect("the queue is an IParamValueQueue")
            })
            .collect();
        vst3::ComWrapper::new(Changes { queues })
            .to_com_ptr::<IParameterChanges>()
            .expect("the changes are IParameterChanges")
    }

    /// A queue of the output parameter changes that records its points.
    struct RecordingQueue {
        id: u32,
        points: Mutex<Vec<(int32, f64)>>,
    }

    impl Class for RecordingQueue {
        type Interfaces = (IParamValueQueue,);
    }

    impl IParamValueQueueTrait for RecordingQueue {
        unsafe fn getParameterId(&self) -> ParamID {
            self.id
        }

        unsafe fn getPointCount(&self) -> int32 {
            self.points.lock().unwrap().len() as int32
        }

        unsafe fn getPoint(&self, _index: int32, _offset: *mut int32, _value: *mut ParamValue) -> tresult {
            kNotImplemented
        }

        unsafe fn addPoint(&self, sample_offset: int32, value: ParamValue, index: *mut int32) -> tresult {
            let mut points = self.points.lock().unwrap();
            // SAFETY: The caller provides a valid pointer.
            unsafe { *index = points.len() as int32 };
            points.push((sample_offset, value));
            kResultOk
        }
    }

    /// The output parameter changes of a process call, recorded.
    #[derive(Default)]
    struct OutputChanges {
        queues: Mutex<Vec<vst3::ComWrapper<RecordingQueue>>>,
    }

    impl OutputChanges {
        /// Each parameter the plugin reported, with its points.
        fn reported(&self) -> Vec<(u32, Vec<(int32, f64)>)> {
            let queues = self.queues.lock().unwrap();
            queues
                .iter()
                .map(|queue| (queue.id, queue.points.lock().unwrap().clone()))
                .collect()
        }
    }

    impl Class for OutputChanges {
        type Interfaces = (IParameterChanges,);
    }

    impl IParameterChangesTrait for OutputChanges {
        unsafe fn getParameterCount(&self) -> int32 {
            self.queues.lock().unwrap().len() as int32
        }

        unsafe fn getParameterData(&self, _index: int32) -> *mut IParamValueQueue {
            std::ptr::null_mut()
        }

        unsafe fn addParameterData(&self, id: *const ParamID, index: *mut int32) -> *mut IParamValueQueue {
            let mut queues = self.queues.lock().unwrap();
            // SAFETY: The caller provides valid pointers.
            let id = unsafe { *id };
            // SAFETY: See above.
            unsafe { *index = queues.len() as int32 };
            let queue = vst3::ComWrapper::new(RecordingQueue { id, points: Mutex::new(Vec::new()) });
            let pointer = queue
                .as_com_ref::<IParamValueQueue>()
                .expect("the queue is an IParamValueQueue")
                .as_ptr();
            queues.push(queue);
            pointer
        }
    }

    /// The parameters of the `StateModel` with two factory presets for the
    /// gain, whose range is 0 to 1.
    #[derive(Default)]
    struct GainPresetModel {
        parameters: StateParameters,
    }

    struct GainPresetProcessor;

    beamer_core::__forward_model!(GainPresetModel, parameters);

    impl Model for GainPresetModel {
        type Setup = ();
        type Processor = GainPresetProcessor;
        const PRESETS: &'static [Preset<Self>] = &[
            Preset::new("Low", |m| m.parameters.gain.set(0.2)),
            Preset::new("High", |m| m.parameters.gain.set(0.9)),
        ];

        fn prepare(self: Arc<Self>, _setup: ()) -> GainPresetProcessor {
            GainPresetProcessor
        }
    }

    impl Processor for GainPresetProcessor {
        fn process(
            &mut self,
            _buffer: &mut beamer_core::Buffer,
            _aux: &mut beamer_core::AuxiliaryBuffers,
            _context: &CoreProcessContext,
        ) {
        }
    }

    #[test]
    fn test_program_change_in_process_applies_and_reports_the_preset() {
        let wrapper = Vst3Processor::<GainPresetModel>::new(&STATE_CONFIG);
        prepare_state_wrapper(&wrapper);
        let output = vst3::ComWrapper::new(OutputChanges::default());
        let output_pointer = output
            .as_com_ref::<IParameterChanges>()
            .expect("the changes are IParameterChanges")
            .as_ptr();

        let changes = changes_of(&[(PROGRAM_CHANGE_PARAM_ID, 1.0)]);
        process_with(&wrapper, 64, changes.as_ptr(), output_pointer);

        // SAFETY: The wrapper is alive for the calls.
        unsafe {
            assert_eq!(wrapper.getParamNormalized(GAIN_ID), 0.9);
            assert_eq!(wrapper.getParamNormalized(PROGRAM_CHANGE_PARAM_ID), 1.0);
        }
        assert_eq!(output.reported(), [(GAIN_ID, vec![(0, 0.9)])]);
    }

    #[test]
    fn test_a_preset_reports_only_the_parameters_it_changed() {
        let wrapper = Vst3Processor::<GainPresetModel>::new(&STATE_CONFIG);
        prepare_state_wrapper(&wrapper);
        let changes = changes_of(&[(PROGRAM_CHANGE_PARAM_ID, 1.0)]);
        process_with(&wrapper, 64, changes.as_ptr(), std::ptr::null_mut());

        // The gain already holds the value of the preset, so the second
        // time there is nothing to report.
        let output = vst3::ComWrapper::new(OutputChanges::default());
        let output_pointer = output
            .as_com_ref::<IParameterChanges>()
            .expect("the changes are IParameterChanges")
            .as_ptr();
        process_with(&wrapper, 64, changes.as_ptr(), output_pointer);
        assert!(output.reported().is_empty());
    }

    #[test]
    fn test_a_parameter_change_after_a_preset_in_the_same_block() {
        let wrapper = Vst3Processor::<GainPresetModel>::new(&STATE_CONFIG);
        prepare_state_wrapper(&wrapper);
        let output = vst3::ComWrapper::new(OutputChanges::default());
        let output_pointer = output
            .as_com_ref::<IParameterChanges>()
            .expect("the changes are IParameterChanges")
            .as_ptr();

        // The host lists the gain before the program change.
        let changes = changes_of(&[(GAIN_ID, 0.4), (PROGRAM_CHANGE_PARAM_ID, 0.0)]);
        process_with(&wrapper, 64, changes.as_ptr(), output_pointer);

        // SAFETY: The wrapper is alive for the call.
        assert_eq!(unsafe { wrapper.getParamNormalized(GAIN_ID) }, 0.4);
        // The host learns the value the gain has after the block.
        assert_eq!(output.reported(), [(GAIN_ID, vec![(0, 0.4)])]);
    }

    thread_local! {
        /// Each process() call of a `SplitProcessor` on this thread: its
        /// length, the gain during it and the offsets of its MIDI.
        static SPLIT_CALLS: std::cell::RefCell<Vec<(usize, f64, Vec<u32>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
        /// The step of the stepped parameter during each of those calls.
        static SPLIT_STEPS: std::cell::RefCell<Vec<i64>> = const { std::cell::RefCell::new(Vec::new()) };
        /// Whether the next `SplitProcessor` processes 64-bit samples itself.
        static SPLIT_DOUBLE: Cell<bool> = const { Cell::new(false) };
    }

    const STEPS_ID: u32 = 2;

    /// A gain and a parameter of five steps.
    struct SplitParameters {
        gain: FloatParameter,
        steps: beamer_core::IntParameter,
    }

    impl Default for SplitParameters {
        fn default() -> Self {
            Self {
                gain: FloatParameter::new("Gain", 0.5, 0.0..=1.0).with_id(GAIN_ID),
                steps: beamer_core::IntParameter::new("Steps", 0, 0..=4).with_id(STEPS_ID),
            }
        }
    }

    impl ParameterGroups for SplitParameters {}

    impl Parameters for SplitParameters {
        fn count(&self) -> usize {
            2
        }
        fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
            Box::new([&self.gain as &dyn ParameterRef, &self.steps as &dyn ParameterRef].into_iter())
        }
        fn by_id(&self, id: u32) -> Option<&dyn ParameterRef> {
            match id {
                GAIN_ID => Some(&self.gain),
                STEPS_ID => Some(&self.steps),
                _ => None,
            }
        }
    }

    impl beamer_core::Saved for SplitParameters {}

    impl beamer_core::WebViewEvents for SplitParameters {}

    /// A plugin with a gain, a stepped parameter, the mod wheel as MIDI and
    /// an aux bus each way, which records its process() calls and writes
    /// each input sample plus the gain to its output.
    #[derive(Default)]
    struct SplitModel {
        parameters: SplitParameters,
    }

    struct SplitProcessor {
        model: Arc<SplitModel>,
        midi_offsets: Vec<u32>,
        double: bool,
    }

    beamer_core::__forward_model!(SplitModel, parameters);

    impl Model for SplitModel {
        type Setup = ();
        type Processor = SplitProcessor;

        fn prepare(self: Arc<Self>, _setup: ()) -> SplitProcessor {
            SplitProcessor {
                model: self,
                midi_offsets: Vec::new(),
                double: SPLIT_DOUBLE.with(Cell::get),
            }
        }

        fn midi_cc_config(&self) -> Option<beamer_core::MidiCcConfig> {
            Some(beamer_core::MidiCcConfig::new().with_mod_wheel())
        }

        fn input_bus_count(&self) -> usize {
            2
        }

        fn output_bus_count(&self) -> usize {
            2
        }

        fn input_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
            match index {
                0 => Some(CoreBusInfo::stereo("Input")),
                1 => Some(CoreBusInfo::aux("Aux Input", 2)),
                _ => None,
            }
        }

        fn output_bus_info(&self, index: usize) -> Option<CoreBusInfo> {
            match index {
                0 => Some(CoreBusInfo::stereo("Output")),
                1 => Some(CoreBusInfo::aux("Aux Output", 2)),
                _ => None,
            }
        }
    }

    /// Write each input sample plus `gain` to the output of the same bus
    /// and channel, for the main bus and the first aux bus.
    fn add_gain<S: beamer_core::Sample>(
        buffer: &mut beamer_core::Buffer<S>,
        aux: &mut beamer_core::AuxiliaryBuffers<S>,
        gain: f64,
    ) {
        for channel in buffer.channels_mut() {
            for sample in channel {
                *sample = S::from_f64(sample.to_f64() + gain);
            }
        }

        let aux_inputs: Vec<Vec<f64>> = aux
            .input(0)
            .map(|bus| bus.iter_inputs().map(|channel| channel.iter().map(|sample| sample.to_f64()).collect()).collect())
            .unwrap_or_default();
        if let Some(mut bus) = aux.output(0) {
            for (input, output) in aux_inputs.iter().zip(bus.iter_outputs()) {
                for (input, output) in input.iter().zip(output.iter_mut()) {
                    *output = S::from_f64(input + gain);
                }
            }
        }
    }

    impl SplitProcessor {
        /// Record a process() call of `len` samples, and return the gain
        /// during it.
        fn record(&mut self, len: usize) -> f64 {
            let gain = self.model.parameters.gain.get_normalized();
            let offsets = std::mem::take(&mut self.midi_offsets);
            SPLIT_CALLS.with(|calls| calls.borrow_mut().push((len, gain, offsets)));
            SPLIT_STEPS.with(|steps| steps.borrow_mut().push(self.model.parameters.steps.get()));
            gain
        }
    }

    impl Processor for SplitProcessor {
        fn process(
            &mut self,
            buffer: &mut beamer_core::Buffer,
            aux: &mut beamer_core::AuxiliaryBuffers,
            _context: &CoreProcessContext,
        ) {
            let gain = self.record(buffer.num_samples());
            add_gain(buffer, aux, gain);
        }

        fn supports_double_precision(&self) -> bool {
            self.double
        }

        fn process_f64(
            &mut self,
            buffer: &mut beamer_core::Buffer<f64>,
            aux: &mut beamer_core::AuxiliaryBuffers<f64>,
            _context: &CoreProcessContext,
        ) {
            let gain = self.record(buffer.num_samples());
            add_gain(buffer, aux, gain);
        }

        fn process_midi(&mut self, input: &[beamer_core::MidiEvent], _output: &mut beamer_core::MidiBuffer) {
            self.midi_offsets.extend(input.iter().map(|event| event.sample_offset));
        }
    }

    static SPLIT_CONFIG: Config = Config::new("Split Test", Category::Effect, "Test", "splt")
        .with_midi_input(true)
        .with_sample_accurate_automation();
    static WHOLE_CONFIG: Config =
        Config::new("Whole Test", Category::Effect, "Test", "whol").with_midi_input(true);

    /// How a test block hands its samples to the plugin.
    #[derive(Clone, Copy)]
    enum SampleFormat {
        Float32,
        /// 64-bit samples the plugin processes itself.
        Float64,
        /// 64-bit samples the wrapper converts for a 32-bit plugin.
        Float64Converted,
    }

    /// The process() calls of a block, the step during each and the
    /// channels of the main and the aux output.
    struct SplitRun {
        calls: Vec<(usize, f64, Vec<u32>)>,
        steps: Vec<i64>,
        outputs: [[Vec<f64>; 2]; 2],
    }

    /// The input of a test block at `index` of a channel of a bus (0 for
    /// the main bus, 1 for the aux bus): every sample of every channel
    /// differs.
    fn split_input(bus: usize, channel: usize, index: usize) -> f64 {
        (200 * bus + 100 * channel + index) as f64
    }

    /// Run one block of 64 frames with the parameter queues and a mod wheel
    /// change at sample 40.
    fn split_run(
        config: &'static Config,
        format: SampleFormat,
        queues: &[(u32, Vec<(int32, f64)>)],
    ) -> SplitRun {
        const FRAMES: usize = 64;
        let double = !matches!(format, SampleFormat::Float32);
        let sample_size = if double { SymbolicSampleSizes_::kSample64 } else { SymbolicSampleSizes_::kSample32 };

        SPLIT_DOUBLE.with(|split_double| split_double.set(matches!(format, SampleFormat::Float64)));
        let wrapper = Vst3Processor::<SplitModel>::new(config);
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: sample_size as int32,
            maxSamplesPerBlock: FRAMES as int32,
            sampleRate: 44100.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        SPLIT_CALLS.with(|calls| calls.borrow_mut().clear());
        SPLIT_STEPS.with(|steps| steps.borrow_mut().clear());

        let mut queues = queues.to_vec();
        queues.push((MidiCcState::parameter_id(1), vec![(40, 0.5)]));
        let changes = changes_with_points(&queues);

        // The buffers of the main and the aux bus, each with two channels,
        // with the input of `split_input`.
        let mut input32 = [[[0.0f32; FRAMES]; 2]; 2];
        let mut output32 = [[[0.0f32; FRAMES]; 2]; 2];
        let mut input64 = [[[0.0f64; FRAMES]; 2]; 2];
        let mut output64 = [[[0.0f64; FRAMES]; 2]; 2];
        for bus in 0..2 {
            for channel in 0..2 {
                for index in 0..FRAMES {
                    let value = split_input(bus, channel, index);
                    input32[bus][channel][index] = value as f32;
                    input64[bus][channel][index] = value;
                }
            }
        }
        let mut input32_channels = input32.each_mut().map(|bus| bus.each_mut().map(|channel| channel.as_mut_ptr()));
        let mut output32_channels = output32.each_mut().map(|bus| bus.each_mut().map(|channel| channel.as_mut_ptr()));
        let mut input64_channels = input64.each_mut().map(|bus| bus.each_mut().map(|channel| channel.as_mut_ptr()));
        let mut output64_channels = output64.each_mut().map(|bus| bus.each_mut().map(|channel| channel.as_mut_ptr()));
        let bus_buffers = |channels32: &mut [*mut f32; 2], channels64: &mut [*mut f64; 2]| AudioBusBuffers {
            numChannels: 2,
            silenceFlags: 0,
            __field0: if double {
                AudioBusBuffers__type0 { channelBuffers64: channels64.as_mut_ptr() }
            } else {
                AudioBusBuffers__type0 { channelBuffers32: channels32.as_mut_ptr() }
            },
        };
        let [main32, aux32] = &mut input32_channels;
        let [main64, aux64] = &mut input64_channels;
        let mut input_buses = [bus_buffers(main32, main64), bus_buffers(aux32, aux64)];
        let [main32, aux32] = &mut output32_channels;
        let [main64, aux64] = &mut output64_channels;
        let mut output_buses = [bus_buffers(main32, main64), bus_buffers(aux32, aux64)];
        let mut data = ProcessData {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: sample_size as int32,
            numSamples: FRAMES as int32,
            numInputs: 2,
            numOutputs: 2,
            inputs: input_buses.as_mut_ptr(),
            outputs: output_buses.as_mut_ptr(),
            inputParameterChanges: changes.as_ptr(),
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: std::ptr::null_mut(),
        };
        // SAFETY: The data and its buffers are valid for the call.
        assert_eq!(unsafe { wrapper.process(&mut data) }, kResultOk);

        let outputs = [0, 1].map(|bus| {
            [0, 1].map(|channel| {
                if double {
                    output64[bus][channel].to_vec()
                } else {
                    output32[bus][channel].map(f64::from).to_vec()
                }
            })
        });
        SplitRun {
            calls: SPLIT_CALLS.with(|calls| calls.borrow().clone()),
            steps: SPLIT_STEPS.with(|steps| steps.borrow().clone()),
            outputs,
        }
    }

    /// The process() calls of a 32-bit block with the gain points.
    fn split_calls(config: &'static Config, gain_points: Vec<(int32, f64)>) -> Vec<(usize, f64, Vec<u32>)> {
        split_run(config, SampleFormat::Float32, &[(GAIN_ID, gain_points)]).calls
    }

    #[test]
    fn test_a_jump_splits_the_block_once() {
        // A host sends a jump as the old value and the new value one sample
        // later. The old value changes nothing and makes no sub-block. The
        // first half runs with the old gain, the second with the new one
        // and gets the mod wheel at 40 relative to its start.
        let calls = split_calls(&SPLIT_CONFIG, vec![(31, 0.5), (32, 0.8)]);
        assert_eq!(calls, [(32, 0.5, vec![]), (32, 0.8, vec![8])]);
    }

    #[test]
    fn test_a_ramp_applies_its_end_value_at_its_start() {
        // A ramp across the block, and one from the start of the block to a
        // point inside it, run as one call that moves towards the end
        // value.
        for points in [vec![(0, 0.5), (63, 0.8)], vec![(32, 0.8)]] {
            let calls = split_calls(&SPLIT_CONFIG, points);
            assert_eq!(calls, [(64, 0.8, vec![40])]);
        }
    }

    #[test]
    fn test_a_curve_splits_at_its_inner_points() {
        // Each segment applies its end value at its start.
        let calls = split_calls(&SPLIT_CONFIG, vec![(0, 0.5), (20, 0.6), (40, 0.7), (63, 0.8)]);
        assert_eq!(calls, [(20, 0.6, vec![]), (20, 0.7, vec![]), (24, 0.8, vec![0])]);

        // A ramp after a jump starts with the jump.
        let calls = split_calls(&SPLIT_CONFIG, vec![(31, 0.5), (32, 0.8), (63, 0.6)]);
        assert_eq!(calls, [(32, 0.5, vec![]), (32, 0.6, vec![8])]);
    }

    #[test]
    fn test_a_point_that_keeps_the_step_makes_no_sub_block() {
        // 0.05 rounds to the step the parameter is at.
        let run = split_run(&SPLIT_CONFIG, SampleFormat::Float32, &[(STEPS_ID, vec![(31, 0.05)])]);
        assert_eq!(run.calls.len(), 1);

        // The old value of a jump, sent between two steps, splits nothing.
        let run = split_run(&SPLIT_CONFIG, SampleFormat::Float32, &[(STEPS_ID, vec![(31, 0.1), (32, 0.25)])]);
        assert_eq!(lengths(&run), [32, 32]);
        assert_eq!(run.steps, [0, 1]);
    }

    /// The length of each process() call of a run.
    fn lengths(run: &SplitRun) -> Vec<usize> {
        run.calls.iter().map(|call| call.0).collect()
    }

    #[test]
    fn test_a_discrete_ramp_changes_where_it_reaches_a_step() {
        // A ramp from step 0 at the start to step 1 at sample 32 reaches
        // the middle between them at sample 16.
        let run = split_run(&SPLIT_CONFIG, SampleFormat::Float32, &[(STEPS_ID, vec![(32, 0.25)])]);
        assert_eq!(lengths(&run), [16, 48]);
        assert_eq!(run.steps, [0, 1]);

        // A ramp over all five steps takes each where the line reaches it.
        let run = split_run(&SPLIT_CONFIG, SampleFormat::Float32, &[(STEPS_ID, vec![(0, 0.0), (63, 1.0)])]);
        assert_eq!(lengths(&run), [8, 16, 16, 16, 8]);
        assert_eq!(run.steps, [0, 1, 2, 3, 4]);

        // And down again.
        let run = split_run(&SPLIT_CONFIG, SampleFormat::Float32, &[(STEPS_ID, vec![(0, 1.0), (63, 0.0)])]);
        assert_eq!(run.steps.first(), Some(&4));
        assert_eq!(run.steps.last(), Some(&0));
        assert_eq!(run.steps.len(), 5);
    }

    #[test]
    fn test_without_sample_accurate_automation_the_block_is_one_call() {
        // The gain changes before the block, and the block gets the mod
        // wheel at its offset in the block.
        let calls = split_calls(&WHOLE_CONFIG, vec![(31, 0.5), (32, 0.8)]);
        assert_eq!(calls, [(64, 0.8, vec![40])]);
    }

    #[test]
    fn test_a_queue_that_does_not_fit_takes_its_last_value_at_the_start() {
        let points = (0..MAX_PARAMETER_POINTS as int32 + 1)
            .map(|index| (index % 64, f64::from(index % 2) * 0.25 + 0.25))
            .collect::<Vec<_>>();
        let last = points.last().unwrap().1;
        // The gain makes no sub-block, and MIDI does not split a block.
        let calls = split_calls(&SPLIT_CONFIG, points);
        assert_eq!(calls, [(64, last, vec![40])]);
    }

    #[test]
    fn test_each_sub_block_reads_and_writes_its_own_samples() {
        // In each sample format and on each channel of the main and the aux
        // bus, every output sample is its own input sample plus the gain:
        // the old gain before the jump and the new one from it on.
        for format in [SampleFormat::Float32, SampleFormat::Float64, SampleFormat::Float64Converted] {
            let run = split_run(&SPLIT_CONFIG, format, &[(GAIN_ID, vec![(31, 0.5), (32, 0.8)])]);
            assert_eq!(run.calls.len(), 2);
            for (bus, channels) in run.outputs.iter().enumerate() {
                for (channel, output) in channels.iter().enumerate() {
                    for (index, sample) in output.iter().enumerate() {
                        let gain = if index < 32 { 0.5 } else { 0.8 };
                        let expected = split_input(bus, channel, index) + gain;
                        assert!(
                            (sample - expected).abs() < 1e-4,
                            "bus {} channel {} sample {} is {}",
                            bus,
                            channel,
                            index,
                            sample
                        );
                    }
                }
            }
        }
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
    fn saved_state<P>(wrapper: &Vst3Processor<P>) -> Vec<u8>
    where
        P: Model,
    {
        let stream = vst3::ComWrapper::new(MemoryStream::default());
        let pointer = stream.to_com_ptr::<IBStream>().expect("the stream is an IBStream");
        // SAFETY: The stream is alive for the call.
        assert_eq!(unsafe { IComponentTrait::getState(wrapper, pointer.as_ptr()) }, kResultOk);
        let bytes = stream.content.lock().unwrap().0.clone();
        bytes
    }

    /// Load a state into a wrapper like a host.
    fn load_state<P>(wrapper: &Vst3Processor<P>, bytes: &[u8]) -> tresult
    where
        P: Model,
    {
        let stream = vst3::ComWrapper::new(MemoryStream {
            content: Mutex::new((bytes.to_vec(), 0)),
        });
        let pointer = stream.to_com_ptr::<IBStream>().expect("the stream is an IBStream");
        // SAFETY: The stream is alive for the call.
        unsafe { IComponentTrait::setState(wrapper, pointer.as_ptr()) }
    }

    fn gain(wrapper: &Vst3Processor<StateModel>) -> f64 {
        // SAFETY: The wrapper is alive.
        unsafe { wrapper.getParamNormalized(GAIN_ID) }
    }

    fn set_gain(wrapper: &Vst3Processor<StateModel>, value: f64) {
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.setParamNormalized(GAIN_ID, value) }, kResultOk);
    }

    #[test]
    fn test_state_is_the_same_prepared_or_not() {
        let wrapper = Vst3Processor::<StateModel>::new(&STATE_CONFIG);

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
        let wrapper = Vst3Processor::<StateModel>::new(&STATE_CONFIG);
        let state = saved_state(&wrapper);
        assert_eq!(load_state(&wrapper, &state), kResultOk);
        let before = smoothing_resets();

        prepare_state_wrapper(&wrapper);
        process_block(&wrapper);
        assert_eq!(smoothing_resets(), before + 1);
    }

    #[test]
    fn test_state_keeps_the_program_the_host_shows() {
        let wrapper = Vst3Processor::<GainPresetModel>::new(&STATE_CONFIG);
        // SAFETY: The wrapper is alive for the calls.
        unsafe {
            // The second preset, then a change of the gain after it.
            assert_eq!(wrapper.setParamNormalized(PROGRAM_CHANGE_PARAM_ID, 1.0), kResultOk);
            assert_eq!(wrapper.setParamNormalized(GAIN_ID, 0.3), kResultOk);
        }
        let state = saved_state(&wrapper);

        // A new instance, as when a host opens the project.
        let reopened = Vst3Processor::<GainPresetModel>::new(&STATE_CONFIG);
        assert_eq!(load_state(&reopened, &state), kResultOk);
        // SAFETY: The wrapper is alive for the calls.
        unsafe {
            assert_eq!(reopened.getParamNormalized(PROGRAM_CHANGE_PARAM_ID), 1.0);
            // The preset is not applied again over the saved gain.
            assert_eq!(reopened.getParamNormalized(GAIN_ID), 0.3);
        }
    }

    #[test]
    fn test_state_without_a_program_shows_the_first() {
        // A plugin without factory presets saves no program.
        let without_presets = Vst3Processor::<StateModel>::new(&STATE_CONFIG);
        let state = saved_state(&without_presets);
        let entries = StateEntries::parse(&state).unwrap().unwrap();
        assert_eq!(entries.factory_preset(), None);

        // A program this plugin does not have counts as none.
        let mut writer = StateWriter::new();
        writer.factory_preset(5);
        let unknown_program = writer.finish();

        for loaded in [&state, &unknown_program] {
            let wrapper = Vst3Processor::<GainPresetModel>::new(&STATE_CONFIG);
            // SAFETY: The wrapper is alive for the call.
            unsafe {
                assert_eq!(wrapper.setParamNormalized(PROGRAM_CHANGE_PARAM_ID, 1.0), kResultOk);
            }
            assert_eq!(load_state(&wrapper, loaded), kResultOk);
            // SAFETY: The wrapper is alive for the call.
            assert_eq!(unsafe { wrapper.getParamNormalized(PROGRAM_CHANGE_PARAM_ID) }, 0.0);
        }
    }

    #[test]
    fn test_setup_processing_gives_the_smoothers_the_host_sample_rate() {
        let wrapper = Vst3Processor::<StateModel>::new(&STATE_CONFIG);

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
        let wrapper = Vst3Processor::<StateModel>::new(&STATE_CONFIG);
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
        let wrapper = Vst3Processor::<StateModel>::new(&STATE_CONFIG);
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
    fn test_process_call_without_samples_resets_the_smoothers_after_a_state_load() {
        let wrapper = prepared_state_wrapper();
        let state = saved_state(&wrapper);
        assert_eq!(load_state(&wrapper, &state), kResultOk);
        let before = smoothing_resets();

        process_with(&wrapper, 0, std::ptr::null_mut(), std::ptr::null_mut());
        assert_eq!(smoothing_resets(), before + 1);
        process_block(&wrapper);
        assert_eq!(smoothing_resets(), before + 1);
    }

    #[test]
    fn test_editor_shares_the_model_in_both_states() {
        let wrapper = Vst3Processor::<StateModel>::new(&STATE_CONFIG);

        // An editor that is opened before audio is set up.
        let editor = wrapper.shared_model();
        editor.parameter_store().set_normalized(GAIN_ID, 0.25);
        assert_eq!(gain(&wrapper), 0.25);

        // Setting up and changing the setup creates processors. The editor
        // keeps reaching the model they use.
        prepare_state_wrapper(&wrapper);
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 96000.0,
        };
        // SAFETY: The setup is valid for the call.
        assert_eq!(unsafe { wrapper.setupProcessing(&mut setup) }, kResultOk);
        editor.parameter_store().set_normalized(GAIN_ID, 0.5);
        process_block(&wrapper);
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.getLatencySamples() }, latency_for(0.5));
        assert_eq!(editor.parameter_store().get_normalized(GAIN_ID), 0.5);

        // An editor that is opened now gets the same model.
        let later = wrapper.shared_model();
        assert!(std::ptr::addr_eq(Arc::as_ptr(&editor), Arc::as_ptr(&later)));

        // An editor that outlives the plugin still has its model.
        drop(later);
        drop(wrapper);
        assert_eq!(editor.parameter_store().get_normalized(GAIN_ID), 0.5);
        editor.parameter_store().set_normalized(GAIN_ID, 0.75);
        assert_eq!(editor.parameter_store().get_normalized(GAIN_ID), 0.75);
    }

    #[test]
    fn test_latency_and_tail_are_published_for_the_host() {
        let wrapper = Vst3Processor::<StateModel>::new(&STATE_CONFIG);
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

    // =========================================================================
    // A host thread next to process()
    // =========================================================================
    //
    // The VST3 SDK lets a host call getState, setState and the controller
    // calls on its UI thread in every state, processing included (the
    // threading notes of IComponent and IEditController in the SDK
    // headers). Hosts also ask for latency and tail at any time, although
    // the notes of IAudioProcessor list them for the UI thread before
    // activation. Only process() runs on the audio thread. The test
    // below makes those calls on one thread while another runs process().
    // Under ThreadSanitizer it shows that the two threads share nothing but
    // atomics and locks (see docs/BUILDING.md).

    use beamer_core::state_format::SavedState;
    use beamer_core::{HandoverReader, SmoothingStyle, State};
    use std::sync::Barrier;

    const LEVEL_ID: u32 = 2;
    /// The layouts the editor of the test picks from. The processor reports
    /// the one it read last as its latency.
    const LAYOUTS: [u32; 3] = [0, 16, 32];
    const THREADED_TAIL_SAMPLES: u32 = 256;
    /// How many rounds of calls the host thread makes.
    const HOST_ROUNDS: usize = 2000;

    /// Parameters as `#[beamer::model]` generates them: a smoothed parameter
    /// and a `State` field that the audio thread reads through a reader.
    struct ThreadedParameters {
        level: FloatParameter,
        layout: State<u32>,
    }

    impl Default for ThreadedParameters {
        fn default() -> Self {
            Self {
                level: FloatParameter::new("Level", 0.5, 0.0..=1.0)
                    .with_id(LEVEL_ID)
                    .with_smoother(SmoothingStyle::Linear(5.0)),
                layout: State::new(LAYOUTS[0]),
            }
        }
    }

    impl ParameterGroups for ThreadedParameters {}

    impl Parameters for ThreadedParameters {
        fn count(&self) -> usize {
            1
        }
        fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
            Box::new(std::iter::once(&self.level as &dyn ParameterRef))
        }
        fn by_id(&self, id: u32) -> Option<&dyn ParameterRef> {
            (id == LEVEL_ID).then_some(&self.level as &dyn ParameterRef)
        }
        fn set_sample_rate(&self, sample_rate: f64) {
            self.level.set_sample_rate(sample_rate);
        }
        fn reset_smoothing(&self) {
            self.level.reset_smoothing();
        }
    }

    impl beamer_core::Saved for ThreadedParameters {
        fn state_generation(&self) -> u64 {
            self.layout.generation()
        }

        fn for_each_state(&self, visit: &mut dyn FnMut(&str, &dyn SavedState)) {
            visit("layout", &self.layout);
        }

        fn save_entries(&self, writer: &mut StateWriter, _prefix: &str) {
            writer.parameter(&LEVEL_ID.to_string(), &self.level);
            writer.state("layout", &self.layout);
        }

        fn load_entries(&self, entries: &StateEntries<'_>, _prefix: &str) {
            entries.load_parameter(&LEVEL_ID.to_string(), &self.level);
            entries.load_state("layout", &self.layout);
        }
    }

    impl beamer_core::WebViewEvents for ThreadedParameters {}

    #[derive(Default)]
    struct ThreadedModel {
        parameters: ThreadedParameters,
    }

    struct ThreadedProcessor {
        model: Arc<ThreadedModel>,
        layout: HandoverReader<u32>,
    }

    beamer_core::__forward_model!(ThreadedModel, parameters);

    impl Model for ThreadedModel {
        type Setup = ();
        type Processor = ThreadedProcessor;
        // Two factory presets for the level.
        const PRESETS: &'static [Preset<Self>] = &[
            Preset::new("Quiet", |m| m.parameters.level.set(0.1)),
            Preset::new("Loud", |m| m.parameters.level.set(0.8)),
        ];

        fn prepare(self: Arc<Self>, _setup: ()) -> ThreadedProcessor {
            ThreadedProcessor {
                layout: self.parameters.layout.reader(),
                model: self,
            }
        }
    }

    impl Processor for ThreadedProcessor {
        fn process(
            &mut self,
            buffer: &mut beamer_core::Buffer,
            _aux: &mut beamer_core::AuxiliaryBuffers,
            _context: &CoreProcessContext,
        ) {
            // Take the layout the editor set last, which `latency_samples`
            // reports.
            self.layout.latest();
            let level = &self.model.parameters.level;
            for index in 0..buffer.num_samples() {
                let gain = level.tick_smoothed_f32();
                for channel in buffer.channels_mut() {
                    channel[index] *= gain;
                }
            }
        }

        fn latency_samples(&self) -> u32 {
            *self.layout.current()
        }

        fn tail_samples(&self) -> u32 {
            THREADED_TAIL_SAMPLES
        }
    }

    /// Sets a flag when dropped.
    struct StopOnDrop<'a>(&'a AtomicBool);

    impl Drop for StopOnDrop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    static THREADED_CONFIG: Config = Config::new("Threaded Test", Category::Effect, "Test", "thrd");

    #[test]
    fn test_host_calls_while_process_runs_on_another_thread() {
        let wrapper = Vst3Processor::<ThreadedModel>::new(&THREADED_CONFIG);
        let model = Arc::clone(&wrapper.model);
        let mut setup = ProcessSetup {
            processMode: ProcessModes_::kRealtime as int32,
            symbolicSampleSize: SymbolicSampleSizes_::kSample32 as int32,
            maxSamplesPerBlock: 64,
            sampleRate: 44100.0,
        };
        // SAFETY: The wrapper is alive and the setup is valid for the calls.
        unsafe {
            assert_eq!(wrapper.setupProcessing(&mut setup), kResultOk);
            assert_eq!(wrapper.setActive(1), kResultOk);
            assert_eq!(wrapper.setProcessing(1), kResultOk);
        }
        // The tail with the bypass ramp, which stays the same.
        // SAFETY: The wrapper is alive.
        let tail = unsafe { wrapper.getTailSamples() };
        assert!(tail >= THREADED_TAIL_SAMPLES);

        // An older state that the host goes back to now and then.
        model.parameters.level.set(0.2);
        model.parameters.layout.set(LAYOUTS[1]);
        let older = saved_state(&wrapper);

        let start = Barrier::new(2);
        let host_done = AtomicBool::new(false);
        let blocks = std::thread::scope(|scope| {
            let audio = scope.spawn(|| {
                start.wait();
                let mut blocks = 0usize;
                loop {
                    // Automation of the level, and a program change now
                    // and then.
                    let mut values = vec![(LEVEL_ID, (blocks % 7) as f64 / 7.0)];
                    if blocks.is_multiple_of(5) {
                        values.push((PROGRAM_CHANGE_PARAM_ID, 1.0));
                    }
                    let changes = changes_of(&values);
                    process_with(&wrapper, 64, changes.as_ptr(), std::ptr::null_mut());
                    blocks += 1;
                    if host_done.load(Ordering::Acquire) {
                        break blocks;
                    }
                }
            });

            start.wait();
            // The audio thread stops when the host is done, also when an
            // assertion below fails.
            let _stop = StopOnDrop(&host_done);
            for round in 0..HOST_ROUNDS {
                // The editor sets the layout and the level.
                model.parameters.layout.set(LAYOUTS[round % LAYOUTS.len()]);
                // SAFETY: The wrapper is alive for the calls.
                unsafe {
                    assert_eq!(
                        wrapper.setParamNormalized(LEVEL_ID, (round % 10) as f64 / 10.0),
                        kResultOk
                    );
                }

                // The host saves, and loads that state or the older one.
                let saved = saved_state(&wrapper);
                let loaded = if round % 2 == 0 { &saved } else { &older };
                assert_eq!(load_state(&wrapper, loaded), kResultOk);

                // SAFETY: The wrapper is alive for the calls.
                unsafe {
                    // A preset from the host's list.
                    let preset = (round % ThreadedModel::PRESETS.len()) as f64;
                    assert_eq!(wrapper.setParamNormalized(PROGRAM_CHANGE_PARAM_ID, preset), kResultOk);

                    // What the plugin reports to the host.
                    assert!((0.0..=1.0).contains(&wrapper.getParamNormalized(LEVEL_ID)));
                    let program = wrapper.getParamNormalized(PROGRAM_CHANGE_PARAM_ID);
                    assert!((0.0..=1.0).contains(&program));
                    assert!(LAYOUTS.contains(&wrapper.getLatencySamples()));
                    assert_eq!(wrapper.getTailSamples(), tail);
                }
            }
            drop(_stop);
            audio.join().expect("the audio thread does not panic")
        });
        assert!(blocks > 0);

        // On one thread again: a loaded state is complete after the next
        // block, the smoothers at the loaded value without a ramp. The
        // layout moves away from the one of the state first, which the last
        // round may have left.
        model.parameters.layout.set(LAYOUTS[2]);
        process_with(&wrapper, 64, std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: The wrapper is alive.
        assert_eq!(unsafe { wrapper.getLatencySamples() }, LAYOUTS[2]);
        assert_eq!(load_state(&wrapper, &older), kResultOk);
        process_with(&wrapper, 64, std::ptr::null_mut(), std::ptr::null_mut());
        // SAFETY: The wrapper is alive for the calls.
        unsafe {
            assert_eq!(wrapper.getParamNormalized(LEVEL_ID), 0.2);
            assert_eq!(wrapper.getLatencySamples(), LAYOUTS[1]);
        }
        assert_eq!(*model.parameters.layout.get(), LAYOUTS[1]);
        assert!((model.parameters.level.smoothed() - 0.2).abs() < 1e-9);
    }
}
