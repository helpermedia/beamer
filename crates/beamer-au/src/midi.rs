//! MIDI event conversion for Audio Unit.
//!
//! This module provides conversion between AU MIDI events and beamer-core MIDI events.
//!
//! # Audio Unit MIDI Format
//!
//! AU uses Apple's Universal MIDI Packet (UMP) format for MIDI 2.0, but most
//! plugins still work with MIDI 1.0 channel voice messages. The conversion
//! functions handle both formats.
//!
//! # Usage
//!
//! ```ignore
//! // In render callback:
//! let midi_events = au_midi_to_beamer(au_event_list);
//! processor.process_midi(&midi_events);
//!
//! let output_events = processor.output_midi();
//! beamer_to_au_midi(output_events, au_output_list);
//! ```

use beamer_core::{ControlChange, MidiEvent, MidiEventKind, NoteOff, NoteOn, PitchBend};

/// MIDI event packet from AU (UMP format).
///
/// This is a simplified representation of Apple's MIDIEventPacket.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AuMidiPacket {
    /// Timestamp in sample frames
    pub time_stamp: u32,
    /// Word count (1-64)
    pub word_count: u32,
    /// UMP words (up to 64)
    pub words: [u32; 64],
}

/// MIDI 1.0 channel voice message types.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Midi1Status {
    NoteOff = 0x80,
    NoteOn = 0x90,
    PolyPressure = 0xA0,
    ControlChange = 0xB0,
    ProgramChange = 0xC0,
    ChannelPressure = 0xD0,
    PitchBend = 0xE0,
}

/// Convert AU MIDI packets to beamer MIDI events.
///
/// Handles MIDI 1.0 channel voice messages in UMP format.
///
/// # Arguments
///
/// * `packets` - Slice of AU MIDI packets
///
/// # Returns
///
/// Vector of beamer MidiEvent
pub fn au_midi_to_beamer(packets: &[AuMidiPacket]) -> Vec<MidiEvent> {
    let mut events = Vec::with_capacity(packets.len());

    for packet in packets {
        if packet.word_count == 0 {
            continue;
        }

        // Check for MIDI 1.0 Channel Voice Message (UMP message type 2)
        let word = packet.words[0];
        let message_type = (word >> 28) & 0xF;

        if message_type == 2 {
            // MIDI 1.0 Channel Voice Message
            // Format: [MT=2][Group][Status][Data1][Data2]
            let status = ((word >> 16) & 0xF0) as u8;
            let channel = ((word >> 16) & 0x0F) as u8;
            let data1 = ((word >> 8) & 0x7F) as u8;
            let data2 = (word & 0x7F) as u8;

            if let Some(event) =
                parse_midi1_message(packet.time_stamp, status, channel, data1, data2)
            {
                events.push(event);
            }
        }
    }

    events
}

/// Parse a MIDI 1.0 channel voice message.
fn parse_midi1_message(
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
            note_id: data1 as i32,
            tuning: 0.0,
        }),
        0x90 => {
            if data2 == 0 {
                // Note On with velocity 0 = Note Off
                MidiEventKind::NoteOff(NoteOff {
                    channel,
                    pitch: data1,
                    velocity: 0.0,
                    note_id: data1 as i32,
                    tuning: 0.0,
                })
            } else {
                MidiEventKind::NoteOn(NoteOn {
                    channel,
                    pitch: data1,
                    velocity: data2 as f32 / 127.0,
                    note_id: data1 as i32,
                    tuning: 0.0,
                    length: 0,
                })
            }
        }
        0xB0 => MidiEventKind::ControlChange(ControlChange {
            channel,
            controller: data1,
            value: data2 as f32 / 127.0,
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
        _ => return None, // Unsupported message type
    };

    Some(MidiEvent {
        sample_offset,
        event: kind,
    })
}

/// Convert beamer MIDI events to AU MIDI packets.
///
/// Creates UMP MIDI 1.0 Channel Voice Messages.
///
/// # Arguments
///
/// * `events` - Slice of beamer MidiEvent
///
/// # Returns
///
/// Vector of AU MIDI packets
pub fn beamer_to_au_midi(events: &[MidiEvent]) -> Vec<AuMidiPacket> {
    let mut packets = Vec::with_capacity(events.len());

    for event in events {
        if let Some(packet) = event_to_ump_packet(event) {
            packets.push(packet);
        }
    }

    packets
}

/// Convert a single beamer event to UMP packet.
fn event_to_ump_packet(event: &MidiEvent) -> Option<AuMidiPacket> {
    let (status, data1, data2, channel) = match &event.event {
        MidiEventKind::NoteOn(note) => (
            0x90,
            note.pitch,
            (note.velocity * 127.0) as u8,
            note.channel,
        ),
        MidiEventKind::NoteOff(note) => (
            0x80,
            note.pitch,
            (note.velocity * 127.0) as u8,
            note.channel,
        ),
        MidiEventKind::ControlChange(cc) => {
            (0xB0, cc.controller, (cc.value * 127.0) as u8, cc.channel)
        }
        MidiEventKind::PitchBend(pb) => {
            let raw = ((pb.value * 8192.0) + 8192.0) as u16;
            let lsb = (raw & 0x7F) as u8;
            let msb = ((raw >> 7) & 0x7F) as u8;
            (0xE0, lsb, msb, pb.channel)
        }
        _ => return None,
    };

    // Build UMP MIDI 1.0 Channel Voice Message word
    // Format: [MT=2][Group=0][Status|Channel][Data1][Data2]
    let word = (2u32 << 28) // Message Type 2 (Group 0 is implicit)
        | ((status | channel) as u32) << 16
        | (data1 as u32) << 8
        | (data2 as u32);

    let mut packet = AuMidiPacket {
        time_stamp: event.sample_offset,
        word_count: 1,
        words: [0; 64],
    };
    packet.words[0] = word;

    Some(packet)
}

/// Pre-allocated MIDI buffer for real-time safe event collection.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_note_on() {
        let event = parse_midi1_message(100, 0x90, 0, 60, 100).unwrap();
        assert_eq!(event.sample_offset, 100);
        match event.event {
            MidiEventKind::NoteOn(note) => {
                assert_eq!(note.channel, 0);
                assert_eq!(note.pitch, 60);
                assert!((note.velocity - 100.0 / 127.0).abs() < 0.01);
            }
            _ => panic!("Expected NoteOn"),
        }
    }

    #[test]
    fn test_note_on_velocity_zero_is_note_off() {
        let event = parse_midi1_message(0, 0x90, 0, 60, 0).unwrap();
        assert!(matches!(event.event, MidiEventKind::NoteOff(_)));
    }

    #[test]
    fn test_pitch_bend() {
        // Center position (8192)
        let event = parse_midi1_message(0, 0xE0, 0, 0, 64).unwrap();
        match event.event {
            MidiEventKind::PitchBend(pb) => {
                assert!(pb.value.abs() < 0.01); // Should be near 0
            }
            _ => panic!("Expected PitchBend"),
        }
    }
}
