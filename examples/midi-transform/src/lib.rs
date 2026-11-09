//! Beamer MIDI Transform - Example MIDI effect demonstrating the Beamer framework.
//!
//! # Two-Struct Pattern
//!
//! Beamer plugins use two structs:
//!
//! 1. **`MidiTransformModel`** - What every thread shares, marked with
//!    `#[beamer::model]` and implementing `Model`: its parameters, state and
//!    events, how the host sees it and how it creates the processor.
//! 2. **`MidiTransformProcessor`** - What the audio thread owns, created by
//!    `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - Nested groups with `#[nested(group = "...")]`
//! - `EnumParameter` for discrete choices (transform modes)
//! - `IntParameter` for note/CC number selection
//! - `BoolParameter` for enable toggles
//! - `FloatParameter` for velocity/value scaling
//! - `process_midi()` for MIDI-only processing
//! - MIDI event modification with the `with()` method
//! - Held-note tracking, so a note-off ends the note its note-on started
//!   when the settings change while the note is held
//! - Sample-accurate automation (`sample_accurate_automation` in
//!   `Config.toml`), since the settings apply to each note
//! - `kind = "note_number"` for note numbers shown as names ("C3"), named
//!   by `middle_c` in `Config.toml`
//! - `kind = "percent"` for the scales
//! - A stream sender from the audio thread that sends every note and CC as
//!   it came in and went out, for the editor
//!
//! # MIDI Event Modification
//!
//! The `with()` method transforms events while preserving timing:
//!
//! ```ignore
//! output.push(event.with(MidiEventKind::NoteOn(NoteOn {
//!     pitch: new_pitch,
//!     velocity: new_velocity,
//!     ..*note_on
//! })));
//! ```
//!
//! # Editor
//!
//! The editor in `webview/` is React, on the packages beamer-api,
//! beamer-controls and beamer-react. bun and Vite build it into
//! `webview/dist/`, which the export macro embeds. It draws the note map
//! and the CC curve from the parameters, with the same rules as
//! [`MidiTransformProcessor`]. The audio thread sends what only it knows:
//! the notes and CCs that pass through, which light up in the displays.

use beamer::prelude::*;
use beamer::serde::Serialize;

// =============================================================================
// Enum Types for Parameter Choices
// =============================================================================

/// Note transformation mode.
///
/// Demonstrates `#[derive(EnumParameter)]` with custom display names.
#[derive(Copy, Clone, PartialEq, EnumParameter)]
pub enum NoteTransformMode {
    /// Pass notes through with optional velocity scaling
    #[default]
    #[name = "Through"]
    Through,
    /// Transpose all notes by semitones
    #[name = "Transpose"]
    Transpose,
    /// Shift notes up one octave (+12 semitones)
    #[name = "Octave Up"]
    OctaveUp,
    /// Shift notes down one octave (-12 semitones)
    #[name = "Octave Down"]
    OctaveDown,
    /// Remap a specific note to another note
    #[name = "Remap Note"]
    Remap,
    /// Invert pitch around middle C (note 60)
    #[name = "Invert"]
    Invert,
}

/// CC transformation mode.
#[derive(Copy, Clone, PartialEq, EnumParameter)]
pub enum CcTransformMode {
    /// Pass CC through unchanged
    #[default]
    #[name = "Through"]
    Through,
    /// Remap CC number (CC X → CC Y)
    #[name = "Remap CC"]
    Remap,
    /// Scale CC value
    #[name = "Scale"]
    Scale,
    /// Invert CC value (127 - value)
    #[name = "Invert"]
    Invert,
    /// Remap and scale CC
    #[name = "Remap + Scale"]
    RemapAndScale,
}

// =============================================================================
// Nested Groups
// =============================================================================

/// Note transformation settings.
///
/// This is a nested group that appears as "Note Transform" in the DAW.
/// Uses **declarative parameter definition** - no manual Default impl needed!
///
/// Its parameters are known by their path: the field this struct sits in and
/// the `id` below, such as `note/enabled`. That is why "enabled" can be used
/// again in the CC group.
#[beamer::group]
pub struct NoteTransformGroup {
    /// Enable note transformation
    #[parameter(id = "enabled", name = "Enabled", default = true)]
    pub enabled: BoolParameter,

    /// Transformation mode
    #[parameter(id = "mode", name = "Mode")]
    pub mode: EnumParameter<NoteTransformMode>,

