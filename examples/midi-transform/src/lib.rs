//! Beamer MIDI Transform - Example VST3 instrument demonstrating advanced parameter features.
//!
//! This plugin showcases the Beamer parameter system and MIDI event handling:
//! - **Nested parameter groups** with `#[nested(group = "...")]`
//! - **EnumParameter** for discrete choices (transform modes)
//! - **IntParameter** for note/CC number selection
//! - **BoolParameter** for enable toggles
//! - **FloatParameter** for velocity/value scaling
//! - **Two-phase Plugin → AudioProcessor lifecycle**
//! - **Clean MIDI event modification** with the `with()` method
//!
//! # Features
//!
//! **Note Transform Group:**
//! - Enable/disable note processing
//! - Multiple transform modes (Transpose, Octave shifts, Remap, Invert)
//! - Note remapping (Note In → Note Out)
//! - Velocity scaling
//!
//! **CC Transform Group:**
//! - Enable/disable CC processing
//! - Multiple CC modes (Remap, Scale, Invert)
//! - CC number remapping (CC X → CC Y)
//! - Value scaling
//!
//! # MIDI Event Handling
//!
//! Use the `with()` method to transform MIDI events while preserving timing.
//! Combined with Rust's struct update syntax (`..`), this is clean and safe:
//!
//! ```ignore
//! MidiEventKind::NoteOn(note_on) => {
//!     output.push(event.clone().with(MidiEventKind::NoteOn(NoteOn {
//!         pitch: new_pitch,
//!         velocity: new_velocity,
//!         ..*note_on  // Copy channel, note_id, tuning, length
//!     })));
//! }
//! ```
//!
//! The `with()` method automatically preserves `sample_offset`, so you only
//! need to specify what changes.

use beamer::prelude::*;

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Unique ID for this plugin component (VST3 only).
#[cfg(feature = "vst3")]
const COMPONENT_UID: beamer::vst3::Steinberg::TUID =
    beamer::vst3::uid(0xA1B2C3D4, 0xE5F6A7B8, 0xC9D0E1F2, 0x03040506);

/// Shared plugin configuration (format-agnostic metadata)
pub static CONFIG: PluginConfig = PluginConfig::new("Beamer MIDI Transform")
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_category("Instrument")
    .with_sub_categories("Instrument");

/// VST3-specific configuration
#[cfg(feature = "vst3")]
pub static VST3_CONFIG: Vst3Config = Vst3Config::new(COMPONENT_UID);

/// AU-specific configuration
/// Uses manufacturer code "Bmer" and subtype "mtrn" for identification
/// MidiProcessor type indicates this processes MIDI data
#[cfg(feature = "au")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::MidiProcessor,
    fourcc!(b"Bmer"),
    fourcc!(b"mtrn"),
);

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
    /// Invert pitch around middle C (C4 = 60)
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
// Nested Parameter Groups
// =============================================================================

/// Note transformation parameters.
///
/// This is a nested parameter group that appears as "Note Transform" in the DAW.
/// Uses **declarative parameter definition** - no manual Default impl needed!
#[derive(Parameters)]
pub struct NoteTransformParameters {
    /// Enable note transformation
    #[parameter(id = "note_enabled", name = "Enabled", default = true)]
    pub enabled: BoolParameter,

    /// Transformation mode
    #[parameter(id = "note_mode", name = "Mode")]
    pub mode: EnumParameter<NoteTransformMode>,

    /// Transpose amount in semitones (-24 to +24)
    #[parameter(id = "note_transpose", name = "Transpose", default = 0, range = -24..=24, kind = "semitones")]
    pub transpose: IntParameter,

    /// Input note for remap mode (0-127)
    #[parameter(id = "note_input", name = "Input Note", default = 60, range = 0..=127)]
    pub input_note: IntParameter,

    /// Output note for remap mode (0-127)
    #[parameter(id = "note_output", name = "Output Note", default = 60, range = 0..=127)]
    pub output_note: IntParameter,

    /// Velocity scale (0.0 to 2.0, where 1.0 = 100%)
    #[parameter(id = "note_velocity", name = "Velocity", default = 1.0, range = 0.0..=2.0)]
    pub velocity_scale: FloatParameter,
}

