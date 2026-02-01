//! MIDI event types for audio plugins.
//!
//! This module provides format-agnostic MIDI event types designed for
//! real-time audio processing. Most basic types (notes, CC, pitch bend)
//! are `Copy` and can be passed without heap allocation.
//!
//! ## SysEx Handling
//!
//! The [`MidiEventKind::SysEx`] variant uses `Box<SysEx>` to avoid stack
//! overflow from the large 512-byte SysEx buffer. As a result, [`MidiEvent`]
//! and [`MidiEventKind`] are `Clone` but not `Copy`.
//!
//! **Note:** Cloning a SysEx event allocates. For pass-through of SysEx in
//! `process_midi()`, consider whether allocation is acceptable for your use case.
//!
//! ## Buffer Sizes
//!
//! SysEx buffer size can be configured via Cargo features:
//! - Default: 512 bytes (a common default for audio plugins)
//! - `sysex-256`: 256 bytes (smaller memory footprint)
//! - `sysex-1024`: 1024 bytes
//! - `sysex-2048`: 2048 bytes

// =============================================================================
// Buffer Size Configuration
// =============================================================================

/// Maximum SysEx payload size in bytes.
///
/// Configurable via Cargo features: `sysex-256`, `sysex-1024`, `sysex-2048`.
/// Default is 512 bytes (a common default for audio plugins).
#[cfg(feature = "sysex-2048")]
pub const MAX_SYSEX_SIZE: usize = 2048;

/// Maximum SysEx payload size in bytes.
#[cfg(all(feature = "sysex-1024", not(feature = "sysex-2048")))]
pub const MAX_SYSEX_SIZE: usize = 1024;

/// Maximum SysEx payload size in bytes.
#[cfg(all(feature = "sysex-256", not(feature = "sysex-1024"), not(feature = "sysex-2048")))]
pub const MAX_SYSEX_SIZE: usize = 256;

/// Maximum SysEx payload size in bytes.
#[cfg(not(any(feature = "sysex-256", feature = "sysex-1024", feature = "sysex-2048")))]
pub const MAX_SYSEX_SIZE: usize = 512;

/// Maximum text size for Note Expression text events.
pub const MAX_EXPRESSION_TEXT_SIZE: usize = 64;

/// Maximum chord name size in bytes.
pub const MAX_CHORD_NAME_SIZE: usize = 32;

/// Maximum scale name size in bytes.
pub const MAX_SCALE_NAME_SIZE: usize = 32;

// =============================================================================
// Basic MIDI Types
// =============================================================================

/// MIDI channel (0-15).
pub type MidiChannel = u8;

/// MIDI note number (0-127, where 60 = middle C).
pub type MidiNote = u8;

/// Unique identifier for tracking note on/off pairs.
/// Use -1 when note ID is not available.
pub type NoteId = i32;

/// A MIDI note-on event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteOn {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Note number (0-127).
    pub pitch: MidiNote,
    /// Velocity (0.0 to 1.0, where 0.0 is silent).
    pub velocity: f32,
    /// Unique note ID for tracking this note instance.
    pub note_id: NoteId,
    /// Pitch offset in cents (-120.0 to +120.0) for microtonal/MPE support.
    pub tuning: f32,
    /// Note length in samples (0 = unknown/not provided).
    pub length: i32,
}

/// A MIDI note-off event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteOff {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Note number (0-127).
    pub pitch: MidiNote,
    /// Release velocity (0.0 to 1.0).
    pub velocity: f32,
    /// Unique note ID matching the original note-on.
    pub note_id: NoteId,
    /// Pitch offset in cents (-120.0 to +120.0) for microtonal/MPE support.
    pub tuning: f32,
}

/// Polyphonic key pressure (aftertouch per note).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolyPressure {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Note number (0-127).
    pub pitch: MidiNote,
    /// Pressure amount (0.0 to 1.0).
    pub pressure: f32,
    /// Unique note ID for tracking this note instance.
    pub note_id: NoteId,
}

/// Control Change (CC) message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControlChange {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Controller number (0-127).
    pub controller: u8,
    /// Controller value (0.0 to 1.0, normalized from 0-127).
    pub value: f32,
}

impl ControlChange {
    /// Check if this is a modulation wheel CC (CC1).
    #[inline]
    pub const fn is_mod_wheel(&self) -> bool {
        self.controller == cc::MOD_WHEEL
    }

    /// Check if this is a sustain pedal CC (CC64).
    #[inline]
    pub const fn is_sustain_pedal(&self) -> bool {
        self.controller == cc::SUSTAIN_PEDAL
    }

    /// Check if this is an expression pedal CC (CC11).
    #[inline]
    pub const fn is_expression(&self) -> bool {
        self.controller == cc::EXPRESSION
    }

    /// Check if this is a volume CC (CC7).
    #[inline]
    pub const fn is_volume(&self) -> bool {
        self.controller == cc::VOLUME
    }

    /// Check if sustain is pressed (value >= 0.5).
    #[inline]
    pub fn is_sustain_on(&self) -> bool {
        self.is_sustain_pedal() && self.value >= 0.5
    }

    // =========================================================================
    // Bank Select Helpers
    // =========================================================================

    /// Check if this is a Bank Select MSB (CC0).
    #[inline]
    pub const fn is_bank_select_msb(&self) -> bool {
        self.controller == cc::BANK_SELECT_MSB
    }

    /// Check if this is a Bank Select LSB (CC32).
    #[inline]
    pub const fn is_bank_select_lsb(&self) -> bool {
        self.controller == cc::BANK_SELECT_LSB
    }

    /// Check if this is any Bank Select message (CC0 or CC32).
    #[inline]
    pub const fn is_bank_select(&self) -> bool {
        self.is_bank_select_msb() || self.is_bank_select_lsb()
    }

    // =========================================================================
    // 14-bit Controller Helpers
    // =========================================================================

    /// Check if this controller is an MSB (CC 0-31) that has a corresponding LSB.
    ///
    /// MIDI defines CC 0-31 as MSB controllers with CC 32-63 as their LSB pairs.
    #[inline]
    pub const fn is_14bit_msb(&self) -> bool {
        self.controller < 32
    }

    /// Check if this controller is an LSB (CC 32-63) that pairs with an MSB.
    ///
    /// MIDI defines CC 32-63 as LSB controllers that pair with CC 0-31.
    #[inline]
    pub const fn is_14bit_lsb(&self) -> bool {
        self.controller >= 32 && self.controller < 64
    }

    /// Returns the LSB controller number for this MSB (CC 0-31 → CC 32-63).
    ///
    /// Returns `None` if this isn't an MSB controller.
    #[inline]
    pub const fn lsb_pair(&self) -> Option<u8> {
        if self.controller < 32 {
            Some(self.controller + 32)
        } else {
            None
        }
    }

    /// Returns the MSB controller number for this LSB (CC 32-63 → CC 0-31).
    ///
    /// Returns `None` if this isn't an LSB controller.
    #[inline]
    pub const fn msb_pair(&self) -> Option<u8> {
        if self.controller >= 32 && self.controller < 64 {
            Some(self.controller - 32)
        } else {
            None
        }
    }

    // =========================================================================
    // RPN/NRPN Detection Helpers
    // =========================================================================

    /// Check if this is an RPN MSB (CC 101).
    #[inline]
    pub const fn is_rpn_msb(&self) -> bool {
        self.controller == cc::RPN_MSB
    }

    /// Check if this is an RPN LSB (CC 100).
    #[inline]
    pub const fn is_rpn_lsb(&self) -> bool {
        self.controller == cc::RPN_LSB
    }

    /// Check if this is any RPN selection message (CC 100 or 101).
    #[inline]
    pub const fn is_rpn_select(&self) -> bool {
        self.controller == cc::RPN_MSB || self.controller == cc::RPN_LSB
    }

    /// Check if this is an NRPN MSB (CC 99).
    #[inline]
    pub const fn is_nrpn_msb(&self) -> bool {
        self.controller == cc::NRPN_MSB
    }

    /// Check if this is an NRPN LSB (CC 98).
    #[inline]
    pub const fn is_nrpn_lsb(&self) -> bool {
        self.controller == cc::NRPN_LSB
    }

    /// Check if this is any NRPN selection message (CC 98 or 99).
    #[inline]
    pub const fn is_nrpn_select(&self) -> bool {
        self.controller == cc::NRPN_MSB || self.controller == cc::NRPN_LSB
    }

    /// Check if this is a Data Entry MSB (CC 6).
    #[inline]
    pub const fn is_data_entry_msb(&self) -> bool {
        self.controller == cc::DATA_ENTRY_MSB
    }

    /// Check if this is a Data Entry LSB (CC 38).
    #[inline]
    pub const fn is_data_entry_lsb(&self) -> bool {
        self.controller == cc::DATA_ENTRY_LSB
    }

