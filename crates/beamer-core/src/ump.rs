//! Universal MIDI Packets: reading them into MIDI events and writing events
//! as them.
//!
//! An Audio Unit host may send MIDI as event lists of Universal MIDI
//! Packets (UMP), in MIDI 1.0 or MIDI 2.0. This module walks a packet's
//! words message by message, turns MIDI 1.0 and MIDI 2.0 channel voice
//! messages into [`MidiEvent`]s and puts SysEx7 packets together into whole
//! messages. The other way, it writes events as MIDI 1.0 bytes or as UMP
//! messages in either protocol. It knows nothing of Audio Units.

use crate::config::MidiProtocol;
use crate::midi::{
    cc, combine_14bit_raw, note_expression, note_id, note_id_parts, scale, split_14bit_raw, tuning_semitones,
    tuning_value, MidiEvent, MidiEventKind, NoteExpressionValue, ParameterChange, ParameterNumber,
    ParameterNumberKind,
};

/// UMP message types (the top four bits of a message's first word).
pub mod message_type {
    /// Utility: no-op, jitter reduction time stamps, delta clock stamps.
    pub const UTILITY: u8 = 0x0;
    /// System common and real time messages (clock, start, stop and the like).
    pub const SYSTEM: u8 = 0x1;
    /// MIDI 1.0 channel voice messages.
    pub const MIDI1_CHANNEL_VOICE: u8 = 0x2;
    /// 7-bit SysEx, up to six bytes per packet.
    pub const SYSEX7: u8 = 0x3;
    /// MIDI 2.0 channel voice messages.
    pub const MIDI2_CHANNEL_VOICE: u8 = 0x4;
    /// 8-bit SysEx and mixed data sets.
    pub const DATA128: u8 = 0x5;
    /// Flex data: tempo, time signature, chord names, lyrics and the like.
    pub const FLEX_DATA: u8 = 0xD;
    /// UMP stream messages: endpoint and function block discovery.
    pub const STREAM: u8 = 0xF;
}

/// The number of words of the message that starts with `first_word`, by
/// its message type. Reserved types have the length the UMP specification
/// gives them, so that a reader can skip them.
#[inline]
pub const fn message_words(first_word: u32) -> usize {
    match first_word >> 28 {
        0x0..=0x2 | 0x6 | 0x7 => 1,
        0x3 | 0x4 | 0x8..=0xA => 2,
        0xB | 0xC => 3,
        _ => 4,
    }
}

/// The message type of a message's first word.
#[inline]
pub const fn message_type_of(first_word: u32) -> u8 {
    (first_word >> 28) as u8
}

/// The group (0 to 15) of a message's first word.
#[inline]
pub const fn group_of(first_word: u32) -> u8 {
    ((first_word >> 24) & 0x0F) as u8
}

/// The words of a packet ended inside a message: the message is shorter
/// than its type says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Truncated;

/// The messages of a packet's words, each as its words.
///
/// A packet that ends inside a message yields [`Truncated`] once, for the
/// rest of its words; then the iteration ends.
#[derive(Debug, Clone)]
pub struct Messages<'a> {
    words: &'a [u32],
}

impl<'a> Messages<'a> {
    /// The messages of `words`.
    pub fn new(words: &'a [u32]) -> Self {
        Self { words }
    }
}

impl<'a> Iterator for Messages<'a> {
    type Item = Result<&'a [u32], Truncated>;

    fn next(&mut self) -> Option<Self::Item> {
        let &first = self.words.first()?;
        let len = message_words(first);
        if len > self.words.len() {
            self.words = &[];
            return Some(Err(Truncated));
        }
        let (message, rest) = self.words.split_at(len);
        self.words = rest;
        Some(Ok(message))
    }
}

// =============================================================================
// Channel voice messages
// =============================================================================

/// What a channel voice message becomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChannelVoice {
    /// An event for the plugin.
    Event(MidiEvent<'static>),
    /// A message Beamer has no event for: MIDI 2.0 per-note management,
    /// or a status no specification defines.
    Unsupported,
}

/// A MIDI 1.0 channel voice message (type 2) as an event.
pub fn midi1_channel_voice(sample_offset: u32, word: u32) -> ChannelVoice {
    let status = ((word >> 16) & 0xF0) as u8;
    let channel = ((word >> 16) & 0x0F) as u8;
    let data1 = ((word >> 8) & 0x7F) as u8;
    let data2 = (word & 0x7F) as u8;
    match MidiEvent::from_midi1_bytes(sample_offset, group_of(word), status, channel, data1, data2) {
        Some(event) => ChannelVoice::Event(event),
        None => ChannelVoice::Unsupported,
    }
}

/// MIDI 2.0 channel voice statuses (the high nibble of the status byte).
mod status {
    pub const REGISTERED_PER_NOTE_CONTROLLER: u8 = 0x0;
    pub const ASSIGNABLE_PER_NOTE_CONTROLLER: u8 = 0x1;
    pub const REGISTERED_CONTROLLER: u8 = 0x2;
    pub const ASSIGNABLE_CONTROLLER: u8 = 0x3;
    pub const RELATIVE_REGISTERED_CONTROLLER: u8 = 0x4;
    pub const RELATIVE_ASSIGNABLE_CONTROLLER: u8 = 0x5;
    pub const PER_NOTE_PITCH_BEND: u8 = 0x6;
    pub const NOTE_OFF: u8 = 0x8;
    pub const NOTE_ON: u8 = 0x9;
    pub const POLY_PRESSURE: u8 = 0xA;
    pub const CONTROL_CHANGE: u8 = 0xB;
    pub const PROGRAM_CHANGE: u8 = 0xC;
    pub const CHANNEL_PRESSURE: u8 = 0xD;
    pub const PITCH_BEND: u8 = 0xE;
}

/// MIDI 2.0 registered per-note controllers that have a note expression
/// type of the same meaning.
mod registered_per_note {
    pub const PITCH_7_25: u8 = 3;
    pub const VOLUME: u8 = 7;
    pub const PAN: u8 = 10;
    pub const EXPRESSION: u8 = 11;
    pub const BRIGHTNESS: u8 = 74;
    pub const VIBRATO_DEPTH: u8 = 77;
}

/// The note attribute type of a pitch in 7.9 fixed point.
const ATTRIBUTE_PITCH_7_9: u8 = 3;

/// A bipolar 32-bit value (center 0x80000000) as 0.0 to 1.0 with its
/// center at exactly 0.5, as VST3's bipolar note expressions.
fn centered(value: u32) -> f64 {
    (scale::bipolar_from_u32(value) + 1.0) / 2.0
}