/// CC transformation parameters.
///
/// This is a nested parameter group that appears as "CC Transform" in the DAW.
/// Uses **declarative parameter definition** - no manual Default impl needed!
#[derive(Parameters)]
pub struct CcTransformParameters {
    /// Enable CC transformation
    #[parameter(id = "cc_enabled", name = "Enabled", default = true)]
    pub enabled: BoolParameter,

    /// Transformation mode
    #[parameter(id = "cc_mode", name = "Mode")]
    pub mode: EnumParameter<CcTransformMode>,

    /// Input CC number (0-127) - Default: Mod wheel (CC 1)
    #[parameter(id = "cc_input", name = "Input CC", default = 1, range = 0..=127)]
    pub input_cc: IntParameter,

    /// Output CC number (0-127) - Default: Expression (CC 11)
    #[parameter(id = "cc_output", name = "Output CC", default = 11, range = 0..=127)]
    pub output_cc: IntParameter,

    /// Value scale (0.0 to 2.0, where 1.0 = 100%)
    #[parameter(id = "cc_scale", name = "Scale", default = 1.0, range = 0.0..=2.0)]
    pub value_scale: FloatParameter,
}

// =============================================================================
// Top-Level Parameters with Nested Groups
// =============================================================================

/// Main parameter structure with nested groups.
///
/// Demonstrates the `#[nested(group = "...")]` attribute for creating
/// hierarchical parameter organization in the DAW.
/// Uses **declarative parameter definition** - no manual Default impl needed!
#[derive(Parameters)]
pub struct MidiTransformParameters {
    /// Global bypass - uses the special `bypass` attribute
    #[parameter(id = "bypass", bypass)]
    pub bypass: BoolParameter,

    /// Note transformation parameters (nested group)
    #[nested(group = "Note Transform")]
    pub note: NoteTransformParameters,

    /// CC transformation parameters (nested group)
    #[nested(group = "CC Transform")]
    pub cc: CcTransformParameters,
}

// =============================================================================
// Plugin (Unprepared State)
// =============================================================================

/// The MIDI transform plugin in its unprepared state.
///
/// This struct holds the parameters before audio configuration is known.
/// When the host calls setupProcessing(), it is transformed into a
/// [`MidiTransformProcessor`] via the [`Plugin::prepare()`] method.
#[derive(Default, HasParameters)]
pub struct MidiTransformPlugin {
    #[parameters]
    parameters: MidiTransformParameters,
}

impl Plugin for MidiTransformPlugin {
    type Setup = SampleRate; // Needs sample rate for parameter smoothing
    type Processor = MidiTransformProcessor;

    fn prepare(mut self, setup: SampleRate) -> MidiTransformProcessor {
        self.parameters.set_sample_rate(setup.hz());

        MidiTransformProcessor {
            parameters: self.parameters,
        }
    }

    fn wants_midi(&self) -> bool {
        true // MIDI transformer needs MIDI input/output
    }
}

// =============================================================================
// Audio Processor (Prepared State)
// =============================================================================

/// MIDI transformer processor, ready for audio/MIDI processing.
///
/// Transforms MIDI notes and CC messages based on parameter settings.
#[derive(HasParameters)]
pub struct MidiTransformProcessor {
    #[parameters]
    parameters: MidiTransformParameters,
}

impl MidiTransformProcessor {
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
        if !self.parameters.note.enabled.get() {
            return Some(pitch);
        }