    /// Check if this is any Data Entry message (CC 6 or 38).
    #[inline]
    pub const fn is_data_entry(&self) -> bool {
        self.controller == cc::DATA_ENTRY_MSB || self.controller == cc::DATA_ENTRY_LSB
    }

    /// Check if this is a Data Increment (CC 96).
    #[inline]
    pub const fn is_data_increment(&self) -> bool {
        self.controller == cc::DATA_INCREMENT
    }

    /// Check if this is a Data Decrement (CC 97).
    #[inline]
    pub const fn is_data_decrement(&self) -> bool {
        self.controller == cc::DATA_DECREMENT
    }

    /// Check if this CC is part of an RPN/NRPN sequence.
    ///
    /// Returns true for CC 6, 38, 96-101.
    #[inline]
    pub const fn is_rpn_nrpn_related(&self) -> bool {
        matches!(
            self.controller,
            cc::DATA_ENTRY_MSB
                | cc::DATA_ENTRY_LSB
                | cc::DATA_INCREMENT
                | cc::DATA_DECREMENT
                | cc::NRPN_LSB
                | cc::NRPN_MSB
                | cc::RPN_LSB
                | cc::RPN_MSB
        )
    }
}

/// Pitch bend message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchBend {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Pitch bend value (-1.0 to 1.0, where 0.0 is center).
    pub value: f32,
}

/// Channel pressure (channel aftertouch).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelPressure {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Pressure amount (0.0 to 1.0).
    pub pressure: f32,
}

/// Program change message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgramChange {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Program number (0-127).
    pub program: u8,
}

// =============================================================================
// Advanced VST3 Events
// =============================================================================

/// System Exclusive (SysEx) message.
///
/// Uses a fixed-size buffer for efficient storage. When used in [`MidiEventKind`],
/// it is boxed (`Box<SysEx>`) to prevent the large buffer from bloating the enum
/// size and causing stack overflow.
///
/// The buffer size is configurable via Cargo features (default 512 bytes).
#[derive(Clone, Copy)]
pub struct SysEx {
    /// Raw SysEx data (excluding F0/F7 framing bytes).
    pub data: [u8; MAX_SYSEX_SIZE],
    /// Actual length of valid data in the buffer.
    pub len: u16,
}

impl SysEx {
    /// Create a new empty SysEx message.
    pub const fn new() -> Self {
        Self {
            data: [0u8; MAX_SYSEX_SIZE],
            len: 0,
        }
    }

    /// Get the valid data slice.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

impl Default for SysEx {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for SysEx {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SysEx")
            .field("len", &self.len)
            .field("data", &self.as_slice())
            .finish()
    }
}

impl PartialEq for SysEx {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.as_slice() == other.as_slice()
    }
}

/// Note Expression value event (f64 precision).
///
/// Used for MPE-style per-note modulation. Each playing note can have
/// independent expression values for volume, pan, tuning, etc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteExpressionValue {
    /// Note ID this expression applies to.
    pub note_id: NoteId,
    /// Expression type (see [`note_expression`] module for constants).
    pub expression_type: u32,
    /// Normalized value. Range depends on expression type:
    /// - Most types: 0.0 to 1.0
    /// - Tuning: -0.5 to 0.5 (semitones, can exceed for wider range)
    pub value: f64,
}

/// Note Expression integer value event.
///
/// Used for discrete expression values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteExpressionInt {
    /// Note ID this expression applies to.
    pub note_id: NoteId,
    /// Expression type.
    pub expression_type: u32,
    /// Integer value.
    pub value: u64,
}

/// Note Expression text event.
///
/// Used for text-based expression like phonemes for vocal synthesis.
#[derive(Clone, Copy)]
pub struct NoteExpressionText {
    /// Note ID this expression applies to.
    pub note_id: NoteId,
    /// Expression type (typically TEXT or PHONEME).
    pub expression_type: u32,
    /// UTF-8 text data.
    pub text: [u8; MAX_EXPRESSION_TEXT_SIZE],
    /// Actual length of text.
    pub text_len: u8,
}

impl NoteExpressionText {
    /// Get the text as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.text[..self.text_len as usize]).unwrap_or("")
    }
}

impl core::fmt::Debug for NoteExpressionText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NoteExpressionText")
            .field("note_id", &self.note_id)
            .field("expression_type", &self.expression_type)
            .field("text", &self.as_str())
            .finish()
    }
}

impl PartialEq for NoteExpressionText {
    fn eq(&self, other: &Self) -> bool {
        self.note_id == other.note_id
            && self.expression_type == other.expression_type
            && self.text_len == other.text_len
            && self.text[..self.text_len as usize] == other.text[..other.text_len as usize]
    }
}

/// Chord information from DAW chord track.
///
/// Provides harmonic context that plugins can use for intelligent processing.
#[derive(Clone, Copy)]
pub struct ChordInfo {
    /// Root note pitch class (0=C, 1=C#, ..., 11=B), -1 = invalid/unknown.
    pub root: i8,
    /// Bass note pitch class (for slash chords like C/G), -1 = same as root.
    pub bass_note: i8,
    /// Bitmask of chord tones relative to root.
    /// Bit 0 = root, bit 1 = minor 2nd, bit 2 = major 2nd, etc.
    pub mask: u16,
    /// Chord name as UTF-8 (e.g., "Cmaj7", "Dm").
    pub name: [u8; MAX_CHORD_NAME_SIZE],
    /// Actual length of name.
    pub name_len: u8,
}

impl ChordInfo {
    /// Get the chord name as a string slice.
    #[inline]
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }

    /// Check if the chord info is valid.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.root >= 0 && self.root < 12
    }
}

impl core::fmt::Debug for ChordInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChordInfo")
            .field("root", &self.root)
            .field("bass_note", &self.bass_note)
            .field("mask", &format_args!("{:#06x}", self.mask))
            .field("name", &self.name_str())
            .finish()
    }
}

impl PartialEq for ChordInfo {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && self.bass_note == other.bass_note
            && self.mask == other.mask
            && self.name_len == other.name_len
            && self.name[..self.name_len as usize] == other.name[..other.name_len as usize]
    }
}

/// Scale/key information from DAW.
///
/// Provides tonal context that plugins can use for scale-aware processing.
#[derive(Clone, Copy)]
pub struct ScaleInfo {
    /// Root note pitch class (0=C, 1=C#, ..., 11=B), -1 = invalid/unknown.
    pub root: i8,
    /// Bitmask of scale degrees (12 bits for chromatic scale).
    /// Bit 0 = root, bit 1 = minor 2nd, bit 2 = major 2nd, etc.
    pub mask: u16,
    /// Scale name as UTF-8 (e.g., "Major", "Dorian", "Pentatonic").
    pub name: [u8; MAX_SCALE_NAME_SIZE],
    /// Actual length of name.
    pub name_len: u8,
}

impl ScaleInfo {
    /// Get the scale name as a string slice.
    #[inline]
    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }

    /// Check if the scale info is valid.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.root >= 0 && self.root < 12
    }

    /// Check if a pitch class (0-11) is in the scale.
    #[inline]
    pub fn contains(&self, pitch_class: u8) -> bool {
        if pitch_class >= 12 {
            return false;
        }
        // Rotate mask so root is at bit 0
        let rotated = if self.root >= 0 {
            let shift = self.root as u32;
            (self.mask >> shift) | (self.mask << (12 - shift))
        } else {
            self.mask
        };
        (rotated & (1 << pitch_class)) != 0
    }
}

impl core::fmt::Debug for ScaleInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScaleInfo")
            .field("root", &self.root)
            .field("mask", &format_args!("{:#06x}", self.mask))
            .field("name", &self.name_str())
            .finish()
    }
}

impl PartialEq for ScaleInfo {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
            && self.mask == other.mask
            && self.name_len == other.name_len
            && self.name[..self.name_len as usize] == other.name[..other.name_len as usize]
    }
}

// =============================================================================
// MIDI 2.0 Types
// =============================================================================

/// MIDI 2.0 Controller identifier.
///
/// Represents a MIDI 2.0 controller which can be either a Registered Parameter
/// or an Assignable Controller, identified by bank and index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Midi2Controller {
    /// Controller bank (0-127).
    pub bank: u8,
    /// True for Registered Parameter, false for Assignable Controller.
    pub registered: bool,
    /// Controller index within the bank.
    pub index: u8,
}

impl Midi2Controller {
    /// Create a new MIDI 2.0 controller.
    pub const fn new(bank: u8, registered: bool, index: u8) -> Self {
        Self {
            bank,
            registered,
            index,
        }
    }

    /// Create a Registered Parameter controller.
    pub const fn registered(bank: u8, index: u8) -> Self {
        Self::new(bank, true, index)
    }

    /// Create an Assignable Controller.
    pub const fn assignable(bank: u8, index: u8) -> Self {
        Self::new(bank, false, index)
    }
}

// =============================================================================
// MIDI Control Change Constants
// =============================================================================

