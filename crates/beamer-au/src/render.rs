//! Render block implementation for Audio Unit.
//!
//! This module provides the `RenderBlock` type that handles audio processing for AU plugins.
//! The render block is created during `allocateRenderResources` and called by the native
//! Objective-C wrapper via C-ABI (`beamer_au_render`).
//!
//! # Architecture
//!
//! In the hybrid ObjC/Rust AU architecture:
//! - Native Objective-C (`BeamerAuWrapper.m`) creates the ObjC render block
//! - The ObjC render block calls `beamer_au_render()` via C-ABI
//! - `beamer_au_render()` delegates to `RenderBlock::process()`
//!
//! # Objective-C Block Callbacks
//!
//! For calling AU host callbacks (musical context, transport state, pull input),
//! we use `std::mem::transmute` to cast block pointers to function pointers.
//! This works because:
//! - Objective-C blocks have a function pointer at a known offset
//! - The first parameter is always the block pointer itself
//! - AU hosts guarantee block validity during render callbacks
//! - We never store blocks beyond the render callback
//!
//! ## Block Types Called
//!
//! 1. **AUHostMusicalContextBlock** (transport.rs): Query tempo, time signature, position
//! 2. **AUHostTransportStateBlock** (render.rs): Query play/stop/record state
//! 3. **AURenderPullInputBlock** (render.rs): Pull audio from auxiliary buses
//! 4. **AUScheduleMIDIEventBlock** (render.rs): Send MIDI output to host

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::buffer_storage::{ProcessBufferStorage, ProcessBufferStorageAuExt};
use crate::buffers::{AudioBuffer, AudioBufferList};
use crate::error::os_status;
use crate::instance::AuPluginInstance;
use crate::objc_block;
use crate::transport::extract_transport_from_au;
use beamer_core::{
    MidiEvent, MidiEventKind, ProcessContext, Sample, SysExOutputPool, MAX_BUSES, MAX_CHANNELS,
};

// =============================================================================
// MIDI Buffer
// =============================================================================

/// Pre-allocated MIDI buffer for real-time safe event collection.
///
/// Uses a `Vec` with pre-allocated capacity to avoid heap allocations during
/// audio processing. Events are collected during the render callback and
/// processed sample-accurately.
pub struct MidiBuffer {
    events: Vec<MidiEvent>,
    capacity: usize,
}

impl MidiBuffer {
    /// Create a new buffer with the specified capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: Vec::with_capacity(capacity),
            capacity,
        }
    }

    /// Clear the buffer without deallocating.
    #[inline]
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// Push an event if there's capacity.
    #[inline]
    pub fn push(&mut self, event: MidiEvent) -> bool {
        if self.events.len() < self.capacity {
            self.events.push(event);
            true
        } else {
            false
        }
    }

    /// Get the events as a slice.
    #[inline]
    pub fn as_slice(&self) -> &[MidiEvent] {
        &self.events
    }

    /// Get the event count.
    #[inline]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Get an iterator over the events.
    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, MidiEvent> {
        self.events.iter()
    }

    /// Sort events by sample offset (ascending).
    ///
    /// AU render event lists are typically ordered by `event_sample_time`, but
    /// the bridge does not rely on that invariant. Sorting here is allocation-free
    /// and enables efficient sub-block processing for sample-accurate timing.
    #[inline]
    pub fn sort_by_sample_offset(&mut self) {
        self.events.sort_by_key(|e| e.sample_offset);
    }

    /// Check if the buffer overflowed (reached capacity).
    #[inline]
    pub fn has_overflowed(&self) -> bool {
        self.events.len() >= self.capacity
    }
}

impl Default for MidiBuffer {
    fn default() -> Self {
        Self::with_capacity(256)
    }
}

// =============================================================================
// Parameter Events
// =============================================================================

/// Immediate parameter value change from host automation.
///
/// These events are applied sample-accurately by splitting the render call into
/// sub-blocks at event boundaries and applying changes at the start of each sub-block.
///
/// - `parameter_address`: Used to look up the target parameter by ID.
///
/// - `value`: Used to set the parameter's normalized value (0.0-1.0).
#[derive(Clone, Debug)]
pub struct AuParameterEvent {
    /// Sample offset within the current buffer.
    pub sample_offset: u32,
    /// AU parameter address (maps to beamer parameter ID)
    pub parameter_address: u64,
    /// New normalized value (0.0 to 1.0)
    pub value: f32,
}

/// Ramped parameter change for smooth automation.
///
/// # Field Usage
///
/// - `sample_offset`: Sample offset where the ramp starts. Like immediate events, ramps are
///   applied sample-accurately by sub-block processing.
///
/// - `parameter_address`: Used to look up the target parameter by ID.
///
/// - `start_value`: Preserved for API completeness with AU's `AURenderEventParameterRamp`.
///   The current implementation applies the ramp as a value change at `sample_offset` and
///   relies on the parameter's smoother (if any) for interpolation.
///
/// - `end_value`: Used to set the parameter's target normalized value.
///
/// - `duration_samples`: **Placeholder for future host-controlled ramping**. beamer_core's
///   Smoother uses a fixed time constant configured at parameter construction (via
///   `SmoothingStyle`). There is no API for dynamic per-event ramp duration configuration.
///   This matches VST3 behavior, which also doesn't use host-provided ramp info.
///
/// # Design Rationale
///
/// The current "set end value, let smoother interpolate" approach is intentional:
/// 1. **VST3 parity**: beamer-vst3 uses the same approach
/// 2. **Consistent behavior**: Plugin smoothers provide predictable transitions
/// 3. **Clear responsibility split**: The host chooses *when* a change starts; the plugin
///    chooses *how* it smooths within its own model
///
/// For most musical parameters, the configured smoother time (e.g., 5ms exponential)
/// provides smooth transitions regardless of the DAW's intended ramp duration.
///
/// The unused fields are preserved for API completeness with AU's
/// `AURenderEventParameterRamp` event structure and potential future support for
/// host-controlled ramp durations.
#[derive(Clone, Debug)]
pub struct AuParameterRampEvent {
    /// Sample offset where ramp starts.
    pub sample_offset: u32,
    /// AU parameter address (maps to beamer parameter ID)
    pub parameter_address: u64,
    /// Value at start of ramp (matches AU's `AURenderEventParameterRamp.value`).
    ///
    /// Currently unused: beamer's parameter smoothing uses `end_value` as target and interpolates
    /// with the parameter's configured `SmoothingStyle`. Future: Could enable host-controlled ramps.
    #[allow(dead_code)]
    pub start_value: f32,
    /// Value at end of ramp (used as target for parameter smoother)
    pub end_value: f32,
    /// Duration of ramp in samples (matches AU's `AURenderEventParameterRamp.ramp_duration_sample_frames`).
    ///
    /// Currently unused: beamer's parameter smoothers use a fixed time constant configured at
    /// parameter construction. Future: Could override smoother duration per-event.
    #[allow(dead_code)]
    pub duration_samples: u32,
}

/// Buffer for parameter events (pre-allocated).
pub struct ParameterEventBuffer {
    pub immediate: Vec<AuParameterEvent>,
    pub ramps: Vec<AuParameterRampEvent>,
}

impl Default for ParameterEventBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl ParameterEventBuffer {
    pub fn new() -> Self {
        Self {
            immediate: Vec::with_capacity(256),
            ramps: Vec::with_capacity(64),
        }
    }

    pub fn clear(&mut self) {
        self.immediate.clear();
        self.ramps.clear();
    }
}

// =============================================================================
// AU Render Event Types
// =============================================================================

/// AU render event types.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AURenderEventType {
    /// Parameter change
    Parameter = 1,
    /// Parameter ramp over time
    ParameterRamp = 2,
    /// MIDI 1.0 event (legacy)
    Midi = 8,
    /// MIDI SysEx event
    MidiSysEx = 9,
    /// MIDI 2.0 UMP event list (iOS 15+, macOS 12+)
    MidiEventList = 10,
}

/// Common header for all AU render events.
///
/// All events are linked via the `next` pointer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AURenderEventHeader {
    /// Pointer to next event in linked list (null if last)
    pub next: *const AURenderEvent,
    /// Sample frame offset within this render call
    pub event_sample_time: i64,
    /// Event type discriminator
    pub event_type: u8,
    /// Reserved, must be 0
    pub reserved: u8,
}

/// Parameter change event.
///
/// Contains an immediate parameter value change from host automation.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AURenderEventParameter {
    /// Pointer to next event
    pub next: *const AURenderEvent,
    /// Sample frame offset
    pub event_sample_time: i64,
    /// Event type (should be AURenderEventType::Parameter)
    pub event_type: u8,
    /// Reserved
    pub reserved: [u8; 3],
    /// Parameter address (u64)
    pub parameter_address: u64,
    /// New parameter value (f32)
    pub value: f32,
}

/// Parameter ramp event.
///
/// Contains a ramped parameter change for smooth automation.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AURenderEventParameterRamp {
    /// Pointer to next event
    pub next: *const AURenderEvent,
    /// Sample frame offset
    pub event_sample_time: i64,
    /// Event type (should be AURenderEventType::ParameterRamp)
    pub event_type: u8,
    /// Reserved
    pub reserved: [u8; 3],
    /// Parameter address (u64)
    pub parameter_address: u64,
    /// Start value (f32)
    pub value: f32,
    /// End value (f32) - Added in AU v3.1
    pub end_value: f32,
    /// Ramp duration in sample frames (u32)
    pub ramp_duration_sample_frames: u32,
}

/// Legacy MIDI 1.0 event.
///
/// Contains standard MIDI bytes (status, data1, data2).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AUMIDIEvent {
    /// Pointer to next event
    pub next: *const AURenderEvent,
    /// Sample frame offset
    pub event_sample_time: i64,
    /// Event type (should be AURenderEventType::Midi or MidiSysEx)
    pub event_type: u8,
    /// Reserved
    pub reserved: u8,
    /// Number of valid MIDI bytes (1-3 for channel voice, more for SysEx)
    pub length: u16,
    /// Virtual cable number
    pub cable: u8,
    /// MIDI data bytes (status, data1, data2)
    pub data: [u8; 3],
}