        let new_pitch = match self.parameters.note.mode.get() {
            NoteTransformMode::Through => pitch as i16,

            NoteTransformMode::Transpose => {
                pitch as i16 + self.parameters.note.transpose.get() as i16
            }

            NoteTransformMode::OctaveUp => pitch as i16 + 12,

            NoteTransformMode::OctaveDown => pitch as i16 - 12,

            NoteTransformMode::Remap => {
                if pitch == self.parameters.note.input_note.get() as u8 {
                    self.parameters.note.output_note.get() as i16
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
        if !self.parameters.note.enabled.get() {
            return velocity;
        }

        let scale = self.parameters.note.velocity_scale.get() as f32;
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
        if !self.parameters.cc.enabled.get() {
            return Some(cc);
        }

        match self.parameters.cc.mode.get() {
            CcTransformMode::Through | CcTransformMode::Scale | CcTransformMode::Invert => {
                Some(cc)
            }
            CcTransformMode::Remap | CcTransformMode::RemapAndScale => {
                if cc == self.parameters.cc.input_cc.get() as u8 {
                    Some(self.parameters.cc.output_cc.get() as u8)
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
    fn transform_cc_value(&self, cc: u8, value: f32) -> f32 {
        if !self.parameters.cc.enabled.get() {
            return value;
        }

        // Only transform if this is the targeted CC (for remap modes)
        // or if we're in a mode that affects all CCs
        let should_transform = match self.parameters.cc.mode.get() {
            CcTransformMode::Through => false,
            CcTransformMode::Scale | CcTransformMode::Invert => true,
            CcTransformMode::Remap => false, // Remap only changes number, not value
            CcTransformMode::RemapAndScale => {
                cc == self.parameters.cc.input_cc.get() as u8
            }
        };

        if !should_transform {
            return value;
        }

        match self.parameters.cc.mode.get() {
            CcTransformMode::Scale | CcTransformMode::RemapAndScale => {
                let scale = self.parameters.cc.value_scale.get() as f32;
                (value * scale).clamp(0.0, 1.0)
            }
            CcTransformMode::Invert => {
                1.0 - value
            }
            _ => value,
        }
    }
}

impl AudioProcessor for MidiTransformProcessor {
    type Plugin = MidiTransformPlugin;

    fn unprepare(self) -> MidiTransformPlugin {
        MidiTransformPlugin {
            parameters: self.parameters,
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        // Pass audio through unchanged
        buffer.copy_to_output();
    }

    fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
        // If bypassed, pass everything through unchanged
        if self.parameters.bypass.get() {
            for event in input {
                output.push(event.clone());
            }
            return;
        }

        for event in input {
            match &event.event {
                MidiEventKind::NoteOn(note_on) => {
                    if let Some(new_pitch) = self.transform_pitch(note_on.pitch) {
                        let new_velocity = self.transform_velocity(note_on.velocity);

                        // Use with() to preserve sample_offset, struct update for the rest
                        output.push(event.clone().with(MidiEventKind::NoteOn(NoteOn {
                            pitch: new_pitch,
                            velocity: new_velocity,
                            ..*note_on
                        })));
                    }
                    // If transform_pitch returns None, the note is filtered out
                }

                MidiEventKind::NoteOff(note_off) => {
                    if let Some(new_pitch) = self.transform_pitch(note_off.pitch) {
                        let new_velocity = self.transform_velocity(note_off.velocity);
                        output.push(event.clone().with(MidiEventKind::NoteOff(NoteOff {
                            pitch: new_pitch,
                            velocity: new_velocity,
                            ..*note_off
                        })));
                    }
                }

                MidiEventKind::PolyPressure(poly) => {
                    if let Some(new_pitch) = self.transform_pitch(poly.pitch) {
                        output.push(event.clone().with(MidiEventKind::PolyPressure(
                            PolyPressure {
                                pitch: new_pitch,
                                ..*poly
                            },
                        )));
                    }
                }

                MidiEventKind::ControlChange(cc) => {
                    if let Some(new_cc) = self.transform_cc_number(cc.controller) {
                        let new_value = self.transform_cc_value(cc.controller, cc.value);
                        output.push(event.clone().with(MidiEventKind::ControlChange(
                            ControlChange {
                                controller: new_cc,
                                value: new_value,
                                ..*cc
                            },
                        )));
                    }
                }

                // Pass through other events unchanged
                _ => {
                    output.push(event.clone());
                }
            }
        }
    }

    fn wants_midi(&self) -> bool {
        true
    }

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(self.parameters.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        self.parameters
            .load_state(data)
            .map_err(PluginError::StateError)
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, MidiTransformPlugin);

// =============================================================================
// Audio Unit Export
// =============================================================================

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, MidiTransformPlugin);