/// Common MIDI Control Change (CC) numbers.
pub mod cc {
    /// Bank Select MSB (CC0).
    pub const BANK_SELECT: u8 = 0;
    /// Bank Select MSB (CC0) - explicit name.
    pub const BANK_SELECT_MSB: u8 = 0;
    /// Bank Select LSB (CC32).
    pub const BANK_SELECT_LSB: u8 = 32;
    /// Modulation Wheel (CC1).
    pub const MOD_WHEEL: u8 = 1;
    /// Breath Controller (CC2).
    pub const BREATH: u8 = 2;
    /// Volume (CC7).
    pub const VOLUME: u8 = 7;
    /// Pan (CC10).
    pub const PAN: u8 = 10;
    /// Expression (CC11).
    pub const EXPRESSION: u8 = 11;
    /// Sustain Pedal (CC64).
    pub const SUSTAIN_PEDAL: u8 = 64;
    /// Portamento (CC65).
    pub const PORTAMENTO: u8 = 65;
    /// Sostenuto Pedal (CC66).
    pub const SOSTENUTO: u8 = 66;
    /// Soft Pedal (CC67).
    pub const SOFT_PEDAL: u8 = 67;
    /// All Sound Off (CC120).
    pub const ALL_SOUND_OFF: u8 = 120;
    /// Reset All Controllers (CC121).
    pub const RESET_ALL_CONTROLLERS: u8 = 121;
    /// All Notes Off (CC123).
    pub const ALL_NOTES_OFF: u8 = 123;

    // =========================================================================
    // System Messages (VST3 SDK 3.8.0)
    // =========================================================================

    /// Poly Pressure (virtual CC 131) - per-note aftertouch via LegacyMIDICCOut.
    pub const POLY_PRESSURE: u8 = 131;
    /// MTC Quarter Frame (virtual CC 132).
    pub const QUARTER_FRAME: u8 = 132;
    /// Song Select (virtual CC 133).
    pub const SONG_SELECT: u8 = 133;
    /// Song Position Pointer (virtual CC 134).
    pub const SONG_POSITION: u8 = 134;
    /// Cable Select (virtual CC 135).
    pub const CABLE_SELECT: u8 = 135;
    /// Tune Request (virtual CC 136).
    pub const TUNE_REQUEST: u8 = 136;
    /// MIDI Clock Start (virtual CC 137).
    pub const CLOCK_START: u8 = 137;
    /// MIDI Clock Continue (virtual CC 138).
    pub const CLOCK_CONTINUE: u8 = 138;
    /// MIDI Clock Stop (virtual CC 139).
    pub const CLOCK_STOP: u8 = 139;
    /// Active Sensing (virtual CC 140).
    pub const ACTIVE_SENSING: u8 = 140;

    // =========================================================================
    // RPN/NRPN Controllers
    // =========================================================================

    /// Data Entry MSB (CC6) - Value for RPN/NRPN.
    pub const DATA_ENTRY_MSB: u8 = 6;
    /// Data Entry LSB (CC38) - Fine value for RPN/NRPN.
    pub const DATA_ENTRY_LSB: u8 = 38;
    /// Data Increment (CC96) - Increment RPN/NRPN value.
    pub const DATA_INCREMENT: u8 = 96;
    /// Data Decrement (CC97) - Decrement RPN/NRPN value.
    pub const DATA_DECREMENT: u8 = 97;
    /// NRPN LSB (CC98) - Non-Registered Parameter Number LSB.
    pub const NRPN_LSB: u8 = 98;
    /// NRPN MSB (CC99) - Non-Registered Parameter Number MSB.
    pub const NRPN_MSB: u8 = 99;
    /// RPN LSB (CC100) - Registered Parameter Number LSB.
    pub const RPN_LSB: u8 = 100;
    /// RPN MSB (CC101) - Registered Parameter Number MSB.
    pub const RPN_MSB: u8 = 101;
}

// =============================================================================
// Registered Parameter Numbers (RPNs)
// =============================================================================

/// Well-known Registered Parameter Numbers (RPNs).
///
/// These are standard MIDI parameters with defined meanings across all devices.
/// RPN messages are sent using CC 101 (MSB) and CC 100 (LSB) to select the
/// parameter, followed by CC 6 (Data Entry MSB) and optionally CC 38 (LSB)
/// to set the value.
///
/// # Example
///
/// To set Pitch Bend Sensitivity to 12 semitones:
/// ```text
/// CC 101 = 0   (RPN MSB = 0)
/// CC 100 = 0   (RPN LSB = 0)  → Selects Pitch Bend Sensitivity
/// CC 6   = 12  (Data Entry = 12 semitones)
/// CC 101 = 127 (RPN Null)
/// CC 100 = 127 (RPN Null)     → Deselect to prevent accidental changes
/// ```
pub mod rpn {
    /// Pitch Bend Sensitivity (semitones + cents).
    ///
    /// Data Entry MSB = semitones (0-127, typically 0-24).
    /// Data Entry LSB = cents (0-127, typically 0).
    /// Default is usually 2 semitones.
    pub const PITCH_BEND_SENSITIVITY: u16 = 0x0000;

    /// Channel Fine Tuning (cents, 14-bit).
    ///
    /// Value 0x2000 (8192) = A440 (no change).
    /// Range: +/- 100 cents (approximately 1 semitone).
    pub const FINE_TUNING: u16 = 0x0001;

    /// Channel Coarse Tuning (semitones).
    ///
    /// Data Entry MSB = semitones offset from A440.
    /// Value 64 = A440 (no change).
    /// Range: +/- 64 semitones.
    pub const COARSE_TUNING: u16 = 0x0002;

    /// Tuning Program Change.
    ///
    /// Selects a tuning program (0-127) from the currently selected bank.
    pub const TUNING_PROGRAM: u16 = 0x0003;

    /// Tuning Bank Select.
    ///
    /// Selects a tuning bank (0-127).
    pub const TUNING_BANK: u16 = 0x0004;

    /// Modulation Depth Range (MPE).
    ///
    /// Sets the range for per-note pitch bend in MPE mode.
    /// Data Entry MSB = semitones, LSB = cents.
    pub const MODULATION_DEPTH: u16 = 0x0005;

    /// MPE Configuration Message.
    ///
    /// Used to configure MPE zones. Sent on the Manager Channel.
    /// Data Entry MSB = number of Member Channels (0 = disable MPE).
    pub const MPE_CONFIGURATION: u16 = 0x0006;

    /// RPN Null - Deselects RPN/NRPN (no parameter selected).
    ///
    /// Send after setting an RPN/NRPN value to prevent accidental
    /// data entry changes from affecting parameters.
    pub const NULL: u16 = 0x7F7F;

    /// Check if a parameter number represents RPN Null.
    #[inline]
    pub const fn is_null(parameter: u16) -> bool {
        parameter == NULL
    }
}

// =============================================================================
// RPN/NRPN Message Types
// =============================================================================

/// Type of parameter number (RPN vs NRPN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterNumberKind {
    /// Registered Parameter Number (CC 100/101).
    /// Standard MIDI parameters with defined meanings.
    Rpn,
    /// Non-Registered Parameter Number (CC 98/99).
    /// Manufacturer/device-specific parameters.
    Nrpn,
}

/// A complete RPN or NRPN message with its 14-bit value.
///
/// This represents a fully-decoded RPN/NRPN sequence after the [`RpnTracker`]
/// has assembled all the CC messages.
///
/// # Example
///
/// ```ignore
/// use beamer_core::{RpnTracker, ControlChange, cc, rpn};
///
/// let mut tracker = RpnTracker::new();
///
/// // Simulate receiving CC sequence for Pitch Bend Sensitivity = 12 semitones
/// tracker.process_cc(&ControlChange { channel: 0, controller: cc::RPN_MSB, value: 0.0 });
/// tracker.process_cc(&ControlChange { channel: 0, controller: cc::RPN_LSB, value: 0.0 });
/// let msg = tracker.process_cc(&ControlChange { channel: 0, controller: cc::DATA_ENTRY_MSB, value: 12.0/127.0 });
///
/// if let Some(msg) = msg {
///     assert!(msg.is_pitch_bend_sensitivity());
///     let (semitones, cents) = msg.pitch_bend_sensitivity();
///     assert_eq!(semitones, 12);
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterNumberMessage {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// RPN or NRPN.
    pub kind: ParameterNumberKind,
    /// 14-bit parameter number (MSB << 7 | LSB).
    pub parameter: u16,
    /// 14-bit data value, normalized to 0.0-1.0.
    pub value: f32,
    /// Whether this was a data increment (+1 to current value).
    pub is_increment: bool,
    /// Whether this was a data decrement (-1 from current value).
    pub is_decrement: bool,
}