/// MIDI 2.0 UMP event list.
///
/// Contains Universal MIDI Packets in the newer MIDI 2.0 format.
#[repr(C)]
pub struct AUMIDIEventList {
    /// Pointer to next event
    pub next: *const AURenderEvent,
    /// Sample frame offset
    pub event_sample_time: i64,
    /// Event type (should be AURenderEventType::MidiEventList)
    pub event_type: u8,
    /// Reserved
    pub reserved: u8,
    /// Virtual cable number
    pub cable: u8,
    // MIDIEventList follows inline (variable length)
}

/// MIDIEventList from CoreMIDI (header only).
#[repr(C)]
pub struct MIDIEventList {
    /// Protocol: 1 = MIDI 1.0, 2 = MIDI 2.0
    pub protocol: u32,
    /// Number of packets in this list
    pub num_packets: u32,
    // MIDIEventPacket array follows (variable length)
}

/// MIDIEventPacket from CoreMIDI.
#[repr(C)]
pub struct MIDIEventPacket {
    /// Timestamp (host time in nanoseconds, 0 = now)
    pub time_stamp: u64,
    /// Number of 32-bit UMP words (1-64)
    pub word_count: u32,
    // UMP words follow (variable length array of u32)
}

impl MIDIEventPacket {
    /// Get the UMP words as a slice.
    ///
    /// # Safety
    /// Caller must ensure `word_count` is valid and memory is readable.
    #[inline]
    pub unsafe fn words(&self) -> &[u32] {
        // SAFETY: Caller guarantees word_count is valid and memory is readable.
        // MIDIEventPacket layout: time_stamp (u64) + word_count (u32) + words[].
        // We skip past the header to get to the words array.
        let words_ptr = unsafe {
            (self as *const Self as *const u8)
                .add(std::mem::size_of::<u64>() + std::mem::size_of::<u32>())
                as *const u32
        };
        // SAFETY: words_ptr points to word_count valid u32 values per caller contract.
        unsafe { std::slice::from_raw_parts(words_ptr, self.word_count as usize) }
    }

    /// Get pointer to the next packet.
    ///
    /// # Safety
    /// Caller must ensure there is a valid next packet.
    #[inline]
    pub unsafe fn next(&self) -> *const MIDIEventPacket {
        // SAFETY: Caller guarantees there is a valid next packet.
        // We calculate the next packet address by skipping past this packet's words.
        let words_ptr = unsafe {
            (self as *const Self as *const u8)
                .add(std::mem::size_of::<u64>() + std::mem::size_of::<u32>())
        };
        // SAFETY: Skip past word_count words to reach the next packet.
        unsafe {
            words_ptr.add(self.word_count as usize * std::mem::size_of::<u32>())
                as *const MIDIEventPacket
        }
    }
}

/// AU render event union.
///
/// Access via `head.event_type` to determine which variant is active.
#[repr(C)]
pub union AURenderEvent {
    /// Common header (always safe to access)
    pub head: AURenderEventHeader,
    /// Parameter change event
    pub parameter: AURenderEventParameter,
    /// Parameter ramp event
    pub ramp: AURenderEventParameterRamp,
    /// Legacy MIDI 1.0 event
    pub midi: AUMIDIEvent,
    // Note: midi_events_list omitted for now
}

/// SMPTE time structure.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SMPTETime {
    pub subframes: i16,
    pub subframe_divisor: i16,
    pub counter: u32,
    pub smpte_type: u32,
    pub flags: u32,
    pub hours: i16,
    pub minutes: i16,
    pub seconds: i16,
    pub frames: i16,
}

/// Audio timestamp structure from Core Audio.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioTimeStamp {
    /// Sample time
    pub sample_time: f64,
    /// Host time (Mach absolute time)
    pub host_time: u64,
    /// Rate scalar
    pub rate_scalar: f64,
    /// Word clock time
    pub word_clock_time: u64,
    /// SMPTE time
    pub smpte_time: SMPTETime,
    /// Flags indicating which fields are valid
    pub flags: u32,
    /// Reserved
    pub reserved: u32,
}


/// AU host transport state flags.
///
/// These flags are returned from the AUHostTransportStateBlock callback
/// to indicate the current transport state (playing, recording, cycling).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AUHostTransportStateFlags(pub u32);

impl AUHostTransportStateFlags {
    /// Transport state has changed since last query
    pub const CHANGED: u32 = 1 << 0;
    /// Transport is moving (playing/recording)
    pub const MOVING: u32 = 1 << 1;
    /// Transport is currently recording
    pub const RECORDING: u32 = 1 << 2;
    /// Transport is cycling (looping)
    pub const CYCLING: u32 = 1 << 3;

    /// Check if transport is moving (playing)
    #[inline]
    pub fn is_playing(self) -> bool {
        (self.0 & Self::MOVING) != 0
    }

    /// Check if transport is recording
    #[inline]
    pub fn is_recording(self) -> bool {
        (self.0 & Self::RECORDING) != 0
    }

    /// Check if transport is cycling (looping)
    #[inline]
    pub fn is_cycling(self) -> bool {
        (self.0 & Self::CYCLING) != 0
    }
}

/// AU render pull input block function signature.
///
/// This is an Objective-C block provided by the AU host that the plugin
/// calls to pull audio from auxiliary input buses (e.g., sidechain).
///
/// # Objective-C Block Signature (from Apple's AU v3 API)
///
/// ```objc
/// typedef OSStatus (^AURenderPullInputBlock)(
///     AudioUnitRenderActionFlags *actionFlags,
///     const AudioTimeStamp *timestamp,
///     AVAudioFrameCount frameCount,
///     NSInteger inputBusNumber,
///     AudioBufferList *inputData
/// );
/// ```
///
/// # Function Signature
///
/// This type alias represents the C function pointer equivalent of the Objective-C block.
/// Note that Objective-C blocks are more complex than function pointers (they include
/// a capture context), but they can be called as function pointers when properly cast.
///
/// # Parameters
///
/// * `action_flags` - Pointer to AudioUnitRenderActionFlags (mutable, host may modify)
/// * `timestamp` - Pointer to AudioTimeStamp for this render call (immutable)
/// * `frame_count` - Number of frames to render (AVAudioFrameCount = u32)
/// * `input_bus_number` - Which input bus to pull from:
///   - 0 = main input bus
///   - 1+ = auxiliary input buses (sidechain, etc.)
/// * `input_data` - Pointer to AudioBufferList to fill with audio data (mutable)
///
/// # Returns
///
/// OSStatus (i32):
/// - 0 (noErr) = success, audio was provided
/// - Non-zero = error occurred, audio may not be valid
///
/// # Safety
///
/// This function is unsafe because:
/// 1. It dereferences raw pointers provided by the caller
/// 2. It must be called with valid pointers that remain valid for the call duration
/// 3. The AudioBufferList must have properly initialized buffer structures
/// 4. The host may write to memory pointed to by input_data
///
/// # Usage
///
/// The plugin calls this block during its render callback to pull audio from
/// auxiliary input buses. The host fills the provided AudioBufferList with audio data.
/// This enables features like sidechain compression, vocoding, etc.
///
/// # Example
///
/// ```ignore
/// // Pull sidechain audio from aux bus 1
/// let status = pull_fn(
///     action_flags,
///     timestamp,
///     frame_count,
///     1,  // aux bus 1
///     &mut buffer_list as *mut AudioBufferList,
/// );
/// if status == 0 {
///     // Audio is available in buffer_list
/// }
/// ```
type AURenderPullInputBlock = unsafe extern "C" fn(
    block: *const c_void,
    action_flags: *mut u32,
    timestamp: *const AudioTimeStamp,
    frame_count: u32,
    input_bus_number: isize,
    input_data: *mut AudioBufferList,
) -> i32;

// =============================================================================
// AudioBufferList Allocation Helpers
// =============================================================================

/// Allocate an AudioBufferList with null data pointers.
///
/// Used for aux buses where the host always provides its own buffer pointers.
/// The host fills in the data pointers when pullInputBlock is called.
///
/// # Arguments
///
/// * `num_buffers` - Number of AudioBuffer entries to allocate
/// * `num_samples` - Number of samples per buffer (for size calculation)
/// * `sample_type_size` - Size of sample type in bytes (4 for f32, 8 for f64)
fn allocate_audio_buffer_list(
    num_buffers: usize,
    num_samples: usize,
    sample_type_size: usize,
) -> Box<AudioBufferList> {
    // Calculate total size needed
    // AudioBufferList has: u32 + [AudioBuffer; 1], but we need [AudioBuffer; num_buffers]
    let base_size = std::mem::size_of::<u32>(); // number_buffers field
    let buffer_size = std::mem::size_of::<AudioBuffer>() * num_buffers;
    let total_size = base_size + buffer_size;

    // Allocate raw memory
    let layout =
        std::alloc::Layout::from_size_align(total_size, std::mem::align_of::<AudioBufferList>())
            .expect("Failed to create layout for AudioBufferList");

    // SAFETY: We allocate memory with the correct layout for AudioBufferList with
    // num_buffers entries. The flexible array member pattern requires manual allocation
    // because Rust can't represent variable-length trailing arrays in structs.
    // We initialize all fields before returning, and Box::from_raw takes ownership.
    unsafe {
        let ptr = std::alloc::alloc(layout) as *mut AudioBufferList;
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }

        // Initialize the structure
        (*ptr).number_buffers = num_buffers as u32;

        // Initialize each buffer
        for i in 0..num_buffers {
            let buffer = (*ptr).buffers.as_mut_ptr().add(i);
            (*buffer).number_channels = 1; // Non-interleaved
            (*buffer).data_byte_size = (num_samples * sample_type_size) as u32;
            (*buffer).data = std::ptr::null_mut(); // Will be filled by host
        }

        Box::from_raw(ptr)
    }
}

// =============================================================================
// MIDI Extraction
// =============================================================================

/// Maximum number of events to process per buffer to prevent infinite loops.
///
/// This limit protects against corrupted event lists that form cycles.
/// 4096 events per buffer is generous - typical buffers have < 100 events.
const MAX_EVENTS_PER_BUFFER: usize = 4096;

