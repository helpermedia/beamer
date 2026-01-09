//! Render block creation for Audio Unit.
//!
//! This module provides the render block that AU hosts call for audio processing.
//! The render block is created once during `allocateRenderResources` and captures
//! a reference to the plugin instance.
//!
//! # Objective-C Block Handling
//!
//! Audio Unit v3 uses Objective-C blocks for callbacks. Blocks are similar to function
//! pointers but include a capture context and have a more complex memory layout.
//!
//! ## Current Approach: Direct Transmute
//!
//! This implementation uses `std::mem::transmute` to cast block pointers to function
//! pointers. This works because:
//! - Objective-C blocks have a function pointer at a known offset
//! - The first parameter is always the block pointer itself
//! - AU hosts guarantee block validity during render callbacks
//! - We never store blocks beyond the render callback
//!
//! ## Safety Considerations
//!
//! The transmute approach requires careful adherence to invariants:
//! 1. **Signature matching**: Function signatures must exactly match Apple's AU API
//! 2. **Lifetime**: Blocks are only valid during the render callback
//! 3. **Threading**: Blocks must only be called from the render thread
//! 4. **Host contract**: We rely on AU hosts following Apple's API contract
//!
//! ## Alternative: block2 Crate
//!
//! A safer approach would use the `block2` crate for proper Objective-C block handling.
//! This would eliminate transmutes but adds a dependency and complexity.
//!
//! ## Block Types Used
//!
//! 1. **AUHostMusicalContextBlock** (transport.rs): Query tempo, time signature, position
//! 2. **AUHostTransportStateBlock** (render.rs): Query play/stop/record state
//! 3. **AURenderPullInputBlock** (render.rs): Pull audio from auxiliary buses

use std::cell::UnsafeCell;
use std::ffi::{c_void, CStr};
use std::sync::{Arc, Mutex};

use block2::{ManualBlockEncoding, RcBlock};
use objc2::encode::{Encode, Encoding, RefEncode};

use crate::buffer_storage::ProcessBufferStorage;
use crate::buffers::{AudioBuffer, AudioBufferList};
use crate::bus_config::{MAX_BUSES, MAX_CHANNELS};
use crate::error::os_status;
use crate::instance::AuPluginInstance;
use crate::midi::MidiBuffer;
use crate::sysex_pool::SysExOutputPool;
use crate::transport::extract_transport_from_au;
use beamer_core::{
    ControlChange, MidiEvent, MidiEventKind, NoteOff, NoteOn, PitchBend, ProcessContext, Sample,
};

// =============================================================================
// Parameter Events
// =============================================================================

/// Immediate parameter value change from host automation.
///
/// # Field Usage
///
/// - `sample_offset`: **Currently unused**. The current implementation applies all
///   parameter changes at the start of the buffer rather than at the exact sample
///   position. This matches VST3 behavior and is acceptable because:
///   - Parameter smoothers interpolate across the buffer anyway
///   - True sample-accurate automation would require sub-block processing
///   - Most plugins don't require sub-sample precision for parameter changes
///
/// - `parameter_address`: Used to look up the target parameter by ID.
///
/// - `value`: Used to set the parameter's normalized value (0.0-1.0).
#[derive(Clone, Debug)]
pub struct AuParameterEvent {
    /// Sample offset within the current buffer.
    /// Note: Currently unused; events are applied at buffer start.
    /// See processor.rs apply_parameter_events() for design rationale.
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
/// - `sample_offset`: **Currently unused**. Ramps are applied at buffer start.
///   See `AuParameterEvent` documentation for rationale.
///
/// - `parameter_address`: Used to look up the target parameter by ID.
///
/// - `start_value`: **Currently unused**. The implementation sets the end value
///   directly, letting the parameter's configured smoother handle interpolation.
///
/// - `end_value`: Used to set the parameter's target normalized value.
///
/// - `duration_samples`: **Currently unused**. beamer_core's Smoother uses a fixed
///   time constant configured at parameter construction (via `SmoothingStyle`).
///   There is no API for dynamic per-event ramp duration configuration.
///   This matches VST3 behavior, which also doesn't use host-provided ramp info.
///
/// # Design Rationale
///
/// The current "set end value, let smoother interpolate" approach is intentional:
/// 1. **VST3 parity**: beamer-vst3 uses the same approach
/// 2. **Consistent behavior**: Plugin smoothers provide predictable transitions
/// 3. **Simplicity**: Avoids complex sub-block processing
///
/// For most musical parameters, the configured smoother time (e.g., 5ms exponential)
/// provides smooth transitions regardless of the DAW's intended ramp duration.
#[derive(Clone, Debug)]
pub struct AuParameterRampEvent {
    /// Sample offset where ramp starts.
    /// Note: Currently unused; see design rationale above.
    pub sample_offset: u32,
    /// AU parameter address (maps to beamer parameter ID)
    pub parameter_address: u64,
    /// Value at start of ramp.
    /// Note: Currently unused; we set end_value directly.
    pub start_value: f32,
    /// Value at end of ramp (used as target for parameter smoother)
    pub end_value: f32,
    /// Duration of ramp in samples.
    /// Note: Currently unused; smoother uses fixed time constant.
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
        let words_ptr = (self as *const Self as *const u8)
            .add(std::mem::size_of::<u64>() + std::mem::size_of::<u32>())
            as *const u32;
        std::slice::from_raw_parts(words_ptr, self.word_count as usize)
    }

