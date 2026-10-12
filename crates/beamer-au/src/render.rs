//! Render block implementation for Audio Unit.
//!
//! This module provides the `RenderBlock` type that handles audio processing for AU plugins.
//! The render block is created when render resources are allocated and called by the native
//! wrappers via C-ABI (`beamer_au_render`).
//!
//! # Architecture
//!
//! In the hybrid ObjC/Rust AU architecture:
//! - The generated wrapper receives the host's render call (the `internalRenderBlock` of the
//!   AUv3 wrapper, `BeamerAuv2Render` of the AUv2 wrapper)
//! - The wrapper calls `beamer_au_render()` via C-ABI
//! - `beamer_au_render()` delegates to `RenderBlock::process()`
//!
//! # Objective-C Block Callbacks
//!
//! To pull audio from auxiliary input buses, the render block calls the host's
//! `AURenderPullInputBlock`. We use `std::mem::transmute` to cast the block
//! pointer to a function pointer. This works because:
//! - Objective-C blocks have a function pointer at a known offset
//! - The first parameter is always the block pointer itself
//! - AU hosts guarantee block validity during render callbacks
//! - We never store blocks beyond the render callback
//!
//! The host's musical context and transport state are not queried here: the
//! wrappers do that in their own language and pass the result as plain data
//! (see `transport.rs`). MIDI output goes the other way through a plain
//! function the wrapper passes (see `MidiOutputSink`).

use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::slice;
use std::sync::{Arc, Mutex};

