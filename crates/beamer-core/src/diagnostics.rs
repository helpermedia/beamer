//! Diagnostics builds: what the host does with a plugin, written to the
//! system log.
//!
//! A plugin built with the `diagnostics` feature
//! (`cargo xtask bundle <plugin> --diagnostics`) measures each block on
//! the audio thread and logs a summary twice per second: block sizes,
//! levels of each bus, whether the host processes in place, MIDI, clicks,
//! dropouts and the plugin's own values (see
//! [`ProcessContext::report_value`](crate::ProcessContext::report_value)).
//! Host calls such as prepare or bypass are logged when they happen.
//!
//! The audio thread does not log: it notes what it measured in a
//! [`DiagnosticsState`] of atomics, without allocating or locking. A
//! reporter thread per instance reads the state, resets it and logs the
//! lines under the target `diagnostics`.
//!
//! The click detection looks for steps in the signal and is meant for
//! low test tones (a sine up to about 500 Hz). Music has steps of its own
//! and would show false clicks.

use std::fmt::{self, Write as _};
use std::sync::atomic::{
    fence, AtomicBool, AtomicI32, AtomicPtr, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering,
};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crate::active_buses::{BusNegotiation, BusRouting};
use crate::buffer_storage::ProcessBufferStorage;
use crate::config::Config;
use crate::midi::{MidiEvent, MidiEventKind};
use crate::sample::Sample;
use crate::types::{MAX_BUSES, MAX_CHANNELS};

/// The log target of the diagnostics lines.
pub const TARGET: &str = "diagnostics";

/// How often the reporter thread logs a summary.
const PERIOD: Duration = Duration::from_millis(500);

/// The smallest second difference counted as a click. A full scale sine
/// of 500 Hz at 44.1 kHz stays below 0.006.
const CLICK_STEP: f32 = 0.01;

/// A peak below this (-60 dB) counts as silence.
const SILENCE: f32 = 0.001;

/// How long after a bypass change a click counts as near it.
const BYPASS_WINDOW_SECONDS: f64 = 0.05;

/// How many values a plugin can report per instance.
const VALUE_SLOTS: usize = 8;

/// Bytes kept of the first part of a SysEx message.
const SYSEX_HEAD: usize = 24;

/// Bytes kept of the last part of a SysEx message.
const SYSEX_TAIL: usize = 8;

/// The number of the next instance, for the labels of the lines.
static NEXT_INSTANCE: AtomicU32 = AtomicU32::new(1);

// =============================================================================
// Atomic building blocks
// =============================================================================

/// The largest peak since the reporter last read it, or none.
///
/// Stored as the bits of the peak plus one: the bits of positive floats
/// order like the floats, so `fetch_max` keeps the largest. 0 means
/// nothing was measured.
#[derive(Default)]
struct Level(AtomicU32);

impl Level {
    fn note(&self, peak: f32) {
        if peak.is_finite() && peak >= 0.0 {
            self.0.fetch_max(peak.to_bits() + 1, Ordering::Relaxed);
        }
    }

    fn take(&self) -> Option<f32> {
        let stored = self.0.swap(0, Ordering::Relaxed);
        (stored != 0).then(|| f32::from_bits(stored - 1))
    }
}

/// A float as bits that order like the float, for `fetch_min` and
/// `fetch_max` on values of either sign.
fn ordered_bits(value: f32) -> u32 {
    let bits = value.to_bits();
    if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits | 0x8000_0000
    }
}

/// The float of [`ordered_bits`].
fn from_ordered_bits(bits: u32) -> f32 {
    if bits & 0x8000_0000 != 0 {
        f32::from_bits(bits & 0x7FFF_FFFF)
    } else {
        f32::from_bits(!bits)
    }
}

/// A value the plugin reports under a name of its own.
struct ValueSlot {
    /// The name's bytes, null while the slot is free.
    name: AtomicPtr<u8>,
    /// The name's length.
    name_len: AtomicUsize,
    /// Set once `name` and `name_len` are written.
    ready: AtomicBool,
    /// Reports since the reporter last read the slot.
    count: AtomicU32,
    /// The smallest value, as [`ordered_bits`].
    smallest: AtomicU32,
    /// The largest value, as [`ordered_bits`].
    largest: AtomicU32,
    /// The last value, as bits.
    last: AtomicU32,
}

impl Default for ValueSlot {
    fn default() -> Self {
        Self {
            name: AtomicPtr::new(std::ptr::null_mut()),
            name_len: AtomicUsize::new(0),
            ready: AtomicBool::new(false),
            count: AtomicU32::new(0),
            smallest: AtomicU32::new(u32::MAX),
            largest: AtomicU32::new(0),
            last: AtomicU32::new(0),
        }
    }
}

impl ValueSlot {
    /// The name, once the slot is taken.
    fn name(&self) -> Option<&'static str> {
        if !self.ready.load(Ordering::Acquire) {
            return None;
        }
        let ptr = self.name.load(Ordering::Relaxed);
        let len = self.name_len.load(Ordering::Relaxed);
        // SAFETY: A taken slot holds the pointer and length of a
        // `&'static str` (see `DiagnosticsState::report_value`), written
        // before `ready` was set.
        Some(unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) })
    }

    fn note(&self, value: f32) {
        let bits = ordered_bits(value);
        self.smallest.fetch_min(bits, Ordering::Relaxed);
        self.largest.fetch_max(bits, Ordering::Relaxed);
        self.last.store(value.to_bits(), Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

/// The last SysEx message the host sent, as a sequence lock: the audio
/// thread writes, the reporter retries a read that overlapped a write.
#[derive(Default)]
struct SysExCapture {
    /// Odd while a message is written.
    sequence: AtomicU32,
    /// Messages since the reporter last read.
    count: AtomicU32,
    /// The full length of the last message.
    len: AtomicU32,
    /// Its first `SYSEX_HEAD` and last `SYSEX_TAIL` bytes (all of a short
    /// one).
    bytes: [AtomicU8; SYSEX_HEAD + SYSEX_TAIL],
}

impl SysExCapture {
    fn note(&self, message: &[u8]) {
        let sequence = self.sequence.load(Ordering::Relaxed);
        self.sequence.store(sequence.wrapping_add(1), Ordering::Relaxed);
        fence(Ordering::Release);
        self.len.store(message.len() as u32, Ordering::Relaxed);
        for (slot, byte) in self.bytes.iter().zip(kept_bytes(message)) {
            slot.store(byte, Ordering::Relaxed);
        }
        self.sequence.store(sequence.wrapping_add(2), Ordering::Release);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    /// The count since the last call, the full length and the kept bytes
    /// of the last message.
    fn take(&self) -> Option<(u32, usize, Vec<u8>)> {
        let count = self.count.swap(0, Ordering::Relaxed);
        if count == 0 {
            return None;
        }
        // A writer that keeps writing leaves the bytes out after a few tries.
        for _ in 0..100 {
            let before = self.sequence.load(Ordering::Acquire);
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let len = self.len.load(Ordering::Relaxed) as usize;
            let kept = len.min(SYSEX_HEAD + SYSEX_TAIL);
            let bytes: Vec<u8> = self.bytes[..kept].iter().map(|b| b.load(Ordering::Relaxed)).collect();
            fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Relaxed) == before {
                return Some((count, len, bytes));
            }
        }
        Some((count, 0, Vec::new()))
    }
}

/// The bytes of a SysEx message that are kept: all of a short one, the
/// first `SYSEX_HEAD` and last `SYSEX_TAIL` of a long one.
fn kept_bytes(message: &[u8]) -> impl Iterator<Item = u8> + '_ {
    let (head, tail) = if message.len() <= SYSEX_HEAD + SYSEX_TAIL {
        (message, &message[..0])
    } else {
        (&message[..SYSEX_HEAD], &message[message.len() - SYSEX_TAIL..])
    };
    head.iter().chain(tail).copied()
}

// =============================================================================
// DiagnosticsState
// =============================================================================

/// How a host delivered a piece of MIDI input, before the wrapper turned it
/// into the plugin's events. A host may pick another form per plugin, so
/// the forms show which path of the wrapper its input takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostMidiForm {
    /// A single MIDI 1.0 event.
    Event,
    /// A SysEx event: one message in bytes.
    SysEx,
    /// An event list of Universal MIDI Packets, with the protocol it says
    /// it has.
    EventList(u32),
    /// The time stamp of a packet in an event list, in samples after its
    /// event.
    PacketTimeStamp(u64),
    /// A message in an event list, by its UMP message type (0 to 15).
    Ump(u8),
    /// A packet that ended inside a message.
    Truncated,
}