/// Extract MIDI events from AU render event linked list.
///
/// Iterates through the event list and converts MIDI events to beamer format.
/// Handles both legacy MIDI 1.0 events and MIDI 2.0 UMP events.
///
/// # Safety
/// The `event_list` pointer must be valid or null.
pub unsafe fn extract_midi_events(event_list: *const AURenderEvent, buffer: &mut MidiBuffer) {
    let mut event_ptr = event_list;
    let mut iterations = 0;

    while !event_ptr.is_null() && iterations < MAX_EVENTS_PER_BUFFER {
        iterations += 1;
        // SAFETY: event_ptr validated non-null above, iterations bounded by MAX_EVENTS_PER_BUFFER.
        // Linked list provided by AU host, valid for this render callback.
        let event = unsafe { &*event_ptr };
        // SAFETY: head field is always safe to access in AURenderEvent union per AU API.
        let event_type = unsafe { event.head.event_type };

        match event_type {
            // Legacy MIDI 1.0 event
            8 => {
                // AURenderEventType::Midi
                // SAFETY: event_type == 8 (Midi), so midi field is the active union variant.
                let midi_event = unsafe { &event.midi };
                if midi_event.length >= 1 {
                    let sample_offset = midi_event.event_sample_time as u32;
                    let status = midi_event.data[0] & 0xF0;
                    let channel = midi_event.data[0] & 0x0F;
                    let data1 = if midi_event.length >= 2 {
                        midi_event.data[1]
                    } else {
                        0
                    };
                    let data2 = if midi_event.length >= 3 {
                        midi_event.data[2]
                    } else {
                        0
                    };

                    if let Some(beamer_event) =
                        MidiEvent::from_midi1_bytes(sample_offset, status, channel, data1, data2)
                    {
                        buffer.push(beamer_event);
                    }
                }
            }
            // MIDI 2.0 UMP event list
            10 => {
                // AURenderEventType::MidiEventList
                // SAFETY: head is always safe to access.
                let sample_offset = unsafe { event.head.event_sample_time as u32 };
                // Get pointer to MIDIEventList (immediately after AUMIDIEventList header)
                // SAFETY: For MidiEventList events, the MIDIEventList follows the header.
                let event_list_ptr = unsafe {
                    (event_ptr as *const u8)
                        .add(std::mem::size_of::<AUMIDIEventList>())
                        as *const MIDIEventList
                };
                // SAFETY: event_list_ptr is valid for MidiEventList events.
                let midi_list = unsafe { &*event_list_ptr };

                // Get first packet
                // SAFETY: MIDIEventList layout: protocol (u32) + num_packets (u32) + packets[].
                let mut packet_ptr = unsafe {
                    (midi_list as *const MIDIEventList as *const u8)
                        .add(std::mem::size_of::<u32>() * 2)
                        as *const MIDIEventPacket
                };

                for _ in 0..midi_list.num_packets {
                    // SAFETY: We iterate only up to num_packets, which is provided by AU host.
                    let packet = unsafe { &*packet_ptr };
                    // SAFETY: words() requires valid word_count, which is set by AU host.
                    let words = unsafe { packet.words() };

                    // Parse UMP words
                    for &word in words {
                        let message_type = (word >> 28) & 0xF;
                        if message_type == 2 {
                            // MIDI 1.0 Channel Voice in UMP format
                            let status = ((word >> 16) & 0xF0) as u8;
                            let channel = ((word >> 16) & 0x0F) as u8;
                            let data1 = ((word >> 8) & 0x7F) as u8;
                            let data2 = (word & 0x7F) as u8;

                            if let Some(beamer_event) =
                                MidiEvent::from_midi1_bytes(sample_offset, status, channel, data1, data2)
                            {
                                buffer.push(beamer_event);
                            }
                        }
                    }

                    // SAFETY: next() is safe when there are more packets to iterate.
                    packet_ptr = unsafe { packet.next() };
                }
            }
            _ => {
                // Ignore parameter events and other types
            }
        }

        // SAFETY: head.next is always safe to access and points to next event or null.
        event_ptr = unsafe { event.head.next };
    }

    if iterations >= MAX_EVENTS_PER_BUFFER {
        log::warn!(
            "MIDI event list exceeded maximum iterations ({}), possible corruption",
            MAX_EVENTS_PER_BUFFER
        );
    }
}

/// Update MidiCcState from incoming MIDI events.
///
/// Scans the MIDI buffer for CC, pitch bend, and channel pressure events,
/// updating the MidiCcState accordingly. This allows plugins to query current
/// controller values via `ProcessContext::midi_cc()`.
///
/// # Implementation Notes
///
/// - CC values are normalized: 0-127 → 0.0-1.0
/// - Pitch bend is converted: -1.0 to 1.0 (beamer format) → 0.0-1.0 (MidiCcState format)
/// - Channel pressure is normalized: 0.0-1.0 (already normalized in beamer)
/// - Uses atomic operations internally for thread safety (MidiCcState takes `&self`)
fn update_midi_cc_state(
    midi_buffer: &MidiBuffer,
    cc_state: &beamer_core::MidiCcState,
) {
    use beamer_core::midi_cc_config::controller;
    use beamer_core::ParameterStore;

    for event in midi_buffer.as_slice() {
        match &event.event {
            MidiEventKind::ControlChange(cc) => {
                // MidiCcState uses parameter IDs, need to call set_normalized with parameter ID
                let param_id = beamer_core::MidiCcState::parameter_id(cc.controller);
                // CC values are already normalized (0.0-1.0) in beamer format
                cc_state.set_normalized(param_id, cc.value as f64);
            }
            MidiEventKind::PitchBend(pb) => {
                // Pitch bend in beamer: -1.0 to 1.0 (bipolar)
                // MidiCcState stores as: 0.0 to 1.0 (normalized, center at 0.5)
                let normalized = (pb.value as f64 + 1.0) / 2.0;
                let param_id = beamer_core::MidiCcState::parameter_id(controller::PITCH_BEND);
                cc_state.set_normalized(param_id, normalized);
            }
            MidiEventKind::ChannelPressure(cp) => {
                // Channel pressure is already normalized (0.0-1.0) in beamer format
                let param_id = beamer_core::MidiCcState::parameter_id(controller::AFTERTOUCH);
                cc_state.set_normalized(param_id, cp.pressure as f64);
            }
            _ => {}
        }
    }
}

// =============================================================================
// Parameter Event Extraction
// =============================================================================

/// Extract parameter events from AU render event linked list.
///
/// Iterates through the event list and extracts parameter change and ramp events.
/// MIDI and other event types are ignored by this function.
///
/// # Safety
/// The `event_list` pointer must be valid or null.
pub unsafe fn extract_parameter_events(
    event_list: *const AURenderEvent,
    buffer: &mut ParameterEventBuffer,
) {
    buffer.clear();

    let mut event_ptr = event_list;
    let mut iterations = 0;

    while !event_ptr.is_null() && iterations < MAX_EVENTS_PER_BUFFER {
        iterations += 1;
        // SAFETY: event_ptr validated non-null above, iterations bounded by MAX_EVENTS_PER_BUFFER.
        // Linked list provided by AU host, valid for this render callback.
        let event = unsafe { &*event_ptr };
        // SAFETY: head field is always safe to access in AURenderEvent union per AU API.
        let event_type = unsafe { event.head.event_type };

        match event_type {
            // AU_RENDER_EVENT_PARAMETER (type 1)
            1 => {
                // SAFETY: event_type == 1 (Parameter), so parameter field is the active variant.
                let param_event = unsafe { &event.parameter };
                buffer.immediate.push(AuParameterEvent {
                    sample_offset: param_event.event_sample_time as u32,
                    parameter_address: param_event.parameter_address,
                    value: param_event.value,
                });
            }
            // AU_RENDER_EVENT_PARAMETER_RAMP (type 2)
            2 => {
                // SAFETY: event_type == 2 (ParameterRamp), so ramp field is the active variant.
                let ramp_event = unsafe { &event.ramp };
                buffer.ramps.push(AuParameterRampEvent {
                    sample_offset: ramp_event.event_sample_time as u32,
                    parameter_address: ramp_event.parameter_address,
                    start_value: ramp_event.value,
                    end_value: ramp_event.end_value,
                    duration_samples: ramp_event.ramp_duration_sample_frames,
                });
            }
            _ => {
                // MIDI and other events handled separately
            }
        }

        // SAFETY: head.next is always safe to access and points to next event or null.
        event_ptr = unsafe { event.head.next };
    }

    if iterations >= MAX_EVENTS_PER_BUFFER {
        log::warn!(
            "Parameter event list exceeded maximum iterations ({}), possible corruption",
            MAX_EVENTS_PER_BUFFER
        );
    }
}

// =============================================================================
// Render Block Trait
// =============================================================================

/// Type-erased trait for render blocks.
///
/// This trait allows storing different sample type render blocks
/// (f32 or f64) in the same type-erased container.
///
/// Clippy Allow: too_many_arguments
///
/// The `process()` signature matches Apple's AU API which requires 8 parameters.
/// Cannot be refactored into a struct without breaking AU host compatibility.
#[allow(clippy::too_many_arguments)]
pub trait RenderBlockTrait: Send + Sync {
    /// Process audio through this render block.
    ///
    /// # Arguments
    /// * `action_flags` - Render action flags
    /// * `timestamp` - Audio timestamp
    /// * `frame_count` - Number of frames to process
    /// * `output_bus_number` - Output bus index
    /// * `output_data` - Output audio buffer list
    /// * `event_list` - Linked list of render events (MIDI, parameter changes)
    /// * `pull_input_block` - Block to pull aux bus inputs
    /// * `input_data` - Input audio buffer list (already pulled by ObjC)
    fn process(
        &self,
        action_flags: *mut u32,
        timestamp: *const AudioTimeStamp,
        frame_count: u32,
        output_bus_number: i32,
        output_data: *mut AudioBufferList,
        event_list: *const AURenderEvent,
        pull_input_block: *const c_void,
        input_data: *const AudioBufferList,
    ) -> i32;

    /// Get a raw pointer to this render block.
    fn as_ptr(&self) -> *const c_void;

    /// Get the sample rate.
    fn sample_rate(&self) -> f64;
}