/// A MIDI 2.0 channel voice message (type 4, two words) as an event, at
/// full resolution.
///
/// | Status | Event |
/// |---|---|
/// | note off, note on | [`NoteOff`](crate::NoteOff), [`NoteOn`](crate::NoteOn): 16-bit velocity; the pitch 7.9 attribute as `tuning` |
/// | poly pressure, control change, channel pressure, pitch bend | the event of MIDI 1.0, 32 bits |
/// | program change | [`ProgramChange`](crate::ProgramChange), with the bank if the message carries one |
/// | registered, assignable controller | [`ParameterNumber`], absolute |
/// | relative registered, assignable controller | [`ParameterNumber`], relative |
/// | registered, assignable per-note controller; per-note pitch bend | [`NoteExpressionValue`] (see [`note_expression`]) |
/// | per-note management | [`ChannelVoice::Unsupported`] |
///
/// A note-on with velocity 0 is a note-on in MIDI 2.0: it gets the smallest
/// velocity, 1 / 65535. Note IDs are [`note_id`] of the group, channel and
/// note.
pub fn midi2_channel_voice(sample_offset: u32, words: [u32; 2]) -> ChannelVoice {
    let [first, data] = words;
    let group = group_of(first);
    let status = ((first >> 20) & 0x0F) as u8;
    let channel = ((first >> 16) & 0x0F) as u8;
    // The note, controller or bank; then the index or attribute type.
    let byte3 = ((first >> 8) & 0xFF) as u8;
    let byte4 = (first & 0xFF) as u8;
    let note = byte3 & 0x7F;
    let id = note_id(group, channel, note);
    let note_expression = |expression_type: u32, value: f64| {
        MidiEvent {
            sample_offset,
            event: MidiEventKind::NoteExpressionValue(NoteExpressionValue {
                note_id: id,
                expression_type,
                value,
            }),
        }
    };
    let parameter_number = |kind: ParameterNumberKind, change: ParameterChange| {
        MidiEvent::parameter_number(
            sample_offset,
            ParameterNumber {
                channel,
                kind,
                parameter: combine_14bit_raw(byte3 & 0x7F, byte4 & 0x7F),
                change,
            },
        )
    };
    let event = match status {
        status::NOTE_OFF | status::NOTE_ON => {
            let velocity = (data >> 16) as u16;
            let tuning = if byte4 == ATTRIBUTE_PITCH_7_9 {
                // The pitch in semitones, 7 bits and a 9-bit fraction.
                let pitch = (data & 0xFFFF) as f32 / 512.0;
                (pitch - note as f32) * 100.0
            } else {
                0.0
            };
            if status == status::NOTE_ON {
                let velocity = scale::velocity_from_u16(velocity.max(1));
                MidiEvent::note_on(sample_offset, channel, note, velocity, id, tuning, 0)
            } else {
                MidiEvent::note_off(sample_offset, channel, note, scale::velocity_from_u16(velocity), id, tuning)
            }
        }
        status::POLY_PRESSURE => {
            MidiEvent::poly_pressure(sample_offset, channel, note, scale::unipolar_from_u32(data), id)
        }
        status::CONTROL_CHANGE => {
            MidiEvent::control_change(sample_offset, channel, byte3 & 0x7F, scale::unipolar_from_u32(data))
        }
        status::PROGRAM_CHANGE => {
            let program = ((data >> 24) & 0x7F) as u8;
            // Option flag 1: the bank is valid.
            let bank = (byte4 & 0x01 != 0)
                .then(|| combine_14bit_raw(((data >> 8) & 0x7F) as u8, (data & 0x7F) as u8));
            MidiEvent::program_change(sample_offset, channel, program, bank)
        }
        status::CHANNEL_PRESSURE => {
            MidiEvent::channel_pressure(sample_offset, channel, scale::unipolar_from_u32(data))
        }
        status::PITCH_BEND => MidiEvent::pitch_bend(sample_offset, channel, scale::bipolar_from_u32(data)),
        status::REGISTERED_CONTROLLER => parameter_number(
            ParameterNumberKind::Rpn,
            ParameterChange::Absolute(scale::unipolar_from_u32(data)),
        ),
        status::ASSIGNABLE_CONTROLLER => parameter_number(
            ParameterNumberKind::Nrpn,
            ParameterChange::Absolute(scale::unipolar_from_u32(data)),
        ),
        status::RELATIVE_REGISTERED_CONTROLLER => parameter_number(
            ParameterNumberKind::Rpn,
            ParameterChange::Relative(data as i32 as f64 / u32::MAX as f64),
        ),
        status::RELATIVE_ASSIGNABLE_CONTROLLER => parameter_number(
            ParameterNumberKind::Nrpn,
            ParameterChange::Relative(data as i32 as f64 / u32::MAX as f64),
        ),
        status::REGISTERED_PER_NOTE_CONTROLLER => {
            let value = scale::unipolar_from_u32(data);
            match byte4 {
                registered_per_note::PITCH_7_25 => {
                    // The pitch in semitones, 7 bits and a 25-bit fraction.
                    let pitch = data as f64 / (1u32 << 25) as f64;
                    note_expression(note_expression::TUNING, tuning_value(pitch - note as f64))
                }
                // The curve of CC 7 (40 log10 v dB) on VST3's volume scale
                // (20 log10 4v dB).
                registered_per_note::VOLUME => note_expression(note_expression::VOLUME, value * value / 4.0),
                registered_per_note::PAN => note_expression(note_expression::PAN, centered(data)),
                registered_per_note::EXPRESSION => note_expression(note_expression::EXPRESSION, value),
                registered_per_note::BRIGHTNESS => note_expression(note_expression::BRIGHTNESS, value),
                registered_per_note::VIBRATO_DEPTH => note_expression(note_expression::VIBRATO, value),
                index => note_expression(note_expression::registered(index), value),
            }
        }
        status::ASSIGNABLE_PER_NOTE_CONTROLLER => {
            note_expression(note_expression::assignable(byte4), scale::unipolar_from_u32(data))
        }
        status::PER_NOTE_PITCH_BEND => note_expression(note_expression::PITCH_BEND, centered(data)),
        _ => return ChannelVoice::Unsupported,
    };
    ChannelVoice::Event(event)
}

// =============================================================================
// SysEx7
// =============================================================================

/// SysEx7 packet statuses.
mod sysex7_status {
    pub const COMPLETE: u8 = 0x0;
    pub const START: u8 = 0x1;
    pub const CONTINUE: u8 = 0x2;
    pub const END: u8 = 0x3;
}

/// What a SysEx7 packet did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysEx7Step<'a> {
    /// A message that is whole now, F0 to F7, valid until the next packet.
    pub complete: Option<&'a [u8]>,
    /// Messages dropped with this packet: malformed, without a start,
    /// interrupted by another start or complete message, too long or of
    /// another group while one is open.
    pub dropped: u32,
}

