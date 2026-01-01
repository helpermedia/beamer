//! Generic VST3 processor wrapper.
//!
//! This module provides [`Vst3Processor`], a generic wrapper that bridges any
//! [`beamr_core::Plugin`] implementation to VST3 COM interfaces.
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

use beamr_core::{
    AudioBuffer, BusType as CoreBusType, ChordInfo, MidiBuffer, MidiEvent, MidiEventKind,
    NoteExpressionInt, NoteExpressionText, NoteExpressionValue as CoreNoteExpressionValue,
    Parameters, Plugin, ScaleInfo, SysEx, MAX_CHORD_NAME_SIZE, MAX_EXPRESSION_TEXT_SIZE,
    MAX_SCALE_NAME_SIZE, MAX_SYSEX_SIZE,
};

use crate::factory::ComponentFactory;
use crate::util::{copy_wstring, len_wstring};
use crate::wrapper::PluginConfig;

// Maximum number of channels we support per bus (for stack allocation)
const MAX_CHANNELS_PER_BUS: usize = 8;
// Maximum number of buses we support (for stack allocation)
const MAX_BUSES: usize = 4;

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
            let mut buf = Vec::with_capacity(buffer_size);
            buf.resize(buffer_size, 0u8);
            buffers.push(buf);
        }
        let mut lengths = Vec::with_capacity(slots);
        lengths.resize(slots, 0usize);

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

/// Generic VST3 processor wrapping any [`Plugin`] implementation.
///
/// This struct implements the VST3 combined component pattern, providing
/// `IComponent`, `IAudioProcessor`, and `IEditController` interfaces that
/// delegate to the wrapped plugin.
///
/// # Usage
///
/// ```ignore
/// use beamr_vst3::{export_vst3, Vst3Processor, PluginConfig};
///
/// struct MyPlugin { /* ... */ }
/// impl Plugin for MyPlugin { /* ... */ }
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
    /// The wrapped plugin instance (uses UnsafeCell for interior mutability)
    plugin: UnsafeCell<P>,
    /// Plugin configuration reference
    config: &'static PluginConfig,
    /// Current sample rate
    sample_rate: UnsafeCell<f64>,
    /// Maximum block size
    max_block_size: UnsafeCell<usize>,
    /// MIDI output buffer (reused each process call)
    midi_output: UnsafeCell<MidiBuffer>,
    /// SysEx output buffer pool (for VST3 DataEvent pointer stability)
    sysex_output_pool: UnsafeCell<SysExOutputPool>,
    /// Marker for the plugin type
    _marker: PhantomData<P>,
}

// Safety: Vst3Processor is Send because:
// - Plugin: Send is required by the Plugin trait
// - UnsafeCell contents are only accessed from VST3's guaranteed single-threaded contexts
unsafe impl<P: Plugin> Send for Vst3Processor<P> {}

// Safety: Vst3Processor is Sync because:
// - VST3 guarantees process() is called from one thread at a time
// - Parameter access through Parameters trait requires Sync
unsafe impl<P: Plugin> Sync for Vst3Processor<P> {}

impl<P: Plugin + 'static> Vst3Processor<P> {
    /// Create a new VST3 processor wrapping the given plugin configuration.
    pub fn new(config: &'static PluginConfig) -> Self {
        Self {
            plugin: UnsafeCell::new(P::create()),
            config,
            sample_rate: UnsafeCell::new(44100.0),
            max_block_size: UnsafeCell::new(1024),
            midi_output: UnsafeCell::new(MidiBuffer::new()),
            sysex_output_pool: UnsafeCell::new(SysExOutputPool::with_capacity(
                config.sysex_slots,
                config.sysex_buffer_size,
            )),
            _marker: PhantomData,
        }
    }

    /// Get a reference to the wrapped plugin.
    ///
    /// # Safety
    /// Must only be called when no mutable reference exists.
    #[inline]
    unsafe fn plugin(&self) -> &P {
        &*self.plugin.get()
    }

    /// Get a mutable reference to the wrapped plugin.
    ///
    /// # Safety
    /// Must only be called from contexts where VST3 guarantees single-threaded access
    /// (e.g., process(), setupProcessing()).
    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn plugin_mut(&self) -> &mut P {
        &mut *self.plugin.get()
    }
}

impl<P: Plugin + 'static> ComponentFactory for Vst3Processor<P> {
    fn create(config: &'static PluginConfig) -> Self {
        Self::new(config)
    }
}