impl ParameterNumberMessage {
    /// Create a new RPN message.
    pub const fn rpn(channel: MidiChannel, parameter: u16, value: f32) -> Self {
        Self {
            channel,
            kind: ParameterNumberKind::Rpn,
            parameter,
            value,
            is_increment: false,
            is_decrement: false,
        }
    }

    /// Create a new NRPN message.
    pub const fn nrpn(channel: MidiChannel, parameter: u16, value: f32) -> Self {
        Self {
            channel,
            kind: ParameterNumberKind::Nrpn,
            parameter,
            value,
            is_increment: false,
            is_decrement: false,
        }
    }

    /// Check if this is an RPN.
    #[inline]
    pub const fn is_rpn(&self) -> bool {
        matches!(self.kind, ParameterNumberKind::Rpn)
    }

    /// Check if this is an NRPN.
    #[inline]
    pub const fn is_nrpn(&self) -> bool {
        matches!(self.kind, ParameterNumberKind::Nrpn)
    }

    /// Check if this is the Pitch Bend Sensitivity RPN.
    #[inline]
    pub fn is_pitch_bend_sensitivity(&self) -> bool {
        self.is_rpn() && self.parameter == rpn::PITCH_BEND_SENSITIVITY
    }

    /// Check if this is the RPN Null message.
    #[inline]
    pub fn is_null(&self) -> bool {
        self.is_rpn() && rpn::is_null(self.parameter)
    }

    /// Get the raw 14-bit value (0-16383).
    #[inline]
    pub fn raw_value(&self) -> u16 {
        (self.value.clamp(0.0, 1.0) * 16383.0) as u16
    }

    /// For Pitch Bend Sensitivity: get semitones and cents.
    ///
    /// Returns (semitones, cents) where MSB = semitones (0-127)
    /// and LSB = cents (0-127).
    #[inline]
    pub fn pitch_bend_sensitivity(&self) -> (u8, u8) {
        let raw = self.raw_value();
        let msb = ((raw >> 7) & 0x7F) as u8;
        let lsb = (raw & 0x7F) as u8;
        (msb, lsb)
    }
}

// =============================================================================
// RPN/NRPN Tracker
// =============================================================================

/// Per-channel RPN/NRPN state for tracking multi-CC sequences.
#[derive(Debug, Clone, Copy, Default)]
struct RpnChannelState {
    /// Currently selected parameter MSB (CC 99/101).
    parameter_msb: Option<u8>,
    /// Currently selected parameter LSB (CC 98/100).
    parameter_lsb: Option<u8>,
    /// Current data entry MSB (CC 6).
    data_msb: Option<u8>,
    /// Current data entry LSB (CC 38).
    data_lsb: Option<u8>,
    /// Whether the current selection is RPN (true) or NRPN (false).
    is_rpn: bool,
}

impl RpnChannelState {
    /// Reset all state (e.g., after RPN Null).
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// Check if we have a complete parameter selection.
    fn has_parameter(&self) -> bool {
        self.parameter_msb.is_some() && self.parameter_lsb.is_some()
    }

    /// Get the 14-bit parameter number if both MSB and LSB are set.
    fn parameter(&self) -> Option<u16> {
        match (self.parameter_msb, self.parameter_lsb) {
            (Some(msb), Some(lsb)) => Some(combine_14bit_raw(msb, lsb)),
            _ => None,
        }
    }

    /// Get the 14-bit data value if MSB is set (LSB defaults to 0).
    fn data_value(&self) -> Option<u16> {
        self.data_msb.map(|msb| {
            let lsb = self.data_lsb.unwrap_or(0);
            combine_14bit_raw(msb, lsb)
        })
    }
}

/// Tracks RPN/NRPN state across all 16 MIDI channels.
///
/// This struct is designed for real-time safety:
/// - Fixed-size array (no heap allocation)
/// - All operations are O(1)
/// - Implements `Copy` for simple value semantics
///
/// # Usage
///
/// Plugins that need to receive RPN/NRPN messages should store an instance
/// of this tracker in their state and call `process_cc` for each incoming
/// Control Change event.
///
/// ```ignore
/// struct MyPlugin {
///     rpn_tracker: RpnTracker,
/// }
///
/// impl Processor for MyPlugin {
///     fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
///         for event in input {
///             if let MidiEventKind::ControlChange(cc) = &event.event {
///                 if let Some(msg) = self.rpn_tracker.process_cc(cc) {
///                     // Handle complete RPN/NRPN message
///                     if msg.is_pitch_bend_sensitivity() {
///                         let (semitones, cents) = msg.pitch_bend_sensitivity();
///                         self.pitch_bend_range = semitones as f32 + cents as f32 / 100.0;
///                     }
///                 }
///             }
///         }
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct RpnTracker {
    /// Per-channel state for all 16 MIDI channels.
    channels: [RpnChannelState; 16],
}

impl Default for RpnTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl RpnTracker {
    /// Create a new RPN tracker with all channels in their default state.
    pub const fn new() -> Self {
        Self {
            channels: [RpnChannelState {
                parameter_msb: None,
                parameter_lsb: None,
                data_msb: None,
                data_lsb: None,
                is_rpn: false,
            }; 16],
        }
    }

    /// Reset all channel states.
    pub fn reset(&mut self) {
        for channel in &mut self.channels {
            channel.reset();
        }
    }

    /// Reset a specific channel's state.
    pub fn reset_channel(&mut self, channel: MidiChannel) {
        if (channel as usize) < 16 {
            self.channels[channel as usize].reset();
        }
    }

    /// Process a Control Change event.
    ///
    /// Returns `Some(ParameterNumberMessage)` when a complete RPN/NRPN
    /// message has been assembled from the CC sequence.
    ///
    /// # Arguments
    /// * `cc` - The Control Change event to process
    ///
    /// # Returns
    /// - `None` for non-RPN/NRPN CCs or incomplete sequences
    /// - `Some(message)` when a complete RPN/NRPN is ready
    pub fn process_cc(&mut self, cc: &ControlChange) -> Option<ParameterNumberMessage> {
        let channel_idx = (cc.channel as usize) & 0x0F;
        let state = &mut self.channels[channel_idx];

        // Convert normalized value back to 7-bit
        let value_7bit = (cc.value.clamp(0.0, 1.0) * 127.0) as u8;

        match cc.controller {
            // RPN parameter selection
            cc::RPN_MSB => {
                state.parameter_msb = Some(value_7bit);
                state.is_rpn = true;
                // Clear data values on new parameter selection
                state.data_msb = None;
                state.data_lsb = None;
                None
            }
            cc::RPN_LSB => {
                state.parameter_lsb = Some(value_7bit);
                state.is_rpn = true;
                // Check for RPN Null
                if let Some(parameter) = state.parameter() {
                    if rpn::is_null(parameter) {
                        state.reset();
                    }
                }
                None
            }

            // NRPN parameter selection
            cc::NRPN_MSB => {
                state.parameter_msb = Some(value_7bit);
                state.is_rpn = false;
                state.data_msb = None;
                state.data_lsb = None;
                None
            }
            cc::NRPN_LSB => {
                state.parameter_lsb = Some(value_7bit);
                state.is_rpn = false;
                None
            }

            // Data Entry MSB - may complete the message
            cc::DATA_ENTRY_MSB => {
                state.data_msb = Some(value_7bit);
                self.try_emit_message(channel_idx, false, false)
            }

            // Data Entry LSB - may complete the message
            cc::DATA_ENTRY_LSB => {
                state.data_lsb = Some(value_7bit);
                // Only emit if we already have MSB
                if self.channels[channel_idx].data_msb.is_some() {
                    self.try_emit_message(channel_idx, false, false)
                } else {
                    None
                }
            }

            // Data Increment
            cc::DATA_INCREMENT => {
                if self.channels[channel_idx].has_parameter() {
                    self.try_emit_message(channel_idx, true, false)
                } else {
                    None
                }
            }

            // Data Decrement
            cc::DATA_DECREMENT => {
                if self.channels[channel_idx].has_parameter() {
                    self.try_emit_message(channel_idx, false, true)
                } else {
                    None
                }
            }

            _ => None,
        }
    }

    /// Try to emit a complete RPN/NRPN message.
    fn try_emit_message(
        &self,
        channel_idx: usize,
        is_increment: bool,
        is_decrement: bool,
    ) -> Option<ParameterNumberMessage> {
        let state = &self.channels[channel_idx];

        let parameter = state.parameter()?;

        // For increment/decrement, we don't need a data value
        let value = if is_increment || is_decrement {
            0.0 // Value is relative, not absolute
        } else {
            let raw = state.data_value()?;
            raw as f32 / 16383.0
        };

        Some(ParameterNumberMessage {
            channel: channel_idx as u8,
            kind: if state.is_rpn {
                ParameterNumberKind::Rpn
            } else {
                ParameterNumberKind::Nrpn
            },
            parameter,
            value,
            is_increment,
            is_decrement,
        })
    }