/// Puts the packets of SysEx7 messages (UMP type 3) together into whole
/// messages, framed by F0 and F7.
///
/// One message is in progress at a time, kept across packets, events and
/// render calls until it ends. A message is dropped and counted when:
///
/// - a packet is malformed: a status above 3, a byte count above 6, a data
///   byte above 0x7F (the message it belongs to goes with it, its other
///   packets are skipped);
/// - it continues or ends without a start (counted once per message);
/// - a start or a complete message comes while one is open (the open one
///   is dropped, the new one taken);
/// - it starts on another group while one is open (UMP lets groups
///   interleave; Beamer keeps one in progress): the newcomer is dropped,
///   its packets skipped;
/// - it is longer than the buffer: the rest is skipped until its end.
///
/// The buffer is allocated once ([`new`](Self::new)); nothing allocates
/// after that.
#[derive(Debug)]
pub struct SysEx7Assembler {
    bytes: Box<[u8]>,
    len: usize,
    /// The group of the message in progress.
    open: Option<u8>,
    /// The message in progress grew past the buffer.
    overflowed: bool,
    /// The groups of messages being skipped (dropped and counted already),
    /// a bit per group.
    skipping: u16,
}

impl SysEx7Assembler {
    /// An assembler for messages of up to `capacity` bytes, F0 and F7
    /// included.
    pub fn new(capacity: usize) -> Self {
        Self {
            bytes: vec![0; capacity].into_boxed_slice(),
            len: 0,
            open: None,
            overflowed: false,
            skipping: 0,
        }
    }

    /// Drop the message in progress, without counting it.
    pub fn reset(&mut self) {
        self.len = 0;
        self.open = None;
        self.overflowed = false;
        self.skipping = 0;
    }

    /// Whether a message is in progress.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Take one SysEx7 packet (its two words).
    pub fn push(&mut self, words: [u32; 2]) -> SysEx7Step<'_> {
        let [first, second] = words;
        let group = group_of(first);
        let status = ((first >> 20) & 0x0F) as u8;
        let count = ((first >> 16) & 0x0F) as usize;
        let data = [
            (first >> 8) as u8,
            first as u8,
            (second >> 24) as u8,
            (second >> 16) as u8,
            (second >> 8) as u8,
            second as u8,
        ];
        let mut dropped = 0;
        let bit = 1u16 << group;
        // A start or continue leaves packets of its message to come.
        let more_to_come = matches!(status, sysex7_status::START | sysex7_status::CONTINUE);

        // The packets of a message that was dropped already. A group being
        // skipped has no open message.
        if self.skipping & bit != 0 {
            match status {
                sysex7_status::CONTINUE => return SysEx7Step { complete: None, dropped },
                sysex7_status::END => {
                    self.skipping &= !bit;
                    return SysEx7Step { complete: None, dropped };
                }
                // A new message on that group ends the skipping.
                _ => self.skipping &= !bit,
            }
        }

        let malformed = status > sysex7_status::END
            || count > data.len()
            || data[..count.min(data.len())].iter().any(|&byte| byte > 0x7F);
        if malformed {
            if self.open == Some(group) {
                self.drop_open();
            }
            if more_to_come {
                self.skipping |= bit;
            }
            return SysEx7Step { complete: None, dropped: 1 };
        }
        let data = &data[..count];

        // Another group while a message is open: the newcomer goes.
        if let Some(open) = self.open {
            if open != group {
                if more_to_come {
                    self.skipping |= bit;
                }
                return SysEx7Step { complete: None, dropped: 1 };
            }
        }

        match status {
            sysex7_status::COMPLETE | sysex7_status::START => {
                if self.open.is_some() {
                    self.drop_open();
                    dropped += 1;
                }
                self.open = Some(group);
                self.len = 0;
                self.overflowed = false;
                self.append(&[0xF0]);
                self.append(data);
                if status == sysex7_status::COMPLETE {
                    return self.finish(dropped);
                }
                SysEx7Step { complete: None, dropped }
            }
            sysex7_status::CONTINUE | sysex7_status::END => {
                if self.open.is_none() {
                    if more_to_come {
                        self.skipping |= bit;
                    }
                    return SysEx7Step { complete: None, dropped: dropped + 1 };
                }
                self.append(data);
                if status == sysex7_status::END {
                    return self.finish(dropped);
                }
                SysEx7Step { complete: None, dropped }
            }
            _ => unreachable!("statuses above 3 are malformed"),
        }
    }

    /// Add bytes to the message in progress, keeping room for its F7.
    fn append(&mut self, data: &[u8]) {
        if self.overflowed {
            return;
        }
        let end = self.len + data.len();
        if end + 1 > self.bytes.len() {
            self.overflowed = true;
            return;
        }
        self.bytes[self.len..end].copy_from_slice(data);
        self.len = end;
    }

    /// End the message in progress: whole, or dropped if it was too long.
    fn finish(&mut self, dropped: u32) -> SysEx7Step<'_> {
        self.open = None;
        if self.overflowed {
            self.overflowed = false;
            return SysEx7Step { complete: None, dropped: dropped + 1 };
        }
        self.bytes[self.len] = 0xF7;
        self.len += 1;
        SysEx7Step {
            complete: Some(&self.bytes[..self.len]),
            dropped,
        }
    }

    fn drop_open(&mut self) {
        self.open = None;
        self.len = 0;
        self.overflowed = false;
    }
}

// =============================================================================
// Encoding
// =============================================================================