/// What the audio thread measured since the reporter last read it.
///
/// Every method for the audio thread is a few atomic operations: no
/// allocation, no lock.
#[derive(Default)]
pub struct DiagnosticsState {
    /// Blocks the plugin processed.
    blocks: AtomicU32,
    /// The smallest block, `u32::MAX` while none was processed (see
    /// `take_block_sizes`).
    smallest_block: AtomicU32,
    /// The largest block.
    largest_block: AtomicU32,
    /// Blocks the host bypassed: the plugin was left out.
    bypassed_blocks: AtomicU32,
    /// Render calls per output bus, for formats that render each output
    /// bus in a call of its own.
    bus_calls: [AtomicU32; MAX_BUSES],
    /// Peak of the main input.
    main_input: Level,
    /// Peak of the main output.
    main_output: Level,
    /// Peak of each aux input, by bus number.
    aux_inputs: [Level; MAX_BUSES],
    /// Peak of each aux output, by bus number.
    aux_outputs: [Level; MAX_BUSES],
    /// Main channels whose input and output were the same memory, as bits.
    shared_channels: AtomicU32,
    /// Main channels whose input was copied into their output, as bits.
    copied_channels: AtomicU32,
    /// Main channels whose output had no memory of the host's, as bits.
    own_channels: AtomicU32,
    /// Blocks in which the host's main output buffers had memory.
    host_memory_blocks: AtomicU32,
    /// Blocks in which they had none (the host asked for in place).
    no_host_memory_blocks: AtomicU32,
    /// Blocks with input on each aux input bus, by bus number.
    aux_input_present: [AtomicU32; MAX_BUSES],
    /// Blocks without input on each aux input bus that is on.
    aux_input_missing: [AtomicU32; MAX_BUSES],
    /// The status of the last block without input, where the host gave one.
    aux_input_status: [AtomicI32; MAX_BUSES],
    /// MIDI input: notes on and off.
    midi_notes: AtomicU32,
    /// MIDI input: control changes.
    midi_controllers: AtomicU32,
    /// MIDI input: SysEx messages.
    midi_sysex: AtomicU32,
    /// MIDI input: everything else.
    midi_other: AtomicU32,
    /// MIDI output events.
    midi_output: AtomicU32,
    /// MIDI input as the host delivered it, before the wrapper converted it
    /// (see [`HostMidiForm`]): single events, SysEx events, event lists.
    host_midi_forms: [AtomicU32; 3],
    /// Event lists by the protocol they say: other, MIDI 1.0, MIDI 2.0.
    host_list_protocols: [AtomicU32; 3],
    /// The messages in event lists by UMP message type.
    host_ump_messages: [AtomicU32; 16],
    /// Packets that ended inside a message.
    host_ump_truncated: AtomicU32,
    /// Packets with a time stamp other than 0, with the largest one.
    host_stamped_packets: AtomicU32,
    host_largest_time_stamp: AtomicU64,
    /// The last SysEx message as the host sent it.
    sysex: SysExCapture,
    /// SysEx messages from the host that were dropped: larger than what was
    /// left of the block's pool, or malformed.
    sysex_dropped: AtomicU32,
    /// Clicks in the main output.
    output_clicks: AtomicU32,
    /// Clicks in the main input.
    input_clicks: AtomicU32,
    /// The largest click in the main output.
    largest_click: Level,
    /// Main output channels with a click, as bits.
    click_channels: AtomicU32,
    /// Output clicks in the first two samples of a block.
    clicks_at_block_start: AtomicU32,
    /// Output clicks shortly after a bypass change.
    clicks_near_bypass: AtomicU32,
    /// Blocks with a silent main output while the main input was not.
    silent_blocks: AtomicU32,
    /// Samples of the main output that are not finite.
    non_finite: AtomicU32,
    /// The bypass parameter: 0 without one, 1 off, 2 on.
    bypass: AtomicU8,
    /// Changes of the bypass parameter.
    bypass_changes: AtomicU32,
    /// Values the plugin reported.
    values: [ValueSlot; VALUE_SLOTS],
}

impl fmt::Debug for DiagnosticsState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiagnosticsState").finish_non_exhaustive()
    }
}

impl DiagnosticsState {
    fn new() -> Self {
        let state = Self::default();
        state.smallest_block.store(u32::MAX, Ordering::Relaxed);
        state
    }