    /// Transpose amount in semitones (-24 to +24)
    #[parameter(id = "transpose", name = "Transpose", default = 0, range = -24..=24, kind = "semitones")]
    pub transpose: IntParameter,

    /// Input note for remap mode (0-127)
    #[parameter(id = "input", name = "Input Note", default = 60, range = 0..=127, kind = "note_number")]
    pub input_note: IntParameter,

    /// Output note for remap mode (0-127)
    #[parameter(id = "output", name = "Output Note", default = 60, range = 0..=127, kind = "note_number")]
    pub output_note: IntParameter,

    /// Velocity scale (0% to 200%)
    #[parameter(id = "velocity", name = "Velocity", default = 1.0, range = 0.0..=2.0, kind = "percent")]
    pub velocity_scale: FloatParameter,
}

/// CC transformation settings.
///
/// This is a nested group that appears as "CC Transform" in the DAW.
/// Uses **declarative parameter definition** - no manual Default impl needed!
#[beamer::group]
pub struct CcTransformGroup {
    /// Enable CC transformation
    #[parameter(id = "enabled", name = "Enabled", default = true)]
    pub enabled: BoolParameter,

    /// Transformation mode
    #[parameter(id = "mode", name = "Mode")]
    pub mode: EnumParameter<CcTransformMode>,

    /// Input CC number (0-127) - Default: Mod wheel (CC 1)
    #[parameter(id = "input", name = "Input CC", default = 1, range = 0..=127)]
    pub input_cc: IntParameter,

    /// Output CC number (0-127) - Default: Expression (CC 11)
    #[parameter(id = "output", name = "Output CC", default = 11, range = 0..=127)]
    pub output_cc: IntParameter,

    /// Value scale (0% to 200%)
    #[parameter(id = "scale", name = "Scale", default = 1.0, range = 0.0..=2.0, kind = "percent")]
    pub value_scale: FloatParameter,
}

// =============================================================================
// Model
// =============================================================================

/// MIDI Transform model: everything the plugin knows, shared by the host
/// and the audio thread for the life of the instance.
///
/// Demonstrates the `#[nested(group = "...")]` attribute for creating
/// hierarchical parameter organization in the DAW.
#[beamer::export]
#[beamer::model]
pub struct MidiTransformModel {
    /// Global bypass - uses the special `bypass` attribute
    #[parameter(id = "bypass", bypass)]
    pub bypass: BoolParameter,

    /// Note transformation settings (nested group)
    #[nested(group = "Note Transform")]
    pub note: NoteTransformGroup,

    /// CC transformation settings (nested group)
    #[nested(group = "CC Transform")]
    pub cc: CcTransformGroup,

    /// Events for the editor. The processor's sender is made from it.
    pub events: WebViewEventQueue,
}

impl Model for MidiTransformModel {
    // A MIDI transformer needs nothing from the host's audio setup.
    // See `beamer::setup` for all available types.
    type Setup = ();
    type Processor = MidiTransformProcessor;

    fn prepare(self: Arc<Self>, _: ()) -> MidiTransformProcessor {
        MidiTransformProcessor {
            transformed_sender: self.events.stream_sender("transformed", TRANSFORMED_CAPACITY),
            pressed_sender: self.events.value_sender("pressed", 0),
            model: self,
            held_notes: [[None; 128]; 16],
            pressed_keys: [0; 16],
            pressed_sent: 0,
        }
    }
}

// =============================================================================
// Editor
// =============================================================================

/// A note or a CC as it came in and as it went out, sent to the editor,
/// which lights up the notes that sound and shows the last CC. As JSON an
/// object with its `type` ("noteOn", "noteOff", "cc" or "reset") and the
/// fields in camelCase.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(
    crate = "beamer::serde",
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Transformed {
    /// A note started: its pitch in and out, `None` when it was filtered
    /// out. Velocities from 0.0 to 1.0.
    NoteOn {
        channel: u8,
        pitch: u8,
        out_pitch: Option<u8>,
        velocity: f32,
        out_velocity: f32,
    },
    /// The note on `pitch` ended, by its incoming pitch.
    NoteOff { channel: u8, pitch: u8 },
    /// A CC in and out. Values from 0.0 to 1.0.
    Cc {
        channel: u8,
        controller: u8,
        value: f64,
        out_controller: u8,
        out_value: f64,
    },
    /// The host reset the plugin: no note sounds any more.
    Reset,
}

/// The CCs that end every note of their channel: All Sound Off and All
/// Notes Off.
const ENDS_NOTES: [u8; 2] = [120, 123];