/// The MIDI 1.0 messages of an event, as bytes, each handed to `emit`.
///
/// Most events are one message. A parameter number change is its selection
/// and data entry control changes, a program change with a bank is bank
/// select (CC 0 and 32) before it. Values are scaled down as MIDI 2.0 does
/// (see [`scale`]); a note-on keeps a velocity of at least 1. SysEx and the
/// events without a MIDI 1.0 form (note expressions, chord and scale info)
/// emit nothing.
pub fn midi1_messages(event: &MidiEventKind, mut emit: impl FnMut(&[u8])) {
    let control_change =
        |channel: u8, controller: u8, value: u8| [0xB0 | (channel & 0x0F), controller & 0x7F, value & 0x7F];
    match event {
        MidiEventKind::NoteOn(note) => emit(&[
            0x90 | (note.channel & 0x0F),
            note.pitch & 0x7F,
            // A Note On with velocity 0 means Note Off on the wire.
            scale::velocity_to_7bit(note.velocity).max(1),
        ]),
        MidiEventKind::NoteOff(note) => emit(&[
            0x80 | (note.channel & 0x0F),
            note.pitch & 0x7F,
            scale::velocity_to_7bit(note.velocity),
        ]),
        MidiEventKind::PolyPressure(pressure) => emit(&[
            0xA0 | (pressure.channel & 0x0F),
            pressure.pitch & 0x7F,
            scale::to_7bit(pressure.pressure),
        ]),
        MidiEventKind::ControlChange(change) => {
            emit(&control_change(change.channel, change.controller, scale::to_7bit(change.value)))
        }
        MidiEventKind::ProgramChange(program) => {
            if let Some(bank) = program.bank {
                let (msb, lsb) = split_14bit_raw(bank);
                emit(&control_change(program.channel, cc::BANK_SELECT_MSB, msb));
                emit(&control_change(program.channel, cc::BANK_SELECT_LSB, lsb));
            }
            emit(&[0xC0 | (program.channel & 0x0F), program.program & 0x7F]);
        }
        MidiEventKind::ChannelPressure(pressure) => {
            emit(&[0xD0 | (pressure.channel & 0x0F), scale::to_7bit(pressure.pressure)])
        }
        MidiEventKind::PitchBend(bend) => {
            let (msb, lsb) = split_14bit_raw(scale::bipolar_to_14bit(bend.value));
            emit(&[0xE0 | (bend.channel & 0x0F), lsb, msb]);
        }
        MidiEventKind::ParameterNumber(change) => {
            let (changes, count) = change.midi1_control_changes();
            for &(controller, value) in &changes[..count] {
                emit(&control_change(change.channel, controller, value));
            }
        }
        MidiEventKind::SysEx(_)
        | MidiEventKind::NoteExpressionValue(_)
        | MidiEventKind::NoteExpressionInt(_)
        | MidiEventKind::NoteExpressionText(_)
        | MidiEventKind::ChordInfo(_)
        | MidiEventKind::ScaleInfo(_) => {}
    }
}

/// The UMP messages of an event in `protocol`, on group 0, each handed to
/// `emit` as its words.
///
/// MIDI 1.0: the messages of [`midi1_messages`] as MIDI 1.0 channel voice
/// words. MIDI 2.0: MIDI 2.0 channel voice messages at full resolution, the
/// inverse of [`midi2_channel_voice`]: a tuning other than 0 goes out as the
/// pitch 7.9 attribute; a parameter number change as a registered or
/// assignable controller (relative for a step); a note expression of a note
/// whose ID [`note_id`] made as its per-note message (a note expression of
/// another type or note has no MIDI 2.0 form). SysEx goes out through
/// [`sysex7_packets`], not here.
pub fn ump_messages(event: &MidiEventKind, protocol: MidiProtocol, mut emit: impl FnMut(&[u32])) {
    match protocol {
        MidiProtocol::Midi1 => midi1_messages(event, |bytes| {
            let data1 = bytes.get(1).copied().unwrap_or(0) as u32;
            let data2 = bytes.get(2).copied().unwrap_or(0) as u32;
            emit(&[0x2000_0000 | (bytes[0] as u32) << 16 | data1 << 8 | data2]);
        }),
        MidiProtocol::Midi2 => {
            if let Some(words) = midi2_message(event) {
                emit(&words);
            }
            // A program change with a bank and a parameter number change
            // are one message each in MIDI 2.0.
        }
    }
}

/// The first word of a MIDI 2.0 channel voice message on group 0.
fn midi2_word(status: u8, channel: u8, byte3: u8, byte4: u8) -> u32 {
    0x4000_0000 | (status as u32 & 0x0F) << 20 | (channel as u32 & 0x0F) << 16 | (byte3 as u32) << 8 | byte4 as u32
}

/// The inverse of `centered`: 0.0 to 1.0 with the center at 0.5 as a
/// bipolar 32-bit value.
fn uncentered(value: f64) -> u32 {
    scale::bipolar_to_u32(value * 2.0 - 1.0)
}