    /// Get pointer to the next packet.
    ///
    /// # Safety
    /// Caller must ensure there is a valid next packet.
    #[inline]
    pub unsafe fn next(&self) -> *const MIDIEventPacket {
        let words_ptr = (self as *const Self as *const u8)
            .add(std::mem::size_of::<u64>() + std::mem::size_of::<u32>());
        words_ptr.add(self.word_count as usize * std::mem::size_of::<u32>())
            as *const MIDIEventPacket
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

// SAFETY: AURenderEvent is a C union with well-defined memory layout.
// We encode it as a union with the largest variant (ParameterRamp) for size.
// This is safe because:
// - The union is always passed as *const AURenderEvent (pointer)
// - objc2 only needs the encoding for type signature, not runtime validation
// - AU hosts provide these pointers, and we only read through the discriminator
unsafe impl Encode for AURenderEvent {
    const ENCODING: Encoding = Encoding::Union(
        "AURenderEvent",
        &[
            // Use a simple encoding - the actual union layout is complex but
            // for pointer types objc2 only needs to know it's a union
            Encoding::Struct(
                "AURenderEventParameterRamp",
                &[
                    Encoding::Pointer(&Encoding::Union("AURenderEvent", &[])), // next
                    Encoding::LongLong,                                        // event_sample_time
                    Encoding::Char,                                            // event_type
                    Encoding::Array(3, &Encoding::Char),                       // reserved
                    Encoding::ULongLong,                                       // parameter_address
                    Encoding::Float,                                           // value
                    Encoding::Float,                                           // end_value
                    Encoding::UInt,                                            // ramp_duration
                ],
            ),
        ],
    );
}

unsafe impl RefEncode for AURenderEvent {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
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

// SAFETY: SMPTETime has a well-defined C memory layout
unsafe impl Encode for SMPTETime {
    const ENCODING: Encoding = Encoding::Struct(
        "SMPTETime",
        &[
            Encoding::Short, // subframes
            Encoding::Short, // subframe_divisor
            Encoding::UInt,  // counter
            Encoding::UInt,  // smpte_type
            Encoding::UInt,  // flags
            Encoding::Short, // hours
            Encoding::Short, // minutes
            Encoding::Short, // seconds
            Encoding::Short, // frames
        ],
    );
}

unsafe impl RefEncode for SMPTETime {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
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

// SAFETY: AudioTimeStamp has a well-defined C memory layout
unsafe impl Encode for AudioTimeStamp {
    const ENCODING: Encoding = Encoding::Struct(
        "AudioTimeStamp",
        &[
            Encoding::Double,                // sample_time
            Encoding::ULongLong,             // host_time
            Encoding::Double,                // rate_scalar
            Encoding::ULongLong,             // word_clock_time
            <SMPTETime as Encode>::ENCODING, // smpte_time
            Encoding::UInt,                  // flags
            Encoding::UInt,                  // reserved
        ],
    );
}

unsafe impl RefEncode for AudioTimeStamp {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

/// The function signature for AU render blocks.
///
/// This matches Apple's `AURenderBlock` typedef:
/// ```c
/// typedef AUAudioUnitStatus (^AURenderBlock)(
///     AudioUnitRenderActionFlags *actionFlags,
///     const AudioTimeStamp *timestamp,
///     AUAudioFrameCount frameCount,
///     NSInteger outputBusNumber,
///     AudioBufferList *outputData,
///     const AURenderEvent *realtimeEventListHead,
///     AURenderPullInputBlock pullInputBlock
/// );
/// ```
///
/// # Thread Safety
///
/// The actual thread safety comes from what the closure captures, not
/// from this type alias. Our `create_objc_render_block` captures an
/// `Arc<dyn RenderBlockTrait>` where `RenderBlockTrait: Send + Sync`,
/// so the resulting block is thread-safe for AU's multi-threaded usage.
pub type AuRenderBlockFn = dyn Fn(
    *mut u32,              // actionFlags
    *const AudioTimeStamp, // timestamp
    u32,                   // frameCount (AUAudioFrameCount)
    isize,                 // outputBusNumber (NSInteger)
    *mut AudioBufferList,  // outputData
    *const AURenderEvent,  // realtimeEventListHead
    *const c_void,         // pullInputBlock
) -> i32;

/// Manual block encoding for AURenderBlock.
///
/// Apple's Audio Unit framework requires blocks with proper type encoding metadata.
/// This struct provides the encoding via the `ManualBlockEncoding` trait, allowing
/// us to use `RcBlock::with_encoding` instead of plain `RcBlock::new`.
///
/// # Encoding Format
///
/// The encoding string follows Apple's `@encode` directive format:
/// - Return type (i = int32/OSStatus)
/// - Total frame size in bytes (64 on 64-bit)
/// - Each argument with type encoding and byte offset
///
/// For AURenderBlock on 64-bit:
/// - @?0  : block pointer (self) at offset 0
/// - ^I8  : *mut u32 (actionFlags) at offset 8
/// - ^{AudioTimeStamp}16 : *const AudioTimeStamp at offset 16
/// - I24  : u32 (frameCount) at offset 24
/// - q32  : isize/NSInteger (outputBusNumber) at offset 32
/// - ^{AudioBufferList}40 : *mut AudioBufferList at offset 40
/// - ^{AURenderEvent}48 : *const AURenderEvent at offset 48
/// - @?56 : block (pullInputBlock) at offset 56
pub struct AuRenderBlockEncoding;

// SAFETY: The encoding string matches Apple's AURenderBlock typedef exactly.
// The Arguments and Return types match the AuRenderBlockFn signature.
unsafe impl ManualBlockEncoding for AuRenderBlockEncoding {
    type Arguments = (
        *mut u32,
        *const AudioTimeStamp,
        u32,
        isize,
        *mut AudioBufferList,
        *const AURenderEvent,
        *const c_void,
    );
    type Return = i32;

    // Encoding for: i32 (^AURenderBlock)(*mut u32, *const AudioTimeStamp, u32, isize,
    //                                     *mut AudioBufferList, *const AURenderEvent, *const c_void)
    //
    // On 64-bit macOS (the only platform AU runs on):
    // - i      : return type (int32/OSStatus)
    // - 64     : total argument frame size
    // - @?0    : block self pointer at offset 0 (8 bytes)
    // - ^I8    : pointer to uint32 at offset 8 (8 bytes)
    // - ^{AudioTimeStamp=dQdQ{SMPTETime=ssIIIsssss}II}16 : pointer to struct at offset 16 (8 bytes)
    // - I24    : uint32 at offset 24 (4 bytes, padded to 8)
    // - q32    : int64 (NSInteger) at offset 32 (8 bytes)
    // - ^{AudioBufferList=I[1{AudioBuffer=II^v}]}40 : pointer to struct at offset 40 (8 bytes)
    // - ^{AURenderEvent=}48 : pointer to union at offset 48 (8 bytes)
    // - @?56   : block (pullInputBlock) at offset 56 (8 bytes)
    //
    // Using simplified struct names since the runtime only checks pointer types.
    const ENCODING_CSTR: &'static CStr =
        c"i64@?0^I8^{AudioTimeStamp}16I24q32^{AudioBufferList}40^{AURenderEvent}48@?56";
}

/// Audio Unit render action flags.
#[repr(u32)]
#[allow(dead_code)]
pub enum AudioUnitRenderActionFlags {
    PreRender = 1 << 2,
    PostRender = 1 << 3,
    OutputIsSilence = 1 << 4,
    OfflinePreflight = 1 << 5,
    OfflineRender = 1 << 6,
    OfflineComplete = 1 << 7,
    PostRenderError = 1 << 8,
    DoNotCheckRenderArgs = 1 << 9,
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
    action_flags: *mut u32,
    timestamp: *const AudioTimeStamp,
    frame_count: u32,
    input_bus_number: isize,
    input_data: *mut AudioBufferList,
) -> i32;

// =============================================================================
// AudioBufferList Allocation Helpers
// =============================================================================

/// Allocate an AudioBufferList with a fixed number of buffers.
///
/// This allocates memory for the flexible array member pattern used by AudioBufferList.
/// The returned Box owns the memory and will free it when dropped.
///
/// # Arguments
///
/// * `num_buffers` - Number of AudioBuffer entries to allocate
/// * `num_samples` - Number of samples per buffer (for size calculation)
/// * `sample_type_size` - Size of sample type in bytes (4 for f32, 8 for f64)
///
/// # Safety
///
/// The returned pointer is valid for the lifetime of the Box.
/// The caller must ensure proper synchronization when accessing the buffers.
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

/// Extract MIDI events from AU render event linked list.
///
/// Iterates through the event list and converts MIDI events to beamer format.
/// Handles both legacy MIDI 1.0 events and MIDI 2.0 UMP events.
///
/// # Safety
/// The `event_list` pointer must be valid or null.
pub unsafe fn extract_midi_events(event_list: *const AURenderEvent, buffer: &mut MidiBuffer) {
    let mut event_ptr = event_list;

    while !event_ptr.is_null() {
        let event = &*event_ptr;
        let event_type = event.head.event_type;

        match event_type {
            // Legacy MIDI 1.0 event
            8 => {
                // AURenderEventType::Midi
                let midi_event = &event.midi;
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
                        parse_midi1_to_beamer(sample_offset, status, channel, data1, data2)
                    {
                        buffer.push(beamer_event);
                    }
                }
            }
            // MIDI 2.0 UMP event list
            10 => {
                // AURenderEventType::MidiEventList
                let sample_offset = event.head.event_sample_time as u32;
                // Get pointer to MIDIEventList (immediately after AUMIDIEventList header)
                let event_list_ptr = (event_ptr as *const u8)
                    .add(std::mem::size_of::<AUMIDIEventList>())
                    as *const MIDIEventList;
                let midi_list = &*event_list_ptr;

                // Get first packet
                let mut packet_ptr = (midi_list as *const MIDIEventList as *const u8)
                    .add(std::mem::size_of::<u32>() * 2)
                    as *const MIDIEventPacket;

                for _ in 0..midi_list.num_packets {
                    let packet = &*packet_ptr;
                    let words = packet.words();

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
                                parse_midi1_to_beamer(sample_offset, status, channel, data1, data2)
                            {
                                buffer.push(beamer_event);
                            }
                        }
                    }

                    packet_ptr = packet.next();
                }
            }
            _ => {
                // Ignore parameter events and other types
            }
        }

        event_ptr = event.head.next;
    }
}

/// Parse MIDI 1.0 message bytes to beamer MidiEvent.
#[inline]
fn parse_midi1_to_beamer(
    sample_offset: u32,
    status: u8,
    channel: u8,
    data1: u8,
    data2: u8,
) -> Option<MidiEvent> {
    let kind = match status {
        0x80 => MidiEventKind::NoteOff(NoteOff {
            channel,
            pitch: data1,
            velocity: data2 as f32 / 127.0,
            note_id: -1,
            tuning: 0.0,
        }),
        0x90 => {
            if data2 == 0 {
                // Note On with velocity 0 = Note Off
                MidiEventKind::NoteOff(NoteOff {
                    channel,
                    pitch: data1,
                    velocity: 0.0,
                    note_id: -1,
                    tuning: 0.0,
                })
            } else {
                MidiEventKind::NoteOn(NoteOn {
                    channel,
                    pitch: data1,
                    velocity: data2 as f32 / 127.0,
                    note_id: -1,
                    tuning: 0.0,
                    length: 0,
                })
            }
        }
        0xA0 => MidiEventKind::PolyPressure(beamer_core::PolyPressure {
            channel,
            pitch: data1,
            pressure: data2 as f32 / 127.0,
            note_id: -1,
        }),
        0xB0 => MidiEventKind::ControlChange(ControlChange {
            channel,
            controller: data1,
            value: data2 as f32 / 127.0,
        }),
        0xC0 => MidiEventKind::ProgramChange(beamer_core::ProgramChange {
            channel,
            program: data1,
        }),
        0xD0 => MidiEventKind::ChannelPressure(beamer_core::ChannelPressure {
            channel,
            pressure: data1 as f32 / 127.0,
        }),
        0xE0 => {
            // Pitch bend: data1 = LSB, data2 = MSB
            let raw_value = ((data2 as u16) << 7) | (data1 as u16);
            let normalized = (raw_value as f32 - 8192.0) / 8192.0;
            MidiEventKind::PitchBend(PitchBend {
                channel,
                value: normalized,
            })
        }
        _ => return None,
    };

    Some(MidiEvent {
        sample_offset,
        event: kind,
    })
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
fn update_midi_cc_state(midi_buffer: &crate::midi::MidiBuffer, cc_state: &beamer_core::MidiCcState) {
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
    while !event_ptr.is_null() {
        let event = &*event_ptr;
        let event_type = event.head.event_type;

        match event_type {
            // AU_RENDER_EVENT_PARAMETER (type 1)
            1 => {
                let param_event = &event.parameter;
                buffer.immediate.push(AuParameterEvent {
                    sample_offset: param_event.event_sample_time as u32,
                    parameter_address: param_event.parameter_address,
                    value: param_event.value,
                });
            }
            // AU_RENDER_EVENT_PARAMETER_RAMP (type 2)
            2 => {
                let ramp_event = &event.ramp;
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

        event_ptr = event.head.next;
    }
}

// =============================================================================
// Render Block Trait
// =============================================================================

/// Type-erased trait for render blocks.
///
/// This trait allows storing different sample type render blocks
/// (f32 or f64) in the same type-erased container.
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
    fn process(
        &self,
        action_flags: *mut u32,
        timestamp: *const AudioTimeStamp,
        frame_count: u32,
        output_bus_number: i32,
        output_data: *mut AudioBufferList,
        event_list: *const AURenderEvent,
        pull_input_block: *const c_void,
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
    /// Maximum frames per render call
    /// Used during initialization to size aux input buffer lists
    #[allow(dead_code)]
    max_frames: u32,
    /// Current sample rate for ProcessContext
    sample_rate: f64,
    /// Pre-allocated AudioBufferList structures for pulling aux input buses
    /// One per aux input bus (bus 1, 2, 3, ...)
    ///
    /// Note: Vec<Box<AudioBufferList>> is intentional despite clippy warning.
    /// AudioBufferList uses flexible array member (FAM) pattern with variable size.
    /// Box maintains the custom allocation created by allocate_audio_buffer_list().
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
}

// SAFETY: The raw pointers are only used within a single render call
// where AU guarantees single-threaded access.
unsafe impl<S: Sample> Send for RenderBlock<S> {}
unsafe impl<S: Sample> Sync for RenderBlock<S> {}

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

        // Pre-allocate AudioBufferList for each aux input bus
        // This ensures zero allocation in the render path
        let mut aux_input_buffer_lists = Vec::with_capacity(aux_input_bus_count);
        let sample_type_size = std::mem::size_of::<S>();

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
            max_frames,
            sample_rate,
            aux_input_buffer_lists: UnsafeCell::new(aux_input_buffer_lists),
            midi_output: UnsafeCell::new(MidiBuffer::with_capacity(1024)),
            sysex_output_pool: UnsafeCell::new(SysExOutputPool::new()),
            schedule_midi_event_block,
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
            let block_fn: AUScheduleMIDIEventBlockFn = std::mem::transmute(block);
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
    fn output_all_midi_events(&self, midi_output: &MidiBuffer, sysex_pool: &SysExOutputPool) -> usize {
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
                MidiEventKind::NoteOn(note) => {
                    let bytes = [
                        0x90 | (note.channel & 0x0F),
                        note.pitch & 0x7F,
                        ((note.velocity * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
                    ];
                    if !self.output_midi_to_host(&bytes, sample_offset) {
                        dropped += 1;
                    }
                }
                MidiEventKind::NoteOff(note) => {
                    let bytes = [
                        0x80 | (note.channel & 0x0F),
                        note.pitch & 0x7F,
                        ((note.velocity * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
                    ];
                    if !self.output_midi_to_host(&bytes, sample_offset) {
                        dropped += 1;
                    }
                }
                MidiEventKind::ControlChange(cc) => {
                    let bytes = [
                        0xB0 | (cc.channel & 0x0F),
                        cc.controller & 0x7F,
                        ((cc.value * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
                    ];
                    if !self.output_midi_to_host(&bytes, sample_offset) {
                        dropped += 1;
                    }
                }
                MidiEventKind::PitchBend(pb) => {
                    // Convert -1.0..1.0 to 0..16383 (14-bit)
                    let raw = (((pb.value + 1.0) * 8192.0).clamp(0.0, 16383.0) as u16) & 0x3FFF;
                    let lsb = (raw & 0x7F) as u8;
                    let msb = ((raw >> 7) & 0x7F) as u8;
                    let bytes = [0xE0 | (pb.channel & 0x0F), lsb, msb];
                    if !self.output_midi_to_host(&bytes, sample_offset) {
                        dropped += 1;
                    }
                }
                MidiEventKind::PolyPressure(pp) => {
                    let bytes = [
                        0xA0 | (pp.channel & 0x0F),
                        pp.pitch & 0x7F,
                        ((pp.pressure * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
                    ];
                    if !self.output_midi_to_host(&bytes, sample_offset) {
                        dropped += 1;
                    }
                }
                MidiEventKind::ChannelPressure(cp) => {
                    let bytes = [
                        0xD0 | (cp.channel & 0x0F),
                        ((cp.pressure * 127.0).clamp(0.0, 127.0) as u8) & 0x7F,
                    ];
                    if !self.output_midi_to_host(&bytes, sample_offset) {
                        dropped += 1;
                    }
                }
                MidiEventKind::ProgramChange(pc) => {
                    let bytes = [0xC0 | (pc.channel & 0x0F), pc.program & 0x7F];
                    if !self.output_midi_to_host(&bytes, sample_offset) {
                        dropped += 1;
                    }
                }
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
                // The following event types don't have standard MIDI 1.0 wire encodings
                // and cannot be output via AU's scheduleMIDIEventBlock:
                // - NoteExpressionValue/Int/Text: MPE/MIDI 2.0 per-note expressions
                // - ChordInfo/ScaleInfo: DAW-specific metadata (not MIDI messages)
                MidiEventKind::NoteExpressionValue(_)
                | MidiEventKind::NoteExpressionInt(_)
                | MidiEventKind::NoteExpressionText(_)
                | MidiEventKind::ChordInfo(_)
                | MidiEventKind::ScaleInfo(_) => {
                    // These event types are not standard MIDI 1.0 messages and cannot
                    // be output via AU's scheduleMIDIEventBlock. They are internal
                    // beamer events for MPE/expression data and DAW metadata.
                    // Silently skip them (not counted as dropped since they're unsupported).
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
    #[allow(clippy::too_many_arguments)]
    fn process_impl(
        &self,
        _action_flags: *mut u32,
        _timestamp: *const AudioTimeStamp,
        frame_count: u32,
        _output_bus_number: i32,
        output_data: *mut AudioBufferList,
        event_list: *const AURenderEvent,
        _pull_input_block: *const c_void,
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
        let midi_output = unsafe { &mut *self.midi_output.get() };
        midi_output.clear();

        // Clear SysEx pool for new block
        let sysex_pool = unsafe { &mut *self.sysex_output_pool.get() };
        sysex_pool.clear();

        // Extract MIDI events from the AU render event linked list
        // SAFETY: event_list is valid for this render call (provided by AU host)
        unsafe {
            extract_midi_events(event_list, midi_buffer);
        }

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
                        *const c_void,  // Block pointer itself (Objective-C convention)
                        *mut u32,       // outTransportStateFlags (AUHostTransportStateFlags)
                        *mut f64,       // outCurrentSamplePosition
                        *mut f64,       // outCycleStartBeatPosition
                        *mut f64,       // outCycleEndBeatPosition
                    ) -> bool;          // Returns true if successful

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
                    let block_fn: TransportStateBlockFn = std::mem::transmute(block);

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

            let sample_position = if !_timestamp.is_null() {
                (*_timestamp).sample_time as i64
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
        // AU uses in-place processing - same buffer for input and output
        // SAFETY: output_data is valid for the duration of this render call
        unsafe {
            storage.collect_outputs(output_data, num_samples);
            storage.collect_inputs(output_data as *const AudioBufferList, num_samples);

            // Pull auxiliary bus inputs if available
            // SAFETY: pull_input_block is valid for this render call (provided by AU host)
            if !_pull_input_block.is_null() {
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
                    // 1. `_pull_input_block` must be a valid AURenderPullInputBlock provided by AU host
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
                    let pull_fn = std::mem::transmute::<*const c_void, AURenderPullInputBlock>(
                        _pull_input_block,
                    );

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
                            _action_flags,
                            _timestamp,
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
        }

        // Build slices from collected pointers
        // SAFETY: Pointers collected above are valid for this render call.
        // The storage methods ensure proper non-overlapping slice construction.
        let input_refs = unsafe { storage.input_slices(num_samples) };
        let mut output_refs = unsafe { storage.output_slices(num_samples) };
        let aux_input_refs = unsafe { storage.aux_input_slices(num_samples) };
        let mut aux_output_refs = unsafe { storage.aux_output_slices(num_samples) };

        // Apply parameter events from host automation
        // This updates parameter values before audio processing for sample-accurate automation
        // If parameter application fails, continue with audio processing anyway
        // (parameters may already be at correct values from previous calls)
        let _ = plugin_guard.apply_parameter_events(
            &parameter_events.immediate,
            &parameter_events.ramps,
        );

        // Build ProcessContext with transport and timing information
        // Include MIDI CC state if configured by the plugin
        //
        // SAFETY: We use a raw pointer to work around borrow checker limitations.
        // This is safe because:
        // 1. MidiCcState uses atomics internally (Sync), so concurrent read access is safe
        // 2. plugin_guard is locked for the entire render call, preventing deallocation
        // 3. The plugin won't be dropped or reallocated during this scope
        // 4. We never mutate the MidiCcState through the plugin_guard during process()
        let cc_state_ptr: Option<*const beamer_core::MidiCcState> =
            plugin_guard.midi_cc_state().map(|cc| cc as *const _);

        let context = if let Some(cc_ptr) = cc_state_ptr {
            let cc_state = unsafe { &*cc_ptr };
            ProcessContext::with_midi_cc(self.sample_rate, num_samples, transport, cc_state)
        } else {
            ProcessContext::new(self.sample_rate, num_samples, transport)
        };

        // Call plugin's process method with aux buses and MIDI (dispatches to f32 or f64)
        let result = self.call_plugin_process_with_midi(
            &mut plugin_guard,
            &input_refs,
            &mut output_refs,
            &aux_input_refs,
            &mut aux_output_refs,
            midi_buffer.as_slice(),
            &context,
        );

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

    /// Call the plugin's process method with the appropriate sample type.
    ///
    /// This method dispatches to either process_with_context (f32) or
    /// process_with_context_f64 (f64) based on the sample type S.
    ///
    /// NOTE: Currently unused in favor of call_plugin_process_with_midi,
    /// but kept for potential future use with plugins that don't need aux buses.
    ///
    /// # Safety Pattern: TypeId Check + Transmute
    ///
    /// This function uses a TypeId-based dispatch pattern with transmute to handle
    /// generic sample types. This pattern is necessary because:
    ///
    /// 1. **Why this pattern is needed:**
    ///    - The RenderBlock is generic over sample type S (f32 or f64)
    ///    - The plugin trait has separate methods for f32 and f64 (process_with_context vs process_with_context_f64)
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
    ///    - If S is neither f32 nor f64 → we return error (last else branch)
    ///    - If TypeId check fails but transmute happens anyway → undefined behavior (crash likely)
    ///    - If buffer layout doesn't match expected type → memory corruption
    ///    - If Sample trait is implemented for non-f32/f64 types → potential transmute mismatch
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
    #[inline]
    #[allow(dead_code)]
    fn call_plugin_process(
        &self,
        plugin: &mut Box<dyn AuPluginInstance>,
        inputs: &[&[S]],
        outputs: &mut [&mut [S]],
        context: &ProcessContext,
    ) -> i32 {
        // Dispatch based on sample type using TypeId check + transmute pattern
        if std::any::TypeId::of::<S>() == std::any::TypeId::of::<f32>() {
            // SAFETY: TypeId check guarantees S is f32.
            // Transmuting &[&[S]] to &[&[f32]] is safe because:
            // - The memory layout is identical (slice of slice pointers)
            // - The underlying audio data is already f32 (host-provided)
            // - We're only changing the type, not the data or layout
            let inputs_f32: &[&[f32]] = unsafe { std::mem::transmute(inputs) };
            let outputs_f32: &mut [&mut [f32]] = unsafe { std::mem::transmute(outputs) };
            match plugin.process_with_context(inputs_f32, outputs_f32, context) {
                Ok(()) => os_status::NO_ERR,
                Err(_) => os_status::K_AUDIO_UNIT_ERR_RENDER,
            }
        } else if std::any::TypeId::of::<S>() == std::any::TypeId::of::<f64>() {
            // SAFETY: TypeId check guarantees S is f64.
            // Transmuting &[&[S]] to &[&[f64]] is safe because:
            // - The memory layout is identical (slice of slice pointers)
            // - The underlying audio data is already f64 (host-provided)
            // - We're only changing the type, not the data or layout
            let inputs_f64: &[&[f64]] = unsafe { std::mem::transmute(inputs) };
            let outputs_f64: &mut [&mut [f64]] = unsafe { std::mem::transmute(outputs) };
            match plugin.process_with_context_f64(inputs_f64, outputs_f64, context) {
                Ok(()) => os_status::NO_ERR,
                Err(_) => os_status::K_AUDIO_UNIT_ERR_RENDER,
            }
        } else {
            // Should never happen - Sample trait is sealed to f32/f64 only
            // If this branch executes, it indicates a serious bug (new Sample impl without updating this code)
            os_status::K_AUDIO_UNIT_ERR_RENDER
        }
    }

    /// Call the plugin's process method with auxiliary buses and MIDI.
    ///
    /// This method dispatches to either process_with_midi (f32) or
    /// process_with_midi_f64 (f64) based on the sample type S.
    ///
    /// # Safety Pattern: TypeId Check + Transmute (with Auxiliary Buses)
    ///
    /// This function uses the same TypeId-based dispatch pattern as `call_plugin_process`,
    /// but handles more complex types including auxiliary bus arrays.
    ///
    /// **Key differences from call_plugin_process:**
    /// - Transmutes `&[Vec<&[S]>]` to `&[Vec<&[f32]>]` or `&[Vec<&[f64]>]`
    /// - Each aux bus is a Vec of channel slices
    /// - Memory layout: outer slice → Vec → inner slice → sample data
    ///
    /// **Why the aux bus transmute is safe:**
    /// - Vec<&[S]> and Vec<&[f32]> have identical memory layout
    /// - Vec stores pointer + length + capacity (no type-specific data)
    /// - The slice references point to host-provided audio buffers in the correct format
    /// - We only transmute the container types, not the actual sample data
    ///
    /// **Additional invariants for aux buses:**
    /// - Aux bus buffer arrays are created with the correct sample type S
    /// - Host fills buffers via pull_input_block with the format we specified
    /// - Buffer layouts match the format descriptor (f32 or f64)
    ///
    /// See `call_plugin_process` documentation for full safety analysis of the TypeId + transmute pattern.
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
        // Dispatch based on sample type using TypeId check + transmute pattern
        if std::any::TypeId::of::<S>() == std::any::TypeId::of::<f32>() {
            // SAFETY: TypeId check guarantees S is f32.
            // All transmutes are safe because:
            // - Memory layouts are identical (see documentation above)
            // - Underlying audio data is already f32 (host-provided)
            // - We're only changing type parameters, not data or layout
            let inputs_f32: &[&[f32]] = unsafe { std::mem::transmute(inputs) };
            let outputs_f32: &mut [&mut [f32]] = unsafe { std::mem::transmute(outputs) };
            let aux_inputs_f32: &[Vec<&[f32]>] = unsafe { std::mem::transmute(aux_inputs) };
            let aux_outputs_f32: &mut [Vec<&mut [f32]>] =
                unsafe { std::mem::transmute(aux_outputs) };
            match plugin.process_with_midi(
                inputs_f32,
                outputs_f32,
                aux_inputs_f32,
                aux_outputs_f32,
                midi_events,
                context,
            ) {
                Ok(()) => os_status::NO_ERR,
                Err(_) => os_status::K_AUDIO_UNIT_ERR_RENDER,
            }
        } else if std::any::TypeId::of::<S>() == std::any::TypeId::of::<f64>() {
            // SAFETY: TypeId check guarantees S is f64.
            // All transmutes are safe because:
            // - Memory layouts are identical (see documentation above)
            // - Underlying audio data is already f64 (host-provided)
            // - We're only changing type parameters, not data or layout
            let inputs_f64: &[&[f64]] = unsafe { std::mem::transmute(inputs) };
            let outputs_f64: &mut [&mut [f64]] = unsafe { std::mem::transmute(outputs) };
            let aux_inputs_f64: &[Vec<&[f64]>] = unsafe { std::mem::transmute(aux_inputs) };
            let aux_outputs_f64: &mut [Vec<&mut [f64]>] =
                unsafe { std::mem::transmute(aux_outputs) };
            match plugin.process_with_midi_f64(
                inputs_f64,
                outputs_f64,
                aux_inputs_f64,
                aux_outputs_f64,
                midi_events,
                context,
            ) {
                Ok(()) => os_status::NO_ERR,
                Err(_) => os_status::K_AUDIO_UNIT_ERR_RENDER,
            }
        } else {
            // Should never happen - Sample trait is sealed to f32/f64 only
            // If this branch executes, it indicates a serious bug (new Sample impl without updating this code)
            os_status::K_AUDIO_UNIT_ERR_RENDER
        }
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
    ) -> i32 {
        self.process_impl(
            action_flags,
            timestamp,
            frame_count,
            output_bus_number,
            output_data,
            event_list,
            pull_input_block,
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

/// Create a no-op Objective-C render block.
///
/// This returns a block that does nothing when invoked, useful when
/// `internalRenderBlock` is called before `allocateRenderResources`.
/// AU hosts may query this property during validation and introspect
/// the block's type encoding metadata.
///
/// # Important
///
/// This block MUST use `with_encoding` to provide proper type metadata.
/// AU hosts (especially auval) validate block type signatures during
/// instantiation. Without proper encoding, the host may crash or reject
/// the plugin during `AudioComponentInstanceNew`.
pub fn create_noop_render_block() -> RcBlock<AuRenderBlockFn> {
    // Use with_encoding to provide proper type metadata that AU hosts expect.
    // This is critical for AU validation during instantiation - without proper
    // encoding metadata, hosts may crash when introspecting the block type.
    RcBlock::with_encoding::<_, _, _, AuRenderBlockEncoding>(
        |_action_flags: *mut u32,
         _timestamp: *const AudioTimeStamp,
         _frame_count: u32,
         _output_bus_number: isize,
         _output_data: *mut AudioBufferList,
         _event_list: *const AURenderEvent,
         _pull_input_block: *const c_void|
         -> i32 {
            // No-op: return success without processing
            0 // noErr
        },
    )
}

/// Create an Objective-C block for AU render callback.
///
/// This function creates a proper Objective-C block using block2 that wraps
/// a Rust render block implementing `RenderBlockTrait`. The block can be
/// returned from `internalRenderBlock` and called by AU hosts during audio
/// processing.
///
/// # Arguments
///
/// * `render_block` - Arc to the Rust render block (type-erased for f32/f64 support)
///
/// # Returns
///
/// An `RcBlock<AuRenderBlockFn>` that captures the render block and forwards
/// calls to its `process()` method. The block has the correct signature for
/// Apple's AURenderBlock typedef.
///
/// # Block Lifetime Safety
///
/// - `RcBlock` uses Objective-C reference counting (retains/releases)
/// - Captured `Arc` ensures render block outlives all block invocations
/// - When the block is released, the Arc is dropped (decrementing refcount)
///
/// # Real-Time Safety
///
/// The block invocation is just a function call through the closure - no
/// allocations occur during audio processing. All render work happens in
/// the pre-allocated `RenderBlock` storage.
pub fn create_objc_render_block(
    render_block: Arc<dyn RenderBlockTrait>,
) -> RcBlock<AuRenderBlockFn> {
    // Use with_encoding to provide proper type metadata that Apple's AU framework expects.
    // Plain RcBlock::new doesn't include type encoding, which causes crashes when AU
    // tries to validate or invoke the block.
    RcBlock::with_encoding::<_, _, _, AuRenderBlockEncoding>(
        move |action_flags: *mut u32,
              timestamp: *const AudioTimeStamp,
              frame_count: u32,
              output_bus_number: isize,
              output_data: *mut AudioBufferList,
              event_list: *const AURenderEvent,
              pull_input_block: *const c_void|
              -> i32 {
            // Forward to the captured render block's process method
            render_block.process(
                action_flags,
                timestamp,
                frame_count,
                output_bus_number as i32,
                output_data,
                event_list,
                pull_input_block,
            )
        },
    )
}