/// Room for the events of two ticks of the editor (a tick is about 16 ms),
/// more than a player or a sequence sends.
const TRANSFORMED_CAPACITY: usize = 1024;

// =============================================================================
// Processor
// =============================================================================

/// MIDI transformer processor, ready for audio/MIDI processing.
///
/// Transforms MIDI notes and CC messages based on parameter settings.
/// Created by `MidiTransformModel::prepare()`.
pub struct MidiTransformProcessor {
    model: Arc<MidiTransformModel>,

    /// The pitch each held note was sent out with, by channel and incoming
    /// pitch (`None` when the note was filtered out). A note-off goes to
    /// the note its note-on started, even when the settings or the bypass
    /// changed while the note was held, so no note hangs. One entry per key:
    /// a second note-on on a held key replaces the first.
    held_notes: [[Option<u8>; 128]; 16],

    /// The incoming keys that are down, a bit per pitch, by channel, also
    /// those whose note was filtered out. Only for the editor: it shows
    /// the notes that sound until they end.
    pressed_keys: [u128; 16],

    /// Sends what passes through to the editor. Sends nothing while no
    /// editor is open.
    transformed_sender: WebViewStreamSender<Transformed>,

    /// Sends how many keys are down, with every block while any is and
    /// once when the last one comes up. A pause while keys are down tells
    /// the editor that the host stopped calling the plugin (Logic's power
    /// button, Reaper's FX switch), so no note-off will come through: it
    /// lets the keys go out then.
    pressed_sender: WebViewValueSender<u32>,

    /// The count `pressed_sender` sent last.
    pressed_sent: u32,
}

impl MidiTransformProcessor {
    /// Mark an incoming key as down or up.
    fn press(&mut self, channel: MidiChannel, pitch: MidiNote, down: bool) {
        let bit = 1u128 << (pitch & 0x7F);
        let keys = &mut self.pressed_keys[(channel & 0x0F) as usize];
        if down {
            *keys |= bit;
        } else {
            *keys &= !bit;
        }
    }

    /// How many incoming keys are down.
    fn pressed_count(&self) -> u32 {
        self.pressed_keys.iter().map(|keys| keys.count_ones()).sum()
    }

    /// The entry in `held_notes` for a channel and an incoming pitch.
    fn held_note(&mut self, channel: MidiChannel, pitch: MidiNote) -> &mut Option<u8> {
        &mut self.held_notes[(channel & 0x0F) as usize][(pitch & 0x7F) as usize]
    }

    /// Transform a MIDI note pitch based on current settings.
    ///
    /// # Transformation Modes
    ///
    /// | Mode       | Formula / Description                          |
    /// |------------|-----------------------------------------------|
    /// | Through    | `pitch` (unchanged)                           |
    /// | Transpose  | `pitch + transpose_amount`                    |
    /// | Octave Up  | `pitch + 12`                                  |
    /// | Octave Down| `pitch - 12`                                  |
    /// | Remap      | `output_note` if `pitch == input_note`        |
    /// | Invert     | `60 + (60 - pitch)` (mirror around middle C)  |
    ///
    /// # Arguments
    /// * `pitch` - Input MIDI pitch (0-127)
    ///
    /// # Returns
    /// * `Some(pitch)` - Transformed pitch within valid MIDI range
    /// * `None` - Pitch out of range (note should be filtered)
    fn transform_pitch(&self, pitch: u8) -> Option<u8> {
        if !self.model.note.enabled.get() {
            return Some(pitch);
        }

        let new_pitch = match self.model.note.mode.get() {
            NoteTransformMode::Through => pitch as i16,

            NoteTransformMode::Transpose => {
                pitch as i16 + self.model.note.transpose.get() as i16
            }

            NoteTransformMode::OctaveUp => pitch as i16 + 12,

            NoteTransformMode::OctaveDown => pitch as i16 - 12,

            NoteTransformMode::Remap => {
                if pitch == self.model.note.input_note.get() as u8 {
                    self.model.note.output_note.get() as i16
                } else {
                    pitch as i16
                }
            }

            NoteTransformMode::Invert => {
                // Invert around middle C (60)
                // Examples: 60→60, 61→59, 72→48, 48→72
                60 + (60 - pitch as i16)
            }
        };

        // Clamp to valid MIDI range, return None if out of range
        if (0..=127).contains(&new_pitch) {
            Some(new_pitch as u8)
        } else {
            None
        }
    }