/// An event as one MIDI 2.0 channel voice message, if it has one.
fn midi2_message(event: &MidiEventKind) -> Option<[u32; 2]> {
    // A note's pitch 7.9 attribute for a tuning in cents.
    let note_attribute = |pitch: u8, tuning: f32| -> (u8, u16) {
        if tuning == 0.0 {
            return (0, 0);
        }
        let pitch_7_9 = ((pitch as f64 + tuning as f64 / 100.0) * 512.0).round();
        (ATTRIBUTE_PITCH_7_9, pitch_7_9.clamp(0.0, u16::MAX as f64) as u16)
    };
    let words = match event {
        MidiEventKind::NoteOn(note) => {
            let (attribute, data) = note_attribute(note.pitch, note.tuning);
            [
                midi2_word(status::NOTE_ON, note.channel, note.pitch & 0x7F, attribute),
                // A note-on with velocity 0 is a note-on in MIDI 2.0, but
                // Beamer's never has one: keep it above 0.
                (scale::velocity_to_u16(note.velocity).max(1) as u32) << 16 | data as u32,
            ]
        }
        MidiEventKind::NoteOff(note) => {
            let (attribute, data) = note_attribute(note.pitch, note.tuning);
            [
                midi2_word(status::NOTE_OFF, note.channel, note.pitch & 0x7F, attribute),
                (scale::velocity_to_u16(note.velocity) as u32) << 16 | data as u32,
            ]
        }
        MidiEventKind::PolyPressure(pressure) => [
            midi2_word(status::POLY_PRESSURE, pressure.channel, pressure.pitch & 0x7F, 0),
            scale::unipolar_to_u32(pressure.pressure),
        ],
        MidiEventKind::ControlChange(change) => [
            midi2_word(status::CONTROL_CHANGE, change.channel, change.controller & 0x7F, 0),
            scale::unipolar_to_u32(change.value),
        ],
        MidiEventKind::ProgramChange(program) => {
            let (flags, msb, lsb) = match program.bank {
                Some(bank) => {
                    let (msb, lsb) = split_14bit_raw(bank);
                    (0x01, msb, lsb)
                }
                None => (0, 0, 0),
            };
            [
                midi2_word(status::PROGRAM_CHANGE, program.channel, 0, flags),
                ((program.program & 0x7F) as u32) << 24 | (msb as u32) << 8 | lsb as u32,
            ]
        }
        MidiEventKind::ChannelPressure(pressure) => [
            midi2_word(status::CHANNEL_PRESSURE, pressure.channel, 0, 0),
            scale::unipolar_to_u32(pressure.pressure),
        ],
        MidiEventKind::PitchBend(bend) => {
            [midi2_word(status::PITCH_BEND, bend.channel, 0, 0), scale::bipolar_to_u32(bend.value)]
        }
        MidiEventKind::ParameterNumber(change) => {
            let (bank, index) = split_14bit_raw(change.parameter & 0x3FFF);
            let (status, data) = match (change.kind, change.change) {
                (ParameterNumberKind::Rpn, ParameterChange::Absolute(value)) => {
                    (status::REGISTERED_CONTROLLER, scale::unipolar_to_u32(value))
                }
                (ParameterNumberKind::Nrpn, ParameterChange::Absolute(value)) => {
                    (status::ASSIGNABLE_CONTROLLER, scale::unipolar_to_u32(value))
                }
                (ParameterNumberKind::Rpn, ParameterChange::Relative(step)) => {
                    (status::RELATIVE_REGISTERED_CONTROLLER, relative_to_u32(step))
                }
                (ParameterNumberKind::Nrpn, ParameterChange::Relative(step)) => {
                    (status::RELATIVE_ASSIGNABLE_CONTROLLER, relative_to_u32(step))
                }
            };
            [midi2_word(status, change.channel, bank, index), data]
        }
        MidiEventKind::NoteExpressionValue(expression) => {
            let (_, channel, pitch) = note_id_parts(expression.note_id)?;
            let value = expression.value;
            let (status, index, data) = match expression.expression_type {
                note_expression::TUNING => {
                    // Pitch 7.25: the note's pitch plus the tuning.
                    let pitch_7_25 = ((pitch as f64 + tuning_semitones(value)) * (1u32 << 25) as f64).round();
                    let data = pitch_7_25.clamp(0.0, u32::MAX as f64) as u32;
                    (status::REGISTERED_PER_NOTE_CONTROLLER, registered_per_note::PITCH_7_25, data)
                }
                note_expression::VOLUME => {
                    // The inverse of `v² / 4`.
                    let data = scale::unipolar_to_u32((value.clamp(0.0, 0.25) * 4.0).sqrt());
                    (status::REGISTERED_PER_NOTE_CONTROLLER, registered_per_note::VOLUME, data)
                }
                note_expression::PAN => {
                    (status::REGISTERED_PER_NOTE_CONTROLLER, registered_per_note::PAN, uncentered(value))
                }
                note_expression::EXPRESSION => (
                    status::REGISTERED_PER_NOTE_CONTROLLER,
                    registered_per_note::EXPRESSION,
                    scale::unipolar_to_u32(value),
                ),
                note_expression::BRIGHTNESS => (
                    status::REGISTERED_PER_NOTE_CONTROLLER,
                    registered_per_note::BRIGHTNESS,
                    scale::unipolar_to_u32(value),
                ),
                note_expression::VIBRATO => (
                    status::REGISTERED_PER_NOTE_CONTROLLER,
                    registered_per_note::VIBRATO_DEPTH,
                    scale::unipolar_to_u32(value),
                ),
                note_expression::PITCH_BEND => (status::PER_NOTE_PITCH_BEND, 0, uncentered(value)),
                expression_type
                    if (note_expression::REGISTERED_START..note_expression::ASSIGNABLE_START)
                        .contains(&expression_type) =>
                {
                    let index = (expression_type - note_expression::REGISTERED_START) as u8;
                    (status::REGISTERED_PER_NOTE_CONTROLLER, index, scale::unipolar_to_u32(value))
                }
                expression_type
                    if (note_expression::ASSIGNABLE_START..note_expression::PITCH_BEND)
                        .contains(&expression_type) =>
                {
                    let index = (expression_type - note_expression::ASSIGNABLE_START) as u8;
                    (status::ASSIGNABLE_PER_NOTE_CONTROLLER, index, scale::unipolar_to_u32(value))
                }
                _ => return None,
            };
            [midi2_word(status, channel, pitch, index), data]
        }
        MidiEventKind::SysEx(_)
        | MidiEventKind::NoteExpressionInt(_)
        | MidiEventKind::NoteExpressionText(_)
        | MidiEventKind::ChordInfo(_)
        | MidiEventKind::ScaleInfo(_) => return None,
    };
    Some(words)
}

/// A relative change as a signed 32-bit value: -0.5 to 0.5 of the range,
/// what a MIDI 2.0 relative controller carries (a larger step saturates).
fn relative_to_u32(step: f64) -> u32 {
    (step.clamp(-0.5, 0.5) * u32::MAX as f64).round() as i32 as u32
}