    /// Note a render call for an output bus. For formats that render each
    /// output bus in a call of its own.
    pub fn note_output_bus_call(&self, bus: usize) {
        if let Some(calls) = self.bus_calls.get(bus) {
            calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Note whether the host's main output buffers had memory of their own
    /// in this block.
    pub fn note_host_output_memory(&self, has_memory: bool) {
        let counter = if has_memory { &self.host_memory_blocks } else { &self.no_host_memory_blocks };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Note that an aux input bus that is on had input in this block.
    pub fn note_aux_input_present(&self, bus: usize) {
        if let Some(present) = self.aux_input_present.get(bus) {
            present.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Note that an aux input bus that is on had no input in this block,
    /// with the host's status where it gave one (0 otherwise).
    pub fn note_aux_input_missing(&self, bus: usize, status: i32) {
        if let (Some(missing), Some(last)) = (self.aux_input_missing.get(bus), self.aux_input_status.get(bus)) {
            missing.fetch_add(1, Ordering::Relaxed);
            last.store(status, Ordering::Relaxed);
        }
    }

    /// Note the MIDI input of a block.
    pub fn note_midi_input(&self, events: &[MidiEvent]) {
        for event in events {
            let counter = match event.event {
                MidiEventKind::NoteOn(_) | MidiEventKind::NoteOff(_) => &self.midi_notes,
                MidiEventKind::ControlChange(_) => &self.midi_controllers,
                MidiEventKind::SysEx(_) => &self.midi_sysex,
                _ => &self.midi_other,
            };
            counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Note how the host delivered a piece of MIDI input.
    pub fn note_host_midi(&self, form: HostMidiForm) {
        let counter = match form {
            HostMidiForm::Event => &self.host_midi_forms[0],
            HostMidiForm::SysEx => &self.host_midi_forms[1],
            HostMidiForm::EventList(protocol) => {
                self.host_midi_forms[2].fetch_add(1, Ordering::Relaxed);
                &self.host_list_protocols[if protocol <= 2 { protocol as usize } else { 0 }]
            }
            HostMidiForm::PacketTimeStamp(0) => return,
            HostMidiForm::PacketTimeStamp(stamp) => {
                self.host_largest_time_stamp.fetch_max(stamp, Ordering::Relaxed);
                &self.host_stamped_packets
            }
            HostMidiForm::Ump(message_type) => &self.host_ump_messages[usize::from(message_type & 0xF)],
            HostMidiForm::Truncated => &self.host_ump_truncated,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Note the number of MIDI output events of a block.
    pub fn note_midi_output(&self, count: usize) {
        self.midi_output.fetch_add(count as u32, Ordering::Relaxed);
    }

    /// Note a SysEx message with the bytes the host sent, framing included
    /// if it sent any.
    pub fn note_sysex_input(&self, message: &[u8]) {
        self.sysex.note(message);
    }

    /// Note SysEx messages from the host that were dropped.
    pub fn note_sysex_dropped(&self, count: u32) {
        self.sysex_dropped.fetch_add(count, Ordering::Relaxed);
    }

    /// Note a value the plugin reports under `name`. See
    /// [`ProcessContext::report_value`](crate::ProcessContext::report_value).
    ///
    /// For one thread at a time, the audio thread of the instance. Two
    /// threads reporting the same new name at once could take two slots.
    pub fn report_value(&self, name: &'static str, value: f32) {
        for slot in &self.values {
            if slot.name.load(Ordering::Acquire).is_null() {
                // Take the free slot. A failed exchange means another call
                // took it right now: it is then compared below.
                let taken = slot
                    .name
                    .compare_exchange(
                        std::ptr::null_mut(),
                        name.as_ptr().cast_mut(),
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok();
                if taken {
                    slot.name_len.store(name.len(), Ordering::Relaxed);
                    slot.ready.store(true, Ordering::Release);
                    slot.note(value);
                    return;
                }
            }
            if slot.name() == Some(name) {
                slot.note(value);
                return;
            }
        }
    }
}

// =============================================================================
// BlockMeter
// =============================================================================

/// The last two samples of a channel, for steps across a block boundary.
#[derive(Clone, Copy, Default)]
struct StepDetector {
    previous: [f32; 2],
    /// How many of `previous` hold samples.
    filled: u8,
}

impl StepDetector {
    /// The largest second difference in `samples` with the samples before
    /// them, with its index.
    fn largest_step<S: Sample>(&mut self, samples: &[S]) -> (f32, usize) {
        let mut largest = (0.0f32, 0);
        for (index, sample) in samples.iter().enumerate() {
            let x = sample.to_f32();
            if !x.is_finite() {
                continue;
            }
            if self.filled == 2 {
                let step = (x - 2.0 * self.previous[1] + self.previous[0]).abs();
                if step > largest.0 {
                    largest = (step, index);
                }
            }
            self.previous = [self.previous[1], x];
            self.filled = (self.filled + 1).min(2);
        }
        largest
    }
}

/// The peak of a channel, leaving out samples that are not finite.
fn peak<S: Sample>(samples: &[S]) -> f32 {
    samples
        .iter()
        .map(|s| s.to_f32().abs())
        .filter(|s| s.is_finite())
        .fold(0.0, f32::max)
}

/// The measurements of the audio thread, with what it keeps from block to
/// block. One per render setup, owned by the audio thread.
pub struct BlockMeter {
    state: Arc<DiagnosticsState>,
    /// Frames during which a click counts as near a bypass change.
    bypass_window: u64,
    inputs: [StepDetector; MAX_CHANNELS],
    outputs: [StepDetector; MAX_CHANNELS],
    /// Frames since the bypass parameter or the host's bypass changed.
    since_bypass_change: u64,
    /// The bypass parameter in the last block.
    bypass: Option<bool>,
    /// Whether the host bypassed the last block.
    host_bypassed: bool,
    /// Whether a block was measured before.
    started: bool,
    /// Frames of the current block.
    frames: usize,
    /// Peak of the main input in the current block, if it has one.
    block_input: Option<f32>,
    /// Peak of the main output in the current block, if it has one.
    block_output: Option<f32>,
}

impl BlockMeter {
    /// A meter that writes into `state`.
    pub fn new(state: Arc<DiagnosticsState>, sample_rate: f64) -> Self {
        Self {
            state,
            bypass_window: (sample_rate * BYPASS_WINDOW_SECONDS) as u64,
            inputs: [StepDetector::default(); MAX_CHANNELS],
            outputs: [StepDetector::default(); MAX_CHANNELS],
            since_bypass_change: u64::MAX,
            bypass: None,
            host_bypassed: false,
            started: false,
            frames: 0,
            block_input: None,
            block_output: None,
        }
    }

    /// The state this meter writes into.
    pub fn state(&self) -> &DiagnosticsState {
        &self.state
    }

    /// Start a block the plugin processes, with the bypass parameter's
    /// value (none without one).
    pub fn begin_block(&mut self, frames: usize, bypass: Option<bool>) {
        let state = &*self.state;
        state.blocks.fetch_add(1, Ordering::Relaxed);
        state.smallest_block.fetch_min(frames as u32, Ordering::Relaxed);
        state.largest_block.fetch_max(frames as u32, Ordering::Relaxed);
        if self.started && bypass != self.bypass {
            state.bypass_changes.fetch_add(1, Ordering::Relaxed);
            self.since_bypass_change = 0;
        }
        self.bypass = bypass;
        state.bypass.store(
            match bypass {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            },
            Ordering::Relaxed,
        );
        self.begin(frames, false);
    }

    /// Start a block the host bypassed: the plugin is left out and the
    /// output measured is what the host gets instead.
    pub fn begin_host_bypassed_block(&mut self, frames: usize) {
        self.state.bypassed_blocks.fetch_add(1, Ordering::Relaxed);
        self.begin(frames, true);
    }

    fn begin(&mut self, frames: usize, host_bypassed: bool) {
        if self.started && host_bypassed != self.host_bypassed {
            self.since_bypass_change = 0;
        }
        self.host_bypassed = host_bypassed;
        self.started = true;
        self.frames = frames;
        self.block_input = None;
        self.block_output = None;
    }

    /// Note how the host passed the main channels: the same memory for
    /// input and output, separate memory or an output without memory. Call
    /// it with the host's pointers, before the wrapper replaces any.
    pub fn note_host_channels<S>(&mut self, inputs: &[*const S], outputs: &[*mut S]) {
        let (mut shared, mut copied, mut own) = (0u32, 0u32, 0u32);
        for (channel, &output) in outputs.iter().enumerate().take(MAX_CHANNELS) {
            let bit = 1 << channel;
            let input = inputs.get(channel).copied().unwrap_or(std::ptr::null());
            if output.is_null() {
                own |= bit;
            } else if input.is_null() {
                // An output without input (an instrument) has nothing to
                // process in place.
            } else if std::ptr::eq(input, output) {
                shared |= bit;
            } else {
                copied |= bit;
            }
        }
        let state = &*self.state;
        state.shared_channels.fetch_or(shared, Ordering::Relaxed);
        state.copied_channels.fetch_or(copied, Ordering::Relaxed);
        state.own_channels.fetch_or(own, Ordering::Relaxed);
    }

    /// Measure a main input channel.
    pub fn main_input<S: Sample>(&mut self, channel: usize, samples: &[S]) {
        let peak = peak(samples);
        self.block_input = Some(self.block_input.unwrap_or(0.0).max(peak));
        self.state.main_input.note(peak);
        if let Some(detector) = self.inputs.get_mut(channel) {
            if detector.largest_step(samples).0 >= CLICK_STEP {
                self.state.input_clicks.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Measure a channel of an aux input bus (bus 1 is the first aux bus).
    pub fn aux_input<S: Sample>(&mut self, bus: usize, samples: &[S]) {
        if let Some(level) = self.state.aux_inputs.get(bus) {
            level.note(peak(samples));
        }
    }

    /// Measure a main output channel.
    pub fn main_output<S: Sample>(&mut self, channel: usize, samples: &[S]) {
        let state = &*self.state;
        let peak = peak(samples);
        self.block_output = Some(self.block_output.unwrap_or(0.0).max(peak));
        state.main_output.note(peak);
        let non_finite = samples.iter().filter(|s| !s.to_f32().is_finite()).count();
        if non_finite > 0 {
            state.non_finite.fetch_add(non_finite as u32, Ordering::Relaxed);
        }
        if let Some(detector) = self.outputs.get_mut(channel) {
            let (step, index) = detector.largest_step(samples);
            if step >= CLICK_STEP {
                state.output_clicks.fetch_add(1, Ordering::Relaxed);
                state.largest_click.note(step);
                state.click_channels.fetch_or(1 << channel, Ordering::Relaxed);
                if index < 2 {
                    state.clicks_at_block_start.fetch_add(1, Ordering::Relaxed);
                }
                if self.since_bypass_change.saturating_add(index as u64) < self.bypass_window {
                    state.clicks_near_bypass.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Measure a channel of an aux output bus (bus 1 is the first aux bus).
    pub fn aux_output<S: Sample>(&mut self, bus: usize, samples: &[S]) {
        if let Some(level) = self.state.aux_outputs.get(bus) {
            level.note(peak(samples));
        }
    }

    /// End the block: a silent main output while the main input was not
    /// counts as a dropout.
    pub fn end_block(&mut self) {
        if let (Some(input), Some(output)) = (self.block_input, self.block_output) {
            if input >= SILENCE && output == 0.0 {
                self.state.silent_blocks.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.since_bypass_change = self.since_bypass_change.saturating_add(self.frames as u64);
    }

    /// Measure the inputs of a block in `storage`, after
    /// [`ProcessBufferStorage::prepare_in_place`] returned true: each main
    /// output channel then holds its input.
    ///
    /// # Safety
    ///
    /// The channels of `storage` must be valid for `frames` samples, with
    /// no mutable reference to them alive.
    pub unsafe fn measure_inputs<S: Sample>(&mut self, storage: &ProcessBufferStorage<S>, frames: usize) {
        for channel in 0..storage.main_inputs.len() {
            let ptr = match storage.main_outputs.get(channel) {
                Some(&output) => output.cast_const(),
                None => storage.main_inputs[channel],
            };
            if !ptr.is_null() {
                // SAFETY: Valid for frames samples (caller).
                self.main_input(channel, unsafe { std::slice::from_raw_parts(ptr, frames) });
            }
        }
        for (index, bus) in storage.aux_inputs.iter().enumerate() {
            for &ptr in bus.iter().filter(|ptr| !ptr.is_null()) {
                // SAFETY: Valid for frames samples (caller).
                self.aux_input(index + 1, unsafe { std::slice::from_raw_parts(ptr, frames) });
            }
        }
    }

    /// Measure the outputs of a block in `storage` after the plugin
    /// processed it, then end the block.
    ///
    /// # Safety
    ///
    /// As for [`measure_inputs`](Self::measure_inputs).
    pub unsafe fn measure_outputs<S: Sample>(&mut self, storage: &ProcessBufferStorage<S>, frames: usize) {
        for (channel, &ptr) in storage.main_outputs.iter().enumerate() {
            if !ptr.is_null() {
                // SAFETY: Valid for frames samples (caller).
                self.main_output(channel, unsafe { std::slice::from_raw_parts(ptr, frames) });
            }
        }
        for (index, bus) in storage.aux_outputs.iter().enumerate() {
            for &ptr in bus.iter().filter(|ptr| !ptr.is_null()) {
                // SAFETY: Valid for frames samples (caller).
                self.aux_output(index + 1, unsafe { std::slice::from_raw_parts(ptr, frames) });
            }
        }
        self.end_block();
    }
}

// =============================================================================
// Reporting
// =============================================================================

/// What an aux input bus showed in the last period that had blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuxInputSeen {
    Unknown,
    Present,
    Missing(i32),
}

/// What the reporter keeps from one period to the next, to log changes.
struct ReportMemory {
    rendering: bool,
    aux_inputs: [AuxInputSeen; MAX_BUSES],
}

impl Default for ReportMemory {
    fn default() -> Self {
        Self { rendering: false, aux_inputs: [AuxInputSeen::Unknown; MAX_BUSES] }
    }
}

/// A peak in dB.
fn db(peak: f32) -> String {
    if peak <= 0.0 {
        return "-inf dB".to_string();
    }
    // A peak just below full scale shows as 0.0, not -0.0.
    let db = 20.0 * peak.log10();
    format!("{:.1} dB", if db.abs() < 0.05 { 0.0 } else { db })
}

/// Bits set in a channel mask.
fn channel_count(bits: u32) -> u32 {
    bits.count_ones()
}

/// The lines for what `state` holds, which is reset. `memory` keeps what
/// the next call compares with.
fn report_lines(state: &DiagnosticsState, memory: &mut ReportMemory) -> Vec<String> {
    let relaxed = Ordering::Relaxed;
    let mut lines = Vec::new();

    let blocks = state.blocks.swap(0, relaxed);
    let bypassed = state.bypassed_blocks.swap(0, relaxed);
    let smallest = state.smallest_block.swap(u32::MAX, relaxed);
    let largest = state.largest_block.swap(0, relaxed);
    let bus_calls: Vec<u32> = state.bus_calls.iter().map(|c| c.swap(0, relaxed)).collect();
    let main_input = state.main_input.take();
    let main_output = state.main_output.take();
    let aux_inputs: Vec<Option<f32>> = state.aux_inputs.iter().map(Level::take).collect();
    let aux_outputs: Vec<Option<f32>> = state.aux_outputs.iter().map(Level::take).collect();
    let shared = state.shared_channels.swap(0, relaxed);
    let copied = state.copied_channels.swap(0, relaxed);
    let own = state.own_channels.swap(0, relaxed);
    let host_memory = state.host_memory_blocks.swap(0, relaxed);
    let no_host_memory = state.no_host_memory_blocks.swap(0, relaxed);
    let present: Vec<u32> = state.aux_input_present.iter().map(|c| c.swap(0, relaxed)).collect();
    let missing: Vec<u32> = state.aux_input_missing.iter().map(|c| c.swap(0, relaxed)).collect();
    let notes = state.midi_notes.swap(0, relaxed);
    let controllers = state.midi_controllers.swap(0, relaxed);
    let sysex = state.midi_sysex.swap(0, relaxed);
    let other = state.midi_other.swap(0, relaxed);
    let midi_output = state.midi_output.swap(0, relaxed);
    let output_clicks = state.output_clicks.swap(0, relaxed);
    let input_clicks = state.input_clicks.swap(0, relaxed);
    let largest_click = state.largest_click.take();
    let click_channels = state.click_channels.swap(0, relaxed);
    let at_block_start = state.clicks_at_block_start.swap(0, relaxed);
    let near_bypass = state.clicks_near_bypass.swap(0, relaxed);
    let silent_blocks = state.silent_blocks.swap(0, relaxed);
    let non_finite = state.non_finite.swap(0, relaxed);
    let bypass_changes = state.bypass_changes.swap(0, relaxed);

    if blocks == 0 && bypassed == 0 {
        if memory.rendering {
            lines.push("render calls stopped".to_string());
            memory.rendering = false;
        }
        return lines;
    }
    memory.rendering = true;

    // The summary line.
    let mut summary = String::new();
    if blocks > 0 {
        if smallest == largest {
            let _ = write!(summary, "{blocks} blocks of {largest}");
        } else {
            let _ = write!(summary, "{blocks} blocks of {smallest} to {largest}");
        }
    }
    if bypassed > 0 {
        let _ = write!(summary, "{}{bypassed} bypassed by the host", if summary.is_empty() { "" } else { ", " });
    }
    if let Some(peak) = main_input {
        let _ = write!(summary, ", in {}", db(peak));
    }
    if let Some(peak) = main_output {
        let _ = write!(summary, ", out {}", db(peak));
    }
    for (bus, peak) in aux_inputs.iter().enumerate() {
        if let Some(peak) = peak {
            let _ = write!(summary, ", aux in {bus} {}", db(*peak));
        }
    }
    for (bus, peak) in aux_outputs.iter().enumerate() {
        if let Some(peak) = peak {
            let _ = write!(summary, ", aux out {bus} {}", db(*peak));
        }
    }
    let classified = channel_count(shared | copied | own);
    if classified > 0 {
        let _ = write!(summary, ", in place {} of {classified}", channel_count(shared));
        if copied != 0 {
            let _ = write!(summary, " ({} copied", channel_count(copied));
            if own != 0 {
                let _ = write!(summary, ", {} without host memory", channel_count(own));
            }
            summary.push(')');
        } else if own != 0 {
            let _ = write!(summary, " ({} without host memory)", channel_count(own));
        }
    }
    if host_memory + no_host_memory > 0 {
        let _ = write!(summary, ", host output memory in {host_memory} of {} blocks", host_memory + no_host_memory);
    }
    if bus_calls.iter().skip(1).any(|&calls| calls > 0) {
        let calls: Vec<String> = bus_calls
            .iter()
            .enumerate()
            .filter(|&(bus, &calls)| bus == 0 || calls > 0)
            .map(|(bus, calls)| format!("{bus}: {calls}"))
            .collect();
        let _ = write!(summary, ", render calls per output bus {}", calls.join(", "));
    }
    if notes + controllers + sysex + other > 0 {
        let _ = write!(summary, ", MIDI in {notes} notes, {controllers} CC, {sysex} SysEx, {other} other");
    }
    if midi_output > 0 {
        let _ = write!(summary, ", MIDI out {midi_output}");
    }
    for slot in &state.values {
        let count = slot.count.swap(0, relaxed);
        let smallest = from_ordered_bits(slot.smallest.swap(u32::MAX, relaxed));
        let largest = from_ordered_bits(slot.largest.swap(0, relaxed));
        let last = f32::from_bits(slot.last.load(relaxed));
        if let (Some(name), true) = (slot.name(), count > 0) {
            let _ = write!(summary, ", {name} {smallest:.2} to {largest:.2} (last {last:.2})");
        }
    }
    lines.push(summary);

    // Lines for what needs attention.
    if output_clicks > 0 {
        let channels: Vec<String> =
            (0..MAX_CHANNELS).filter(|c| click_channels & (1 << c) != 0).map(|c| c.to_string()).collect();
        lines.push(format!(
            "clicks in the output: {output_clicks} (largest step {:.3}, channel {}), {at_block_start} at a block start, {near_bypass} near a bypass change, {input_clicks} in the input",
            largest_click.unwrap_or(0.0),
            channels.join(", "),
        ));
    } else if input_clicks > 0 {
        lines.push(format!("clicks in the input only: {input_clicks}"));
    }
    if silent_blocks > 0 {
        lines.push(format!("dropouts: {silent_blocks} blocks with a silent output while the input was not"));
    }
    if non_finite > 0 {
        lines.push(format!("{non_finite} output samples were not finite"));
    }
    if bypass_changes > 0 {
        let now = match state.bypass.load(relaxed) {
            2 => "on",
            _ => "off",
        };
        lines.push(format!("bypass parameter {now} ({bypass_changes} changes)"));
    }
    for bus in 1..MAX_BUSES {
        let seen = if missing[bus] > 0 {
            AuxInputSeen::Missing(state.aux_input_status[bus].load(relaxed))
        } else if present[bus] > 0 {
            AuxInputSeen::Present
        } else {
            continue;
        };
        if seen == memory.aux_inputs[bus] {
            continue;
        }
        memory.aux_inputs[bus] = seen;
        let blocks_with_bus = missing[bus] + present[bus];
        lines.push(match seen {
            AuxInputSeen::Missing(0) => {
                format!("aux in {bus} has no input in {} of {blocks_with_bus} blocks", missing[bus])
            }
            AuxInputSeen::Missing(status) => {
                format!("aux in {bus} has no input in {} of {blocks_with_bus} blocks (status {status})", missing[bus])
            }
            _ => format!("aux in {bus} has input"),
        });
    }
    let dropped = state.sysex_dropped.swap(0, Ordering::Relaxed);
    let dropped_text = if dropped > 0 { format!(" ({dropped} dropped)") } else { String::new() };
    if let Some((count, len, bytes)) = state.sysex.take() {
        let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let shown = if len > SYSEX_HEAD + SYSEX_TAIL {
            format!("{} ... {}", hex[..SYSEX_HEAD].join(" "), hex[SYSEX_HEAD..].join(" "))
        } else {
            hex.join(" ")
        };
        lines.push(format!("SysEx in: {count} messages{dropped_text}, last {len} bytes: {shown}"));
    } else if dropped > 0 {
        // Messages the wrapper dropped before it could note their bytes.
        lines.push(format!("SysEx in: {dropped} dropped"));
    }

    // How the host delivered its MIDI input.
    let take = |counters: &[AtomicU32]| -> Vec<u32> {
        counters.iter().map(|counter| counter.swap(0, Ordering::Relaxed)).collect()
    };
    let forms = take(&state.host_midi_forms);
    let protocols = take(&state.host_list_protocols);
    let ump = take(&state.host_ump_messages);
    let truncated = state.host_ump_truncated.swap(0, Ordering::Relaxed);
    let stamped = state.host_stamped_packets.swap(0, Ordering::Relaxed);
    let largest_stamp = state.host_largest_time_stamp.swap(0, Ordering::Relaxed);
    if forms.iter().any(|&count| count > 0) {
        let mut line = format!("host MIDI: {} events, {} SysEx events, {} event lists", forms[0], forms[1], forms[2]);
        if forms[2] > 0 {
            let _ = write!(line, " (MIDI 1.0: {}, MIDI 2.0: {}", protocols[1], protocols[2]);
            if protocols[0] > 0 {
                let _ = write!(line, ", other: {}", protocols[0]);
            }
            line.push(')');
        }
        let types: Vec<String> = ump
            .iter()
            .enumerate()
            .filter(|&(_, &count)| count > 0)
            .map(|(message_type, count)| format!("type {message_type:X}: {count}"))
            .collect();
        if !types.is_empty() {
            let _ = write!(line, "; UMP {}", types.join(", "));
        }
        if stamped > 0 {
            let _ = write!(line, "; {stamped} packets with time stamps, up to {largest_stamp}");
        }
        if truncated > 0 {
            let _ = write!(line, "; {truncated} packets cut short");
        }
        lines.push(line);
    }

    lines
}

// =============================================================================
// Diagnostics
// =============================================================================

/// The diagnostics of one plugin instance: the state its audio thread
/// writes and the reporter thread that logs it. Dropping it stops the
/// thread after a last report.
pub struct Diagnostics {
    state: Arc<DiagnosticsState>,
    label: Arc<str>,
    stop: Arc<AtomicBool>,
    reporter: Option<JoinHandle<()>>,
}

impl Diagnostics {
    /// Start the diagnostics of a new instance of the plugin of `config`
    /// in `format` ("au", "vst3"). The lines carry the plugin's package name
    /// (the one `cargo xtask log` takes) with a number per instance and the
    /// format.
    pub fn start(config: &Config, format: &str) -> Self {
        let number = NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed);
        let label: Arc<str> = format!("{}#{number} {format}", package_name(config)).into();
        let state = Arc::new(DiagnosticsState::new());
        let stop = Arc::new(AtomicBool::new(false));

        let reporter = {
            let (state, label, stop) = (Arc::clone(&state), Arc::clone(&label), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("beamer-diagnostics".to_string())
                .spawn(move || {
                    let mut memory = ReportMemory::default();
                    loop {
                        std::thread::park_timeout(PERIOD);
                        let stopping = stop.load(Ordering::Acquire);
                        for line in report_lines(&state, &mut memory) {
                            log::info!(target: TARGET, "{label} {line}");
                        }
                        if stopping {
                            break;
                        }
                    }
                })
        };
        let reporter = match reporter {
            Ok(handle) => Some(handle),
            Err(error) => {
                log::warn!(target: TARGET, "{label} no reporter thread: {error}");
                None
            }
        };

        let diagnostics = Self { state, label, stop, reporter };
        diagnostics.note(format_args!("diagnostics on"));
        diagnostics
    }

    /// The state the audio thread writes into.
    pub fn state(&self) -> &Arc<DiagnosticsState> {
        &self.state
    }

    /// Log a line at once. For host threads.
    pub fn note(&self, message: fmt::Arguments<'_>) {
        log::info!(target: TARGET, "{} {message}", self.label);
    }
}

impl Drop for Diagnostics {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(reporter) = self.reporter.take() {
            reporter.thread().unpark();
            let _ = reporter.join();
        }
        self.note(format_args!("diagnostics off"));
    }
}

impl fmt::Debug for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Diagnostics").field("label", &self.label).finish_non_exhaustive()
    }
}

// =============================================================================
// Helpers for the wrappers
// =============================================================================

/// The package name of a plugin: the end of its log subsystem
/// (`com.beamer.<package>`), or its name without one.
fn package_name(config: &Config) -> String {
    match config.log_subsystem.rsplit('.').next() {
        Some(package) if !package.is_empty() => package.to_string(),
        _ => config.name.to_lowercase().replace(' ', "-"),
    }
}

/// The buses of a negotiation as the host and the plugin have them:
/// `in 2, 2 (plugin only); out 2`.
pub fn describe_buses(buses: &BusNegotiation) -> String {
    let describe = |count: usize, channels: &dyn Fn(usize) -> Option<u32>, routing: &dyn Fn(usize) -> BusRouting| {
        let parts: Vec<String> = (0..count)
            .map(|bus| {
                let channels = channels(bus).map_or("off".to_string(), |c| c.to_string());
                match routing(bus) {
                    BusRouting::Connected | BusRouting::Off => channels,
                    BusRouting::PluginOnly => format!("{channels} (plugin only)"),
                    BusRouting::HostOnly => format!("{channels} (host only)"),
                }
            })
            .collect();
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(", ")
        }
    };
    let accepted = buses.accepted();
    format!(
        "in {}; out {}",
        describe(accepted.input_bus_count(), &|bus| accepted.input(bus), &|bus| buses.input_routing(bus)),
        describe(accepted.output_bus_count(), &|bus| accepted.output(bus), &|bus| buses.output_routing(bus)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sine of `frequency` Hz at 48 kHz, starting at sample `start`.
    fn sine(frequency: f32, amplitude: f32, start: usize, len: usize) -> Vec<f32> {
        (start..start + len)
            .map(|n| amplitude * (std::f32::consts::TAU * frequency * n as f32 / 48_000.0).sin())
            .collect()
    }

    fn meter() -> BlockMeter {
        BlockMeter::new(Arc::new(DiagnosticsState::new()), 48_000.0)
    }

    #[test]
    fn test_db_text() {
        assert_eq!(db(0.0), "-inf dB");
        assert_eq!(db(1.0), "0.0 dB");
        assert_eq!(db(0.999), "0.0 dB");
        assert_eq!(db(0.5), "-6.0 dB");
    }

    #[test]
    fn test_ordered_bits_keep_the_order() {
        let values = [-3.5f32, -1.0, -0.0, 0.0, 0.25, 7.0];
        for pair in values.windows(2) {
            assert!(ordered_bits(pair[0]) <= ordered_bits(pair[1]));
        }
        for value in values {
            assert_eq!(from_ordered_bits(ordered_bits(value)), value);
        }
    }

    #[test]
    fn test_level_keeps_the_largest_peak() {
        let level = Level::default();
        assert_eq!(level.take(), None);
        level.note(0.0);
        assert_eq!(level.take(), Some(0.0));
        level.note(0.5);
        level.note(0.25);
        level.note(f32::NAN);
        assert_eq!(level.take(), Some(0.5));
        assert_eq!(level.take(), None);
    }

    #[test]
    fn test_a_low_sine_has_no_clicks() {
        let mut meter = meter();
        for block in 0..20 {
            meter.begin_block(256, None);
            meter.main_input(0, &sine(500.0, 1.0, block * 256, 256));
            meter.main_output(0, &sine(500.0, 1.0, block * 256, 256));
            meter.end_block();
        }
        let state = meter.state();
        assert_eq!(state.output_clicks.load(Ordering::Relaxed), 0);
        assert_eq!(state.input_clicks.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_a_step_at_a_block_start_is_a_click() {
        let mut meter = meter();
        meter.begin_block(256, Some(false));
        meter.main_output(0, &sine(100.0, 0.25, 0, 256));
        meter.end_block();
        meter.begin_block(256, Some(true));
        meter.main_output(0, &sine(100.0, 0.5, 256, 256));
        meter.end_block();

        let state = meter.state();
        assert_eq!(state.output_clicks.load(Ordering::Relaxed), 1);
        assert_eq!(state.clicks_at_block_start.load(Ordering::Relaxed), 1);
        assert_eq!(state.clicks_near_bypass.load(Ordering::Relaxed), 1);
        assert_eq!(state.bypass_changes.load(Ordering::Relaxed), 1);
        assert_eq!(state.click_channels.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_a_click_long_after_a_bypass_change_is_not_near_it() {
        let mut meter = meter();
        meter.begin_block(4800, Some(true));
        meter.main_output(0, &sine(100.0, 0.5, 0, 4800));
        meter.end_block();
        meter.begin_block(4800, Some(false));
        meter.main_output(0, &sine(100.0, 0.5, 4800, 4800));
        meter.end_block();
        let mut step = sine(100.0, 0.5, 9600, 256);
        step[100] += 0.2;
        meter.begin_block(256, Some(false));
        meter.main_output(0, &step);
        meter.end_block();

        let state = meter.state();
        assert_eq!(state.output_clicks.load(Ordering::Relaxed), 1);
        assert_eq!(state.clicks_at_block_start.load(Ordering::Relaxed), 0);
        assert_eq!(state.clicks_near_bypass.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_host_bypass_counts_as_a_bypass_change() {
        let mut meter = meter();
        meter.begin_block(256, None);
        meter.main_output(0, &sine(100.0, 0.25, 0, 256));
        meter.end_block();
        meter.begin_host_bypassed_block(256);
        meter.main_output(0, &sine(100.0, 0.5, 256, 256));
        meter.end_block();

        let state = meter.state();
        assert_eq!(state.bypassed_blocks.load(Ordering::Relaxed), 1);
        assert_eq!(state.clicks_near_bypass.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_a_sine_through_host_bypass_has_no_input_click() {
        let mut meter = meter();
        for block in 0..6 {
            let samples = sine(100.0, 0.25, block * 256, 256);
            if (2..4).contains(&block) {
                // The host bypass: the output is the input, measured as both.
                meter.begin_host_bypassed_block(256);
            } else {
                meter.begin_block(256, None);
            }
            meter.main_input(0, &samples);
            meter.main_output(0, &samples);
            meter.end_block();
        }
        let state = meter.state();
        assert_eq!(state.input_clicks.load(Ordering::Relaxed), 0);
        assert_eq!(state.output_clicks.load(Ordering::Relaxed), 0);
        assert_eq!(state.bypassed_blocks.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_a_silent_output_with_input_is_a_dropout() {
        let mut meter = meter();
        meter.begin_block(64, None);
        meter.main_input(0, &sine(100.0, 0.5, 0, 64));
        meter.main_output(0, &[0.0f32; 64]);
        meter.end_block();
        meter.begin_block(64, None);
        meter.main_input(0, &[0.0f32; 64]);
        meter.main_output(0, &[0.0f32; 64]);
        meter.end_block();
        assert_eq!(meter.state().silent_blocks.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_non_finite_samples_are_counted() {
        let mut meter = meter();
        meter.begin_block(4, None);
        meter.main_output(0, &[0.0f32, f32::NAN, f32::INFINITY, 0.0]);
        meter.end_block();
        assert_eq!(meter.state().non_finite.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_host_channels_are_classified() {
        let mut meter = meter();
        let mut a = [0.0f32; 4];
        let mut b = [0.0f32; 4];
        let c = [0.0f32; 4];
        let inputs = [a.as_ptr(), c.as_ptr(), c.as_ptr()];
        let outputs = [a.as_mut_ptr(), b.as_mut_ptr(), std::ptr::null_mut()];
        meter.note_host_channels(&inputs, &outputs);
        let state = meter.state();
        assert_eq!(state.shared_channels.load(Ordering::Relaxed), 0b001);
        assert_eq!(state.copied_channels.load(Ordering::Relaxed), 0b010);
        assert_eq!(state.own_channels.load(Ordering::Relaxed), 0b100);
    }

    #[test]
    fn test_reported_values_keep_smallest_largest_and_last() {
        let state = DiagnosticsState::new();
        state.report_value("gain reduction dB", 3.0);
        state.report_value("gain reduction dB", -1.0);
        state.report_value("gain reduction dB", 2.0);
        state.report_value("other", 5.0);
        let slot = &state.values[0];
        assert_eq!(slot.name(), Some("gain reduction dB"));
        assert_eq!(from_ordered_bits(slot.smallest.load(Ordering::Relaxed)), -1.0);
        assert_eq!(from_ordered_bits(slot.largest.load(Ordering::Relaxed)), 3.0);
        assert_eq!(f32::from_bits(slot.last.load(Ordering::Relaxed)), 2.0);
        assert_eq!(state.values[1].name(), Some("other"));
    }

    #[test]
    fn test_values_beyond_the_slots_are_dropped() {
        const NAMES: [&str; VALUE_SLOTS + 1] = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        let state = DiagnosticsState::new();
        for name in NAMES {
            state.report_value(name, 1.0);
        }
        assert!(state.values.iter().all(|slot| slot.name().is_some()));
        assert!(state.values.iter().all(|slot| slot.name() != Some("i")));
    }

    #[test]
    fn test_sysex_keeps_head_and_tail() {
        let state = DiagnosticsState::new();
        let message: Vec<u8> = (0..40).collect();
        state.note_sysex_input(&[0xf0, 0x7e, 0xf7]);
        state.note_sysex_input(&message);
        let (count, len, bytes) = state.sysex.take().unwrap();
        assert_eq!((count, len), (2, 40));
        assert_eq!(&bytes[..SYSEX_HEAD], &message[..SYSEX_HEAD]);
        assert_eq!(&bytes[SYSEX_HEAD..], &message[40 - SYSEX_TAIL..]);
        assert!(state.sysex.take().is_none());
    }

    #[test]
    fn test_report_tells_how_the_host_sent_midi() {
        let mut meter = meter();
        let mut memory = ReportMemory::default();
        meter.begin_block(64, None);
        meter.end_block();
        let state = meter.state();
        state.note_host_midi(HostMidiForm::Event);
        state.note_host_midi(HostMidiForm::SysEx);
        state.note_host_midi(HostMidiForm::EventList(1));
        state.note_host_midi(HostMidiForm::EventList(2));
        state.note_host_midi(HostMidiForm::PacketTimeStamp(0));
        state.note_host_midi(HostMidiForm::PacketTimeStamp(17));
        state.note_host_midi(HostMidiForm::PacketTimeStamp(5));
        state.note_host_midi(HostMidiForm::Ump(2));
        state.note_host_midi(HostMidiForm::Ump(3));
        state.note_host_midi(HostMidiForm::Ump(3));
        state.note_host_midi(HostMidiForm::Truncated);
        let lines = report_lines(state, &mut memory);
        assert_eq!(
            lines.last().map(String::as_str),
            Some(
                "host MIDI: 1 events, 1 SysEx events, 2 event lists (MIDI 1.0: 1, MIDI 2.0: 1); \
                 UMP type 2: 1, type 3: 2; 2 packets with time stamps, up to 17; 1 packets cut short"
            )
        );
        // Read once: nothing new, no line.
        meter.begin_block(64, None);
        meter.end_block();
        let lines = report_lines(meter.state(), &mut memory);
        assert!(lines.iter().all(|line| !line.starts_with("host MIDI")));
    }

    #[test]
    fn test_report_counts_dropped_sysex() {
        let mut meter = meter();
        let mut memory = ReportMemory::default();
        meter.begin_block(64, None);
        meter.end_block();
        meter.state().note_sysex_input(&[0xf0, 0x7e, 0xf7]);
        meter.state().note_sysex_input(&[0x41]);
        meter.state().note_sysex_dropped(1);
        let lines = report_lines(meter.state(), &mut memory);
        assert_eq!(lines.last().map(String::as_str), Some("SysEx in: 2 messages (1 dropped), last 1 bytes: 41"));

        // Messages dropped before their bytes were noted.
        meter.begin_block(64, None);
        meter.end_block();
        meter.state().note_sysex_dropped(2);
        let lines = report_lines(meter.state(), &mut memory);
        assert_eq!(lines.last().map(String::as_str), Some("SysEx in: 2 dropped"));
    }

    #[test]
    fn test_report_summarizes_and_resets() {
        let mut meter = meter();
        let mut memory = ReportMemory::default();
        let mut buffer = sine(100.0, 0.5, 0, 256);
        let inputs = [buffer.as_ptr()];
        let outputs = [buffer.as_mut_ptr()];
        meter.note_host_channels(&inputs, &outputs);
        meter.begin_block(256, Some(false));
        meter.main_input(0, &buffer);
        meter.aux_input(1, &[0.0f32; 256]);
        meter.main_output(0, &buffer);
        meter.end_block();
        meter.state().note_aux_input_present(1);
        meter.state().report_value("gain", 1.5);

        let lines = report_lines(meter.state(), &mut memory);
        assert_eq!(
            lines,
            [
                "1 blocks of 256, in -6.0 dB, out -6.0 dB, aux in 1 -inf dB, in place 1 of 1, gain 1.50 to 1.50 (last 1.50)",
                "aux in 1 has input",
            ]
        );

        // Nothing new: the rendering stopped, once.
        assert_eq!(report_lines(meter.state(), &mut memory), ["render calls stopped"]);
        assert!(report_lines(meter.state(), &mut memory).is_empty());
    }

    #[test]
    fn test_report_logs_a_missing_aux_input_once() {
        let mut meter = meter();
        let mut memory = ReportMemory::default();
        meter.begin_block(64, None);
        meter.end_block();
        meter.state().note_aux_input_missing(1, -10867);
        let lines = report_lines(meter.state(), &mut memory);
        assert_eq!(lines, ["1 blocks of 64", "aux in 1 has no input in 1 of 1 blocks (status -10867)"]);

        meter.begin_block(64, None);
        meter.end_block();
        meter.state().note_aux_input_missing(1, -10867);
        assert_eq!(report_lines(meter.state(), &mut memory), ["1 blocks of 64"]);
    }

    #[test]
    fn test_package_name_comes_from_the_subsystem() {
        let mut config = Config::new("Beamer Gain", crate::config::Category::Effect, "Bmer", "gain");
        config.log_subsystem = "com.beamer.gain";
        assert_eq!(package_name(&config), "gain");
        config.log_subsystem = "";
        assert_eq!(package_name(&config), "beamer-gain");
    }

    #[test]
    fn test_diagnostics_stop_their_reporter() {
        let config = Config::new("Beamer Gain", crate::config::Category::Effect, "Bmer", "gain");
        let diagnostics = Diagnostics::start(&config, "au");
        diagnostics.state().note_output_bus_call(0);
        drop(diagnostics);
    }
}