impl<P: Plugin + 'static> Class for Vst3Processor<P> {
    type Interfaces = (
        IComponent,
        IAudioProcessor,
        IProcessContextRequirements,
        IEditController,
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

impl<P: Plugin + 'static> IPluginBaseTrait for Vst3Processor<P> {
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

impl<P: Plugin + 'static> IComponentTrait for Vst3Processor<P> {
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
        let plugin = self.plugin();

        match media_type as MediaTypes {
            MediaTypes_::kAudio => match dir as BusDirections {
                BusDirections_::kInput => plugin.input_bus_count() as i32,
                BusDirections_::kOutput => plugin.output_bus_count() as i32,
                _ => 0,
            },
            MediaTypes_::kEvent => {
                // Return 1 event bus in each direction if plugin wants MIDI
                if plugin.wants_midi() {
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

        let plugin = self.plugin();

        match media_type as MediaTypes {
            MediaTypes_::kAudio => {
                let info = match dir as BusDirections {
                    BusDirections_::kInput => plugin.input_bus_info(index as usize),
                    BusDirections_::kOutput => plugin.output_bus_info(index as usize),
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
                if index != 0 || !plugin.wants_midi() {
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
        let plugin = self.plugin_mut();
        plugin.set_active(state != 0);
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

        // Load into plugin
        let plugin = self.plugin_mut();
        match plugin.load_state(&buffer) {
            Ok(()) => kResultOk,
            Err(_) => kResultFalse,
        }
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        if state.is_null() {
            return kInvalidArgument;
        }

        // Get plugin state as bytes
        let plugin = self.plugin();
        let data = match plugin.save_state() {
            Ok(d) => d,
            Err(_) => return kResultFalse,
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

impl<P: Plugin + 'static> IAudioProcessorTrait for Vst3Processor<P> {
    unsafe fn setBusArrangements(
        &self,
        inputs: *mut SpeakerArrangement,
        num_ins: i32,
        outputs: *mut SpeakerArrangement,
        num_outs: i32,
    ) -> tresult {
        let plugin = self.plugin();

        // Check if the requested arrangement matches our bus configuration
        if num_ins as usize != plugin.input_bus_count()
            || num_outs as usize != plugin.output_bus_count()
        {
            return kResultFalse;
        }

        // Validate each input bus
        for i in 0..num_ins as usize {
            if let Some(info) = plugin.input_bus_info(i) {
                let requested = *inputs.add(i);
                let expected = channel_count_to_speaker_arrangement(info.channel_count);
                if requested != expected {
                    return kResultFalse;
                }
            }
        }

        // Validate each output bus
        for i in 0..num_outs as usize {
            if let Some(info) = plugin.output_bus_info(i) {
                let requested = *outputs.add(i);
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

        let plugin = self.plugin();
        let info = match dir as BusDirections {
            BusDirections_::kInput => plugin.input_bus_info(index as usize),
            BusDirections_::kOutput => plugin.output_bus_info(index as usize),
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
            _ => kNotImplemented, // TODO: Support 64-bit in the future
        }
    }

    unsafe fn getLatencySamples(&self) -> u32 {
        self.plugin().latency_samples()
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        if setup.is_null() {
            return kInvalidArgument;
        }

        let setup = &*setup;

        // Store setup parameters
        *self.sample_rate.get() = setup.sampleRate;
        *self.max_block_size.get() = setup.maxSamplesPerBlock as usize;

        // Notify the plugin
        let plugin = self.plugin_mut();
        plugin.setup(setup.sampleRate, setup.maxSamplesPerBlock as usize);

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
            let params = self.plugin().params();
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

        // 2. Handle MIDI events
        let mut midi_input: Vec<MidiEvent> = Vec::with_capacity(128);

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

        // Process MIDI events
        let plugin = self.plugin_mut();
        plugin.process_midi(&midi_input, midi_output);

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

        // 3. Build AudioBuffer from VST3 ProcessData
        // We use Vec for channel slices - this allocates but is simpler and correct.
        // Future optimization: use a pre-allocated buffer pool.

        // Collect input channel slices
        let mut input_slices: Vec<&[f32]> = Vec::with_capacity(MAX_CHANNELS_PER_BUS * MAX_BUSES);

        if process_data.numInputs > 0 && !process_data.inputs.is_null() {
            let input_buses =
                slice::from_raw_parts(process_data.inputs, process_data.numInputs as usize);
            for bus in input_buses {
                let num_channels = (bus.numChannels as usize).min(MAX_CHANNELS_PER_BUS);
                if num_channels > 0 && !bus.__field0.channelBuffers32.is_null() {
                    let channel_ptrs =
                        slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels);
                    for &ptr in channel_ptrs {
                        if !ptr.is_null() {
                            input_slices.push(slice::from_raw_parts(ptr, num_samples));
                        }
                    }
                }
            }
        }

        // Collect output channel slices
        let mut output_slices: Vec<&mut [f32]> =
            Vec::with_capacity(MAX_CHANNELS_PER_BUS * MAX_BUSES);

        if process_data.numOutputs > 0 && !process_data.outputs.is_null() {
            let output_buses =
                slice::from_raw_parts(process_data.outputs, process_data.numOutputs as usize);
            for bus in output_buses {
                let num_channels = (bus.numChannels as usize).min(MAX_CHANNELS_PER_BUS);
                if num_channels > 0 && !bus.__field0.channelBuffers32.is_null() {
                    let channel_ptrs =
                        slice::from_raw_parts(bus.__field0.channelBuffers32, num_channels);
                    for &ptr in channel_ptrs {
                        if !ptr.is_null() {
                            output_slices.push(slice::from_raw_parts_mut(ptr, num_samples));
                        }
                    }
                }
            }
        }

        // 3. Create AudioBuffer and call plugin process
        let mut buffer = AudioBuffer::new(&input_slices, &mut output_slices, num_samples);

        let plugin = self.plugin_mut();
        plugin.process(&mut buffer);

        kResultOk
    }

    unsafe fn getTailSamples(&self) -> u32 {
        self.plugin().tail_samples()
    }
}

impl<P: Plugin + 'static> IProcessContextRequirementsTrait for Vst3Processor<P> {
    unsafe fn getProcessContextRequirements(&self) -> u32 {
        0
    }
}

// =============================================================================
// IEditController implementation
// =============================================================================

impl<P: Plugin + 'static> IEditControllerTrait for Vst3Processor<P> {
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
        self.plugin().params().count() as i32
    }

    unsafe fn getParameterInfo(&self, param_index: i32, info: *mut ParameterInfo) -> tresult {
        if info.is_null() {
            return kInvalidArgument;
        }

        let params = self.plugin().params();

        if let Some(param_info) = params.info(param_index as usize) {
            let info = &mut *info;
            info.id = param_info.id;
            copy_wstring(param_info.name, &mut info.title);
            copy_wstring(param_info.short_name, &mut info.shortTitle);
            copy_wstring(param_info.units, &mut info.units);
            info.stepCount = param_info.step_count;
            info.defaultNormalizedValue = param_info.default_normalized;
            info.unitId = 0;
            info.flags = if param_info.flags.can_automate {
                ParameterInfo_::ParameterFlags_::kCanAutomate
            } else {
                0
            };
            kResultOk
        } else {
            kInvalidArgument
        }
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

        let params = self.plugin().params();
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
            let params = self.plugin().params();
            if let Some(value) = params.string_to_normalized(id, &s) {
                *value_normalized = value;
                return kResultOk;
            }
        }
        kInvalidArgument
    }

    unsafe fn normalizedParamToPlain(&self, id: u32, value_normalized: f64) -> f64 {
        self.plugin().params().normalized_to_plain(id, value_normalized)
    }

    unsafe fn plainParamToNormalized(&self, id: u32, plain_value: f64) -> f64 {
        self.plugin().params().plain_to_normalized(id, plain_value)
    }

    unsafe fn getParamNormalized(&self, id: u32) -> f64 {
        self.plugin().params().get_normalized(id)
    }

    unsafe fn setParamNormalized(&self, id: u32, value: f64) -> tresult {
        self.plugin().params().set_normalized(id, value);
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
// IMidiMapping implementation (VST3 SDK 3.8.0)
// =============================================================================

impl<P: Plugin + 'static> IMidiMappingTrait for Vst3Processor<P> {
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

        let plugin = self.plugin();
        if let Some(param_id) =
            plugin.midi_cc_to_param(bus_index, channel, midi_controller_number as u8)
        {
            *id = param_id;
            kResultOk
        } else {
            kResultFalse
        }
    }
}

// =============================================================================
// IMidiLearn implementation (VST3 SDK 3.8.0)
// =============================================================================

impl<P: Plugin + 'static> IMidiLearnTrait for Vst3Processor<P> {
    unsafe fn onLiveMIDIControllerInput(
        &self,
        bus_index: i32,
        channel: i16,
        midi_cc: i16,
    ) -> tresult {
        let plugin = self.plugin_mut();
        if plugin.on_midi_learn(bus_index, channel, midi_cc as u8) {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

// =============================================================================
// IMidiMapping2 implementation (VST3 SDK 3.8.0 - MIDI 2.0)
// =============================================================================

impl<P: Plugin + 'static> IMidiMapping2Trait for Vst3Processor<P> {
    unsafe fn getNumMidi1ControllerAssignments(&self, direction: BusDirections) -> u32 {
        // Only support input direction
        if direction != BusDirections_::kInput {
            return 0;
        }
        let plugin = self.plugin();
        plugin.midi1_assignments().len() as u32
    }

    unsafe fn getMidi1ControllerAssignments(
        &self,
        direction: BusDirections,
        list: *const Midi1ControllerParamIDAssignmentList,
    ) -> tresult {
        if list.is_null() || direction != BusDirections_::kInput {
            return kInvalidArgument;
        }

        let plugin = self.plugin();
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
        let plugin = self.plugin();
        plugin.midi2_assignments().len() as u32
    }

    unsafe fn getMidi2ControllerAssignments(
        &self,
        direction: BusDirections,
        list: *const Midi2ControllerParamIDAssignmentList,
    ) -> tresult {
        if list.is_null() || direction != BusDirections_::kInput {
            return kInvalidArgument;
        }

        let plugin = self.plugin();
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

impl<P: Plugin + 'static> IMidiLearn2Trait for Vst3Processor<P> {
    unsafe fn onLiveMidi1ControllerInput(
        &self,
        bus_index: i32,
        channel: u8,
        midi_cc: i16,
    ) -> tresult {
        let plugin = self.plugin_mut();
        if plugin.on_midi1_learn(bus_index, channel, midi_cc as u8) {
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
        let plugin = self.plugin_mut();
        let controller = beamr_core::Midi2Controller {
            bank: midi_cc.bank,
            registered: midi_cc.registered != 0,
            index: midi_cc.index,
        };
        if plugin.on_midi2_learn(bus_index, channel, controller) {
            kResultOk
        } else {
            kResultFalse
        }
    }
}

// =============================================================================
// INoteExpressionController implementation (VST3 SDK 3.5.0)
// =============================================================================

impl<P: Plugin + 'static> INoteExpressionControllerTrait for Vst3Processor<P> {
    unsafe fn getNoteExpressionCount(&self, bus_index: i32, channel: i16) -> i32 {
        let plugin = self.plugin();
        plugin.note_expression_count(bus_index, channel) as i32
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

        let plugin = self.plugin();
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

        let plugin = self.plugin();
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
            let plugin = self.plugin();
            if let Some(value) = plugin.note_expression_string_to_value(bus_index, channel, id, &s)
            {
                *value_normalized = value;
                return kResultOk;
            }
        }
        kResultFalse
    }
}

// =============================================================================
// IKeyswitchController implementation (VST3 SDK 3.5.0)
// =============================================================================

impl<P: Plugin + 'static> IKeyswitchControllerTrait for Vst3Processor<P> {
    unsafe fn getKeyswitchCount(&self, bus_index: i32, channel: i16) -> i32 {
        let plugin = self.plugin();
        plugin.keyswitch_count(bus_index, channel) as i32
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

        let plugin = self.plugin();
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

impl<P: Plugin + 'static> INoteExpressionPhysicalUIMappingTrait for Vst3Processor<P> {
    unsafe fn getPhysicalUIMapping(
        &self,
        bus_index: i32,
        channel: i16,
        list: *mut PhysicalUIMapList,
    ) -> tresult {
        if list.is_null() {
            return kInvalidArgument;
        }

        let plugin = self.plugin();
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

impl<P: Plugin + 'static> IVst3WrapperMPESupportTrait for Vst3Processor<P> {
    unsafe fn enableMPEInputProcessing(&self, state: TBool) -> tresult {
        let plugin = self.plugin_mut();
        if plugin.enable_mpe_input_processing(state != 0) {
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
        let plugin = self.plugin_mut();
        let settings = beamr_core::MpeInputDeviceSettings {
            master_channel,
            member_begin_channel,
            member_end_channel,
        };
        if plugin.set_mpe_input_device_settings(settings) {
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
                    event: MidiEventKind::SysEx(sysex),
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