    /// Get the currently selected parameter for a channel, if any.
    pub fn current_parameter(&self, channel: MidiChannel) -> Option<(ParameterNumberKind, u16)> {
        let state = &self.channels[(channel as usize) & 0x0F];
        state.parameter().map(|p| {
            let kind = if state.is_rpn {
                ParameterNumberKind::Rpn
            } else {
                ParameterNumberKind::Nrpn
            };
            (kind, p)
        })
    }
}

// =============================================================================
// 14-bit Controller Utilities
// =============================================================================

/// Combines MSB and LSB controller values into a single 14-bit normalized value.
///
/// MIDI CC 0-31 are MSB controllers, and CC 32-63 are their corresponding LSB pairs.
/// Together they provide 14-bit resolution (0-16383) instead of 7-bit (0-127).
///
/// # Arguments
/// * `msb_value` - MSB controller value (0.0 to 1.0, normalized from CC 0-31)
/// * `lsb_value` - LSB controller value (0.0 to 1.0, normalized from CC 32-63)
///
/// # Returns
/// Combined 14-bit value normalized to 0.0-1.0
///
/// # Example
/// ```
/// use beamer_core::midi::combine_14bit_cc;
///
/// // Full resolution: MSB=127, LSB=127 → 16383 → 1.0
/// assert!((combine_14bit_cc(1.0, 1.0) - 1.0).abs() < 0.001);
///
/// // Center value: MSB=64, LSB=0 → 8192 → ~0.5
/// assert!((combine_14bit_cc(0.504, 0.0) - 0.5).abs() < 0.01);
/// ```
#[inline]
pub fn combine_14bit_cc(msb_value: f32, lsb_value: f32) -> f32 {
    let msb = (msb_value.clamp(0.0, 1.0) * 127.0) as u16;
    let lsb = (lsb_value.clamp(0.0, 1.0) * 127.0) as u16;
    let combined = (msb << 7) | (lsb & 0x7F);
    combined as f32 / 16383.0
}

/// Splits a 14-bit normalized value into MSB and LSB controller values.
///
/// This is the inverse of [`combine_14bit_cc`].
///
/// # Arguments
/// * `value` - Combined 14-bit value (0.0 to 1.0)
///
/// # Returns
/// Tuple of (msb_value, lsb_value), both normalized to 0.0-1.0
///
/// # Example
/// ```
/// use beamer_core::midi::{split_14bit_cc, combine_14bit_cc};
///
/// // Round-trip test: split then combine should give same value
/// let original = 0.75;
/// let (msb, lsb) = split_14bit_cc(original);
/// let reconstructed = combine_14bit_cc(msb, lsb);
/// assert!((original - reconstructed).abs() < 0.001);
///
/// // Full value splits to (1.0, 1.0)
/// let (msb, lsb) = split_14bit_cc(1.0);
/// assert!((msb - 1.0).abs() < 0.01);
/// assert!((lsb - 1.0).abs() < 0.01);
/// ```
#[inline]
pub fn split_14bit_cc(value: f32) -> (f32, f32) {
    let raw = (value.clamp(0.0, 1.0) * 16383.0) as u16;
    let msb = ((raw >> 7) & 0x7F) as f32 / 127.0;
    let lsb = (raw & 0x7F) as f32 / 127.0;
    (msb, lsb)
}

/// Combines two raw 7-bit values into a 14-bit value.
///
/// # Arguments
/// * `msb` - MSB value (0-127)
/// * `lsb` - LSB value (0-127)
///
/// # Returns
/// Combined 14-bit value (0-16383)
#[inline]
pub const fn combine_14bit_raw(msb: u8, lsb: u8) -> u16 {
    ((msb as u16) << 7) | ((lsb as u16) & 0x7F)
}

/// Splits a 14-bit value into MSB and LSB components.
///
/// # Arguments
/// * `value` - 14-bit value (0-16383)
///
/// # Returns
/// Tuple of (msb, lsb), both 0-127
#[inline]
pub const fn split_14bit_raw(value: u16) -> (u8, u8) {
    let msb = ((value >> 7) & 0x7F) as u8;
    let lsb = (value & 0x7F) as u8;
    (msb, lsb)
}

// =============================================================================
// Note Expression Constants
// =============================================================================

/// VST3 Note Expression type IDs.
///
/// These constants identify the type of per-note expression in
/// [`NoteExpressionValue`], [`NoteExpressionInt`], and [`NoteExpressionText`] events.
pub mod note_expression {
    /// Per-note volume (0.0 = silent, 1.0 = full).
    pub const VOLUME: u32 = 0;
    /// Per-note pan (-1.0 = left, 0.0 = center, 1.0 = right).
    pub const PAN: u32 = 1;
    /// Per-note tuning in semitones. Critical for MPE pitch bend.
    /// Typically -0.5 to 0.5 for standard pitch bend range.
    pub const TUNING: u32 = 2;
    /// Per-note vibrato depth (0.0 to 1.0).
    pub const VIBRATO: u32 = 3;
    /// Per-note expression (general purpose, 0.0 to 1.0).
    pub const EXPRESSION: u32 = 4;
    /// Per-note brightness/timbre (0.0 to 1.0).
    pub const BRIGHTNESS: u32 = 5;
    /// Text expression type.
    pub const TEXT: u32 = 6;
    /// Phoneme expression type (for vocal synthesis).
    pub const PHONEME: u32 = 7;
    /// Start of custom expression type range.
    pub const CUSTOM_START: u32 = 100000;
    /// End of custom expression type range.
    pub const CUSTOM_END: u32 = 200000;
    /// Invalid type ID.
    pub const INVALID: u32 = u32::MAX;
}

// =============================================================================
// MIDI Event Enum
// =============================================================================

/// MIDI event types.
///
/// Most variants are small (8-32 bytes). The `SysEx` variant uses `Box<SysEx>`
/// to avoid bloating the enum size and prevent stack overflow.
#[derive(Debug, Clone, PartialEq)]
pub enum MidiEventKind {
    // =========================================================================
    // Note-related events (have note_id for tracking)
    // =========================================================================

    /// Note on event.
    NoteOn(NoteOn),
    /// Note off event.
    NoteOff(NoteOff),
    /// Polyphonic key pressure (per-note aftertouch).
    PolyPressure(PolyPressure),

    // =========================================================================
    // Channel-wide events
    // =========================================================================

    /// Control change (CC).
    ControlChange(ControlChange),
    /// Pitch bend.
    PitchBend(PitchBend),
    /// Channel pressure (channel aftertouch).
    ChannelPressure(ChannelPressure),
    /// Program change.
    ProgramChange(ProgramChange),

    // =========================================================================
    // Advanced VST3 events
    // =========================================================================

    /// System Exclusive (SysEx) message.
    ///
    /// Uses `Box<SysEx>` to avoid bloating the enum size. SysEx messages are
    /// relatively rare compared to notes and CCs, so the heap allocation is acceptable.
    SysEx(Box<SysEx>),
    /// Per-note expression value (MPE, f64 precision).
    NoteExpressionValue(NoteExpressionValue),
    /// Per-note expression integer value.
    NoteExpressionInt(NoteExpressionInt),
    /// Per-note expression text.
    NoteExpressionText(NoteExpressionText),
    /// Chord information from DAW chord track.
    ChordInfo(ChordInfo),
    /// Scale/key information from DAW.
    ScaleInfo(ScaleInfo),
}

/// A sample-accurate MIDI event.
///
/// The `sample_offset` field specifies when within the current audio buffer
/// this event should be processed, enabling sample-accurate MIDI timing.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiEvent {
    /// Sample offset within the current buffer (0 = start of buffer).
    pub sample_offset: u32,
    /// The MIDI event data.
    pub event: MidiEventKind,
}

impl Default for MidiEvent {
    /// Creates a default MidiEvent (NoteOff with all fields zeroed).
    ///
    /// Used for buffer initialization. Does not allocate.
    fn default() -> Self {
        Self {
            sample_offset: 0,
            event: MidiEventKind::NoteOff(NoteOff {
                channel: 0,
                pitch: 0,
                velocity: 0.0,
                note_id: -1,
                tuning: 0.0,
            }),
        }
    }
}