    /// Transform a velocity value based on current settings.
    ///
    /// Applies velocity scaling: `output = input * scale`
    ///
    /// # Arguments
    /// * `velocity` - Input velocity (0.0-1.0)
    ///
    /// # Returns
    /// Scaled velocity, clamped to 0.0-1.0
    fn transform_velocity(&self, velocity: f32) -> f32 {
        if !self.model.note.enabled.get() {
            return velocity;
        }

        let scale = self.model.note.velocity_scale.get() as f32;
        (velocity * scale).clamp(0.0, 1.0)
    }

    /// Transform a CC number based on current settings.
    ///
    /// Only Remap and RemapAndScale modes change the CC number.
    /// Other modes pass the CC number through unchanged.
    ///
    /// # Arguments
    /// * `cc` - Input CC number (0-127)
    ///
    /// # Returns
    /// * `Some(cc)` - Output CC number (possibly remapped)
    /// * `None` - CC should be filtered (not currently used)
    fn transform_cc_number(&self, cc: u8) -> Option<u8> {
        if !self.model.cc.enabled.get() {
            return Some(cc);
        }

        match self.model.cc.mode.get() {
            CcTransformMode::Through | CcTransformMode::Scale | CcTransformMode::Invert => {
                Some(cc)
            }
            CcTransformMode::Remap | CcTransformMode::RemapAndScale => {
                if cc == self.model.cc.input_cc.get() as u8 {
                    Some(self.model.cc.output_cc.get() as u8)
                } else {
                    Some(cc)
                }
            }
        }
    }

    /// Transform a CC value based on current settings.
    ///
    /// # Transformation Modes
    ///
    /// | Mode           | Formula                        |
    /// |----------------|--------------------------------|
    /// | Through        | `value` (unchanged)            |
    /// | Scale          | `value * scale_factor`         |
    /// | Invert         | `1.0 - value`                  |
    /// | Remap          | `value` (only number changes)  |
    /// | RemapAndScale  | `value * scale_factor`         |
    ///
    /// # Arguments
    /// * `cc` - CC number (used to check if this CC should be transformed)
    /// * `value` - Input CC value (0.0-1.0)
    ///
    /// # Returns
    /// Transformed CC value, clamped to 0.0-1.0
    fn transform_cc_value(&self, cc: u8, value: f64) -> f64 {
        if !self.model.cc.enabled.get() {
            return value;
        }

        // Only transform if this is the targeted CC (for remap modes)
        // or if we're in a mode that affects all CCs
        let should_transform = match self.model.cc.mode.get() {
            CcTransformMode::Through => false,
            CcTransformMode::Scale | CcTransformMode::Invert => true,
            CcTransformMode::Remap => false, // Remap only changes number, not value
            CcTransformMode::RemapAndScale => {
                cc == self.model.cc.input_cc.get() as u8
            }
        };

        if !should_transform {
            return value;
        }

        match self.model.cc.mode.get() {
            CcTransformMode::Scale | CcTransformMode::RemapAndScale => {
                let scale = self.model.cc.value_scale.get();
                (value * scale).clamp(0.0, 1.0)
            }
            CcTransformMode::Invert => {
                1.0 - value
            }
            _ => value,
        }
    }
}

impl Processor for MidiTransformProcessor {
    fn process(
        &mut self,
        _buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        // Audio passes through unchanged: the buffer already holds the input.

        // While keys are down the count goes out with every block, also
        // when it holds still. When it falls to 0 it goes out once more.
        let pressed = self.pressed_count();
        if pressed > 0 || self.pressed_sent > 0 {
            self.pressed_sender.send(&pressed);
            self.pressed_sent = pressed;
        }
    }

    fn set_active(&mut self, active: bool) {
        // A reset ends every note, so none is held any more.
        if active {
            self.held_notes = [[None; 128]; 16];
            self.pressed_keys = [0; 16];
            self.transformed_sender.push(Transformed::Reset);
        }
    }

    fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
        let bypassed = self.model.bypass.get();

