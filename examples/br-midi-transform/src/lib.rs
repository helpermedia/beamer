//! BR Midi Transform - Example VST3 instrument demonstrating MIDI processing.
//!
//! This is a **VST3 instrument** that transforms MIDI notes:
//! - MIDI In → Transpose +2 semitones → MIDI Out
//! - Audio passes through unchanged (or silence if no input)
//!
//! Use case: Insert before another instrument to transpose all notes.
//! Example: Play C1 on keyboard → BR Midi Transform → D1 sent to synth
//!
//! It demonstrates:
//! - VST3 instrument category (not an effect)
//! - Processing MIDI events with `process_midi`
//! - MIDI bus configuration with `wants_midi`
//! - Note transformation while preserving noteId

use beamr::prelude::*;
use beamr::vst3_impl::vst3;

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Unique ID for this plugin component.
/// Generated using random values - must be unique for each plugin.
const COMPONENT_UID: vst3::Steinberg::TUID =
    vst3::uid(0xA1B2C3D4, 0xE5F6A7B8, 0xC9D0E1F2, 0x03040506);

/// Plugin configuration exported to the VST3 factory.
///
/// Note: This is categorized as "Instrument" so it appears in the instrument
/// section of the DAW's plugin browser, not in effects.
pub static CONFIG: PluginConfig = PluginConfig::new("BR Midi Transform", COMPONENT_UID)
    .with_vendor("BEAMR Framework")
    .with_url("https://github.com/helpermedia/beamr")
    .with_email("support@example.com")
    .with_version("1.0.0")
    .with_sub_categories("Instrument");

// =============================================================================
// Plugin Implementation
// =============================================================================

/// Semitones to transpose notes up.
const TRANSPOSE_SEMITONES: i8 = 2;

/// MIDI note transformer plugin.
///
/// Transposes all incoming MIDI notes up by 2 semitones.
/// Audio is passed through unchanged.
pub struct MidiTransformProcessor {
    params: NoParams,
}

impl AudioProcessor for MidiTransformProcessor {
    fn setup(&mut self, _sample_rate: f64, _max_buffer_size: usize) {
        // No sample-rate dependent state needed
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        // Pass audio through unchanged
        buffer.copy_to_output();
    }

    fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
        for event in input {
            match &event.event {
                MidiEventKind::NoteOn(note_on) => {
                    // Transpose pitch up, clamp to valid MIDI range (0-127)
                    let new_pitch = (note_on.pitch as i16 + TRANSPOSE_SEMITONES as i16)
                        .clamp(0, 127) as u8;

                    output.push(MidiEvent::note_on(
                        event.sample_offset,
                        note_on.channel,
                        new_pitch,
                        note_on.velocity,
                        note_on.note_id, // Preserve note ID for proper tracking!
                        note_on.tuning,  // Preserve tuning for MPE/microtonal
                        note_on.length,  // Preserve note length
                    ));
                }
                MidiEventKind::NoteOff(note_off) => {
                    // Must transpose note-off to match the transposed note-on
                    let new_pitch = (note_off.pitch as i16 + TRANSPOSE_SEMITONES as i16)
                        .clamp(0, 127) as u8;

                    output.push(MidiEvent::note_off(
                        event.sample_offset,
                        note_off.channel,
                        new_pitch,
                        note_off.velocity,
                        note_off.note_id,
                        note_off.tuning, // Preserve tuning for MPE/microtonal
                    ));
                }
                MidiEventKind::PolyPressure(poly) => {
                    // Transpose poly pressure to match the transposed notes
                    let new_pitch = (poly.pitch as i16 + TRANSPOSE_SEMITONES as i16)
                        .clamp(0, 127) as u8;

                    output.push(MidiEvent::poly_pressure(
                        event.sample_offset,
                        poly.channel,
                        new_pitch,
                        poly.pressure,
                        poly.note_id,
                    ));
                }
                // Pass through channel-wide events unchanged
                MidiEventKind::ControlChange(_)
                | MidiEventKind::PitchBend(_)
                | MidiEventKind::ChannelPressure(_)
                | MidiEventKind::ProgramChange(_) => {
                    output.push(*event);
                }

                // Pass through advanced VST3 events unchanged
                // (SysEx, Note Expression, Chord, Scale info)
                MidiEventKind::SysEx(_)
                | MidiEventKind::NoteExpressionValue(_)
                | MidiEventKind::NoteExpressionInt(_)
                | MidiEventKind::NoteExpressionText(_)
                | MidiEventKind::ChordInfo(_)
                | MidiEventKind::ScaleInfo(_) => {
                    output.push(*event);
                }
            }
        }
    }

    fn wants_midi(&self) -> bool {
        true
    }
}

impl Plugin for MidiTransformProcessor {
    type Params = NoParams;

    fn params(&self) -> &Self::Params {
        &self.params
    }

    fn params_mut(&mut self) -> &mut Self::Params {
        &mut self.params
    }

    fn create() -> Self {
        Self {
            params: NoParams,
        }
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

// Export VST3 entry points using the generic wrapper
export_vst3!(CONFIG, Vst3Processor<MidiTransformProcessor>);