impl MidiEvent {
    /// Create a new note-on event.
    pub const fn note_on(
        sample_offset: u32,
        channel: MidiChannel,
        pitch: MidiNote,
        velocity: f32,
        note_id: NoteId,
        tuning: f32,
        length: i32,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::NoteOn(NoteOn {
                channel,
                pitch,
                velocity,
                note_id,
                tuning,
                length,
            }),
        }
    }

    /// Create a new note-off event.
    pub const fn note_off(
        sample_offset: u32,
        channel: MidiChannel,
        pitch: MidiNote,
        velocity: f32,
        note_id: NoteId,
        tuning: f32,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::NoteOff(NoteOff {
                channel,
                pitch,
                velocity,
                note_id,
                tuning,
            }),
        }
    }

    /// Create a polyphonic pressure event.
    pub const fn poly_pressure(
        sample_offset: u32,
        channel: MidiChannel,
        pitch: MidiNote,
        pressure: f32,
        note_id: NoteId,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::PolyPressure(PolyPressure {
                channel,
                pitch,
                pressure,
                note_id,
            }),
        }
    }

    /// Create a control change event.
    pub const fn control_change(
        sample_offset: u32,
        channel: MidiChannel,
        controller: u8,
        value: f32,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::ControlChange(ControlChange {
                channel,
                controller,
                value,
            }),
        }
    }

    /// Create a pitch bend event.
    pub const fn pitch_bend(sample_offset: u32, channel: MidiChannel, value: f32) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::PitchBend(PitchBend { channel, value }),
        }
    }

    /// Create a channel pressure event.
    pub const fn channel_pressure(
        sample_offset: u32,
        channel: MidiChannel,
        pressure: f32,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::ChannelPressure(ChannelPressure { channel, pressure }),
        }
    }

    /// Create a program change event.
    pub const fn program_change(sample_offset: u32, channel: MidiChannel, program: u8) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::ProgramChange(ProgramChange { channel, program }),
        }
    }

    // =========================================================================
    // Raw MIDI 1.0 byte parsing
    // =========================================================================

    /// Parse a MIDI 1.0 channel voice message from raw bytes.
    ///
    /// This is the standard way to convert raw MIDI bytes (as received from
    /// plugin hosts or hardware MIDI) into beamer's `MidiEvent` format.
    ///
    /// # Arguments
    ///
    /// * `sample_offset` - Sample position within the current buffer
    /// * `status` - MIDI status byte with channel masked out (0x80-0xF0)
    /// * `channel` - MIDI channel (0-15)
    /// * `data1` - First data byte (note number, CC number, etc.)
    /// * `data2` - Second data byte (velocity, CC value, etc.)
    ///
    /// # Returns
    ///
    /// `Some(MidiEvent)` for supported channel voice messages, `None` for
    /// unsupported message types (system messages, etc.)
    ///
    /// # Supported Messages
    ///
    /// | Status | Message Type     | data1        | data2         |
    /// |--------|------------------|--------------|---------------|
    /// | 0x80   | Note Off         | note number  | velocity      |
    /// | 0x90   | Note On          | note number  | velocity      |
    /// | 0xA0   | Poly Pressure    | note number  | pressure      |
    /// | 0xB0   | Control Change   | CC number    | value         |
    /// | 0xC0   | Program Change   | program      | (ignored)     |
    /// | 0xD0   | Channel Pressure | pressure     | (ignored)     |
    /// | 0xE0   | Pitch Bend       | LSB          | MSB           |
    ///
    /// # Notes
    ///
    /// - Note On with velocity 0 is converted to Note Off (per MIDI spec)
    /// - Velocities and CC values are normalized: 0-127 → 0.0-1.0
    /// - Pitch bend is normalized: 0-16383 → -1.0 to 1.0 (center at 8192)
    /// - `note_id` is set to the pitch value for basic voice allocation
    /// - `tuning` and `length` default to 0
    ///
    /// # Example
    ///
    /// ```
    /// use beamer_core::MidiEvent;
    ///
    /// // Parse a Note On: channel 0, note 60 (middle C), velocity 100
    /// let event = MidiEvent::from_midi1_bytes(0, 0x90, 0, 60, 100);
    /// assert!(event.is_some());
    /// ```
    #[inline]
    pub fn from_midi1_bytes(
        sample_offset: u32,
        status: u8,
        channel: MidiChannel,
        data1: u8,
        data2: u8,
    ) -> Option<Self> {
        match status {
            0x80 => Some(Self::note_off(
                sample_offset,
                channel,
                data1,
                data2 as f32 / 127.0,
                data1 as NoteId,
                0.0,
            )),
            0x90 => {
                if data2 == 0 {
                    // Note On with velocity 0 = Note Off (per MIDI spec)
                    Some(Self::note_off(
                        sample_offset,
                        channel,
                        data1,
                        0.0,
                        data1 as NoteId,
                        0.0,
                    ))
                } else {
                    Some(Self::note_on(
                        sample_offset,
                        channel,
                        data1,
                        data2 as f32 / 127.0,
                        data1 as NoteId,
                        0.0,
                        0,
                    ))
                }
            }
            0xA0 => Some(Self::poly_pressure(
                sample_offset,
                channel,
                data1,
                data2 as f32 / 127.0,
                data1 as NoteId,
            )),
            0xB0 => Some(Self::control_change(
                sample_offset,
                channel,
                data1,
                data2 as f32 / 127.0,
            )),
            0xC0 => Some(Self::program_change(sample_offset, channel, data1)),
            0xD0 => Some(Self::channel_pressure(
                sample_offset,
                channel,
                data1 as f32 / 127.0,
            )),
            0xE0 => {
                // Pitch bend: data1 = LSB (bits 0-6), data2 = MSB (bits 7-13)
                // Raw value: 0-16383, center at 8192
                // Normalized: -1.0 to 1.0
                let raw_value = ((data2 as u16) << 7) | (data1 as u16);
                let normalized = (raw_value as f32 - 8192.0) / 8192.0;
                Some(Self::pitch_bend(sample_offset, channel, normalized))
            }
            _ => None, // System messages, etc. not supported
        }
    }

    // =========================================================================
    // Advanced VST3 event constructors
    // =========================================================================

    /// Create a SysEx event.
    ///
    /// Note: This allocates the SysEx data on the heap. SysEx messages are
    /// relatively rare, so the allocation is acceptable.
    pub fn sysex(sample_offset: u32, data: &[u8]) -> Self {
        let mut sysex = SysEx::new();
        let copy_len = data.len().min(MAX_SYSEX_SIZE);
        sysex.data[..copy_len].copy_from_slice(&data[..copy_len]);
        sysex.len = copy_len as u16;
        Self {
            sample_offset,
            event: MidiEventKind::SysEx(Box::new(sysex)),
        }
    }

    /// Create a Note Expression value event.
    pub const fn note_expression_value(
        sample_offset: u32,
        note_id: NoteId,
        expression_type: u32,
        value: f64,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::NoteExpressionValue(NoteExpressionValue {
                note_id,
                expression_type,
                value,
            }),
        }
    }

    /// Create a Note Expression integer event.
    pub const fn note_expression_int(
        sample_offset: u32,
        note_id: NoteId,
        expression_type: u32,
        value: u64,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::NoteExpressionInt(NoteExpressionInt {
                note_id,
                expression_type,
                value,
            }),
        }
    }

    /// Create a Note Expression text event.
    ///
    /// Note: This is not `const` because it initializes the fixed-size buffer.
    pub fn note_expression_text(
        sample_offset: u32,
        note_id: NoteId,
        expression_type: u32,
        text: &str,
    ) -> Self {
        let mut expr = NoteExpressionText {
            note_id,
            expression_type,
            text: [0u8; MAX_EXPRESSION_TEXT_SIZE],
            text_len: 0,
        };
        let bytes = text.as_bytes();
        let copy_len = bytes.len().min(MAX_EXPRESSION_TEXT_SIZE);
        expr.text[..copy_len].copy_from_slice(&bytes[..copy_len]);
        expr.text_len = copy_len as u8;
        Self {
            sample_offset,
            event: MidiEventKind::NoteExpressionText(expr),
        }
    }

    /// Create a Chord info event.
    ///
    /// Note: This is not `const` because it initializes the fixed-size buffer.
    pub fn chord_info(
        sample_offset: u32,
        root: i8,
        bass_note: i8,
        mask: u16,
        name: &str,
    ) -> Self {
        let mut info = ChordInfo {
            root,
            bass_note,
            mask,
            name: [0u8; MAX_CHORD_NAME_SIZE],
            name_len: 0,
        };
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(MAX_CHORD_NAME_SIZE);
        info.name[..copy_len].copy_from_slice(&bytes[..copy_len]);
        info.name_len = copy_len as u8;
        Self {
            sample_offset,
            event: MidiEventKind::ChordInfo(info),
        }
    }

    /// Create a Scale info event.
    ///
    /// Note: This is not `const` because it initializes the fixed-size buffer.
    pub fn scale_info(sample_offset: u32, root: i8, mask: u16, name: &str) -> Self {
        let mut info = ScaleInfo {
            root,
            mask,
            name: [0u8; MAX_SCALE_NAME_SIZE],
            name_len: 0,
        };
        let bytes = name.as_bytes();
        let copy_len = bytes.len().min(MAX_SCALE_NAME_SIZE);
        info.name[..copy_len].copy_from_slice(&bytes[..copy_len]);
        info.name_len = copy_len as u8;
        Self {
            sample_offset,
            event: MidiEventKind::ScaleInfo(info),
        }
    }

    // =========================================================================
    // Event transformation
    // =========================================================================

    /// Create a new event with the same timing but different event data.
    ///
    /// This preserves the `sample_offset` while replacing the `MidiEventKind`.
    /// Useful when transforming MIDI events where you've already matched on
    /// the event type and want to create a modified version.
    ///
    /// # Arguments
    /// * `kind` - The new event data
    ///
    /// # Returns
    /// A new `MidiEvent` with the same `sample_offset` but new event data.
    ///
    /// # Example
    /// ```ignore
    /// MidiEventKind::NoteOn(note_on) => {
    ///     output.push(event.clone().with(MidiEventKind::NoteOn(NoteOn {
    ///         pitch: new_pitch,
    ///         velocity: new_velocity,
    ///         ..*note_on  // Copy channel, note_id, tuning, length
    ///     })));
    /// }
    /// ```
    pub fn with(self, kind: MidiEventKind) -> Self {
        MidiEvent {
            sample_offset: self.sample_offset,
            event: kind,
        }
    }
}