        for event in input {
            match &event.event {
                MidiEventKind::NoteOn(note_on) => {
                    // When bypassed, a note goes out unchanged.
                    let new_pitch = if bypassed {
                        Some(note_on.pitch)
                    } else {
                        self.transform_pitch(note_on.pitch)
                    };
                    *self.held_note(note_on.channel, note_on.pitch) = new_pitch;
                    self.press(note_on.channel, note_on.pitch, true);
                    let new_velocity = if bypassed {
                        note_on.velocity
                    } else {
                        self.transform_velocity(note_on.velocity)
                    };

                    // If transform_pitch returns None, the note is filtered out.
                    if let Some(new_pitch) = new_pitch {
                        // Use with() to preserve sample_offset, struct update for the rest
                        output.push(event.with(MidiEventKind::NoteOn(NoteOn {
                            pitch: new_pitch,
                            velocity: new_velocity,
                            ..*note_on
                        })));
                    }
                    self.transformed_sender.push(Transformed::NoteOn {
                        channel: note_on.channel,
                        pitch: note_on.pitch,
                        out_pitch: new_pitch,
                        velocity: note_on.velocity,
                        out_velocity: new_velocity,
                    });
                }

                MidiEventKind::NoteOff(note_off) => {
                    self.press(note_off.channel, note_off.pitch, false);
                    self.transformed_sender.push(Transformed::NoteOff {
                        channel: note_off.channel,
                        pitch: note_off.pitch,
                    });

                    // The pitch the note-on went out with, not the one the
                    // current settings give. Nothing to end for a note that
                    // was filtered out.
                    if let Some(new_pitch) = self.held_note(note_off.channel, note_off.pitch).take() {
                        let new_velocity = if bypassed {
                            note_off.velocity
                        } else {
                            self.transform_velocity(note_off.velocity)
                        };
                        output.push(event.with(MidiEventKind::NoteOff(NoteOff {
                            pitch: new_pitch,
                            velocity: new_velocity,
                            ..*note_off
                        })));
                    }
                }

                MidiEventKind::PolyPressure(poly) => {
                    // Pressure follows the note it belongs to and is dropped
                    // for a note that is not sounding.
                    let held = *self.held_note(poly.channel, poly.pitch);
                    if let Some(new_pitch) = held {
                        output.push(event.with(MidiEventKind::PolyPressure(
                            PolyPressure {
                                pitch: new_pitch,
                                ..*poly
                            },
                        )));
                    }
                }

                MidiEventKind::ControlChange(cc) => {
                    // When bypassed, a CC goes out unchanged.
                    let new_cc = if bypassed {
                        Some((cc.controller, cc.value))
                    } else {
                        self.transform_cc_number(cc.controller).map(|controller| {
                            (controller, self.transform_cc_value(cc.controller, cc.value))
                        })
                    };
                    if let Some((new_controller, new_value)) = new_cc {
                        // The editor shows no note of the channel after it.
                        if ENDS_NOTES.contains(&new_controller) {
                            self.pressed_keys[(cc.channel & 0x0F) as usize] = 0;
                        }
                        output.push(event.with(MidiEventKind::ControlChange(
                            ControlChange {
                                controller: new_controller,
                                value: new_value,
                                ..*cc
                            },
                        )));
                        self.transformed_sender.push(Transformed::Cc {
                            channel: cc.channel,
                            controller: cc.controller,
                            value: cc.value,
                            out_controller: new_controller,
                            out_value: new_value,
                        });
                    }
                }

                // The bank select CCs before a program change pass on as
                // CCs, so the program change goes without its bank: with it,
                // the bank would be selected twice.
                MidiEventKind::ProgramChange(program) => {
                    output.push(event.with(MidiEventKind::ProgramChange(ProgramChange {
                        bank: None,
                        ..*program
                    })));
                }

                // Pass through other events unchanged
                _ => {
                    output.push(*event);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note_on(pitch: MidiNote) -> MidiEvent<'static> {
        MidiEvent::note_on(0, 0, pitch, 0.8, -1, 0.0, 0)
    }

    fn note_off(pitch: MidiNote) -> MidiEvent<'static> {
        MidiEvent::note_off(0, 0, pitch, 0.0, -1, 0.0)
    }

    /// The events the processor sends out for `input`, as (is note-on, pitch).
    fn run(processor: &mut MidiTransformProcessor, input: &[MidiEvent]) -> Vec<(bool, MidiNote)> {
        let mut output = MidiBuffer::new_boxed(0);
        processor.process_midi(input, &mut output);
        output
            .iter()
            .filter_map(|event| match &event.event {
                MidiEventKind::NoteOn(note) => Some((true, note.pitch)),
                MidiEventKind::NoteOff(note) => Some((false, note.pitch)),
                _ => None,
            })
            .collect()
    }

    fn remap(output_note: i64) -> (Arc<MidiTransformModel>, MidiTransformProcessor) {
        let model = Arc::new(MidiTransformModel::default());
        model.note.mode.set(NoteTransformMode::Remap);
        model.note.input_note.set(60);
        model.note.output_note.set(output_note);
        let processor = Arc::clone(&model).prepare(());
        (model, processor)
    }

    #[test]
    fn test_transformed_json_is_what_the_editor_reads() {
        // The shapes webview/src/transformed.ts checks.
        let note_on = Transformed::NoteOn {
            channel: 0,
            pitch: 60,
            out_pitch: None,
            velocity: 0.5,
            out_velocity: 0.25,
        };
        assert_eq!(
            serde_json::to_string(&note_on).unwrap(),
            r#"{"type":"noteOn","channel":0,"pitch":60,"outPitch":null,"velocity":0.5,"outVelocity":0.25}"#
        );
        let cc = Transformed::Cc { channel: 1, controller: 1, value: 0.5, out_controller: 11, out_value: 1.0 };
        assert_eq!(
            serde_json::to_string(&cc).unwrap(),
            r#"{"type":"cc","channel":1,"controller":1,"value":0.5,"outController":11,"outValue":1.0}"#
        );
        assert_eq!(serde_json::to_string(&Transformed::Reset).unwrap(), r#"{"type":"reset"}"#);
    }

    #[test]
    fn test_pressed_keys_count_every_key_that_is_down() {
        let model = Arc::new(MidiTransformModel::default());
        model.note.mode.set(NoteTransformMode::OctaveUp);
        let mut processor = Arc::clone(&model).prepare(());

        // A filtered note counts too: its key is down.
        run(&mut processor, &[note_on(60), note_on(120)]);
        assert_eq!(processor.pressed_count(), 2);
        run(&mut processor, &[note_off(60)]);
        assert_eq!(processor.pressed_count(), 1);

        // All Notes Off ends every note of its channel.
        run(&mut processor, &[MidiEvent::control_change(0, 0, 123, 0.0)]);
        assert_eq!(processor.pressed_count(), 0);

        run(&mut processor, &[note_on(64)]);
        processor.set_active(true);
        assert_eq!(processor.pressed_count(), 0);
    }

    #[test]
    fn test_bank_select_passes_on_once() {
        let model = Arc::new(MidiTransformModel::default());
        let mut processor = Arc::clone(&model).prepare(());
        let input = [
            MidiEvent::control_change(0, 0, 0, 0.0),
            MidiEvent::program_change(1, 0, 5, Some(0)),
        ];
        let mut output = MidiBuffer::new_boxed(0);
        processor.process_midi(&input, &mut output);
        assert_eq!(output.len(), 2);
        assert!(matches!(output.as_slice()[0].event, MidiEventKind::ControlChange(_)));
        assert!(matches!(
            output.as_slice()[1].event,
            MidiEventKind::ProgramChange(ProgramChange { program: 5, bank: None, .. })
        ));
    }

    #[test]
    fn test_note_off_ends_the_note_after_a_change() {
        let (model, mut processor) = remap(72);
        assert_eq!(run(&mut processor, &[note_on(60)]), [(true, 72)]);

        model.note.output_note.set(74);
        assert_eq!(run(&mut processor, &[note_off(60)]), [(false, 72)]);

        // The next note takes the new setting.
        assert_eq!(run(&mut processor, &[note_on(60), note_off(60)]), [(true, 74), (false, 74)]);
    }

    #[test]
    fn test_note_off_ends_the_note_across_a_bypass() {
        let (model, mut processor) = remap(72);
        assert_eq!(run(&mut processor, &[note_on(60)]), [(true, 72)]);
        model.bypass.set(true);
        assert_eq!(run(&mut processor, &[note_off(60)]), [(false, 72)]);

        // A note started while bypassed ends unchanged after the bypass.
        assert_eq!(run(&mut processor, &[note_on(60)]), [(true, 60)]);
        model.bypass.set(false);
        assert_eq!(run(&mut processor, &[note_off(60)]), [(false, 60)]);
    }

    #[test]
    fn test_filtered_note_sends_no_note_off() {
        let model = Arc::new(MidiTransformModel::default());
        model.note.mode.set(NoteTransformMode::OctaveUp);
        let mut processor = Arc::clone(&model).prepare(());

        // 120 + 12 is out of range, so the note is filtered out.
        assert_eq!(run(&mut processor, &[note_on(120)]), []);
        model.note.mode.set(NoteTransformMode::Through);
        assert_eq!(run(&mut processor, &[note_off(120)]), []);
    }
}