/// The SysEx7 packets of a SysEx message on group 0, each handed to `emit`
/// as its two words. `message` may hold its F0 and F7 or not; data bytes
/// above 0x7F are masked. An empty message is one complete packet.
pub fn sysex7_packets(message: &[u8], mut emit: impl FnMut([u32; 2])) {
    let message = message.strip_prefix(&[0xF0]).unwrap_or(message);
    let message = message.strip_suffix(&[0xF7]).unwrap_or(message);
    let chunks = message.len().div_ceil(6).max(1);
    for (index, chunk) in message.chunks(6).chain((message.is_empty()).then_some(&[][..])).enumerate() {
        let status = match (index, chunks) {
            (_, 1) => sysex7_status::COMPLETE,
            (0, _) => sysex7_status::START,
            (index, chunks) if index + 1 == chunks => sysex7_status::END,
            _ => sysex7_status::CONTINUE,
        };
        let mut bytes = [0u8; 6];
        for (byte, &data) in bytes.iter_mut().zip(chunk) {
            *byte = data & 0x7F;
        }
        emit([
            0x3000_0000
                | (status as u32) << 20
                | (chunk.len() as u32) << 16
                | (bytes[0] as u32) << 8
                | bytes[1] as u32,
            u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
        ]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::{NoteOff, NoteOn, ProgramChange, RELATIVE_14BIT_STEP};

    #[test]
    fn test_messages_are_walked_by_their_length() {
        // Utility, MIDI 1.0 note, SysEx7, MIDI 2.0 note, flex data (4),
        // reserved type 6 (1) and C (3).
        let words = [
            0x0000_0000, 0x2090_3C40, 0x3016_7E7F, 0x0601_1122, 0x4090_3C00, 0x8000_0000, 0xD000_0000, 1, 2, 3,
            0x6000_0000, 0xC000_0000, 4, 5,
        ];
        let lengths: Vec<usize> = Messages::new(&words).map(|message| message.unwrap().len()).collect();
        assert_eq!(lengths, [1, 1, 2, 2, 4, 1, 3]);
    }

    #[test]
    fn test_a_message_cut_short_ends_the_packet() {
        let words = [0x2090_3C40, 0x4090_3C00];
        let mut messages = Messages::new(&words);
        assert_eq!(messages.next(), Some(Ok(&words[..1])));
        assert_eq!(messages.next(), Some(Err(Truncated)));
        assert_eq!(messages.next(), None);
    }

    fn event(words: [u32; 2]) -> MidiEventKind<'static> {
        match midi2_channel_voice(7, words) {
            ChannelVoice::Event(event) => {
                assert_eq!(event.sample_offset, 7);
                event.event
            }
            ChannelVoice::Unsupported => panic!("{words:08x?} is unsupported"),
        }
    }

    #[test]
    fn test_midi2_notes_keep_their_resolution() {
        // Group 2, channel 3, note 62, velocity 0x1234, pitch 60.25.
        let MidiEventKind::NoteOn(on) = event([0x4293_3E03, 0x1234_7880]) else { panic!() };
        assert_eq!(
            on,
            NoteOn {
                channel: 3,
                pitch: 62,
                velocity: scale::velocity_from_u16(0x1234),
                note_id: note_id(2, 3, 62),
                tuning: -175.0,
                length: 0,
            }
        );
        // Velocity 0 is a note-on in MIDI 2.0.
        let MidiEventKind::NoteOn(on) = event([0x4093_3E00, 0x0000_0000]) else { panic!() };
        assert_eq!(on.velocity, 1.0 / 65535.0);
        // Another attribute type is not a pitch.
        let MidiEventKind::NoteOff(off) = event([0x4083_3E01, 0xFFFF_7880]) else { panic!() };
        assert_eq!(
            off,
            NoteOff { channel: 3, pitch: 62, velocity: 1.0, note_id: note_id(0, 3, 62), tuning: 0.0 }
        );
    }

    #[test]
    fn test_midi2_controllers_are_32_bits() {
        let MidiEventKind::ControlChange(change) = event([0x40B0_0100, 0x1234_5678]) else { panic!() };
        assert_eq!((change.controller, change.value), (1, 0x1234_5678 as f64 / u32::MAX as f64));
        let MidiEventKind::PitchBend(bend) = event([0x40E0_0000, 0xFFFF_FFFF]) else { panic!() };
        assert_eq!(bend.value, 1.0);
        let MidiEventKind::PitchBend(bend) = event([0x40E0_0000, 0x8000_0000]) else { panic!() };
        assert_eq!(bend.value, 0.0);
        let MidiEventKind::ChannelPressure(pressure) = event([0x40D0_0000, 0xFFFF_FFFF]) else { panic!() };
        assert_eq!(pressure.pressure, 1.0);
        let MidiEventKind::PolyPressure(pressure) = event([0x40A0_3C00, 0x0000_0000]) else { panic!() };
        assert_eq!((pressure.pitch, pressure.pressure), (60, 0.0));
    }

    #[test]
    fn test_midi2_program_change_with_and_without_a_bank() {
        assert_eq!(
            event([0x40C0_0001, 0x0500_0102]),
            MidiEventKind::ProgramChange(ProgramChange { channel: 0, program: 5, bank: Some((1 << 7) | 2) })
        );
        // Without the flag the bank bytes do not count (Apple fills them in).
        assert_eq!(
            event([0x40C0_0000, 0x0500_0102]),
            MidiEventKind::ProgramChange(ProgramChange { channel: 0, program: 5, bank: None })
        );
    }

    #[test]
    fn test_midi2_controllers_are_parameter_numbers() {
        let MidiEventKind::ParameterNumber(change) = event([0x4021_0000, 0x1800_0000]) else { panic!() };
        assert_eq!((change.channel, change.kind, change.parameter), (1, ParameterNumberKind::Rpn, 0));
        assert_eq!(change.pitch_bend_sensitivity(), Some((12, 0)));
        let MidiEventKind::ParameterNumber(change) = event([0x4030_0102, 0x0000_0000]) else { panic!() };
        assert_eq!((change.kind, change.parameter), (ParameterNumberKind::Nrpn, (1 << 7) | 2));
        let MidiEventKind::ParameterNumber(change) = event([0x4040_0000, (-(1i32 << 18)) as u32]) else { panic!() };
        let ParameterChange::Relative(step) = change.change else { panic!() };
        assert!((step + RELATIVE_14BIT_STEP).abs() < 1e-15);
    }

    #[test]
    fn test_midi2_per_note_messages_are_note_expressions() {
        let expression = |words| match event(words) {
            MidiEventKind::NoteExpressionValue(value) => (value.note_id, value.expression_type, value.value),
            other => panic!("{other:?}"),
        };
        // Pitch 7.25 of 62.5 on note 60: 2.5 semitones.
        let pitch = (62u32 << 25) | (1 << 24);
        assert_eq!(expression([0x4000_3C03, pitch]), (60, note_expression::TUNING, tuning_value(2.5)));
        assert_eq!(expression([0x4000_3C07, 0xFFFF_FFFF]), (60, note_expression::VOLUME, 0.25));
        assert_eq!(expression([0x4000_3C0A, 0x8000_0000]), (60, note_expression::PAN, 0.5));
        assert_eq!(expression([0x4000_3C4A, 0]), (60, note_expression::BRIGHTNESS, 0.0));
        assert_eq!(expression([0x4000_3C01, 0]), (60, note_expression::registered(1), 0.0));
        assert_eq!(expression([0x4010_3C05, 0]), (60, note_expression::assignable(5), 0.0));
        assert_eq!(expression([0x4060_3C00, 0x8000_0000]), (60, note_expression::PITCH_BEND, 0.5));
        assert_eq!(expression([0x4060_3C00, 0xFFFF_FFFF]), (60, note_expression::PITCH_BEND, 1.0));
        assert_eq!(midi2_channel_voice(0, [0x40F0_3C01, 0]), ChannelVoice::Unsupported);
    }

    #[test]
    fn test_midi1_words_are_midi1_messages() {
        let ChannelVoice::Event(event) = midi1_channel_voice(3, 0x2591_3C40) else { panic!() };
        let MidiEventKind::NoteOn(on) = event.event else { panic!() };
        assert_eq!((on.channel, on.pitch, on.note_id), (1, 60, note_id(5, 1, 60)));
        assert_eq!(midi1_channel_voice(0, 0x20F8_0000), ChannelVoice::Unsupported);
    }

    /// A SysEx7 packet of `status` on `group` with `data`.
    fn packet(group: u8, status: u8, data: &[u8]) -> [u32; 2] {
        let mut bytes = [0u8; 6];
        bytes[..data.len()].copy_from_slice(data);
        [
            0x3000_0000 | (group as u32) << 24 | (status as u32) << 20 | (data.len() as u32) << 16
                | (bytes[0] as u32) << 8 | bytes[1] as u32,
            u32::from_be_bytes([bytes[2], bytes[3], bytes[4], bytes[5]]),
        ]
    }

    /// Push packets; the whole messages and the dropped count.
    fn assemble(assembler: &mut SysEx7Assembler, packets: &[[u32; 2]]) -> (Vec<Vec<u8>>, u32) {
        let mut complete = Vec::new();
        let mut dropped = 0;
        for &words in packets {
            let step = assembler.push(words);
            dropped += step.dropped;
            complete.extend(step.complete.map(<[u8]>::to_vec));
        }
        (complete, dropped)
    }

    #[test]
    fn test_sysex7_complete_and_split_messages() {
        let mut assembler = SysEx7Assembler::new(64);
        let (complete, dropped) = assemble(
            &mut assembler,
            &[
                packet(0, 0, &[0x7E, 0x7F, 0x06, 0x01]),
                packet(0, 1, &[0x7D, 1, 2, 3, 4, 5]),
                packet(0, 2, &[6, 7, 8, 9, 10, 11]),
                packet(0, 3, &[12]),
                packet(0, 0, &[]),
            ],
        );
        assert_eq!(dropped, 0);
        assert_eq!(
            complete,
            [
                vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7],
                vec![0xF0, 0x7D, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0xF7],
                vec![0xF0, 0xF7],
            ]
        );
    }

    #[test]
    fn test_sysex7_waits_across_calls_until_its_end() {
        let mut assembler = SysEx7Assembler::new(64);
        assert_eq!(assemble(&mut assembler, &[packet(1, 1, &[1, 2])]), (vec![], 0));
        assert!(assembler.is_open());
        assert_eq!(assemble(&mut assembler, &[packet(1, 3, &[3])]), (vec![vec![0xF0, 1, 2, 3, 0xF7]], 0));
        assert!(!assembler.is_open());
    }

    #[test]
    fn test_sysex7_that_does_not_fit_is_dropped_whole() {
        // Room for F0, 6 bytes and F7.
        let mut assembler = SysEx7Assembler::new(8);
        let fits = [packet(0, 1, &[1, 2, 3]), packet(0, 3, &[4, 5, 6])];
        assert_eq!(assemble(&mut assembler, &fits).0, [vec![0xF0, 1, 2, 3, 4, 5, 6, 0xF7]]);
        let too_long = [packet(0, 1, &[1, 2, 3]), packet(0, 2, &[4, 5, 6]), packet(0, 3, &[7])];
        assert_eq!(assemble(&mut assembler, &too_long), (vec![], 1));
        // The next message is taken again.
        assert_eq!(assemble(&mut assembler, &[packet(0, 0, &[9])]).0, [vec![0xF0, 9, 0xF7]]);
        // Without a buffer every message is dropped.
        let mut none = SysEx7Assembler::new(0);
        assert_eq!(assemble(&mut none, &[packet(0, 0, &[])]), (vec![], 1));
    }

    #[test]
    fn test_sysex7_without_a_start_is_dropped_once() {
        let mut assembler = SysEx7Assembler::new(64);
        let orphan = [packet(0, 2, &[1]), packet(0, 2, &[2]), packet(0, 3, &[3]), packet(0, 0, &[4])];
        assert_eq!(assemble(&mut assembler, &orphan), (vec![vec![0xF0, 4, 0xF7]], 1));
        assert_eq!(assemble(&mut assembler, &[packet(0, 3, &[5])]), (vec![], 1));
    }

    #[test]
    fn test_sysex7_start_while_open_drops_the_open_one() {
        let mut assembler = SysEx7Assembler::new(64);
        let packets = [packet(0, 1, &[1]), packet(0, 1, &[2]), packet(0, 3, &[3]), packet(0, 1, &[4]), packet(0, 0, &[5])];
        assert_eq!(assemble(&mut assembler, &packets), (vec![vec![0xF0, 2, 3, 0xF7], vec![0xF0, 5, 0xF7]], 2));
    }

    #[test]
    fn test_sysex7_malformed_packets_drop_their_message() {
        let mut assembler = SysEx7Assembler::new(64);
        let mut too_many = packet(0, 2, &[1]);
        too_many[0] = (too_many[0] & !0x000F_0000) | 7 << 16;
        let mut bad_status = packet(0, 0, &[1]);
        bad_status[0] |= 4 << 20;
        let packets = [
            packet(0, 1, &[1]),
            too_many,
            packet(0, 3, &[2]),
            bad_status,
            packet(0, 0, &[0x80]),
            packet(0, 0, &[3]),
        ];
        // The open message with its malformed continue (its end is
        // skipped), the status, the byte.
        assert_eq!(assemble(&mut assembler, &packets), (vec![vec![0xF0, 3, 0xF7]], 3));
        // A malformed start takes its message: the rest is skipped.
        let packets = [packet(0, 1, &[0x80]), packet(0, 2, &[2]), packet(0, 3, &[3])];
        assert_eq!(assemble(&mut assembler, &packets), (vec![], 1));
    }

    #[test]
    fn test_sysex7_of_another_group_is_dropped_not_mixed() {
        let mut assembler = SysEx7Assembler::new(64);
        let packets = [
            packet(0, 1, &[1]),
            packet(1, 1, &[0x51]),
            packet(0, 2, &[2]),
            packet(1, 2, &[0x52]),
            packet(1, 3, &[0x53]),
            packet(0, 3, &[3]),
            packet(1, 0, &[0x54]),
        ];
        assert_eq!(
            assemble(&mut assembler, &packets),
            (vec![vec![0xF0, 1, 2, 3, 0xF7], vec![0xF0, 0x54, 0xF7]], 1)
        );
    }

    #[test]
    fn test_sysex7_of_two_other_groups_counts_each_once() {
        let mut assembler = SysEx7Assembler::new(64);
        let packets = [
            packet(0, 1, &[1]),
            packet(1, 1, &[0x51]),
            packet(2, 1, &[0x61]),
            packet(1, 2, &[0x52]),
            packet(2, 2, &[0x62]),
            packet(1, 3, &[0x53]),
            packet(2, 3, &[0x63]),
            packet(0, 3, &[2]),
        ];
        assert_eq!(assemble(&mut assembler, &packets), (vec![vec![0xF0, 1, 2, 0xF7]], 2));
        // A skipped message stays skipped after the open one ended.
        let packets = [packet(0, 1, &[1]), packet(1, 1, &[0x51]), packet(0, 3, &[2]), packet(1, 3, &[0x53])];
        assert_eq!(assemble(&mut assembler, &packets), (vec![vec![0xF0, 1, 2, 0xF7]], 1));
    }

    /// The MIDI 2.0 message of an event, read back.
    fn midi2_round_trip(event: MidiEvent<'static>) -> MidiEventKind<'static> {
        let mut words = Vec::new();
        ump_messages(&event.event, MidiProtocol::Midi2, |message| words.push(message.to_vec()));
        assert_eq!(words.len(), 1, "{event:?} gives {words:08x?}");
        assert_eq!(words[0].len(), 2);
        match midi2_channel_voice(event.sample_offset, [words[0][0], words[0][1]]) {
            ChannelVoice::Event(back) => back.event,
            ChannelVoice::Unsupported => panic!("{words:08x?} is unsupported"),
        }
    }

    #[test]
    fn test_midi2_messages_round_trip() {
        let id = note_id(0, 3, 62);
        let events = [
            MidiEvent::note_on(0, 3, 62, scale::velocity_from_u16(0x1234), id, 0.0, 0),
            MidiEvent::note_off(0, 3, 62, scale::velocity_from_u16(0xFFFF), id, 0.0),
            MidiEvent::poly_pressure(0, 3, 62, scale::unipolar_from_u32(0x1234_5678), id),
            MidiEvent::control_change(0, 4, 74, scale::unipolar_from_u32(0x8765_4321)),
            MidiEvent::program_change(0, 5, 9, Some((3 << 7) | 4)),
            MidiEvent::program_change(0, 5, 9, None),
            MidiEvent::channel_pressure(0, 6, 1.0),
            MidiEvent::pitch_bend(0, 7, scale::bipolar_from_u32(0x1234_5678)),
            MidiEvent::pitch_bend(0, 7, -1.0),
            MidiEvent::parameter_number(0, ParameterNumber::rpn(8, 0, scale::unipolar_from_u32(0x1800_0000))),
            MidiEvent::parameter_number(0, ParameterNumber::nrpn(8, (1 << 7) | 2, scale::unipolar_from_u32(0x4000_0000))),
            MidiEvent::parameter_number(
                0,
                ParameterNumber {
                    channel: 9,
                    kind: ParameterNumberKind::Rpn,
                    parameter: 1,
                    change: ParameterChange::Relative(-((1u32 << 18) as f64) / u32::MAX as f64),
                },
            ),
        ];
        for event in events {
            assert_eq!(midi2_round_trip(event), event.event);
        }
    }

    #[test]
    fn test_midi2_note_tuning_goes_out_as_its_pitch_attribute() {
        // 60.25 is exact in 7.9 bits.
        let event = MidiEvent::note_on(0, 0, 62, scale::velocity_from_u16(0x8000), note_id(0, 0, 62), -175.0, 0);
        assert_eq!(midi2_round_trip(event), event.event);
        // A tuning between steps goes to the nearest step of 1/512 semitone.
        let event = MidiEvent::note_on(0, 0, 62, 0.5, note_id(0, 0, 62), 33.3, 0);
        let MidiEventKind::NoteOn(back) = midi2_round_trip(event) else { panic!() };
        assert!((back.tuning - 33.3).abs() <= 100.0 / 1024.0, "{}", back.tuning);
    }

    #[test]
    fn test_note_expressions_round_trip_as_per_note_messages() {
        let id = note_id(0, 2, 60);
        let expression = |expression_type: u32, value: f64| {
            MidiEvent::note_expression_value(0, id, expression_type, value)
        };
        for event in [
            expression(note_expression::TUNING, tuning_value(2.5)),
            expression(note_expression::VOLUME, 0.25),
            expression(note_expression::PAN, 0.5),
            expression(note_expression::EXPRESSION, 1.0),
            expression(note_expression::BRIGHTNESS, 0.0),
            expression(note_expression::VIBRATO, 1.0),
            expression(note_expression::PITCH_BEND, 0.5),
            expression(note_expression::PITCH_BEND, 1.0),
            expression(note_expression::registered(1), 0.0),
            expression(note_expression::assignable(200), 1.0),
        ] {
            assert_eq!(midi2_round_trip(event), event.event);
        }
        // A note ID that note_id did not make, or a VST3 custom type, has
        // no MIDI 2.0 form.
        let mut none = 0;
        ump_messages(
            &MidiEvent::note_expression_value(0, 1 << 20, note_expression::PAN, 0.5).event,
            MidiProtocol::Midi2,
            |_| none += 1,
        );
        ump_messages(&expression(note_expression::CUSTOM_START, 0.5).event, MidiProtocol::Midi2, |_| none += 1);
        assert_eq!(none, 0);
    }

    #[test]
    fn test_midi1_words_carry_the_midi1_messages() {
        let mut words = Vec::new();
        let rpn = MidiEvent::parameter_number(0, ParameterNumber::rpn(1, 0, scale::from_14bit(12 << 7)));
        ump_messages(&rpn.event, MidiProtocol::Midi1, |message| words.extend_from_slice(message));
        assert_eq!(words, [0x20B1_6500, 0x20B1_6400, 0x20B1_060C, 0x20B1_2600]);
        words.clear();
        let program = MidiEvent::program_change(0, 0, 5, None);
        ump_messages(&program.event, MidiProtocol::Midi1, |message| words.extend_from_slice(message));
        assert_eq!(words, [0x20C0_0500]);
        // Every MIDI 1.0 word reads back as its event.
        let on = MidiEvent::note_on(0, 0, 60, scale::velocity_from_7bit(64), note_id(0, 0, 60), 0.0, 0);
        words.clear();
        ump_messages(&on.event, MidiProtocol::Midi1, |message| words.extend_from_slice(message));
        assert_eq!(midi1_channel_voice(0, words[0]), ChannelVoice::Event(on));
    }

    #[test]
    fn test_sysex7_packets_come_together_again() {
        for len in [0usize, 1, 5, 6, 7, 12, 13, 400] {
            let message: Vec<u8> = (0..len).map(|byte| (byte % 128) as u8).collect();
            let mut assembler = SysEx7Assembler::new(512);
            let mut complete = None;
            let mut packets = 0;
            sysex7_packets(&message, |words| {
                packets += 1;
                let step = assembler.push(words);
                assert_eq!(step.dropped, 0);
                if let Some(bytes) = step.complete {
                    complete = Some(bytes.to_vec());
                }
            });
            assert_eq!(packets, len.div_ceil(6).max(1));
            let mut framed = vec![0xF0];
            framed.extend_from_slice(&message);
            framed.push(0xF7);
            assert_eq!(complete, Some(framed));
        }
        // A framed message is framed once.
        let mut first = None;
        sysex7_packets(&[0xF0, 0x7D, 0xF7], |words| first = Some(words));
        assert_eq!(first, Some([0x3001_7D00, 0]));
    }

    #[test]
    fn test_sysex7_reset_forgets_the_open_message() {
        let mut assembler = SysEx7Assembler::new(64);
        assemble(&mut assembler, &[packet(0, 1, &[1])]);
        assembler.reset();
        assert_eq!(assemble(&mut assembler, &[packet(0, 3, &[2])]), (vec![], 1));
    }
}