/// Maximum number of MIDI events per buffer.
/// This is a reasonable limit for real-time processing.
pub const MAX_MIDI_EVENTS: usize = 1024;

/// A buffer for collecting MIDI events during processing.
///
/// Uses a fixed-size array to avoid heap allocation during processing.
/// Events should be added in chronological order (by sample_offset).
#[derive(Debug)]
pub struct MidiBuffer {
    events: [MidiEvent; MAX_MIDI_EVENTS],
    len: usize,
    /// Set to true when a push fails due to buffer exhaustion
    overflowed: bool,
}

impl MidiBuffer {
    /// Create a new empty MIDI buffer.
    ///
    /// Uses `std::array::from_fn` with `MidiEvent::default()` since
    /// `MidiEvent` is no longer `Copy` (due to `Box<SysEx>`).
    pub fn new() -> Self {
        Self {
            events: std::array::from_fn(|_| MidiEvent::default()),
            len: 0,
            overflowed: false,
        }
    }

    /// Clear all events from the buffer.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
        self.overflowed = false;
    }

    /// Returns the number of events in the buffer.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if the buffer is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns true if any push failed since the last clear.
    #[inline]
    pub fn has_overflowed(&self) -> bool {
        self.overflowed
    }

    /// Push an event to the buffer.
    ///
    /// Returns `true` if the event was added, `false` if the buffer is full.
    /// Sets the overflow flag when the buffer is exhausted.
    #[inline]
    pub fn push(&mut self, event: MidiEvent) -> bool {
        if self.len < MAX_MIDI_EVENTS {
            self.events[self.len] = event;
            self.len += 1;
            true
        } else {
            self.overflowed = true;
            false
        }
    }

    /// Iterate over events in the buffer.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &MidiEvent> {
        self.events[..self.len].iter()
    }

    /// Get the events as a slice.
    ///
    /// This is useful for passing to functions that expect `&[MidiEvent]`.
    #[inline]
    pub fn as_slice(&self) -> &[MidiEvent] {
        &self.events[..self.len]
    }
}

impl Default for MidiBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Note Expression Controller Types (VST3 SDK 3.5.0)
// =============================================================================

/// Flags for Note Expression type configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoteExpressionTypeFlags(pub i32);

impl NoteExpressionTypeFlags {
    /// No special flags.
    pub const NONE: Self = Self(0);
    /// Event is bipolar (centered around 0), otherwise unipolar (0 to 1).
    pub const IS_BIPOLAR: Self = Self(1 << 0);
    /// Event occurs only once at the start of the note.
    pub const IS_ONE_SHOT: Self = Self(1 << 1);
    /// Expression applies absolute change (not relative/offset).
    pub const IS_ABSOLUTE: Self = Self(1 << 2);
    /// The associated_parameter_id field is valid.
    pub const ASSOCIATED_PARAMETER_ID_VALID: Self = Self(1 << 3);

    /// Check if a flag is set.
    pub const fn contains(&self, flag: Self) -> bool {
        (self.0 & flag.0) != 0
    }

    /// Combine flags.
    pub const fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Value description for a Note Expression type.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NoteExpressionValueDesc {
    /// Minimum value (usually 0.0).
    pub minimum: f64,
    /// Maximum value (usually 1.0).
    pub maximum: f64,
    /// Default/center value.
    pub default_value: f64,
    /// Number of discrete steps (0 = continuous).
    pub step_count: i32,
}

impl NoteExpressionValueDesc {
    /// Create a continuous unipolar value description (0.0 to 1.0).
    pub const fn unipolar() -> Self {
        Self {
            minimum: 0.0,
            maximum: 1.0,
            default_value: 0.0,
            step_count: 0,
        }
    }

    /// Create a continuous bipolar value description (-1.0 to 1.0, center at 0.0).
    pub const fn bipolar() -> Self {
        Self {
            minimum: -1.0,
            maximum: 1.0,
            default_value: 0.0,
            step_count: 0,
        }
    }

    /// Create a tuning value description (in semitones).
    pub const fn tuning(range_semitones: f64) -> Self {
        Self {
            minimum: -range_semitones,
            maximum: range_semitones,
            default_value: 0.0,
            step_count: 0,
        }
    }
}

/// Maximum length for note expression title strings.
pub const MAX_NOTE_EXPRESSION_TITLE_SIZE: usize = 64;

/// Information about a Note Expression type.
///
/// Used to advertise which note expressions the plugin supports.
#[derive(Clone, Copy)]
pub struct NoteExpressionTypeInfo {
    /// Unique identifier for this expression type.
    /// Use constants from [`note_expression`] module or custom IDs.
    pub type_id: u32,
    /// Display title (e.g., "Volume", "Tuning").
    pub title: [u8; MAX_NOTE_EXPRESSION_TITLE_SIZE],
    /// Title length.
    pub title_len: u8,
    /// Short title (e.g., "Vol", "Tun").
    pub short_title: [u8; MAX_NOTE_EXPRESSION_TITLE_SIZE],
    /// Short title length.
    pub short_title_len: u8,
    /// Unit label (e.g., "dB", "semitones").
    pub units: [u8; MAX_NOTE_EXPRESSION_TITLE_SIZE],
    /// Units length.
    pub units_len: u8,
    /// Unit ID for grouping (-1 for none).
    pub unit_id: i32,
    /// Value range description.
    pub value_desc: NoteExpressionValueDesc,
    /// Associated parameter ID for automation mapping (-1 for none).
    pub associated_parameter_id: i32,
    /// Configuration flags.
    pub flags: NoteExpressionTypeFlags,
}

impl NoteExpressionTypeInfo {
    /// Create a new Note Expression type info.
    pub fn new(type_id: u32, title: &str, short_title: &str) -> Self {
        let mut info = Self {
            type_id,
            title: [0u8; MAX_NOTE_EXPRESSION_TITLE_SIZE],
            title_len: 0,
            short_title: [0u8; MAX_NOTE_EXPRESSION_TITLE_SIZE],
            short_title_len: 0,
            units: [0u8; MAX_NOTE_EXPRESSION_TITLE_SIZE],
            units_len: 0,
            unit_id: -1,
            value_desc: NoteExpressionValueDesc::unipolar(),
            associated_parameter_id: -1,
            flags: NoteExpressionTypeFlags::NONE,
        };
        info.set_title(title);
        info.set_short_title(short_title);
        info
    }

    /// Set the title.
    pub fn set_title(&mut self, title: &str) {
        let bytes = title.as_bytes();
        let len = bytes.len().min(MAX_NOTE_EXPRESSION_TITLE_SIZE);
        self.title[..len].copy_from_slice(&bytes[..len]);
        self.title_len = len as u8;
    }

    /// Set the short title.
    pub fn set_short_title(&mut self, short_title: &str) {
        let bytes = short_title.as_bytes();
        let len = bytes.len().min(MAX_NOTE_EXPRESSION_TITLE_SIZE);
        self.short_title[..len].copy_from_slice(&bytes[..len]);
        self.short_title_len = len as u8;
    }

    /// Set the units label.
    pub fn set_units(&mut self, units: &str) {
        let bytes = units.as_bytes();
        let len = bytes.len().min(MAX_NOTE_EXPRESSION_TITLE_SIZE);
        self.units[..len].copy_from_slice(&bytes[..len]);
        self.units_len = len as u8;
    }

    /// Get the title as a string slice.
    pub fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len as usize]).unwrap_or("")
    }

    /// Get the short title as a string slice.
    pub fn short_title_str(&self) -> &str {
        core::str::from_utf8(&self.short_title[..self.short_title_len as usize]).unwrap_or("")
    }

    /// Get the units as a string slice.
    pub fn units_str(&self) -> &str {
        core::str::from_utf8(&self.units[..self.units_len as usize]).unwrap_or("")
    }

    /// Builder: set value description.
    pub fn with_value_desc(mut self, desc: NoteExpressionValueDesc) -> Self {
        self.value_desc = desc;
        self
    }

    /// Builder: set flags.
    pub fn with_flags(mut self, flags: NoteExpressionTypeFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Builder: set units.
    pub fn with_units(mut self, units: &str) -> Self {
        self.set_units(units);
        self
    }

    /// Builder: set associated parameter.
    pub fn with_associated_parameter(mut self, parameter_id: i32) -> Self {
        self.associated_parameter_id = parameter_id;
        self.flags = self.flags.or(NoteExpressionTypeFlags::ASSOCIATED_PARAMETER_ID_VALID);
        self
    }
}