use crate::buffer_storage::{ProcessBufferStorage, ProcessBufferStorageAuExt};
use crate::buffers::{AudioBuffer, AudioBufferList};
use crate::error::os_status;
use crate::instance::AuPluginInstance;
use crate::objc_block;
use crate::transport::{transport_from_host, BeamerAuTransport};
use beamer_core::{
    MidiEvent, MidiEventKind, ProcessContext, Sample, MAX_BUSES, MAX_CHANNELS, MAX_SYSEX_SIZE,
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

    /// Get the events as a mutable slice.
    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [MidiEvent] {
        &mut self.events
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
/// - `value`: The new value in AU format: the plain value, or the index for an
///   indexed parameter. It is converted to a normalized value when it is applied.
#[derive(Clone, Debug)]
pub struct AuParameterEvent {
    /// Sample offset within the current buffer.
    pub sample_offset: u32,
    /// AU parameter address (maps to beamer parameter ID)
    pub parameter_address: u64,
    /// New value in AU format (plain value, or index for indexed parameters)
    pub value: f32,
}

/// Ramped parameter change for smooth automation.
///
/// AU describes a ramp by its end value and duration. The ramp is applied as a
/// value change at `sample_offset` (sample-accurately, by sub-block processing)
/// and the parameter's own smoother interpolates towards it. The format wrappers
/// share this approach: the host chooses when a change starts and the plugin
/// chooses how it smooths.
#[derive(Clone, Debug)]
pub struct AuParameterRampEvent {
    /// Sample offset where ramp starts.
    pub sample_offset: u32,
    /// AU parameter address (maps to beamer parameter ID)
    pub parameter_address: u64,
    /// Value at the end of the ramp, in AU format like `AuParameterEvent::value`.
    pub end_value: f32,
    /// Duration of the ramp in samples.
    ///
    /// Not used: the parameter smoothers have a fixed time constant that is
    /// configured at parameter construction.
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
/// All events are linked via the `next` pointer. Like the other event structs
/// it mirrors the layout of the AudioToolbox declaration, which is packed to
/// 4 bytes.
#[repr(C, packed(4))]
#[derive(Debug, Clone, Copy)]
pub struct AURenderEventHeader {
    /// Pointer to next event in linked list (null if last)
    pub next: *const AURenderEvent,
    /// Sample time at which the event is scheduled, on the timeline of the
    /// render timestamp
    pub event_sample_time: i64,
    /// Event type discriminator
    pub event_type: u8,
    /// Reserved, must be 0
    pub reserved: u8,
}

/// Parameter change event, immediate or ramped.
///
/// The event type tells the two apart: `AURenderEventType::Parameter` or
/// `AURenderEventType::ParameterRamp`.
#[repr(C, packed(4))]
#[derive(Debug, Clone, Copy)]
pub struct AUParameterEvent {
    /// Pointer to next event
    pub next: *const AURenderEvent,
    /// Sample time at which the event is scheduled
    pub event_sample_time: i64,
    /// Event type (Parameter or ParameterRamp)
    pub event_type: u8,
    /// Reserved
    pub reserved: [u8; 3],
    /// Ramp duration in sample frames, 0 for an immediate change
    pub ramp_duration_sample_frames: u32,
    /// Parameter address (u64)
    pub parameter_address: u64,
    /// New value, or the value at the end of the ramp (f32)
    pub value: f32,
}

/// Legacy MIDI 1.0 event.
///
/// Contains standard MIDI bytes (status, data1, data2).
#[repr(C, packed(4))]
#[derive(Debug, Clone, Copy)]
pub struct AUMIDIEvent {
    /// Pointer to next event
    pub next: *const AURenderEvent,
    /// Sample time at which the event is scheduled
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
#[repr(C, packed(4))]
pub struct AUMIDIEventList {
    /// Pointer to next event
    pub next: *const AURenderEvent,
    /// Sample time at which the event is scheduled
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
#[repr(C, packed(4))]
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
    /// Parameter change or ramp event
    pub parameter: AUParameterEvent,
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
    // We initialize all fields before returning and Box::from_raw takes ownership.
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

/// Convert the sample time of a render event to an offset in the current buffer.
///
/// AU render events carry an absolute sample time on the timeline of the
/// render timestamp, so the start of the buffer is subtracted. Events that
/// fall outside the buffer (late or early) move to its nearest sample. The
/// AUAudioUnit base class has already translated "immediate" times.
fn buffer_offset(event_sample_time: i64, buffer_start: i64, num_samples: usize) -> u32 {
    let last = num_samples.saturating_sub(1) as i64;
    let offset = event_sample_time.saturating_sub(buffer_start);
    offset.clamp(0, last) as u32
}

/// Extract MIDI events from AU render event linked list.
///
/// Iterates through the event list and converts MIDI events to beamer format.
/// Handles both legacy MIDI 1.0 events and MIDI 2.0 UMP events. The sample
/// offsets of the extracted events are relative to `buffer_start`, the sample
/// time of the render timestamp.
///
/// # Safety
/// The `event_list` pointer must be valid or null.
pub unsafe fn extract_midi_events(
    event_list: *const AURenderEvent,
    buffer: &mut MidiBuffer,
    buffer_start: i64,
    num_samples: usize,
) {
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
                    let sample_offset =
                        buffer_offset(midi_event.event_sample_time, buffer_start, num_samples);
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
                let sample_offset = buffer_offset(
                    unsafe { event.head.event_sample_time },
                    buffer_start,
                    num_samples,
                );
                // Get pointer to MIDIEventList (immediately after AUMIDIEventList header,
                // which is 20 bytes in the packed layout)
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
/// Scans the MIDI buffer for CC, pitch bend and channel pressure events,
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
// MIDI Output
// =============================================================================

/// Function that receives one MIDI message the plugin produced (matches
/// BeamerAuMidiOutputFn in header).
///
/// `bytes` holds one complete MIDI 1.0 message of `length` bytes that is only
/// valid during the call. `sample_offset` is its position in the current buffer.
pub type BeamerAuMidiOutputFn =
    unsafe extern "C" fn(context: *mut c_void, sample_offset: u32, bytes: *const u8, length: u32);

/// Where the MIDI output of a render call goes.
///
/// The wrapper passes it to `beamer_au_render` when the host listens to the
/// plugin's MIDI output, and forwards each message to the host in the form its
/// API generation expects.
#[derive(Clone, Copy)]
pub struct MidiOutputSink {
    send: BeamerAuMidiOutputFn,
    context: *mut c_void,
}

impl MidiOutputSink {
    /// Create a sink from the function and context a wrapper passed.
    ///
    /// # Safety
    ///
    /// `send` must be safe to call with `context` from the render thread for as
    /// long as the sink is used.
    pub unsafe fn new(send: BeamerAuMidiOutputFn, context: *mut c_void) -> Self {
        Self { send, context }
    }

    fn send(&self, sample_offset: u32, bytes: &[u8]) {
        // SAFETY: Guaranteed by the caller of `new`. The slice is valid for the call.
        unsafe {
            (self.send)(
                self.context,
                sample_offset,
                bytes.as_ptr(),
                bytes.len() as u32,
            )
        }
    }
}

/// Convert a normalized value to a 7-bit MIDI data byte.
///
/// Rounds, so that a value that came in as a data byte goes out as the same byte.
fn to_data_byte(value: f32) -> u8 {
    (value * 127.0).round().clamp(0.0, 127.0) as u8
}

/// Encode a MIDI event as a MIDI 1.0 channel message.
///
/// Returns the bytes and how many of them are used (2 or 3). Returns `None`
/// for SysEx, which `send_midi_output` handles, and for events that have no
/// MIDI 1.0 encoding (note expression, chord and scale info).
fn encode_midi_event(event: &MidiEventKind) -> Option<([u8; 3], usize)> {
    match event {
        MidiEventKind::NoteOn(note) => Some((
            [
                0x90 | (note.channel & 0x0F),
                note.pitch & 0x7F,
                // A Note On with velocity 0 means Note Off on the wire.
                to_data_byte(note.velocity).max(1),
            ],
            3,
        )),
        MidiEventKind::NoteOff(note) => Some((
            [
                0x80 | (note.channel & 0x0F),
                note.pitch & 0x7F,
                to_data_byte(note.velocity),
            ],
            3,
        )),
        MidiEventKind::PolyPressure(pp) => Some((
            [
                0xA0 | (pp.channel & 0x0F),
                pp.pitch & 0x7F,
                to_data_byte(pp.pressure),
            ],
            3,
        )),
        MidiEventKind::ControlChange(cc) => Some((
            [
                0xB0 | (cc.channel & 0x0F),
                cc.controller & 0x7F,
                to_data_byte(cc.value),
            ],
            3,
        )),
        MidiEventKind::ProgramChange(pc) => {
            Some(([0xC0 | (pc.channel & 0x0F), pc.program & 0x7F, 0], 2))
        }
        MidiEventKind::ChannelPressure(cp) => Some((
            [0xD0 | (cp.channel & 0x0F), to_data_byte(cp.pressure), 0],
            2,
        )),
        MidiEventKind::PitchBend(pb) => {
            // -1.0..1.0 maps to the 14-bit range 0..16383 with the center at 8192.
            let raw = (pb.value * 8192.0 + 8192.0).round().clamp(0.0, 16383.0) as u16;
            let lsb = (raw & 0x7F) as u8;
            let msb = (raw >> 7) as u8;
            Some(([0xE0 | (pb.channel & 0x0F), lsb, msb], 3))
        }
        MidiEventKind::SysEx(_)
        | MidiEventKind::NoteExpressionValue(_)
        | MidiEventKind::NoteExpressionInt(_)
        | MidiEventKind::NoteExpressionText(_)
        | MidiEventKind::ChordInfo(_)
        | MidiEventKind::ScaleInfo(_) => None,
    }
}

/// Send the MIDI events a plugin produced to the host.
///
/// Every event becomes one complete MIDI 1.0 message. Events without a MIDI 1.0
/// encoding are skipped.
fn send_midi_output(events: &[MidiEvent], sink: MidiOutputSink) {
    for event in events {
        match &event.event {
            MidiEventKind::SysEx(sysex) => {
                // The event holds the payload. On the wire the message is framed
                // by F0 and F7, which are added unless the payload has them.
                let data = sysex.as_slice();
                let mut message = [0u8; MAX_SYSEX_SIZE + 2];
                let mut len = 0;
                if data.first() != Some(&0xF0) {
                    message[len] = 0xF0;
                    len += 1;
                }
                message[len..len + data.len()].copy_from_slice(data);
                len += data.len();
                if data.last() != Some(&0xF7) {
                    message[len] = 0xF7;
                    len += 1;
                }
                sink.send(event.sample_offset, &message[..len]);
            }
            other => {
                if let Some((bytes, len)) = encode_midi_event(other) {
                    sink.send(event.sample_offset, &bytes[..len]);
                }
            }
        }
    }
}

// =============================================================================
// Parameter Event Extraction
// =============================================================================

/// Extract parameter events from AU render event linked list.
///
/// Iterates through the event list and extracts parameter change and ramp events.
/// MIDI and other event types are ignored by this function. The sample offsets
/// of the extracted events are relative to `buffer_start`, the sample time of
/// the render timestamp.
///
/// # Safety
/// The `event_list` pointer must be valid or null.
pub unsafe fn extract_parameter_events(
    event_list: *const AURenderEvent,
    buffer: &mut ParameterEventBuffer,
    buffer_start: i64,
    num_samples: usize,
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
                    sample_offset: buffer_offset(
                        param_event.event_sample_time,
                        buffer_start,
                        num_samples,
                    ),
                    parameter_address: param_event.parameter_address,
                    value: param_event.value,
                });
            }
            // AU_RENDER_EVENT_PARAMETER_RAMP (type 2)
            2 => {
                // SAFETY: event_type == 2 (ParameterRamp), so parameter field is the active variant.
                let ramp_event = unsafe { &event.parameter };
                buffer.ramps.push(AuParameterRampEvent {
                    sample_offset: buffer_offset(
                        ramp_event.event_sample_time,
                        buffer_start,
                        num_samples,
                    ),
                    parameter_address: ramp_event.parameter_address,
                    end_value: ramp_event.value,
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
/// The `process()` signature follows the arguments of an AU render call, plus
/// what the wrapper collected from the host for it.
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
    /// * `host_transport` - Transport and musical context from the host, if any
    /// * `midi_output` - Receiver of the plugin's MIDI output, if the host listens
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
        host_transport: Option<&BeamerAuTransport>,
        midi_output: Option<MidiOutputSink>,
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
    /// Pre-allocated cache for auxiliary output bus data.
    ///
    /// AU hosts call the render block once per output bus. On bus 0, the plugin
    /// processes all audio and aux outputs are written here. On bus N>0, cached
    /// data is copied to the host's output buffers.
    /// Layout: `[aux_bus_index][channel_index][sample_data]`
    aux_output_cache: UnsafeCell<Vec<Vec<Vec<S>>>>,
    /// Sample time of the last full render (bus 0), used to detect new render cycles.
    /// Initialized to NaN so it never matches on the first call.
    last_render_sample_time: UnsafeCell<f64>,
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
    /// * `max_frames` - Maximum frames per render call
    /// * `sample_rate` - Current sample rate in Hz
    pub fn new(
        plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
        storage: ProcessBufferStorage<S>,
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

        // Pre-allocate cache for auxiliary output buses (render-once, copy-on-aux-bus pattern).
        // AU hosts call the render block once per output bus. On bus 0 we render everything
        // and cache aux output data here. On bus N>0 we copy from cache to host buffers.
        let aux_output_bus_count = storage.aux_output_bus_count();
        let mut aux_output_cache = Vec::with_capacity(aux_output_bus_count);
        for bus_idx in 0..aux_output_bus_count {
            let channels = storage.aux_output_capacity(bus_idx);
            let mut bus_cache = Vec::with_capacity(channels);
            for _ in 0..channels {
                bus_cache.push(vec![S::ZERO; max_frames as usize]);
            }
            aux_output_cache.push(bus_cache);
        }
        Self {
            plugin,
            storage: UnsafeCell::new(storage),
            midi_buffer: UnsafeCell::new(MidiBuffer::with_capacity(1024)),
            parameter_events: UnsafeCell::new(ParameterEventBuffer::new()),
            sample_rate,
            aux_input_buffer_lists: UnsafeCell::new(aux_input_buffer_lists),
            midi_output: UnsafeCell::new(MidiBuffer::with_capacity(1024)),
            aux_output_cache: UnsafeCell::new(aux_output_cache),
            last_render_sample_time: UnsafeCell::new(f64::NAN),
        }
    }

    /// Copy cached auxiliary output data to the host's output AudioBufferList.
    ///
    /// Called when the AU host requests an auxiliary output bus (bus N>0) that was
    /// already rendered and cached during the bus 0 render call.
    ///
    /// # Safety
    ///
    /// - `output_data` must be a valid, non-null AudioBufferList pointer
    /// - `num_samples` must not exceed the cache buffer capacity
    unsafe fn copy_aux_cache_to_output(
        &self,
        aux_idx: usize,
        output_data: *mut AudioBufferList,
        num_samples: usize,
    ) {
        // SAFETY: AU hosts call render sequentially per instance, always completing
        // bus 0 before calling bus N>0. Cache is written during bus 0 and only read here.
        let cache = unsafe { &*self.aux_output_cache.get() };
        if aux_idx >= cache.len() {
            // Out-of-bounds aux bus requested - zero the output to avoid stale data.
            // SAFETY: Caller guarantees output_data is a valid, non-null pointer.
            unsafe { Self::zero_output(output_data, num_samples) };
            return;
        }

        let bus_cache = &cache[aux_idx];
        debug_assert!(
            bus_cache.first().is_none_or(|ch| num_samples <= ch.len()),
            "num_samples ({num_samples}) exceeds cache capacity"
        );
        // SAFETY: Caller guarantees output_data is a valid, non-null pointer.
        let list = unsafe { &mut *output_data };

        for ch in 0..list.number_buffers.min(bus_cache.len() as u32) {
            // SAFETY: ch is within number_buffers bounds.
            let buffer = unsafe { list.buffer_at_mut(ch) };
            if buffer.data.is_null() || buffer.number_channels != 1 {
                continue;
            }
            let available = buffer.data_byte_size as usize / std::mem::size_of::<S>();
            let copy_count = num_samples.min(available);
            let dst = buffer.data as *mut S;
            let src = bus_cache[ch as usize].as_ptr();
            // SAFETY: dst points to host buffer valid for `available` elements.
            // src points to pre-allocated cache with capacity >= max_frames >= num_samples.
            // We copy min(num_samples, available) to avoid out-of-bounds writes.
            unsafe {
                std::ptr::copy_nonoverlapping(src, dst, copy_count);
            }
        }
    }

    /// Zero the host's output AudioBufferList (for edge case where aux bus
    /// is requested before the main bus has been rendered).
    ///
    /// # Safety
    ///
    /// - `output_data` must be a valid, non-null AudioBufferList pointer
    unsafe fn zero_output(output_data: *mut AudioBufferList, num_samples: usize) {
        // SAFETY: Caller guarantees output_data is a valid, non-null pointer.
        let list = unsafe { &mut *output_data };
        for i in 0..list.number_buffers {
            // SAFETY: i is within number_buffers bounds.
            let buffer = unsafe { list.buffer_at_mut(i) };
            if !buffer.data.is_null() && buffer.data_byte_size > 0 {
                let byte_count = num_samples * std::mem::size_of::<S>();
                // SAFETY: buffer.data is non-null (checked above) and we clamp to
                // data_byte_size to avoid out-of-bounds access.
                let bytes = unsafe {
                    std::slice::from_raw_parts_mut(
                        buffer.data as *mut u8,
                        byte_count.min(buffer.data_byte_size as usize),
                    )
                };
                bytes.fill(0);
            }
        }
    }

    /// Extract the sample time from an AudioTimeStamp pointer.
    ///
    /// # Safety
    ///
    /// - `timestamp` must be a valid, non-null AudioTimeStamp pointer
    ///   (bridge.rs validates non-null before calling process_impl).
    unsafe fn sample_time_from(timestamp: *const AudioTimeStamp) -> f64 {
        // SAFETY: bridge.rs rejects null timestamps before reaching process_impl.
        unsafe { (*timestamp).sample_time }
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
        output_bus_number: i32,
        output_data: *mut AudioBufferList,
        event_list: *const AURenderEvent,
        pull_input_block: *const c_void,
        input_data: *const AudioBufferList,
        host_transport: Option<&BeamerAuTransport>,
        midi_output_sink: Option<MidiOutputSink>,
    ) -> i32 {
        let num_samples = frame_count as usize;

        // AU multi-output bus handling: the host calls render once per output bus.
        // Bus 0 triggers a full render; bus N>0 copies cached aux output data.
        if output_bus_number > 0 {
            // SAFETY: AU hosts call render sequentially per instance, always
            // completing bus 0 before calling bus N>0.
            let last_time = unsafe { *self.last_render_sample_time.get() };
            // SAFETY: bridge.rs validates timestamp non-null before calling process_impl.
            let current_time = unsafe { Self::sample_time_from(timestamp) };

            let aux_idx = (output_bus_number - 1) as usize;

            // to_bits(): bitwise comparison so NaN == NaN works correctly,
            // ensuring the NaN-initialized last_render_sample_time never
            // matches a real timestamp on the first call.
            if current_time.to_bits() == last_time.to_bits() {
                // Same render cycle - copy cached data to host's output buffers
                // SAFETY: output_data validated non-null by caller (bridge.rs).
                // aux_idx bounds checked inside copy_aux_cache_to_output.
                unsafe {
                    self.copy_aux_cache_to_output(aux_idx, output_data, num_samples);
                }
            } else {
                // Aux bus requested before bus 0 was rendered (edge case) - output silence
                // SAFETY: output_data validated non-null by caller.
                unsafe {
                    Self::zero_output(output_data, num_samples);
                }
            }
            return os_status::NO_ERR;
        }

        // Bus 0: full render path
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

        // Use pre-allocated storage instead of Vec allocations
        // SAFETY: We have exclusive access via &self and AU guarantees
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

        // AU's eventSampleTime is an absolute sample position on the timeline of
        // the render timestamp. The extraction below turns it into an offset in
        // this buffer.
        // SAFETY: bridge.rs validates timestamp non-null before calling process_impl.
        let buffer_start_sample = unsafe { (*timestamp).sample_time as i64 };

        // Extract MIDI events from the AU render event linked list
        // SAFETY: event_list is valid for this render call (provided by AU host)
        unsafe {
            extract_midi_events(event_list, midi_buffer, buffer_start_sample, num_samples);
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

        // Hand the plugin's MIDI output to the host. Without a sink the host is
        // not listening and the events are discarded.
        if let Some(sink) = midi_output_sink {
            send_midi_output(midi_output.as_slice(), sink);
        }
        if midi_output.has_overflowed() {
            log::warn!(
                "MIDI output buffer overflow: {} events reached capacity, some events were dropped",
                midi_output.len()
            );
        }

        // Clear and extract parameter events from AU event list
        // SAFETY: Same reasoning as storage - single-threaded render calls
        let parameter_events = unsafe { &mut *self.parameter_events.get() };
        parameter_events.clear();

        // Extract parameter events from the AU render event linked list
        // SAFETY: event_list is valid for this render call (provided by AU host)
        unsafe {
            extract_parameter_events(
                event_list,
                parameter_events,
                buffer_start_sample,
                num_samples,
            );
        }

        // Sort by sample offset to enable sample-accurate application via sub-block processing.
        parameter_events.immediate.sort_by_key(|e| e.sample_offset);
        parameter_events.ramps.sort_by_key(|e| e.sample_offset);

        // Transport info from the AU host, with the render clock as fallback
        // for the position.
        let transport = transport_from_host(host_transport, buffer_start_sample);

        // Collect pointers from AudioBufferList
        // SAFETY: output_data is valid for the duration of this render call
        unsafe {
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

            // Now collect output pointers (after in-place fixup)
            storage.collect_outputs(output_data, num_samples);

            if input_data.is_null() {
                // No main input: the plugin has no input bus or the host did not
                // feed it. Start from silence, so that a processor that only
                // transforms its input does not leave whatever was in the
                // buffers (the host's, or our own for null output pointers) in
                // the output.
                for &channel in &storage.main_outputs {
                    // SAFETY: collect_outputs only keeps pointers that are valid
                    // for num_samples samples.
                    std::ptr::write_bytes(channel, 0, num_samples);
                }
            }

            // Collect auxiliary output pointers from pre-allocated cache buffers.
            // AU calls render per-bus, so we render aux outputs into our cache on
            // bus 0 and copy them out when the host requests bus N>0.
            // Note: storage.clear() above resets aux_outputs, so push() here repopulates
            // from a clean state each render cycle (no unbounded growth).
            let aux_cache = &mut *self.aux_output_cache.get();
            debug_assert_eq!(aux_cache.len(), storage.aux_outputs.len());
            for (bus_cache, aux_output) in
                aux_cache.iter_mut().zip(storage.aux_outputs.iter_mut())
            {
                for ch_buf in bus_cache.iter_mut() {
                    // Zero the cache before rendering into it
                    ch_buf[..num_samples].fill(S::ZERO);
                    aux_output.push(ch_buf.as_mut_ptr());
                }
            }

            // Note: With a main input we do NOT zero output buffers here because:
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
                // No inputs and no outputs collected: there is nothing to write to.
                return os_status::NO_ERR;
            }
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
        // MidiCcState uses atomics internally and we only read it.
        let cc_state_ptr: Option<*const beamer_core::MidiCcState> =
            plugin_guard.midi_cc_state().map(|cc| cc as *const _);

        // Sample-accurate automation via sub-block processing.
        // We split at parameter event boundaries and apply changes exactly at the start of each sub-block.
        let mut result = os_status::NO_ERR;

        let immediate = &parameter_events.immediate;
        let ramps = &parameter_events.ramps;
        let midi_events_all = midi_buffer.as_mut_slice();

        let mut imm_idx: usize = 0;
        let mut ramp_idx: usize = 0;
        let mut midi_idx: usize = 0;

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
            // Events are sorted by sample_offset and each belongs to exactly one sub-block,
            // so we rebase offsets in-place and restore after processing.
            // This is safe because the plugin receives &[MidiEvent] (immutable). If the
            // signature ever changes to &mut, the restore step would need revisiting.
            while midi_idx < midi_events_all.len()
                && (midi_events_all[midi_idx].sample_offset as usize) < block_start
            {
                midi_idx += 1;
            }
            let midi_end = {
                let mut scan = midi_idx;
                while scan < midi_events_all.len()
                    && (midi_events_all[scan].sample_offset as usize) < (block_start + block_len)
                {
                    scan += 1;
                }
                scan
            };

            // Rebase offsets in-place to be relative to sub-block start.
            let block_start_u32 = block_start as u32;
            for ev in &mut midi_events_all[midi_idx..midi_end] {
                ev.sample_offset = ev.sample_offset.saturating_sub(block_start_u32);
            }

            // Adjust transport sample positions for this sub-block.
            let mut seg_transport = transport;
            if let Some(t) = seg_transport.project_time_samples {
                seg_transport.project_time_samples = Some(t + block_start as i64);
            }
            if let Some(t) = seg_transport.continuous_time_samples {
                seg_transport.continuous_time_samples = Some(t + block_start as i64);
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
                &midi_events_all[midi_idx..midi_end],
                &context,
            );

            // Restore original offsets after processing. This runs before the
            // early-exit check below, so offsets are always restored even on error.
            for ev in &mut midi_events_all[midi_idx..midi_end] {
                ev.sample_offset += block_start_u32;
            }
            midi_idx = midi_end;

            if block_status != os_status::NO_ERR {
                result = block_status;
                break;
            }

            block_start = next_boundary;
        }

        // Record the sample time for this render cycle so that subsequent
        // aux bus calls (output_bus_number > 0) can detect they belong to
        // the same cycle and copy cached data instead of re-rendering.
        // SAFETY: AU hosts call render sequentially per instance, always
        // completing bus 0 before calling bus N>0. Written here at the end of
        // bus 0, read at the top of bus N>0.
        // bridge.rs validates timestamp non-null before calling process_impl.
        unsafe {
            *self.last_render_sample_time.get() = Self::sample_time_from(timestamp);
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
    ///    - The `RenderBlock<S>` is created with the same S used by the host's format
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
    ///    - `RenderBlock<S>` is created based on AU format (kAudioFormatFlagIsFloat + bits per channel)
    ///    - create_render_block_f32/f64 ensure S matches the actual host format
    ///    - TypeId check is a runtime guarantee that S == f32 or S == f64
    ///    - Buffer slices have the same memory layout regardless of Sample type
    ///      (both are just `&[T]` where T is a 32-bit or 64-bit float)
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
    /// - `Vec<&[S]>` and `Vec<&[f32]>` have identical memory layout
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
        host_transport: Option<&BeamerAuTransport>,
        midi_output: Option<MidiOutputSink>,
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
            host_transport,
            midi_output,
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
/// * `max_frames` - Maximum frames per render call
/// * `sample_rate` - Current sample rate in Hz
pub fn create_render_block_f32(
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
    storage: ProcessBufferStorage<f32>,
    max_frames: u32,
    sample_rate: f64,
) -> Box<dyn RenderBlockTrait> {
    Box::new(RenderBlock::<f32>::new(
        plugin,
        storage,
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
/// * `max_frames` - Maximum frames per render call
/// * `sample_rate` - Current sample rate in Hz
pub fn create_render_block_f64(
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
    storage: ProcessBufferStorage<f64>,
    max_frames: u32,
    sample_rate: f64,
) -> Box<dyn RenderBlockTrait> {
    Box::new(RenderBlock::<f64>::new(
        plugin,
        storage,
        max_frames,
        sample_rate,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    /// Sample time of the buffer start used by the tests. It is larger than
    /// `u32::MAX` so that a truncated time cannot pass.
    const START: i64 = 5_000_000_000;

    fn parameter_event(event_type: u8, time: i64, value: f32, ramp: u32) -> AURenderEvent {
        AURenderEvent {
            parameter: AUParameterEvent {
                next: std::ptr::null(),
                event_sample_time: time,
                event_type,
                reserved: [0; 3],
                ramp_duration_sample_frames: ramp,
                parameter_address: 7,
                value,
            },
        }
    }

    fn midi_event(time: i64, data: [u8; 3]) -> AURenderEvent {
        AURenderEvent {
            midi: AUMIDIEvent {
                next: std::ptr::null(),
                event_sample_time: time,
                event_type: AURenderEventType::Midi as u8,
                reserved: 0,
                length: 3,
                cable: 0,
                data,
            },
        }
    }

    /// Link the events into the list form the host passes to render.
    fn link(events: &mut [AURenderEvent]) -> *const AURenderEvent {
        for i in 1..events.len() {
            let next: *const AURenderEvent = &events[i];
            events[i - 1].head.next = next;
        }
        events.as_ptr()
    }

    #[test]
    fn test_event_layout_matches_audio_toolbox() {
        // The AudioToolbox declarations are packed to 4 bytes.
        assert_eq!(size_of::<AURenderEventHeader>(), 20);
        assert_eq!(offset_of!(AURenderEventHeader, event_type), 16);

        assert_eq!(size_of::<AUParameterEvent>(), 36);
        assert_eq!(
            offset_of!(AUParameterEvent, ramp_duration_sample_frames),
            20
        );
        assert_eq!(offset_of!(AUParameterEvent, parameter_address), 24);
        assert_eq!(offset_of!(AUParameterEvent, value), 32);

        assert_eq!(size_of::<AUMIDIEvent>(), 24);
        assert_eq!(offset_of!(AUMIDIEvent, length), 18);
        assert_eq!(offset_of!(AUMIDIEvent, cable), 20);
        assert_eq!(offset_of!(AUMIDIEvent, data), 21);

        // The MIDIEventList starts right after this header.
        assert_eq!(size_of::<AUMIDIEventList>(), 20);
        assert_eq!(offset_of!(AUMIDIEventList, cable), 18);

        assert_eq!(size_of::<MIDIEventPacket>(), 12);
        assert_eq!(offset_of!(MIDIEventPacket, word_count), 8);

        assert_eq!(std::mem::align_of::<AURenderEvent>(), 4);
    }

    #[test]
    fn test_midi_event_list_is_read_at_its_packed_offset() {
        // An AUMIDIEventList header (20 bytes), a MIDIEventList header (8 bytes)
        // and one packet (12 bytes) with a single word: Note On 60, velocity 100,
        // as a MIDI 1.0 channel voice message.
        let mut bytes = [0u8; 48];
        bytes[8..16].copy_from_slice(&(START + 40).to_ne_bytes());
        bytes[16] = AURenderEventType::MidiEventList as u8;
        bytes[20..24].copy_from_slice(&1u32.to_ne_bytes());
        bytes[24..28].copy_from_slice(&1u32.to_ne_bytes());
        bytes[36..40].copy_from_slice(&1u32.to_ne_bytes());
        bytes[40..44].copy_from_slice(&0x2090_3C64u32.to_ne_bytes());

        // Copy into storage with the alignment of a real event.
        let mut storage = [0u64; 6];
        // SAFETY: Both regions are 48 bytes and do not overlap.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), storage.as_mut_ptr().cast::<u8>(), 48);
        }

        let mut buffer = MidiBuffer::with_capacity(16);
        // SAFETY: The storage holds one complete event with a null `next`.
        unsafe { extract_midi_events(storage.as_ptr().cast(), &mut buffer, START, 512) };

        let events = buffer.as_slice();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].sample_offset, 40);
        match &events[0].event {
            MidiEventKind::NoteOn(note) => assert_eq!(note.pitch, 60),
            _ => panic!("expected a Note On"),
        }
    }

    #[test]
    fn test_buffer_offset() {
        assert_eq!(buffer_offset(START, START, 512), 0);
        assert_eq!(buffer_offset(START + 300, START, 512), 300);
        // Late and early events move to the nearest sample of the buffer.
        assert_eq!(buffer_offset(START - 10, START, 512), 0);
        assert_eq!(buffer_offset(START + 512, START, 512), 511);
        assert_eq!(buffer_offset(i64::MIN, START, 512), 0);
        assert_eq!(buffer_offset(START, START, 0), 0);
    }

    #[test]
    fn test_parameter_events_are_relative_to_the_buffer() {
        let mut events = [
            parameter_event(AURenderEventType::Parameter as u8, START + 256, -6.0, 0),
            parameter_event(AURenderEventType::ParameterRamp as u8, START + 100, 3.0, 64),
            midi_event(START + 10, [0x90, 60, 100]),
        ];
        let list = link(&mut events);

        let mut buffer = ParameterEventBuffer::new();
        // SAFETY: The list is valid and ends with a null `next`.
        unsafe { extract_parameter_events(list, &mut buffer, START, 512) };

        assert_eq!(buffer.immediate.len(), 1);
        assert_eq!(buffer.immediate[0].sample_offset, 256);
        assert_eq!(buffer.immediate[0].parameter_address, 7);
        assert_eq!(buffer.immediate[0].value, -6.0);

        assert_eq!(buffer.ramps.len(), 1);
        assert_eq!(buffer.ramps[0].sample_offset, 100);
        assert_eq!(buffer.ramps[0].end_value, 3.0);
        assert_eq!(buffer.ramps[0].duration_samples, 64);
    }

    #[test]
    fn test_midi_events_are_relative_to_the_buffer() {
        let mut events = [
            midi_event(START + 300, [0x90, 60, 100]),
            parameter_event(AURenderEventType::Parameter as u8, START, 1.0, 0),
            midi_event(START + 511, [0x80, 60, 0]),
        ];
        let list = link(&mut events);

        let mut buffer = MidiBuffer::with_capacity(16);
        // SAFETY: The list is valid and ends with a null `next`.
        unsafe { extract_midi_events(list, &mut buffer, START, 512) };

        let events = buffer.as_slice();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sample_offset, 300);
        assert!(matches!(events[0].event, MidiEventKind::NoteOn(_)));
        assert_eq!(events[1].sample_offset, 511);
        assert!(matches!(events[1].event, MidiEventKind::NoteOff(_)));
    }

    /// A sink function that records what it receives.
    unsafe extern "C" fn record(
        context: *mut c_void,
        sample_offset: u32,
        bytes: *const u8,
        length: u32,
    ) {
        // SAFETY: `sent` passes a pointer to its vector, and the bytes are valid
        // for the call.
        let (received, bytes) = unsafe {
            (
                &mut *context.cast::<Vec<(u32, Vec<u8>)>>(),
                slice::from_raw_parts(bytes, length as usize),
            )
        };
        received.push((sample_offset, bytes.to_vec()));
    }

    /// Send the events through a sink and return the messages it received.
    fn sent(events: &[MidiEvent]) -> Vec<(u32, Vec<u8>)> {
        let mut received: Vec<(u32, Vec<u8>)> = Vec::new();
        let context: *mut Vec<(u32, Vec<u8>)> = &mut received;
        // SAFETY: `record` only uses the context while `received` is alive.
        let sink = unsafe { MidiOutputSink::new(record, context.cast()) };
        send_midi_output(events, sink);
        received
    }

    #[test]
    fn test_midi_output_matches_midi_input() {
        // Every channel message that can come in goes out as the same bytes.
        let messages: &[&[u8]] = &[
            &[0x90, 60, 100],
            &[0x93, 0, 1],
            &[0x80, 60, 64],
            &[0xA1, 61, 127],
            &[0xB2, 1, 33],
            &[0xC4, 5],
            &[0xD5, 99],
            &[0xE0, 0x00, 0x40],
            &[0xE6, 0x7F, 0x7F],
            &[0xE6, 0x00, 0x00],
        ];
        for (i, message) in messages.iter().enumerate() {
            let data1 = message[1];
            let data2 = message.get(2).copied().unwrap_or(0);
            let event = MidiEvent::from_midi1_bytes(
                i as u32,
                message[0] & 0xF0,
                message[0] & 0x0F,
                data1,
                data2,
            )
            .expect("channel message");
            assert_eq!(sent(&[event]), vec![(i as u32, message.to_vec())]);
        }

        // All velocities survive the round trip through the normalized value.
        for velocity in 1..=127u8 {
            let event = MidiEvent::from_midi1_bytes(0, 0x90, 0, 60, velocity).unwrap();
            assert_eq!(sent(&[event])[0].1, vec![0x90, 60, velocity]);
        }
    }

    #[test]
    fn test_midi_output_note_on_keeps_a_velocity() {
        // A velocity that rounds to 0 would turn the Note On into a Note Off.
        let event = MidiEvent::note_on(0, 0, 60, 0.001, 60, 0.0, 0);
        assert_eq!(sent(&[event])[0].1, vec![0x90, 60, 1]);
    }

    #[test]
    fn test_midi_output_frames_sysex() {
        let payload = MidiEvent::sysex(7, &[0x7E, 0x00, 0x06, 0x01]);
        let framed = MidiEvent::sysex(9, &[0xF0, 0x7E, 0x00, 0x06, 0x01, 0xF7]);
        let expected = vec![0xF0, 0x7E, 0x00, 0x06, 0x01, 0xF7];

        assert_eq!(
            sent(&[payload, framed]),
            vec![(7, expected.clone()), (9, expected)]
        );

        // A payload of the maximum size still fits with its framing.
        let largest = MidiEvent::sysex(0, &[0x01; MAX_SYSEX_SIZE]);
        assert_eq!(sent(&[largest])[0].1.len(), MAX_SYSEX_SIZE + 2);
    }

    #[test]
    fn test_midi_output_skips_events_without_midi_encoding() {
        let events = [
            MidiEvent::note_expression_value(0, 1, 0, 0.5),
            MidiEvent::from_midi1_bytes(3, 0x90, 0, 60, 100).unwrap(),
        ];
        assert_eq!(sent(&events), vec![(3, vec![0x90, 60, 100])]);
    }
}