/// Generic render block implementation.
///
/// Generic over sample type S (f32 or f64) to support both single and double precision.
/// The render block is Arc-wrapped in audio_unit.rs to ensure proper
/// lifetime management - the pointer returned by internalRenderBlock
/// remains valid as long as the Arc is held.
pub struct RenderBlock<S: Sample> {
    /// Reference to the plugin for audio processing
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
    /// Pre-allocated buffer storage for zero-allocation rendering
    storage: UnsafeCell<ProcessBufferStorage<S>>,
    /// Pre-allocated MIDI buffer for zero-allocation MIDI processing
    midi_buffer: UnsafeCell<MidiBuffer>,
    /// Pre-allocated parameter event buffer for zero-allocation parameter automation
    parameter_events: UnsafeCell<ParameterEventBuffer>,
    /// Musical context block from AU host for transport info
    musical_context_block: Option<*const c_void>,
    /// Transport state block from AU host for playback state (is_playing, etc.)
    transport_state_block: Option<*const c_void>,
    /// Current sample rate for ProcessContext
    sample_rate: f64,
    /// Pre-allocated AudioBufferList structures for pulling aux input buses
    /// One per aux input bus (bus 1, 2, 3, ...)
    ///
    /// Clippy Allow: vec_box
    ///
    /// `Vec<Box<AudioBufferList>>` is necessary because AudioBufferList uses a flexible array
    /// member (FAM) pattern with variable size determined at allocation time. Each Box maintains
    /// the custom allocation from `allocate_audio_buffer_list()`. Cannot use `Vec<AudioBufferList>`
    /// because AudioBufferList is not Sized (contains variable-length array).
    #[allow(clippy::vec_box)]
    aux_input_buffer_lists: UnsafeCell<Vec<Box<AudioBufferList>>>,
    /// Pre-allocated MIDI output buffer for zero-allocation MIDI output processing
    midi_output: UnsafeCell<MidiBuffer>,
    /// SysEx output pool for real-time safe SysEx message output
    sysex_output_pool: UnsafeCell<SysExOutputPool>,
    /// Host-provided block for scheduling MIDI output events.
    ///
    /// This is an `AUScheduleMIDIEventBlock` provided by the AU host.
    /// Only available for component types that support MIDI output:
    /// - `aumu` (Music Device/Instrument)
    /// - `aumf` (MIDI Effect)
    ///
    /// Effects (`aufx`) typically don't receive this block from hosts.
    schedule_midi_event_block: Option<*const c_void>,
    /// Per-instance warmup counter to silence initial renders.
    ///
    /// The first few render calls may contain garbage from host-provided buffers.
    /// This counter ensures each RenderBlock instance independently silences its
    /// first 4 renders, even after channel config changes that recreate the instance.
    warmup_count: AtomicUsize,
}

// SAFETY: The raw pointers are only used within a single render call
// where AU guarantees single-threaded access.
unsafe impl<S: Sample> Send for RenderBlock<S> {}

// SAFETY: Same as Send impl. Raw pointers in UnsafeCell are accessed only during
// render callbacks which AU guarantees are single-threaded.
unsafe impl<S: Sample> Sync for RenderBlock<S> {}

// =============================================================================
// Sample Type Dispatch Macro
// =============================================================================

/// Dispatch to f32 or f64 plugin method based on sample type S.
///
/// This macro eliminates the repeated TypeId check + transmute pattern used when
/// calling plugin methods. The pattern is necessary because:
/// - RenderBlock is generic over S (f32 or f64)
/// - Plugin trait has separate methods for each type (e.g., process_with_context vs process_with_context_f64)
/// - We need runtime dispatch to call the correct method
///
/// # Usage
///
/// ```ignore
/// dispatch_sample_type!(S,
///     f32 => { plugin.process_with_context(inputs_f32, outputs_f32, context) },
///     f64 => { plugin.process_with_context_f64(inputs_f64, outputs_f64, context) }
/// )
/// ```
///
/// # Safety
///
/// The macro relies on these invariants:
/// - S is either f32 or f64 (enforced by Sample trait being sealed)
/// - TypeId check guarantees type match before any transmute occurs in caller code
/// - Returns error OSStatus if S is neither type (should never happen)
///
/// The caller is responsible for ensuring that any transmutes performed within
/// the f32/f64 blocks are safe. The TypeId check performed by this macro
/// guarantees that the type parameter S matches the branch being executed.
macro_rules! dispatch_sample_type {
    ($sample_type:ty, f32 => $f32_expr:expr, f64 => $f64_expr:expr) => {
        if std::any::TypeId::of::<$sample_type>() == std::any::TypeId::of::<f32>() {
            match $f32_expr {
                Ok(()) => os_status::NO_ERR,
                Err(_) => os_status::K_AUDIO_UNIT_ERR_RENDER,
            }
        } else if std::any::TypeId::of::<$sample_type>() == std::any::TypeId::of::<f64>() {
            match $f64_expr {
                Ok(()) => os_status::NO_ERR,
                Err(_) => os_status::K_AUDIO_UNIT_ERR_RENDER,
            }
        } else {
            // Should never happen - Sample trait is sealed to f32/f64 only
            os_status::K_AUDIO_UNIT_ERR_RENDER
        }
    };
}

impl<S: Sample> RenderBlock<S> {
    /// Create a new render block.
    ///
    /// # Arguments
    ///
    /// * `plugin` - Arc-wrapped plugin instance for audio processing
    /// * `storage` - Pre-allocated buffer storage (created from bus config)
    /// * `musical_context_block` - Optional AU host musical context block for transport info
    /// * `transport_state_block` - Optional AU host transport state block for playback state
    /// * `schedule_midi_event_block` - Optional AU host MIDI output block (for instruments/MIDI effects)
    /// * `max_frames` - Maximum frames per render call
    /// * `sample_rate` - Current sample rate in Hz
    pub fn new(
        plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
        storage: ProcessBufferStorage<S>,
        musical_context_block: Option<*const c_void>,
        transport_state_block: Option<*const c_void>,
        schedule_midi_event_block: Option<*const c_void>,
        max_frames: u32,
        sample_rate: f64,
    ) -> Self {
        let aux_input_bus_count = storage.aux_input_bus_count();
        let sample_type_size = std::mem::size_of::<S>();

        // Pre-allocate AudioBufferList for each aux input bus
        // This ensures zero allocation in the render path
        let mut aux_input_buffer_lists = Vec::with_capacity(aux_input_bus_count);

        for _ in 0..aux_input_bus_count {
            // Each aux bus can have up to MAX_CHANNELS
            // The host will fill in the actual channel count when we call pullInputBlock
            let buffer_list =
                allocate_audio_buffer_list(MAX_CHANNELS, max_frames as usize, sample_type_size);
            aux_input_buffer_lists.push(buffer_list);
        }

        Self {
            plugin,
            storage: UnsafeCell::new(storage),
            midi_buffer: UnsafeCell::new(MidiBuffer::with_capacity(1024)),
            parameter_events: UnsafeCell::new(ParameterEventBuffer::new()),
            musical_context_block,
            transport_state_block,
            sample_rate,
            aux_input_buffer_lists: UnsafeCell::new(aux_input_buffer_lists),
            midi_output: UnsafeCell::new(MidiBuffer::with_capacity(1024)),
            sysex_output_pool: UnsafeCell::new(SysExOutputPool::new()),
            schedule_midi_event_block,
            warmup_count: AtomicUsize::new(0),
        }
    }

    /// Output a MIDI event to the host via scheduleMIDIEventBlock.
    ///
    /// This function sends MIDI data to the AU host if the scheduleMIDIEventBlock
    /// was provided. This block is only available for component types that support
    /// MIDI output (aumu instruments and aumf MIDI effects).
    ///
    /// # Arguments
    ///
    /// * `midi_bytes` - Raw MIDI bytes to send (status + data bytes, or full SysEx)
    /// * `sample_offset` - Sample offset within the current buffer
    ///
    /// # Returns
    ///
    /// `true` if the event was sent successfully, `false` if MIDI output is not available.
    ///
    /// # Safety
    ///
    /// This function is safe to call from the render thread. The scheduleMIDIEventBlock
    /// is guaranteed to be valid for the duration of the render callback by the AU host.
    fn output_midi_to_host(&self, midi_bytes: &[u8], sample_offset: u32) -> bool {
        let Some(block) = self.schedule_midi_event_block else {
            return false;
        };

        // AUScheduleMIDIEventBlock signature (from Apple's Audio Unit v3 API):
        //
        // typedef void (^AUScheduleMIDIEventBlock)(
        //     AUEventSampleTime eventSampleTime,  // i64
        //     uint8_t cable,                      // u8
        //     NSInteger length,                   // isize
        //     const uint8_t *midiBytes            // *const u8
        // );
        //
        // Define the function signature that matches Apple's AUScheduleMIDIEventBlock.
        // The first parameter is the block pointer itself (Objective-C block convention).
        type AUScheduleMIDIEventBlockFn = unsafe extern "C" fn(
            block: *const c_void,   // Block pointer itself (Objective-C convention)
            event_sample_time: i64, // AUEventSampleTime
            cable: u8,              // Virtual cable number (typically 0)
            length: isize,          // NSInteger - number of MIDI bytes
            midi_bytes: *const u8,  // Pointer to MIDI data
        );

        // SAFETY: This transmute is required because Rust doesn't have native Objective-C block support.
        //
        // Why this transmute is needed:
        // - AU hosts provide the MIDI output callback as an Objective-C block (*const c_void)
        // - We need to call this block to send MIDI events to the host
        // - The block must be cast to a function pointer with the correct signature
        //
        // Invariants that must hold:
        // 1. `block` must be a valid AUScheduleMIDIEventBlock provided by AU host
        // 2. The block must remain valid for the duration of this render callback
        // 3. The function signature must exactly match Apple's documented AUScheduleMIDIEventBlock
        // 4. Must be called from the AU render thread only
        // 5. midi_bytes must point to valid MIDI data for the duration of the call
        //
        // What could go wrong:
        // - If block pointer is invalid/corrupted -> undefined behavior (crash)
        // - If signature doesn't match -> argument misalignment, undefined behavior
        // - If called from wrong thread -> race conditions (violates AU threading model)
        //
        // Why this is safe in practice:
        // - AU hosts guarantee the block is valid during the render callback
        // - Our signature matches Apple's documented API exactly
        // - We only call from within render callback, never store the pointer
        // - midi_bytes points to our pre-allocated pool which outlives this call
        unsafe {
            let invoke = objc_block::invoke_ptr(block);
            let block_fn: AUScheduleMIDIEventBlockFn = std::mem::transmute(invoke);
            block_fn(
                block,
                sample_offset as i64,
                0, // cable 0 (default virtual cable)
                midi_bytes.len() as isize,
                midi_bytes.as_ptr(),
            );
        }

        true
    }