impl core::fmt::Debug for NoteExpressionTypeInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("NoteExpressionTypeInfo")
            .field("type_id", &self.type_id)
            .field("title", &self.title_str())
            .field("short_title", &self.short_title_str())
            .field("units", &self.units_str())
            .field("value_desc", &self.value_desc)
            .field("flags", &self.flags)
            .finish()
    }
}

impl Default for NoteExpressionTypeInfo {
    fn default() -> Self {
        Self::new(note_expression::INVALID, "", "")
    }
}

// =============================================================================
// Keyswitch Controller Types (VST3 SDK 3.5.0)
// =============================================================================

/// Keyswitch type identifiers.
pub mod keyswitch_type {
    /// Keyswitch triggered by a single note on/off.
    pub const NOTE_ON_KEY: u32 = 0;
    /// Keyswitch that must be held (pressed while playing).
    pub const ON_THE_FLY: u32 = 1;
    /// Keyswitch that toggles on/off with repeated presses.
    pub const ON_RELEASE: u32 = 2;
    /// Keyswitch triggered by a range of keys.
    pub const KEY_RANGE: u32 = 3;
}

/// Maximum length for keyswitch title strings.
pub const MAX_KEYSWITCH_TITLE_SIZE: usize = 64;

/// Information about a keyswitch (articulation).
///
/// Used by sample libraries and orchestral instruments to describe
/// available articulation switches.
#[derive(Clone, Copy)]
pub struct KeyswitchInfo {
    /// Keyswitch type (see [`keyswitch_type`] module).
    pub type_id: u32,
    /// Display title (e.g., "Staccato", "Legato").
    pub title: [u8; MAX_KEYSWITCH_TITLE_SIZE],
    /// Title length.
    pub title_len: u8,
    /// Short title (e.g., "Stac", "Leg").
    pub short_title: [u8; MAX_KEYSWITCH_TITLE_SIZE],
    /// Short title length.
    pub short_title_len: u8,
    /// Minimum key in the keyswitch range (MIDI note 0-127).
    pub keyswitch_min: i32,
    /// Maximum key in the keyswitch range (MIDI note 0-127).
    pub keyswitch_max: i32,
    /// Remapped key (-1 if not remapped).
    pub key_remapped: i32,
    /// Unit ID for grouping (-1 for none).
    pub unit_id: i32,
    /// Flags (reserved for future use).
    pub flags: i32,
}

impl KeyswitchInfo {
    /// Create a new keyswitch info for a single key.
    pub fn new(type_id: u32, title: &str, key: i32) -> Self {
        let mut info = Self {
            type_id,
            title: [0u8; MAX_KEYSWITCH_TITLE_SIZE],
            title_len: 0,
            short_title: [0u8; MAX_KEYSWITCH_TITLE_SIZE],
            short_title_len: 0,
            keyswitch_min: key,
            keyswitch_max: key,
            key_remapped: -1,
            unit_id: -1,
            flags: 0,
        };
        info.set_title(title);
        info
    }

    /// Create a keyswitch for a range of keys.
    pub fn key_range(type_id: u32, title: &str, min_key: i32, max_key: i32) -> Self {
        let mut info = Self::new(type_id, title, min_key);
        info.keyswitch_max = max_key;
        info
    }

    /// Set the title.
    pub fn set_title(&mut self, title: &str) {
        let bytes = title.as_bytes();
        let len = bytes.len().min(MAX_KEYSWITCH_TITLE_SIZE);
        self.title[..len].copy_from_slice(&bytes[..len]);
        self.title_len = len as u8;
    }

    /// Set the short title.
    pub fn set_short_title(&mut self, short_title: &str) {
        let bytes = short_title.as_bytes();
        let len = bytes.len().min(MAX_KEYSWITCH_TITLE_SIZE);
        self.short_title[..len].copy_from_slice(&bytes[..len]);
        self.short_title_len = len as u8;
    }

    /// Get the title as a string slice.
    pub fn title_str(&self) -> &str {
        core::str::from_utf8(&self.title[..self.title_len as usize]).unwrap_or("")
    }

    /// Get the short title as a string slice.
    pub fn short_title_str(&self) -> &str {
        core::str::from_utf8(&self.short_title[..self.short_title_len as usize]).unwrap_or("")
    }

    /// Builder: set short title.
    pub fn with_short_title(mut self, short_title: &str) -> Self {
        self.set_short_title(short_title);
        self
    }

    /// Builder: set remapped key.
    pub fn with_key_remapped(mut self, key: i32) -> Self {
        self.key_remapped = key;
        self
    }
}

impl core::fmt::Debug for KeyswitchInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("KeyswitchInfo")
            .field("type_id", &self.type_id)
            .field("title", &self.title_str())
            .field("keyswitch_min", &self.keyswitch_min)
            .field("keyswitch_max", &self.keyswitch_max)
            .finish()
    }
}

impl Default for KeyswitchInfo {
    fn default() -> Self {
        Self::new(keyswitch_type::NOTE_ON_KEY, "", 0)
    }
}

// =============================================================================
// Physical UI Mapping Types (VST3 SDK 3.6.11)
// =============================================================================

/// Physical UI type identifiers for MPE and physical controllers.
pub mod physical_ui {
    /// X-axis movement (horizontal slide on MPE controllers).
    pub const X_MOVEMENT: u32 = 0;
    /// Y-axis movement (vertical slide / "Slide" on MPE controllers).
    pub const Y_MOVEMENT: u32 = 1;
    /// Pressure (aftertouch on MPE controllers).
    pub const PRESSURE: u32 = 2;
    /// Type face (for instruments with multiple playing styles).
    pub const TYPE_FACE: u32 = 3;
    /// Reserved value for unassigned/unknown.
    pub const INVALID: u32 = u32::MAX;
}

/// Maps a physical UI input to a Note Expression output.
///
/// Used to define how MPE controllers map to note expression types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhysicalUIMap {
    /// Physical UI type (see [`physical_ui`] module).
    pub physical_ui_type_id: u32,
    /// Note expression type to map to (see [`note_expression`] module).
    pub note_expression_type_id: u32,
}

impl PhysicalUIMap {
    /// Create a new physical UI mapping.
    pub const fn new(physical_ui_type_id: u32, note_expression_type_id: u32) -> Self {
        Self {
            physical_ui_type_id,
            note_expression_type_id,
        }
    }

    /// Map X-axis to a note expression.
    pub const fn x_axis(note_expression_type_id: u32) -> Self {
        Self::new(physical_ui::X_MOVEMENT, note_expression_type_id)
    }

    /// Map Y-axis (Slide) to a note expression.
    pub const fn y_axis(note_expression_type_id: u32) -> Self {
        Self::new(physical_ui::Y_MOVEMENT, note_expression_type_id)
    }

    /// Map Pressure to a note expression.
    pub const fn pressure(note_expression_type_id: u32) -> Self {
        Self::new(physical_ui::PRESSURE, note_expression_type_id)
    }
}

// =============================================================================
// MPE Support Types (VST3 SDK 3.6.12)
// =============================================================================

/// MPE (MIDI Polyphonic Expression) input device settings.
///
/// Defines the MPE zone configuration for incoming MIDI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MpeInputDeviceSettings {
    /// Master channel (0 = channel 1, typically 0 for lower zone).
    pub master_channel: i32,
    /// First member channel (typically 1 for lower zone).
    pub member_begin_channel: i32,
    /// Last member channel (typically 14 for lower zone).
    pub member_end_channel: i32,
}

impl Default for MpeInputDeviceSettings {
    fn default() -> Self {
        Self {
            master_channel: 0,
            member_begin_channel: 1,
            member_end_channel: 14,
        }
    }
}

impl MpeInputDeviceSettings {
    /// Create MPE settings for the lower zone (default configuration).
    pub const fn lower_zone() -> Self {
        Self {
            master_channel: 0,
            member_begin_channel: 1,
            member_end_channel: 14,
        }
    }

    /// Create MPE settings for the upper zone.
    pub const fn upper_zone() -> Self {
        Self {
            master_channel: 15,
            member_begin_channel: 14,
            member_end_channel: 1,
        }
    }

    /// Create custom MPE settings.
    pub const fn new(master: i32, begin: i32, end: i32) -> Self {
        Self {
            master_channel: master,
            member_begin_channel: begin,
            member_end_channel: end,
        }
    }
}