    /// Output a SysEx message to the host.
    ///
    /// SysEx messages are sent as raw MIDI bytes including F0 (start) and F7 (end).
    ///
    /// # Arguments
    ///
    /// * `sysex_data` - Full SysEx message bytes (F0 ... F7)
    /// * `sample_offset` - Sample offset within the current buffer
    ///
    /// # Returns
    ///
    /// `true` if sent successfully, `false` if MIDI output is not available.
    #[inline]
    fn output_sysex_to_host(&self, sysex_data: &[u8], sample_offset: u32) -> bool {
        self.output_midi_to_host(sysex_data, sample_offset)
    }

    /// Encode a MIDI event to bytes for transmission.
    ///
    /// Returns `Some([bytes])` for standard MIDI 1.0 messages that can be sent via
    /// `scheduleMIDIEventBlock`. Returns `None` for SysEx (which requires separate handling)
    /// and unsupported event types (MPE/expression data, DAW metadata).
    ///
    /// # MIDI 1.0 Status Bytes
    ///
    /// - 0x80: Note Off
    /// - 0x90: Note On
    /// - 0xA0: Polyphonic Key Pressure (Aftertouch)
    /// - 0xB0: Control Change
    /// - 0xC0: Program Change
    /// - 0xD0: Channel Pressure (Aftertouch)
    /// - 0xE0: Pitch Bend Change
    ///
    /// All status bytes are OR'd with the channel (0x00-0x0F) to create the final status byte.
    fn encode_midi_event(event: &MidiEventKind) -> Option<[u8; 3]> {
        match event {
            MidiEventKind::NoteOn(note) => Some([
                0x90 | (note.channel & 0x0F),
                note.pitch & 0x7F,
                ((note.velocity * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
            ]),
            MidiEventKind::NoteOff(note) => Some([
                0x80 | (note.channel & 0x0F),
                note.pitch & 0x7F,
                ((note.velocity * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
            ]),
            MidiEventKind::ControlChange(cc) => Some([
                0xB0 | (cc.channel & 0x0F),
                cc.controller & 0x7F,
                ((cc.value * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
            ]),
            MidiEventKind::PitchBend(pb) => {
                // Convert -1.0..1.0 to 0..16383 (14-bit)
                let raw = (((pb.value + 1.0) * 8192.0).clamp(0.0, 16383.0) as u16) & 0x3FFF;
                let lsb = (raw & 0x7F) as u8;
                let msb = ((raw >> 7) & 0x7F) as u8;
                Some([0xE0 | (pb.channel & 0x0F), lsb, msb])
            }
            MidiEventKind::PolyPressure(pp) => Some([
                0xA0 | (pp.channel & 0x0F),
                pp.pitch & 0x7F,
                ((pp.pressure * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
            ]),
            MidiEventKind::ChannelPressure(cp) => Some([
                0xD0 | (cp.channel & 0x0F),
                ((cp.pressure * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
                0, // Unused third byte (2-byte message)
            ]),
            MidiEventKind::ProgramChange(pc) => Some([
                0xC0 | (pc.channel & 0x0F),
                pc.program & 0x7F,
                0, // Unused third byte (2-byte message)
            ]),
            // SysEx requires separate handling via output_sysex_to_host
            MidiEventKind::SysEx(_) => None,
            // The following event types don't have standard MIDI 1.0 wire encodings
            // and cannot be output via AU's scheduleMIDIEventBlock:
            // - NoteExpressionValue/Int/Text: MPE/MIDI 2.0 per-note expressions
            // - ChordInfo/ScaleInfo: DAW-specific metadata (not MIDI messages)
            MidiEventKind::NoteExpressionValue(_)
            | MidiEventKind::NoteExpressionInt(_)
            | MidiEventKind::NoteExpressionText(_)
            | MidiEventKind::ChordInfo(_)
            | MidiEventKind::ScaleInfo(_) => None,
        }
    }

    /// Output all MIDI events from the output buffer to the host.
    ///
    /// This function iterates through the MIDI output buffer and sends each event
    /// to the host via scheduleMIDIEventBlock. If no block is available (e.g., for
    /// effect plugins), events are counted and a warning is logged.
    ///
    /// # Arguments
    ///
    /// * `midi_output` - Buffer containing MIDI events to send
    /// * `sysex_pool` - Pool containing allocated SysEx data
    ///
    /// # Returns
    ///
    /// The number of events that could not be sent (0 if all sent or no events).
    fn output_all_midi_events(
        &self,
        midi_output: &MidiBuffer,
        sysex_pool: &SysExOutputPool,
    ) -> usize {
        if midi_output.is_empty() {
            return 0;
        }

        // If no MIDI output block is available, count dropped events
        if self.schedule_midi_event_block.is_none() {
            return midi_output.len();
        }

        let mut dropped = 0;

        // Track SysEx slot index to match events with pool allocations
        let mut sysex_slot = 0;

        for event in midi_output.iter() {
            let sample_offset = event.sample_offset;

            match &event.event {
                MidiEventKind::SysEx(sysex) => {
                    // SysEx data was allocated to the pool; use it if available
                    // The pool stores SysEx in order, so we track the slot index
                    if sysex_slot < sysex_pool.used() {
                        // Send the SysEx data directly from the event
                        // (pool allocation was for stability, but we can use original data here)
                        if !self.output_sysex_to_host(sysex.as_slice(), sample_offset) {
                            dropped += 1;
                        }
                        sysex_slot += 1;
                    } else {
                        // Pool exhausted for this SysEx
                        dropped += 1;
                    }
                }
                other => {
                    // Try to encode as standard MIDI 1.0 message
                    if let Some(bytes) = Self::encode_midi_event(other) {
                        // Determine actual message length (some messages are 2 bytes)
                        let len = match other {
                            MidiEventKind::ProgramChange(_) | MidiEventKind::ChannelPressure(_) => {
                                2
                            }
                            _ => 3,
                        };
                        if !self.output_midi_to_host(&bytes[..len], sample_offset) {
                            dropped += 1;
                        }
                    }
                    // Note: unsupported events (MPE/expression, DAW metadata) are silently
                    // skipped and not counted as dropped since they have no MIDI 1.0 encoding
                }
            }
        }

        dropped
    }

    /// Process audio through this render block (generic implementation).
    ///
    /// This is the core audio processing function that would be called
    /// by the AU host's render block.
    ///
    /// This implementation uses pre-allocated storage to eliminate Vec allocations
    /// in the render path, ensuring real-time safety.
    ///
    /// Clippy Allow: too_many_arguments - Signature dictated by AU API requirements.
    #[allow(clippy::too_many_arguments)]
    fn process_impl(
        &self,
        action_flags: *mut u32,
        timestamp: *const AudioTimeStamp,
        frame_count: u32,
        _output_bus_number: i32,
        output_data: *mut AudioBufferList,
        event_list: *const AURenderEvent,
        pull_input_block: *const c_void,
        input_data: *const AudioBufferList,
    ) -> i32 {
        // Real-time safety: use try_lock to avoid blocking
        let mut plugin_guard = match self.plugin.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                // Lock contention - return error to host
                return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT;
            }
        };

        // Check if plugin is prepared
        if !plugin_guard.is_prepared() {
            return os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED;
        }

        let num_samples = frame_count as usize;

        // Use pre-allocated storage instead of Vec allocations
        // SAFETY: We have exclusive access via &self, and AU guarantees
        // single-threaded render calls. The UnsafeCell allows interior
        // mutability for the storage reuse pattern.
        let storage = unsafe { &mut *self.storage.get() };

        // Clear storage - O(1) operation, no deallocation
        storage.clear();

        // Clear and extract MIDI events from AU event list
        // SAFETY: Same reasoning as storage - single-threaded render calls
        let midi_buffer = unsafe { &mut *self.midi_buffer.get() };
        midi_buffer.clear();

        // Clear MIDI output buffer for new block
        // SAFETY: AU guarantees single-threaded render calls. No aliasing possible.
        let midi_output = unsafe { &mut *self.midi_output.get() };
        midi_output.clear();

        // Clear SysEx pool for new block
        // SAFETY: AU guarantees single-threaded render calls. No aliasing possible.
        let sysex_pool = unsafe { &mut *self.sysex_output_pool.get() };
        sysex_pool.clear();

        // Extract MIDI events from the AU render event linked list
        // SAFETY: event_list is valid for this render call (provided by AU host)
        unsafe {
            extract_midi_events(event_list, midi_buffer);
        }

        // Convert absolute sample times to relative buffer offsets.
        //
        // AU's eventSampleTime is an ABSOLUTE sample position (like the transport).
        // We need to subtract the timestamp's sample_time (the buffer start position)
        // to get a relative offset within [0, frame_count).
        // SAFETY: timestamp is valid for this render call (provided by AU host).
        // We check for null before dereferencing.
        let buffer_start_sample = unsafe {
            if !timestamp.is_null() {
                (*timestamp).sample_time as i64
            } else {
                0
            }
        };

        for event in midi_buffer.events.iter_mut() {
            let absolute_time = event.sample_offset as i64;
            let relative_offset = absolute_time - buffer_start_sample;

            // Clamp to buffer bounds (handle late/early events gracefully)
            event.sample_offset = relative_offset.clamp(0, (num_samples - 1) as i64) as u32;
        }

        // Ensure events are ordered by sample offset so we can slice them efficiently
        // during sub-block processing (sample-accurate automation).
        midi_buffer.sort_by_sample_offset();

        // Update MIDI CC state from incoming events
        // This allows plugins to query current CC values via context.midi_cc()
        if let Some(cc_state) = plugin_guard.midi_cc_state() {
            update_midi_cc_state(midi_buffer, cc_state);
        }

        // Process MIDI events (input → output transformation)
        // This allows plugins to transform, generate, or pass through MIDI
        plugin_guard.process_midi(midi_buffer.as_slice(), midi_output);

        // Clear and extract parameter events from AU event list
        // SAFETY: Same reasoning as storage - single-threaded render calls
        let parameter_events = unsafe { &mut *self.parameter_events.get() };
        parameter_events.clear();

        // Extract parameter events from the AU render event linked list
        // SAFETY: event_list is valid for this render call (provided by AU host)
        unsafe {
            extract_parameter_events(event_list, parameter_events);
        }

        // Sort by sample offset to enable sample-accurate application via sub-block processing.
        parameter_events.immediate.sort_by_key(|e| e.sample_offset);
        parameter_events.ramps.sort_by_key(|e| e.sample_offset);

        // Extract transport info from AU host
        // SAFETY: timestamp and transport_state_block are valid for this render call
        let transport = unsafe {
            // Extract is_playing from transport state block if available
            let is_playing = match self.transport_state_block {
                Some(block) => {
                    // AUHostTransportStateBlock signature (from Apple's Audio Unit v3 API):
                    // BOOL (^)(AUHostTransportStateFlags *outTransportStateFlags,
                    //          double *outCurrentSamplePosition,
                    //          double *outCycleStartBeatPosition,
                    //          double *outCycleEndBeatPosition)
                    //
                    // Define the function signature that matches Apple's AUHostTransportStateBlock.
                    // The first parameter is the block pointer itself (Objective-C block convention).
                    type TransportStateBlockFn = unsafe extern "C" fn(
                        *const c_void, // Block pointer itself (Objective-C convention)
                        *mut u32,      // outTransportStateFlags (AUHostTransportStateFlags)
                        *mut f64,      // outCurrentSamplePosition
                        *mut f64,      // outCycleStartBeatPosition
                        *mut f64,      // outCycleEndBeatPosition
                    ) -> bool; // Returns true if successful

                    let mut flags: u32 = 0;
                    let mut current_sample_pos: f64 = 0.0;
                    let mut cycle_start: f64 = 0.0;
                    let mut cycle_end: f64 = 0.0;

                    // SAFETY: This transmute is required because Rust doesn't have native Objective-C block support.
                    //
                    // Why this transmute is needed:
                    // - AU hosts provide callbacks as Objective-C blocks (*const c_void)
                    // - Objective-C blocks are callable objects with a function pointer at a known offset
                    // - We must call this function with the correct signature to retrieve transport state
                    //
                    // Invariants that must hold:
                    // 1. `block` must be a valid AUHostTransportStateBlock provided by the AU host
                    // 2. The block must remain valid for the duration of this render callback
                    // 3. The function signature must exactly match Apple's documented AUHostTransportStateBlock:
                    //    - First arg: block pointer itself (Objective-C convention)
                    //    - outTransportStateFlags: bitfield indicating state (playing, recording, cycling)
                    //    - outCurrentSamplePosition: current sample position in timeline
                    //    - outCycleStartBeatPosition: loop start position in beats
                    //    - outCycleEndBeatPosition: loop end position in beats
                    // 4. The block must be called from the AU render thread only
                    //
                    // What could go wrong:
                    // - If block pointer is invalid/corrupted → undefined behavior (crash likely)
                    // - If signature doesn't match → argument misalignment, undefined behavior
                    // - If called from wrong thread → race conditions (violates AU threading model)
                    // - If block is used after render callback → use-after-free
                    //
                    // Why this is safe in practice:
                    // - AU hosts guarantee the block is valid during the render callback
                    // - Our signature matches Apple's documented API exactly
                    // - We only call from within render callback, never store the pointer
                    // - The block is host-provided, not user-created
                    //
                    // Alternative approach:
                    // - Use `block2` crate for proper Objective-C block handling (adds dependency)
                    let invoke = objc_block::invoke_ptr(block);
                    let block_fn: TransportStateBlockFn = std::mem::transmute(invoke);

                    // Call the block to retrieve transport state
                    let success = block_fn(
                        block,
                        &mut flags,
                        &mut current_sample_pos,
                        &mut cycle_start,
                        &mut cycle_end,
                    );

                    // If call succeeded, check if MOVING flag is set
                    if success {
                        AUHostTransportStateFlags(flags).is_playing()
                    } else {
                        false
                    }
                }
                None => false, // No transport state block, default to stopped
            };

            let sample_position = if !timestamp.is_null() {
                (*timestamp).sample_time as i64
            } else {
                0
            };

            match self.musical_context_block {
                Some(block) => extract_transport_from_au(block, sample_position, is_playing),
                None => beamer_core::Transport {
                    project_time_samples: Some(sample_position),
                    is_playing,
                    ..Default::default()
                },
            }
        };

        // Collect pointers from AudioBufferList
        // SAFETY: output_data is valid for the duration of this render call
        unsafe {
            // Silence the first few renders entirely to avoid any host-provided garbage making it
            // to the outputs.
            let warmup_idx = self.warmup_count.fetch_add(1, Ordering::Relaxed);
            if warmup_idx < 4 {
                // Zero output buffers during warmup
                if !output_data.is_null() {
                    let list = &mut *output_data;
                    for i in 0..list.number_buffers {
                        let buf = list.buffer_at_mut(i);
                        if !buf.data.is_null() && buf.data_byte_size > 0 {
                            let bytes = std::slice::from_raw_parts_mut(
                                buf.data as *mut u8,
                                buf.data_byte_size as usize,
                            );
                            bytes.fill(0);
                        }
                    }
                }
                return os_status::NO_ERR;
            }

            // Use input_data provided by ObjC (already pulled using AVAudioPCMBuffer).
            // This matches Apple's pattern where input pulling happens in ObjC.
            if !input_data.is_null() {
                let in_list = &*input_data;

                // Collect inputs from the ObjC-provided buffer
                storage.collect_inputs(input_data, num_samples);

                // Handle in-place processing: if output buffers have null data pointers,
                // fill them with input buffer pointers (like Apple's FilterDemo does).
                if !output_data.is_null() {
                    let out_list = &mut *output_data;

                    // Check if ANY output buffer has null data (in-place processing expected)
                    let mut needs_in_place = false;
                    for i in 0..out_list.number_buffers {
                        if out_list.buffer_at(i).data.is_null() {
                            needs_in_place = true;
                            break;
                        }
                    }

                    // If in-place processing is needed, fill null output pointers with input pointers
                    if needs_in_place {
                        let num_to_fill = out_list.number_buffers.min(in_list.number_buffers);
                        for i in 0..num_to_fill {
                            let out_buf = out_list.buffer_at_mut(i);
                            if out_buf.data.is_null() {
                                let in_buf = in_list.buffer_at(i);
                                out_buf.data = in_buf.data;
                                out_buf.data_byte_size = in_buf.data_byte_size;
                            }
                        }
                    }
                }
            }
            // If input_data is null, this is an instrument with no inputs

            // Now collect output pointers (after in-place fixup)
            storage.collect_outputs(output_data, num_samples);

            // Note: We do NOT zero output buffers here because:
            // 1. For in-place processing, output now points to input data which we need
            // 2. The plugin's process() will overwrite output anyway
            // 3. Zeroing would destroy the input data in the in-place case

            // Pull auxiliary bus inputs if available
            // SAFETY: pull_input_block is valid for this render call (provided by AU host)
            if !pull_input_block.is_null() {
                let aux_buffer_lists = &mut *self.aux_input_buffer_lists.get();
                let aux_input_count = aux_buffer_lists.len();

                if aux_input_count > 0 {
                    // SAFETY: This transmute is required because Rust doesn't have native Objective-C block support.
                    //
                    // Why this transmute is needed:
                    // - AU hosts provide the pull input callback as an Objective-C block (*const c_void)
                    // - We need to call this block to pull audio from auxiliary input buses (e.g., sidechain)
                    // - The block must be cast to a function pointer with the correct signature
                    //
                    // Invariants that must hold:
                    // 1. `pull_input_block` must be a valid AURenderPullInputBlock provided by AU host
                    // 2. The block must remain valid for the duration of this render callback
                    // 3. The function signature must exactly match Apple's documented AURenderPullInputBlock:
                    //    - action_flags: pointer to AudioUnitRenderActionFlags
                    //    - timestamp: pointer to AudioTimeStamp for this render call
                    //    - frame_count: number of frames to render
                    //    - input_bus_number: which input bus to pull from (0=main, 1+=aux)
                    //    - input_data: AudioBufferList to fill with audio data
                    //    - Returns: OSStatus (0 = success)
                    // 4. Must be called from the AU render thread only
                    // 5. The AudioBufferList passed must have valid buffer structure
                    //
                    // What could go wrong:
                    // - If block pointer is invalid/corrupted → undefined behavior (crash)
                    // - If signature doesn't match → argument misalignment, undefined behavior
                    // - If called from wrong thread → race conditions (violates AU threading model)
                    // - If AudioBufferList structure is invalid → host may write to wrong memory
                    // - If bus_number is out of range → host may return error or undefined behavior
                    //
                    // Why this is safe in practice:
                    // - AU hosts guarantee the block is valid during the render callback
                    // - Our signature matches Apple's documented API (see AURenderPullInputBlock type alias above)
                    // - We only call from within render callback, never store the pointer
                    // - We pre-allocate valid AudioBufferList structures with correct sizes
                    // - We only request aux buses that exist (based on bus_config)
                    //
                    // Alternative approach:
                    // - Use `block2` crate for proper Objective-C block handling (adds dependency)
                    let invoke = objc_block::invoke_ptr(pull_input_block);
                    let pull_fn: AURenderPullInputBlock = std::mem::transmute(invoke);

                    // Use stack-based array to avoid heap allocation in render path
                    // This is real-time safe since MAX_BUSES is a compile-time constant
                    let mut buffer_list_ptrs: [*const AudioBufferList; MAX_BUSES] =
                        [std::ptr::null(); MAX_BUSES];

                    // Pull audio from each auxiliary input bus (bus index starts at 1)
                    for (aux_idx, buffer_list) in aux_buffer_lists.iter_mut().enumerate() {
                        let bus_number = (aux_idx + 1) as isize; // Bus 0 is main, 1+ are aux

                        // Reset buffer data pointers to null before calling pull
                        // The host will fill them in
                        for i in 0..buffer_list.number_buffers {
                            let buffer = buffer_list.buffers.as_mut_ptr().add(i as usize);
                            (*buffer).data = std::ptr::null_mut();
                        }

                        // Call the pull input block to get audio from this aux bus
                        let status = pull_fn(
                            pull_input_block,
                            action_flags,
                            timestamp,
                            frame_count,
                            bus_number,
                            &mut **buffer_list as *mut AudioBufferList,
                        );

                        // If pull succeeded, store pointer for collection
                        if status == os_status::NO_ERR {
                            buffer_list_ptrs[aux_idx] = &**buffer_list as *const AudioBufferList;
                        }
                        // If pull failed, leave as null (already initialized to null)
                    }

                    // Collect auxiliary input pointers from pulled buffer lists
                    // Pass only the slice we actually need (not the whole array)
                    storage.collect_aux_inputs(&buffer_list_ptrs[..aux_input_count], num_samples);
                }
            }

            // Fallback for instruments (no inputs): if we still have no output channels collected,
            // the host provided null output buffers but there were no input buffers to use.
            // In this case, we can't process audio (instruments would need their own output buffers).
            if storage.output_channel_count() == 0 && storage.main_inputs.is_empty() {
                // No inputs and no outputs collected - return silence
                return os_status::NO_ERR;
            }

            // Note: We intentionally do NOT zero output buffers here because:
            // 1. For in-place processing, output pointers point to input data which we need
            // 2. The plugin's process() will overwrite output anyway
            // 3. Zeroing would destroy the input data in the in-place case (Logic Pro uses this)
        }

        let _main_inputs = storage.input_channel_count();
        let _main_outputs = storage.output_channel_count();
        let _aux_buses = storage.aux_input_bus_count();
        let _aux_input_channels: usize = storage.aux_inputs.iter().map(|bus| bus.len()).sum();

        // Build slices from collected pointers.
        // NOTE: these helper methods currently allocate Vecs, but only once per render call.
        // We avoid per-sub-block allocations by converting to raw pointer lists once and
        // rebuilding slice views for each sub-block.
        // SAFETY: Pointers in storage are valid for this render call (collected from AU host buffers).
        // num_samples matches the frame_count provided by the host.
        let input_refs = unsafe { storage.input_slices(num_samples) };
        // SAFETY: Same as above, outputs were collected from valid AU buffers.
        let mut output_refs = unsafe { storage.output_slices(num_samples) };
        // SAFETY: Same as above, aux inputs were collected from valid AU buffers.
        let aux_input_refs = unsafe { storage.aux_input_slices(num_samples) };
        // SAFETY: Same as above, aux outputs were collected from valid AU buffers.
        let mut aux_output_refs = unsafe { storage.aux_output_slices(num_samples) };

        let input_ptrs: Vec<*const S> = input_refs.iter().map(|s| s.as_ptr()).collect();
        let output_ptrs: Vec<*mut S> = output_refs.iter_mut().map(|s| s.as_mut_ptr()).collect();
        let aux_input_ptrs: Vec<Vec<*const S>> = aux_input_refs
            .iter()
            .map(|bus| bus.iter().map(|ch| ch.as_ptr()).collect())
            .collect();
        let aux_output_ptrs: Vec<Vec<*mut S>> = aux_output_refs
            .iter_mut()
            .map(|bus| bus.iter_mut().map(|ch| ch.as_mut_ptr()).collect())
            .collect();

        // Release the full-buffer slice views so we can safely create sub-slices
        // for each sub-block from raw pointers.
        drop(input_refs);
        drop(output_refs);
        drop(aux_input_refs);
        drop(aux_output_refs);

        // Pre-allocate per-sub-block slice containers (no per-boundary allocations).
        let mut segment_inputs: Vec<&[S]> = Vec::with_capacity(input_ptrs.len());
        let mut segment_outputs: Vec<&mut [S]> = Vec::with_capacity(output_ptrs.len());
        let mut segment_aux_inputs: Vec<Vec<&[S]>> = aux_input_ptrs
            .iter()
            .map(|bus| Vec::with_capacity(bus.len()))
            .collect();
        let mut segment_aux_outputs: Vec<Vec<&mut [S]>> = aux_output_ptrs
            .iter()
            .map(|bus| Vec::with_capacity(bus.len()))
            .collect();

        // SAFETY: We use a raw pointer to work around borrow checker limitations.
        // MidiCcState uses atomics internally, and we only read it.
        let cc_state_ptr: Option<*const beamer_core::MidiCcState> =
            plugin_guard.midi_cc_state().map(|cc| cc as *const _);

        // Sample-accurate automation via sub-block processing.
        // We split at parameter event boundaries and apply changes exactly at the start of each sub-block.
        let mut result = os_status::NO_ERR;

        let immediate = &parameter_events.immediate;
        let ramps = &parameter_events.ramps;
        let midi_events_all = midi_buffer.as_slice();

        let mut imm_idx: usize = 0;
        let mut ramp_idx: usize = 0;
        let mut midi_idx: usize = 0;

        // Scratch MIDI buffer for sub-block processing (reused, no per-boundary allocations).
        let mut segment_midi: Vec<MidiEvent> = Vec::with_capacity(midi_events_all.len());

        let mut block_start: usize = 0;
        while block_start < num_samples {
            // Collect all parameter events scheduled at or before this boundary.
            let imm_apply_start = imm_idx;
            while imm_idx < immediate.len()
                && immediate[imm_idx].sample_offset as usize <= block_start
            {
                imm_idx += 1;
            }
            let ramp_apply_start = ramp_idx;
            while ramp_idx < ramps.len() && ramps[ramp_idx].sample_offset as usize <= block_start {
                ramp_idx += 1;
            }

            // Determine next boundary.
            let mut next_boundary = num_samples;
            if imm_idx < immediate.len() {
                next_boundary = next_boundary.min(immediate[imm_idx].sample_offset as usize);
            }
            if ramp_idx < ramps.len() {
                next_boundary = next_boundary.min(ramps[ramp_idx].sample_offset as usize);
            }
            if next_boundary < block_start {
                next_boundary = block_start;
            }

            let block_len = next_boundary.saturating_sub(block_start);
            if block_len == 0 {
                // Guard against malformed event lists with duplicate/unsorted offsets.
                // Advance by 1 sample to prevent an infinite loop.
                block_start += 1;
                continue;
            }

            // Apply parameter changes scheduled at this boundary.
            // If application fails, continue processing (parameters may already be correct).
            let _ = plugin_guard.apply_parameter_events(
                &immediate[imm_apply_start..imm_idx],
                &ramps[ramp_apply_start..ramp_idx],
            );

            // Build main bus slices for this sub-block.
            segment_inputs.clear();
            for &ptr in &input_ptrs {
                // SAFETY: ptr is valid for the render call; block_start+block_len is within num_samples.
                let ch = unsafe { slice::from_raw_parts(ptr.add(block_start), block_len) };
                segment_inputs.push(ch);
            }

            segment_outputs.clear();
            for &ptr in &output_ptrs {
                // SAFETY: ptr is valid for the render call; block_start+block_len is within num_samples.
                let ch = unsafe { slice::from_raw_parts_mut(ptr.add(block_start), block_len) };
                segment_outputs.push(ch);
            }

            // Build aux bus slices for this sub-block.
            for (bus_idx, bus_ptrs) in aux_input_ptrs.iter().enumerate() {
                let bus = &mut segment_aux_inputs[bus_idx];
                bus.clear();
                for &ptr in bus_ptrs {
                    // SAFETY: ptr is valid for render call; block_start+block_len within num_samples.
                    let ch = unsafe { slice::from_raw_parts(ptr.add(block_start), block_len) };
                    bus.push(ch);
                }
            }
            for (bus_idx, bus_ptrs) in aux_output_ptrs.iter().enumerate() {
                let bus = &mut segment_aux_outputs[bus_idx];
                bus.clear();
                for &ptr in bus_ptrs {
                    // SAFETY: ptr is valid for render call; block_start+block_len within num_samples.
                    let ch = unsafe { slice::from_raw_parts_mut(ptr.add(block_start), block_len) };
                    bus.push(ch);
                }
            }

            // Slice and rebase MIDI events for this sub-block.
            // MIDI offsets passed to the plugin must be relative to the start of the current block.
            segment_midi.clear();

            while midi_idx < midi_events_all.len()
                && (midi_events_all[midi_idx].sample_offset as usize) < block_start
            {
                midi_idx += 1;
            }
            let mut midi_scan = midi_idx;
            while midi_scan < midi_events_all.len()
                && (midi_events_all[midi_scan].sample_offset as usize) < (block_start + block_len)
            {
                let mut ev = midi_events_all[midi_scan].clone();
                ev.sample_offset = ev.sample_offset.saturating_sub(block_start as u32);
                segment_midi.push(ev);
                midi_scan += 1;
            }
            midi_idx = midi_scan;

            // Adjust transport sample position for this sub-block.
            let mut seg_transport = transport;
            if let Some(t) = seg_transport.project_time_samples {
                seg_transport.project_time_samples = Some(t + block_start as i64);
            }

            let context = if let Some(cc_ptr) = cc_state_ptr {
                // SAFETY: cc_ptr obtained from plugin_guard earlier in this function.
                // MidiCcState uses atomics internally and is safe to read.
                let cc_state = unsafe { &*cc_ptr };
                ProcessContext::with_midi_cc(self.sample_rate, block_len, seg_transport, cc_state)
            } else {
                ProcessContext::new(self.sample_rate, block_len, seg_transport)
            };

            let block_status = self.call_plugin_process_with_midi(
                &mut plugin_guard,
                &segment_inputs,
                &mut segment_outputs,
                &segment_aux_inputs,
                &mut segment_aux_outputs,
                &segment_midi,
                &context,
            );

            if block_status != os_status::NO_ERR {
                result = block_status;
                break;
            }

            block_start = next_boundary;
        }

        // Handle MIDI output via scheduleMIDIEventBlock (if available)
        //
        // AU MIDI output depends on component type:
        // - `aumu` (Music Device/Instrument): MIDI output supported via scheduleMIDIEventBlock
        // - `aumf` (MIDI Effect): MIDI output supported
        // - `aufx` (Effect): MIDI output NOT typically supported by hosts
        //
        // For effects, most hosts don't provide scheduleMIDIEventBlock, so MIDI output
        // events will be dropped with a warning.

        // First, allocate SysEx messages to the pool for stable pointers
        for midi_event in midi_output.iter() {
            if let MidiEventKind::SysEx(sysex) = &midi_event.event {
                // Allocate from pool for stable pointer during output
                let _ = sysex_pool.allocate_slice(sysex.as_slice());
            }
        }

        // Now output all MIDI events to the host
        let dropped_events = self.output_all_midi_events(midi_output, sysex_pool);

        // Log warnings for dropped events
        if dropped_events > 0 {
            if self.schedule_midi_event_block.is_none() {
                // No MIDI output block - this is expected for effect plugins (aufx)
                // Only log at debug level to avoid spamming for effect plugins that
                // generate MIDI output (which is unusual but possible)
                log::debug!(
                    "AU MIDI output not available: {} events dropped. \
                     MIDI output is only supported for instrument (aumu) and MIDI effect (aumf) plugins. \
                     Effects (aufx) typically do not support MIDI output.",
                    dropped_events
                );
            } else {
                // Block is available but events still dropped (shouldn't happen)
                log::warn!(
                    "MIDI output error: {} events could not be sent to host",
                    dropped_events
                );
            }
        }

        // Check for MIDI output buffer overflow
        if midi_output.has_overflowed() {
            log::warn!(
                "MIDI output buffer overflow: {} events reached capacity, some events were dropped",
                midi_output.len()
            );
        }

        // Check for SysEx pool overflow
        if sysex_pool.has_overflowed() {
            log::warn!(
                "SysEx output pool overflow: {} slots exhausted, some SysEx messages were dropped",
                sysex_pool.capacity()
            );
        }

        result
    }

    /// Call the plugin's process method with auxiliary buses and MIDI.
    ///
    /// This method dispatches to either process_with_midi (f32) or
    /// process_with_midi_f64 (f64) based on the sample type S.
    ///
    /// # Safety Pattern: TypeId Check + Transmute
    ///
    /// This function uses a TypeId-based dispatch pattern with transmute to handle
    /// generic sample types. This pattern is necessary because:
    ///
    /// 1. **Why this pattern is needed:**
    ///    - The RenderBlock is generic over sample type S (f32 or f64)
    ///    - The plugin trait has separate methods for f32 and f64 (process_with_midi vs process_with_midi_f64)
    ///    - We need to dispatch to the correct method based on the actual type at runtime
    ///    - Generic dispatch alone can't choose between different method names
    ///
    /// 2. **Invariants that must hold:**
    ///    - S must be either f32 or f64 (enforced by Sample trait bound)
    ///    - The RenderBlock<S> is created with the same S used by the host's format
    ///    - Audio Unit v3 only supports f32 and f64 (kAudioFormatFlagsNativeFloatPacked)
    ///    - The TypeId check ensures we only transmute when types actually match
    ///
    /// 3. **What could go wrong:**
    ///    - If S is neither f32 nor f64 -> we return error (last else branch)
    ///    - If TypeId check fails but transmute happens anyway -> undefined behavior (crash likely)
    ///    - If buffer layout doesn't match expected type -> memory corruption
    ///    - If Sample trait is implemented for non-f32/f64 types -> potential transmute mismatch
    ///
    /// 4. **Why this is safe in practice:**
    ///    - Sample trait is sealed and only implemented for f32 and f64
    ///    - RenderBlock<S> is created based on AU format (kAudioFormatFlagIsFloat + bits per channel)
    ///    - create_render_block_f32/f64 ensure S matches the actual host format
    ///    - TypeId check is a runtime guarantee that S == f32 or S == f64
    ///    - Buffer slices have the same memory layout regardless of Sample type
    ///      (both are just &[T] where T is a 32-bit or 64-bit float)
    ///
    /// 5. **Alternative approaches:**
    ///    - Use an enum for sample format and store non-generic buffers (requires Vec allocation)
    ///    - Use trait objects with dynamic dispatch (requires heap allocation)
    ///    - Duplicate the entire RenderBlock for f32 and f64 (code duplication)
    ///    - Make AuPluginInstance generic (breaks trait object usage)
    ///
    /// 6. **Why transmute is sound here:**
    ///    - &[&[f32]] and &[&[f64]] have identical memory layout (slice of slice pointers)
    ///    - We only transmute when TypeId proves the types match
    ///    - The underlying audio buffer data is already in the correct format (host-provided)
    ///    - We never transmute the actual sample data, only the slice references
    ///
    /// **Auxiliary bus transmutes:**
    /// - Transmutes `&[Vec<&[S]>]` to `&[Vec<&[f32]>]` or `&[Vec<&[f64]>]`
    /// - Each aux bus is a Vec of channel slices
    /// - Vec<&[S]> and Vec<&[f32]> have identical memory layout
    /// - Vec stores pointer + length + capacity (no type-specific data)
    /// - The slice references point to host-provided audio buffers in the correct format
    ///
    /// Clippy Allow: too_many_arguments - Needs all parameters from process_impl plus MIDI data.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn call_plugin_process_with_midi(
        &self,
        plugin: &mut Box<dyn AuPluginInstance>,
        inputs: &[&[S]],
        outputs: &mut [&mut [S]],
        aux_inputs: &[Vec<&[S]>],
        aux_outputs: &mut [Vec<&mut [S]>],
        midi_events: &[MidiEvent],
        context: &ProcessContext,
    ) -> i32 {
        // dispatch_sample_type! performs a runtime TypeId check that guarantees S == f32
        // or S == f64 in the respective branch. The transmutes are sound because slice
        // and Vec layouts are identical regardless of the inner type.
        dispatch_sample_type!(S,
            f32 => {
                // SAFETY: Runtime TypeId check guarantees S == f32 in this branch.
                // &[&[S]] and &[&[f32]] have identical layouts when S == f32.
                let inputs_f32: &[&[f32]] = unsafe { std::mem::transmute(inputs) };
                // SAFETY: Runtime TypeId check guarantees S == f32 in this branch.
                let outputs_f32: &mut [&mut [f32]] = unsafe { std::mem::transmute(outputs) };
                // SAFETY: Runtime TypeId check guarantees S == f32 in this branch.
                // Vec<&[S]> and Vec<&[f32]> have identical layouts when S == f32.
                let aux_inputs_f32: &[Vec<&[f32]>] = unsafe { std::mem::transmute(aux_inputs) };
                // SAFETY: Runtime TypeId check guarantees S == f32 in this branch.
                let aux_outputs_f32: &mut [Vec<&mut [f32]>] =
                    unsafe { std::mem::transmute(aux_outputs) };
                plugin.process_with_midi(
                    inputs_f32,
                    outputs_f32,
                    aux_inputs_f32,
                    aux_outputs_f32,
                    midi_events,
                    context,
                )
            },
            f64 => {
                // SAFETY: Runtime TypeId check guarantees S == f64 in this branch.
                // &[&[S]] and &[&[f64]] have identical layouts when S == f64.
                let inputs_f64: &[&[f64]] = unsafe { std::mem::transmute(inputs) };
                // SAFETY: Runtime TypeId check guarantees S == f64 in this branch.
                let outputs_f64: &mut [&mut [f64]] = unsafe { std::mem::transmute(outputs) };
                // SAFETY: Runtime TypeId check guarantees S == f64 in this branch.
                // Vec<&[S]> and Vec<&[f64]> have identical layouts when S == f64.
                let aux_inputs_f64: &[Vec<&[f64]>] = unsafe { std::mem::transmute(aux_inputs) };
                // SAFETY: Runtime TypeId check guarantees S == f64 in this branch.
                let aux_outputs_f64: &mut [Vec<&mut [f64]>] =
                    unsafe { std::mem::transmute(aux_outputs) };
                plugin.process_with_midi_f64(
                    inputs_f64,
                    outputs_f64,
                    aux_inputs_f64,
                    aux_outputs_f64,
                    midi_events,
                    context,
                )
            }
        )
    }
}

/// Implement RenderBlockTrait for both f32 and f64.
impl<S: Sample> RenderBlockTrait for RenderBlock<S> {
    fn process(
        &self,
        action_flags: *mut u32,
        timestamp: *const AudioTimeStamp,
        frame_count: u32,
        output_bus_number: i32,
        output_data: *mut AudioBufferList,
        event_list: *const AURenderEvent,
        pull_input_block: *const c_void,
        input_data: *const AudioBufferList,
    ) -> i32 {
        self.process_impl(
            action_flags,
            timestamp,
            frame_count,
            output_bus_number,
            output_data,
            event_list,
            pull_input_block,
            input_data,
        )
    }

    fn as_ptr(&self) -> *const c_void {
        self as *const Self as *const c_void
    }

    fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}

/// Create the AU render block for audio processing (f32).
///
/// Returns a boxed RenderBlock that can be stored and used for audio processing.
///
/// # Arguments
///
/// * `plugin` - Arc-wrapped plugin instance
/// * `storage` - Pre-allocated buffer storage (created from bus config)
/// * `musical_context_block` - Optional AU host musical context block for transport info
/// * `transport_state_block` - Optional AU host transport state block for playback state
/// * `schedule_midi_event_block` - Optional AU host MIDI output block (for instruments/MIDI effects)
/// * `max_frames` - Maximum frames per render call
/// * `sample_rate` - Current sample rate in Hz
pub fn create_render_block_f32(
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
    storage: ProcessBufferStorage<f32>,
    musical_context_block: Option<*const c_void>,
    transport_state_block: Option<*const c_void>,
    schedule_midi_event_block: Option<*const c_void>,
    max_frames: u32,
    sample_rate: f64,
) -> Box<dyn RenderBlockTrait> {
    Box::new(RenderBlock::<f32>::new(
        plugin,
        storage,
        musical_context_block,
        transport_state_block,
        schedule_midi_event_block,
        max_frames,
        sample_rate,
    ))
}

/// Create the AU render block for audio processing (f64).
///
/// Returns a boxed RenderBlock that can be stored and used for audio processing.
///
/// # Arguments
///
/// * `plugin` - Arc-wrapped plugin instance
/// * `storage` - Pre-allocated buffer storage (created from bus config)
/// * `musical_context_block` - Optional AU host musical context block for transport info
/// * `transport_state_block` - Optional AU host transport state block for playback state
/// * `schedule_midi_event_block` - Optional AU host MIDI output block (for instruments/MIDI effects)
/// * `max_frames` - Maximum frames per render call
/// * `sample_rate` - Current sample rate in Hz
pub fn create_render_block_f64(
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
    storage: ProcessBufferStorage<f64>,
    musical_context_block: Option<*const c_void>,
    transport_state_block: Option<*const c_void>,
    schedule_midi_event_block: Option<*const c_void>,
    max_frames: u32,
    sample_rate: f64,
) -> Box<dyn RenderBlockTrait> {
    Box::new(RenderBlock::<f64>::new(
        plugin,
        storage,
        musical_context_block,
        transport_state_block,
        schedule_midi_event_block,
        max_frames,
        sample_rate,
    ))
}
