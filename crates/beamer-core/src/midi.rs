//! MIDI event types for audio plugins.
//!
//! This module provides format-agnostic MIDI event types designed for
//! real-time audio processing. The event types are `Copy` and pass
//! without heap allocation.
//!
//! ## SysEx Handling
//!
//! A [`MidiEventKind::SysEx`] event borrows its payload from the
//! [`MidiBuffer`] it is in: each buffer keeps the SysEx bytes of a block in
//! a pool that is allocated once (see `Config::sysex_bytes_per_block`). So
//! [`MidiEvent`] is `Copy` and passing events on does not allocate.
//! [`MidiBuffer::push`] copies the bytes of a SysEx event into the buffer's
//! pool.

/// Maximum text size for Note Expression text events.
pub const MAX_EXPRESSION_TEXT_SIZE: usize = 64;

/// Maximum chord name size in bytes.
pub const MAX_CHORD_NAME_SIZE: usize = 32;

/// Maximum scale name size in bytes.
pub const MAX_SCALE_NAME_SIZE: usize = 32;

/// The longest start of `text` that is at most `max` bytes and ends after a
/// whole character.
fn utf8_prefix(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// =============================================================================
// Basic MIDI Types
// =============================================================================

/// MIDI channel (0-15).
pub type MidiChannel = u8;

/// MIDI note number (0-127, where 60 = middle C).
pub type MidiNote = u8;

/// Unique identifier for tracking note on/off pairs.
/// Use -1 when note ID is not available.
///
/// MIDI input has no note IDs of its own: the wrappers give a note the ID
/// [`note_id`] makes from its group, channel and pitch.
pub type NoteId = i32;

/// The note ID of a MIDI note: `group << 11 | channel << 7 | pitch`.
///
/// One pitch playing on two channels or groups gets two IDs, so a per-note
/// message finds its note. Every value is masked to its bits.
#[inline]
pub const fn note_id(group: u8, channel: MidiChannel, pitch: MidiNote) -> NoteId {
    (((group & 0x0F) as NoteId) << 11) | (((channel & 0x0F) as NoteId) << 7) | (pitch & 0x7F) as NoteId
}

/// The group, channel and pitch of a note ID that [`note_id`] made, or
/// `None` for an ID it cannot have made.
#[inline]
pub const fn note_id_parts(id: NoteId) -> Option<(u8, MidiChannel, MidiNote)> {
    if id < 0 || id >= 1 << 15 {
        return None;
    }
    Some((((id >> 11) & 0x0F) as u8, ((id >> 7) & 0x0F) as u8, (id & 0x7F) as u8))
}

/// A MIDI note-on event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteOn {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Note number (0-127).
    pub pitch: MidiNote,
    /// Velocity (0.0 to 1.0), above 0. A MIDI 1.0 note-on with velocity 0
    /// reaches the plugin as a [`NoteOff`]; a MIDI 2.0 note-on with velocity
    /// 0 (a note-on in MIDI 2.0) has the smallest velocity, 1 / 65535.
    pub velocity: f32,
    /// Unique note ID for tracking this note instance.
    pub note_id: NoteId,
    /// Pitch offset in cents for microtonal and per-note pitch: VST3's
    /// `tuning`, or a MIDI 2.0 note's pitch 7.9 attribute minus its pitch.
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
    /// Release velocity (0.0 to 1.0). A MIDI 1.0 note-on with velocity 0
    /// has the default release velocity, 64 of 127 (0x8000 of 0xFFFF in
    /// MIDI 2.0, just above 0.5).
    pub velocity: f32,
    /// Unique note ID matching the original note-on.
    pub note_id: NoteId,
    /// Pitch offset in cents, as [`NoteOn::tuning`].
    pub tuning: f32,
}

/// Polyphonic key pressure (aftertouch per note).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PolyPressure {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Note number (0-127).
    pub pitch: MidiNote,
    /// Pressure amount (0.0 to 1.0), 32 bits from MIDI 2.0.
    pub pressure: f64,
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
    /// Controller value (0.0 to 1.0), 32 bits from MIDI 2.0. A MIDI 1.0
    /// value is scaled up as MIDI 2.0 does (see [`scale`]), so 0 is 0.0,
    /// 64 is the center and 127 is 1.0 in either protocol. Use
    /// [`scale::to_7bit`] for the 7-bit value.
    pub value: f64,
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
    /// Pitch bend value (-1.0 to 1.0, where 0.0 is center), 32 bits from
    /// MIDI 2.0. Both ends are reached exactly (see [`scale::bipolar_from_u32`]).
    pub value: f64,
}

/// Channel pressure (channel aftertouch).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelPressure {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Pressure amount (0.0 to 1.0), 32 bits from MIDI 2.0.
    pub pressure: f64,
}

/// Program change message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProgramChange {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// Program number (0-127).
    pub program: u8,
    /// The bank, MSB << 7 | LSB, if one was selected: by a MIDI 2.0
    /// program change that carries one, or by bank select (CC 0 and CC 32)
    /// on the channel before. Bank select stays in effect until the next
    /// one, as in MIDI 1.0; the CCs still reach the plugin too.
    pub bank: Option<u16>,
}

// =============================================================================
// Advanced VST3 Events
// =============================================================================

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
    /// Normalized value, 0.0 to 1.0, as VST3 defines it per type; a
    /// bipolar type (pan, tuning, [`note_expression::PITCH_BEND`]) has its
    /// center at 0.5. Tuning is `240 * (value - 0.5)` semitones (see
    /// [`tuning_semitones`](Self::tuning_semitones)), volume
    /// `20 * log10(4 * value)` dB.
    pub value: f64,
}

impl NoteExpressionValue {
    /// The tuning in semitones of a [`note_expression::TUNING`] value.
    #[inline]
    pub fn tuning_semitones(&self) -> f64 {
        tuning_semitones(self.value)
    }
}

/// The semitones of a normalized tuning value: `240 * (value - 0.5)`.
#[inline]
pub fn tuning_semitones(value: f64) -> f64 {
    240.0 * (value - 0.5)
}

/// The normalized tuning value of semitones (-120 to 120).
#[inline]
pub fn tuning_value(semitones: f64) -> f64 {
    (semitones / 240.0 + 0.5).clamp(0.0, 1.0)
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
    /// Get the text as a string slice: empty if `text_len` is out of range
    /// or the text is not UTF-8.
    #[inline]
    pub fn as_str(&self) -> &str {
        self.text
            .get(..self.text_len as usize)
            .and_then(|text| core::str::from_utf8(text).ok())
            .unwrap_or("")
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
    /// Get the chord name as a string slice: empty if `name_len` is out of
    /// range or the name is not UTF-8.
    #[inline]
    pub fn name_str(&self) -> &str {
        self.name
            .get(..self.name_len as usize)
            .and_then(|name| core::str::from_utf8(name).ok())
            .unwrap_or("")
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
    /// Get the scale name as a string slice: empty if `name_len` is out of
    /// range or the name is not UTF-8.
    #[inline]
    pub fn name_str(&self) -> &str {
        self.name
            .get(..self.name_len as usize)
            .and_then(|name| core::str::from_utf8(name).ok())
            .unwrap_or("")
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
/// These are standard MIDI parameters with defined meanings across all
/// devices: the [`ParameterNumber::parameter`] of an RPN (MSB << 7 | LSB).
/// In MIDI 1.0 CC 101 (MSB) and CC 100 (LSB) select the parameter, then
/// CC 6 (Data Entry MSB) and optionally CC 38 (LSB) set the value; a plugin
/// gets one [`ParameterNumber`] event for them.
///
/// # Example
///
/// To set Pitch Bend Sensitivity to 12 semitones:
/// ```text
/// CC 101 = 0   (RPN MSB = 0)
/// CC 100 = 0   (RPN LSB = 0): selects Pitch Bend Sensitivity
/// CC 6   = 12  (Data Entry = 12 semitones)
/// CC 101 = 127 (RPN Null)
/// CC 100 = 127 (RPN Null): deselects, to prevent accidental changes
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
    pub const NULL: u16 = 0x3FFF;

    /// Check if a parameter number represents RPN Null.
    #[inline]
    pub const fn is_null(parameter: u16) -> bool {
        parameter == NULL
    }
}

// =============================================================================
// RPN/NRPN Messages
// =============================================================================

/// Type of parameter number (RPN vs NRPN).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterNumberKind {
    /// Registered Parameter Number (MIDI 2.0: registered controller).
    Rpn,
    /// Non-Registered Parameter Number (MIDI 2.0: assignable controller).
    Nrpn,
}

/// A change of an RPN or NRPN.
///
/// MIDI 1.0 sets one with a sequence of control changes (CC 101 and 100 or
/// 99 and 98 select it, CC 6 and 38 set it, CC 96 and 97 step it); MIDI 2.0
/// with one message. The wrappers assemble the sequences, so a plugin gets
/// this event in either protocol and never the selection CCs (see
/// [`Midi1Translator`](crate::Midi1Translator)).
///
/// # Example
///
/// ```
/// use beamer_core::{MidiEventKind, ParameterNumber};
///
/// fn pitch_bend_range(event: &MidiEventKind) -> Option<f32> {
///     match event {
///         MidiEventKind::ParameterNumber(change) => change
///             .pitch_bend_sensitivity()
///             .map(|(semitones, cents)| semitones as f32 + cents as f32 / 100.0),
///         _ => None,
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParameterNumber {
    /// MIDI channel (0-15).
    pub channel: MidiChannel,
    /// RPN or NRPN.
    pub kind: ParameterNumberKind,
    /// The parameter, MSB << 7 | LSB (MIDI 2.0: bank << 7 | index). See
    /// [`rpn`] for the registered ones.
    pub parameter: u16,
    /// The new value or a step.
    pub change: ParameterChange,
}

/// How a [`ParameterNumber`] changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParameterChange {
    /// The value, 0.0 to 1.0, 32 bits from MIDI 2.0. MIDI 1.0 data entry
    /// (CC 6 and 38) gives 14 bits, scaled up as MIDI 2.0 does.
    Absolute(f64),
    /// A step, -1.0 to 1.0 of the full range: a MIDI 2.0 relative change
    /// over `u32::MAX`, or [`RELATIVE_14BIT_STEP`] for each data increment
    /// (CC 96) or decrement (CC 97) of MIDI 1.0.
    Relative(f64),
}

/// The step of one MIDI 1.0 data increment: one 14-bit step on the 32-bit
/// scale of MIDI 2.0.
pub const RELATIVE_14BIT_STEP: f64 = (1u32 << 18) as f64 / u32::MAX as f64;

impl ParameterNumber {
    /// An absolute change of an RPN.
    pub const fn rpn(channel: MidiChannel, parameter: u16, value: f64) -> Self {
        Self {
            channel,
            kind: ParameterNumberKind::Rpn,
            parameter,
            change: ParameterChange::Absolute(value),
        }
    }

    /// An absolute change of an NRPN.
    pub const fn nrpn(channel: MidiChannel, parameter: u16, value: f64) -> Self {
        Self {
            channel,
            kind: ParameterNumberKind::Nrpn,
            parameter,
            change: ParameterChange::Absolute(value),
        }
    }

    /// Whether this is an RPN.
    #[inline]
    pub const fn is_rpn(&self) -> bool {
        matches!(self.kind, ParameterNumberKind::Rpn)
    }

    /// Whether this is an NRPN.
    #[inline]
    pub const fn is_nrpn(&self) -> bool {
        matches!(self.kind, ParameterNumberKind::Nrpn)
    }

    /// The value of an absolute change as 14 bits (MSB << 7 | LSB), as MIDI
    /// 1.0 data entry sends it.
    #[inline]
    pub fn value_14bit(&self) -> Option<u16> {
        match self.change {
            ParameterChange::Absolute(value) => Some(scale::to_14bit(value)),
            ParameterChange::Relative(_) => None,
        }
    }

    /// The semitones and cents of an absolute change of the pitch bend
    /// sensitivity (RPN 0).
    #[inline]
    pub fn pitch_bend_sensitivity(&self) -> Option<(u8, u8)> {
        if !self.is_rpn() || self.parameter != rpn::PITCH_BEND_SENSITIVITY {
            return None;
        }
        self.value_14bit().map(split_14bit_raw)
    }

    /// The MIDI 1.0 control changes that send this change, as (controller,
    /// value) pairs, with how many of the four are used: the selection (CC
    /// 101 and 100, or 99 and 98), then data entry (CC 6 and 38) for an
    /// absolute change or one increment (CC 96) or decrement (CC 97) for a
    /// relative one, whatever its size.
    pub fn midi1_control_changes(&self) -> ([(u8, u8); 4], usize) {
        let (msb, lsb) = split_14bit_raw(self.parameter & 0x3FFF);
        let (select_msb, select_lsb) = match self.kind {
            ParameterNumberKind::Rpn => (cc::RPN_MSB, cc::RPN_LSB),
            ParameterNumberKind::Nrpn => (cc::NRPN_MSB, cc::NRPN_LSB),
        };
        let mut changes = [(select_msb, msb), (select_lsb, lsb), (0, 0), (0, 0)];
        match self.change {
            ParameterChange::Absolute(value) => {
                let (data_msb, data_lsb) = split_14bit_raw(scale::to_14bit(value));
                changes[2] = (cc::DATA_ENTRY_MSB, data_msb);
                changes[3] = (cc::DATA_ENTRY_LSB, data_lsb);
                (changes, 4)
            }
            ParameterChange::Relative(step) => {
                let controller = if step < 0.0 { cc::DATA_DECREMENT } else { cc::DATA_INCREMENT };
                changes[2] = (controller, 0);
                (changes, 3)
            }
        }
    }
}

// =============================================================================
// Resolution Scaling
// =============================================================================

/// Scaling between MIDI 1.0 and MIDI 2.0 resolutions and Beamer's
/// normalized values.
///
/// A MIDI 1.0 value is scaled up as the MIDI 2.0 specification does it
/// (min-center-max: shifted up to the center, the bits repeated above it),
/// which is what a host does when it translates MIDI 1.0 to 2.0. So a
/// plugin gets the same value whether a host sent MIDI 1.0 or translated
/// it: 0 is 0.0, 127 is 1.0 and 64 is the center. Scaling down rounds to
/// the nearest step of the wider value and shifts (as MIDI 2.0 does), so
/// every value that came in leaves as the same byte; a plugin's `n / 127.0`
/// gives byte `n`. Any other value leaves as its top 7 bits, about
/// `floor(value * 128)`.
pub mod scale {
    /// Scale a 7-bit value up to 16 bits.
    #[inline]
    pub const fn upscale_7_to_16(value: u8) -> u16 {
        let value = (value & 0x7F) as u16;
        let shifted = value << 9;
        if value <= 0x40 {
            return shifted;
        }
        let repeat = value & 0x3F;
        shifted | (repeat << 3) | (repeat >> 3)
    }

    /// Scale a 7-bit value up to 32 bits.
    #[inline]
    pub const fn upscale_7_to_32(value: u8) -> u32 {
        let value = (value & 0x7F) as u32;
        let shifted = value << 25;
        if value <= 0x40 {
            return shifted;
        }
        let repeat = value & 0x3F;
        shifted | (repeat << 19) | (repeat << 13) | (repeat << 7) | (repeat << 1) | (repeat >> 5)
    }

    /// Scale a 14-bit value up to 32 bits.
    #[inline]
    pub const fn upscale_14_to_32(value: u16) -> u32 {
        let value = (value & 0x3FFF) as u32;
        let shifted = value << 18;
        if value <= 0x2000 {
            return shifted;
        }
        let repeat = value & 0x1FFF;
        shifted | (repeat << 5) | (repeat >> 8)
    }

    /// A 32-bit value as 0.0 to 1.0.
    #[inline]
    pub fn unipolar_from_u32(value: u32) -> f64 {
        value as f64 / u32::MAX as f64
    }

    /// 0.0 to 1.0 as a 32-bit value, rounded.
    #[inline]
    pub fn unipolar_to_u32(value: f64) -> u32 {
        (value.clamp(0.0, 1.0) * u32::MAX as f64).round() as u32
    }

    /// The center of a 32-bit bipolar value.
    const CENTER: u32 = 0x8000_0000;

    /// A 32-bit bipolar value (center 0x80000000) as -1.0 to 1.0, with
    /// each end reached exactly.
    #[inline]
    pub fn bipolar_from_u32(value: u32) -> f64 {
        if value >= CENTER {
            (value - CENTER) as f64 / (u32::MAX - CENTER) as f64
        } else {
            -((CENTER - value) as f64 / CENTER as f64)
        }
    }

    /// -1.0 to 1.0 as a 32-bit bipolar value, rounded.
    #[inline]
    pub fn bipolar_to_u32(value: f64) -> u32 {
        let value = value.clamp(-1.0, 1.0);
        if value >= 0.0 {
            CENTER + (value * (u32::MAX - CENTER) as f64).round() as u32
        } else {
            CENTER - (-value * CENTER as f64).round() as u32
        }
    }

    /// A 7-bit value as 0.0 to 1.0.
    #[inline]
    pub fn from_7bit(value: u8) -> f64 {
        unipolar_from_u32(upscale_7_to_32(value))
    }

    /// 0.0 to 1.0 as a 7-bit value.
    #[inline]
    pub fn to_7bit(value: f64) -> u8 {
        (unipolar_to_u32(value) >> 25) as u8
    }

    /// A 14-bit value as 0.0 to 1.0.
    #[inline]
    pub fn from_14bit(value: u16) -> f64 {
        unipolar_from_u32(upscale_14_to_32(value))
    }

    /// 0.0 to 1.0 as a 14-bit value.
    #[inline]
    pub fn to_14bit(value: f64) -> u16 {
        (unipolar_to_u32(value) >> 18) as u16
    }

    /// A 14-bit bipolar value (pitch bend, center 8192) as -1.0 to 1.0.
    #[inline]
    pub fn bipolar_from_14bit(value: u16) -> f64 {
        bipolar_from_u32(upscale_14_to_32(value))
    }

    /// -1.0 to 1.0 as a 14-bit bipolar value (pitch bend).
    #[inline]
    pub fn bipolar_to_14bit(value: f64) -> u16 {
        (bipolar_to_u32(value) >> 18) as u16
    }

    /// A 16-bit velocity as 0.0 to 1.0.
    #[inline]
    pub fn velocity_from_u16(value: u16) -> f32 {
        (value as f64 / u16::MAX as f64) as f32
    }

    /// A velocity (0.0 to 1.0) as 16 bits, rounded.
    #[inline]
    pub fn velocity_to_u16(value: f32) -> u16 {
        (value.clamp(0.0, 1.0) as f64 * u16::MAX as f64).round() as u16
    }

    /// A 7-bit velocity as 0.0 to 1.0.
    #[inline]
    pub fn velocity_from_7bit(value: u8) -> f32 {
        velocity_from_u16(upscale_7_to_16(value))
    }

    /// A velocity (0.0 to 1.0) as 7 bits.
    #[inline]
    pub fn velocity_to_7bit(value: f32) -> u8 {
        (velocity_to_u16(value) >> 9) as u8
    }
}

// =============================================================================
// 14-bit Controller Utilities
// =============================================================================

/// Combines MSB and LSB controller values into a single 14-bit normalized value.
///
/// MIDI CC 0-31 are MSB controllers and CC 32-63 are their corresponding LSB pairs.
/// Together they provide 14-bit resolution (0-16383) instead of 7-bit (0-127).
/// The values are scaled as in [`scale`].
///
/// # Example
/// ```
/// use beamer_core::midi::{combine_14bit_cc, scale};
///
/// // Full resolution: MSB=127, LSB=127 gives 16383, 1.0.
/// assert_eq!(combine_14bit_cc(1.0, 1.0), 1.0);
///
/// // MSB=64, LSB=0 gives 8192, the center.
/// assert_eq!(combine_14bit_cc(scale::from_7bit(64), 0.0), scale::from_14bit(8192));
/// ```
#[inline]
pub fn combine_14bit_cc(msb_value: f64, lsb_value: f64) -> f64 {
    scale::from_14bit(combine_14bit_raw(scale::to_7bit(msb_value), scale::to_7bit(lsb_value)))
}

/// Splits a 14-bit normalized value into MSB and LSB controller values.
///
/// This is the inverse of [`combine_14bit_cc`].
///
/// # Example
/// ```
/// use beamer_core::midi::{split_14bit_cc, combine_14bit_cc};
///
/// // Round trip: split then combine gives the 14-bit step of the value.
/// let (msb, lsb) = split_14bit_cc(0.75);
/// assert!((combine_14bit_cc(msb, lsb) - 0.75).abs() < 1.0 / 16383.0);
///
/// // The full value splits to (1.0, 1.0).
/// assert_eq!(split_14bit_cc(1.0), (1.0, 1.0));
/// ```
#[inline]
pub fn split_14bit_cc(value: f64) -> (f64, f64) {
    let (msb, lsb) = split_14bit_raw(scale::to_14bit(value));
    (scale::from_7bit(msb), scale::from_7bit(lsb))
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

/// Note Expression type IDs.
///
/// These constants identify the type of per-note expression in
/// [`NoteExpressionValue`], [`NoteExpressionInt`] and [`NoteExpressionText`]
/// events. The types up to [`PHONEME`](note_expression::PHONEME) and the custom range are VST3's,
/// with VST3's normalized values. MIDI 2.0 per-note messages (Audio Unit)
/// arrive as the VST3 type of the same meaning where there is one, else as
/// the types from [`PITCH_BEND`](note_expression::PITCH_BEND) on, which lie past VST3's custom range.
pub mod note_expression {
    /// Per-note volume: `20 * log10(4 * value)` dB (0.25 = 0 dB, 1.0 =
    /// +12 dB). MIDI 2.0 registered per-note controller 7 (volume, `v`)
    /// arrives as `v² / 4`, the curve of CC 7.
    pub const VOLUME: u32 = 0;
    /// Per-note pan (0.0 = left, 0.5 = center, 1.0 = right). MIDI 2.0
    /// registered per-note controller 10, its center 0x80000000 at exactly
    /// 0.5.
    pub const PAN: u32 = 1;
    /// Per-note tuning: `240 * (value - 0.5)` semitones (0.5 = none), see
    /// [`tuning_semitones`](super::tuning_semitones). MIDI 2.0 registered
    /// per-note controller 3 (pitch 7.25) arrives as its pitch minus the
    /// note's.
    pub const TUNING: u32 = 2;
    /// Per-note vibrato depth (0.0 to 1.0). MIDI 2.0 registered per-note
    /// controller 77 (vibrato depth).
    pub const VIBRATO: u32 = 3;
    /// Per-note expression (general purpose, 0.0 to 1.0). MIDI 2.0
    /// registered per-note controller 11.
    pub const EXPRESSION: u32 = 4;
    /// Per-note brightness/timbre (0.0 to 1.0). MIDI 2.0 registered per-note
    /// controller 74.
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

    /// First of the MIDI 2.0 registered per-note controllers without a VST3
    /// type: [`registered`] gives index 0 to 255.
    pub const REGISTERED_START: u32 = 300_000;
    /// First of the MIDI 2.0 assignable per-note controllers: [`assignable`]
    /// gives index 0 to 255.
    pub const ASSIGNABLE_START: u32 = 300_256;
    /// MIDI 2.0 per-note pitch bend (0.0 to 1.0, 0.5 = none). The plugin
    /// applies its range, as for channel pitch bend; MPE's default is 48
    /// semitones each way.
    pub const PITCH_BEND: u32 = 300_512;

    /// A MIDI 2.0 registered per-note controller that has no VST3 type, by
    /// its index (0.0 to 1.0).
    #[inline]
    pub const fn registered(index: u8) -> u32 {
        REGISTERED_START + index as u32
    }

    /// A MIDI 2.0 assignable per-note controller, by its index (0.0 to 1.0).
    #[inline]
    pub const fn assignable(index: u8) -> u32 {
        ASSIGNABLE_START + index as u32
    }
}

// =============================================================================
// MIDI Event Enum
// =============================================================================

/// MIDI event types.
///
/// The lifetime is that of the bytes a SysEx event borrows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidiEventKind<'a> {
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
    /// An RPN or NRPN change, from MIDI 1.0 data entry or a MIDI 2.0
    /// registered or assignable controller.
    ParameterNumber(ParameterNumber),

    // =========================================================================
    // Advanced VST3 events
    // =========================================================================

    /// System Exclusive (SysEx) message: its payload, without the F0 and F7
    /// that frame it on the wire. The wrappers take them off incoming
    /// messages and add them to outgoing ones.
    SysEx(&'a [u8]),
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
/// The lifetime is that of the bytes a SysEx event borrows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiEvent<'a> {
    /// Sample offset within the current buffer (0 = start of buffer).
    pub sample_offset: u32,
    /// The MIDI event data.
    pub event: MidiEventKind<'a>,
}

impl Default for MidiEvent<'_> {
    /// Creates a default MidiEvent (NoteOff with all fields zeroed).
    ///
    /// Used for buffer initialization.
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

impl<'a> MidiEvent<'a> {
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
        pressure: f64,
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
        value: f64,
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
    pub const fn pitch_bend(sample_offset: u32, channel: MidiChannel, value: f64) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::PitchBend(PitchBend { channel, value }),
        }
    }

    /// Create a channel pressure event.
    pub const fn channel_pressure(
        sample_offset: u32,
        channel: MidiChannel,
        pressure: f64,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::ChannelPressure(ChannelPressure { channel, pressure }),
        }
    }

    /// Create a program change event.
    pub const fn program_change(
        sample_offset: u32,
        channel: MidiChannel,
        program: u8,
        bank: Option<u16>,
    ) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::ProgramChange(ProgramChange { channel, program, bank }),
        }
    }

    /// Create an RPN or NRPN change event.
    pub const fn parameter_number(sample_offset: u32, change: ParameterNumber) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::ParameterNumber(change),
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
    /// * `group` - UMP group (0-15), 0 outside Universal MIDI Packets
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
    /// - Note On with velocity 0 is a Note Off with the default release
    ///   velocity, 0.5 (per MIDI spec)
    /// - Values are scaled up as MIDI 2.0 does (see [`scale`]): 0 is 0.0,
    ///   64 the center, 127 is 1.0; pitch bend is -1.0 to 1.0
    /// - `note_id` is [`note_id`] of the group, channel and pitch
    /// - `tuning` and `length` default to 0; a program change has no bank
    ///
    /// # Example
    ///
    /// ```
    /// use beamer_core::MidiEvent;
    ///
    /// // Parse a Note On: channel 0, note 60 (middle C), velocity 100
    /// let event = MidiEvent::from_midi1_bytes(0, 0, 0x90, 0, 60, 100);
    /// assert!(event.is_some());
    /// ```
    #[inline]
    pub fn from_midi1_bytes(
        sample_offset: u32,
        group: u8,
        status: u8,
        channel: MidiChannel,
        data1: u8,
        data2: u8,
    ) -> Option<Self> {
        let channel = channel & 0x0F;
        let data1 = data1 & 0x7F;
        let data2 = data2 & 0x7F;
        let id = note_id(group, channel, data1);
        match status {
            0x80 => Some(Self::note_off(
                sample_offset,
                channel,
                data1,
                scale::velocity_from_7bit(data2),
                id,
                0.0,
            )),
            // Note On with velocity 0 is a Note Off with the default
            // release velocity (per MIDI spec).
            0x90 if data2 == 0 => Some(Self::note_off(
                sample_offset,
                channel,
                data1,
                scale::velocity_from_7bit(64),
                id,
                0.0,
            )),
            0x90 => Some(Self::note_on(
                sample_offset,
                channel,
                data1,
                scale::velocity_from_7bit(data2),
                id,
                0.0,
                0,
            )),
            0xA0 => Some(Self::poly_pressure(sample_offset, channel, data1, scale::from_7bit(data2), id)),
            0xB0 => Some(Self::control_change(sample_offset, channel, data1, scale::from_7bit(data2))),
            0xC0 => Some(Self::program_change(sample_offset, channel, data1, None)),
            0xD0 => Some(Self::channel_pressure(sample_offset, channel, scale::from_7bit(data1))),
            // Pitch bend: data1 is the LSB, data2 the MSB.
            0xE0 => Some(Self::pitch_bend(
                sample_offset,
                channel,
                scale::bipolar_from_14bit(combine_14bit_raw(data2, data1)),
            )),
            // System messages are not supported.
            _ => None,
        }
    }

    // =========================================================================
    // Advanced VST3 event constructors
    // =========================================================================

    /// Create a SysEx event from its payload, without the F0 and F7 that frame
    /// it on the wire. The event borrows the payload; pushing it into a
    /// [`MidiBuffer`] copies it.
    pub const fn sysex(sample_offset: u32, payload: &'a [u8]) -> Self {
        Self {
            sample_offset,
            event: MidiEventKind::SysEx(payload),
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

    /// Create a Note Expression text event. A text longer than
    /// [`MAX_EXPRESSION_TEXT_SIZE`] bytes is cut after the last whole
    /// character that fits.
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
        let bytes = utf8_prefix(text, MAX_EXPRESSION_TEXT_SIZE).as_bytes();
        expr.text[..bytes.len()].copy_from_slice(bytes);
        expr.text_len = bytes.len() as u8;
        Self {
            sample_offset,
            event: MidiEventKind::NoteExpressionText(expr),
        }
    }

    /// Create a Chord info event. A name longer than [`MAX_CHORD_NAME_SIZE`]
    /// bytes is cut after the last whole character that fits.
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
        let bytes = utf8_prefix(name, MAX_CHORD_NAME_SIZE).as_bytes();
        info.name[..bytes.len()].copy_from_slice(bytes);
        info.name_len = bytes.len() as u8;
        Self {
            sample_offset,
            event: MidiEventKind::ChordInfo(info),
        }
    }

    /// Create a Scale info event. A name longer than [`MAX_SCALE_NAME_SIZE`]
    /// bytes is cut after the last whole character that fits.
    ///
    /// Note: This is not `const` because it initializes the fixed-size buffer.
    pub fn scale_info(sample_offset: u32, root: i8, mask: u16, name: &str) -> Self {
        let mut info = ScaleInfo {
            root,
            mask,
            name: [0u8; MAX_SCALE_NAME_SIZE],
            name_len: 0,
        };
        let bytes = utf8_prefix(name, MAX_SCALE_NAME_SIZE).as_bytes();
        info.name[..bytes.len()].copy_from_slice(bytes);
        info.name_len = bytes.len() as u8;
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
    ///     output.push(event.with(MidiEventKind::NoteOn(NoteOn {
    ///         pitch: new_pitch,
    ///         velocity: new_velocity,
    ///         ..*note_on  // Copy channel, note_id, tuning, length
    ///     })));
    /// }
    /// ```
    pub fn with<'b>(self, kind: MidiEventKind<'b>) -> MidiEvent<'b> {
        MidiEvent {
            sample_offset: self.sample_offset,
            event: kind,
        }
    }

    /// The event without a borrow, to keep it past the call it came with.
    /// `None` for a SysEx event, whose payload is borrowed: copy the payload
    /// to keep it.
    pub fn to_static(&self) -> Option<MidiEvent<'static>> {
        kind_or_payload(self.event).ok().map(|kind| self.with(kind))
    }
}

/// Maximum number of MIDI events per buffer.
/// This is a reasonable limit for real-time processing.
pub const MAX_MIDI_EVENTS: usize = 1024;

/// An event in a [`MidiBuffer`]. The payload of a SysEx event points into
/// the pool of that buffer: the `'static` is only a placeholder, the
/// buffer narrows it to its own borrow wherever events leave it. The type
/// is private and nothing outside this module gets one by value or by
/// `&mut`, so an event cannot move into another buffer (see
/// [`MidiEventsMut`]).
#[derive(Clone, Copy, Default)]
#[repr(transparent)]
struct BufferedEvent(MidiEvent<'static>);

/// View `events` as the events they wrap, with payloads borrowed for `'a`.
///
/// # Safety
///
/// The payloads of `events` must stay valid and unchanged for `'a`.
unsafe fn unwrap_events<'a>(events: &'a [BufferedEvent]) -> &'a [MidiEvent<'a>] {
    // SAFETY: `BufferedEvent` is a transparent wrapper of a `MidiEvent`, so
    // the slices have one layout. The caller guarantees the payloads for
    // `'a`.
    unsafe { std::slice::from_raw_parts(events.as_ptr().cast::<MidiEvent<'a>>(), events.len()) }
}

/// The kind of an event that borrows nothing, with any lifetime, or the
/// payload of a SysEx event.
fn kind_or_payload<'a, 'b>(kind: MidiEventKind<'a>) -> Result<MidiEventKind<'b>, &'a [u8]> {
    Ok(match kind {
        MidiEventKind::NoteOn(event) => MidiEventKind::NoteOn(event),
        MidiEventKind::NoteOff(event) => MidiEventKind::NoteOff(event),
        MidiEventKind::PolyPressure(event) => MidiEventKind::PolyPressure(event),
        MidiEventKind::ControlChange(event) => MidiEventKind::ControlChange(event),
        MidiEventKind::PitchBend(event) => MidiEventKind::PitchBend(event),
        MidiEventKind::ChannelPressure(event) => MidiEventKind::ChannelPressure(event),
        MidiEventKind::ProgramChange(event) => MidiEventKind::ProgramChange(event),
        MidiEventKind::ParameterNumber(event) => MidiEventKind::ParameterNumber(event),
        MidiEventKind::NoteExpressionValue(event) => MidiEventKind::NoteExpressionValue(event),
        MidiEventKind::NoteExpressionInt(event) => MidiEventKind::NoteExpressionInt(event),
        MidiEventKind::NoteExpressionText(event) => MidiEventKind::NoteExpressionText(event),
        MidiEventKind::ChordInfo(event) => MidiEventKind::ChordInfo(event),
        MidiEventKind::ScaleInfo(event) => MidiEventKind::ScaleInfo(event),
        MidiEventKind::SysEx(payload) => return Err(payload),
    })
}

/// A buffer for collecting MIDI events during processing.
///
/// The events live in an array of a fixed size and the SysEx bytes in a
/// pool that is allocated with the buffer, so pushing, reading and clearing
/// never allocate. Each SysEx message takes its payload plus two bytes (the
/// F0 and F7 it is stored with) of the pool until the buffer is cleared.
/// Events should be added in chronological order (by sample_offset).
pub struct MidiBuffer {
    /// The events, of which the first `len` are in the buffer. Those past
    /// `len` may point at pool bytes that were written again after a clear:
    /// read only the first `len`.
    events: [BufferedEvent; MAX_MIDI_EVENTS],
    len: usize,
    /// The SysEx bytes of the events, each message framed by F0 and F7.
    /// Its length is set at creation and never changes, so the bytes do not
    /// move while the buffer lives. It is written and read only through
    /// `as_mut_ptr` and `as_ptr`, which do not claim the whole allocation,
    /// so writing a new message leaves the payloads already handed out
    /// intact.
    sysex: Vec<u8>,
    /// Bytes of `sysex` in use.
    sysex_len: usize,
    /// Set when a push failed because the events were full.
    overflowed: bool,
    /// Set when a SysEx push failed because the pool was full.
    sysex_overflowed: bool,
}

impl MidiBuffer {
    /// Create an empty buffer on the heap, with a pool of `sysex_bytes` bytes
    /// for the SysEx messages of a block. Allocates; not for the audio
    /// thread.
    ///
    /// The `[MidiEvent; MAX_MIDI_EVENTS]` array (about 80 KB) is written
    /// straight to the heap, never as a temporary on the stack, which an
    /// audio thread's small stack could not hold in a debug build.
    pub fn new_boxed(sysex_bytes: usize) -> Box<Self> {
        let mut boxed = Box::<Self>::new_uninit();
        let ptr = boxed.as_mut_ptr();
        // SAFETY: Every field is written before `assume_init`. Each event is
        // a small value written to the heap through the pointer.
        unsafe {
            std::ptr::addr_of_mut!((*ptr).len).write(0);
            std::ptr::addr_of_mut!((*ptr).sysex).write(vec![0; sysex_bytes]);
            std::ptr::addr_of_mut!((*ptr).sysex_len).write(0);
            std::ptr::addr_of_mut!((*ptr).overflowed).write(false);
            std::ptr::addr_of_mut!((*ptr).sysex_overflowed).write(false);
            let events_ptr = std::ptr::addr_of_mut!((*ptr).events) as *mut BufferedEvent;
            for i in 0..MAX_MIDI_EVENTS {
                events_ptr.add(i).write(BufferedEvent::default());
            }
            boxed.assume_init()
        }
    }

    /// Clear all events and SysEx bytes from the buffer.
    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
        self.sysex_len = 0;
        self.overflowed = false;
        self.sysex_overflowed = false;
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

    /// Returns true if a push failed since the last clear because the
    /// events were full.
    #[inline]
    pub fn has_overflowed(&self) -> bool {
        self.overflowed
    }

    /// Returns true if a SysEx push failed since the last clear because the
    /// pool was full.
    #[inline]
    pub fn has_sysex_overflowed(&self) -> bool {
        self.sysex_overflowed
    }

    /// Push an event to the buffer. The payload of a SysEx event is copied
    /// into the buffer's pool; a payload that still has its F0 or F7 is
    /// stored framed once (no data byte of a SysEx message is 0x80 or
    /// above).
    ///
    /// Returns `false` and drops the event if the events or the pool are
    /// full; a SysEx message is never cut.
    pub fn push(&mut self, event: MidiEvent<'_>) -> bool {
        if self.len == MAX_MIDI_EVENTS {
            self.overflowed = true;
            return false;
        }
        let kind = match kind_or_payload(event.event) {
            Ok(kind) => kind,
            Err(payload) => match self.store_sysex(payload) {
                Some(stored) => MidiEventKind::SysEx(stored),
                None => {
                    self.sysex_overflowed = true;
                    return false;
                }
            },
        };
        self.events[self.len] = BufferedEvent(MidiEvent {
            sample_offset: event.sample_offset,
            event: kind,
        });
        self.len += 1;
        true
    }

    /// Copy a payload into the pool, framed by F0 and F7. Returns the
    /// stored payload, or `None` when it does not fit.
    fn store_sysex(&mut self, payload: &[u8]) -> Option<&'static [u8]> {
        let payload = payload.strip_prefix(&[0xF0]).unwrap_or(payload);
        let payload = payload.strip_suffix(&[0xF7]).unwrap_or(payload);
        let start = self.sysex_len;
        let needed = payload.len().checked_add(2)?;
        if needed > self.sysex.len() - start {
            return None;
        }
        let base = self.sysex.as_mut_ptr();
        // SAFETY: `start + needed` is within the pool. The range lies past
        // every stored message, so no handed-out payload covers it. Nor is
        // `payload` in it: the borrow checker keeps this buffer's own
        // payloads from reaching `push`, which takes `&mut self`.
        unsafe {
            base.add(start).write(0xF0);
            std::ptr::copy_nonoverlapping(payload.as_ptr(), base.add(start + 1), payload.len());
            base.add(start + 1 + payload.len()).write(0xF7);
        }
        self.sysex_len = start + needed;
        // SAFETY: The bytes were just written and do not change until
        // `clear`, which takes `&mut self`; the pool never moves. Events
        // leave the buffer only with the lifetime of its borrow.
        Some(unsafe { std::slice::from_raw_parts(self.sysex.as_ptr().add(start + 1), payload.len()) })
    }

    /// Remove the event at `index`, keeping the order of the others. Its
    /// SysEx bytes stay in the pool until the buffer is cleared.
    ///
    /// # Panics
    ///
    /// If `index` is not less than [`len`](Self::len).
    pub fn remove(&mut self, index: usize) {
        assert!(index < self.len, "index {index} out of {} events", self.len);
        self.events[index..self.len].rotate_left(1);
        self.len -= 1;
    }

    /// Replace or remove events in place, keeping their order and offsets.
    /// `f` gets the kind of each event without a SysEx payload and returns
    /// what takes its place, or `None` to remove it. SysEx events stay.
    pub(crate) fn map_kinds(&mut self, mut f: impl FnMut(MidiEventKind<'static>) -> Option<MidiEventKind<'static>>) {
        let mut kept = 0;
        for index in 0..self.len {
            let BufferedEvent(event) = self.events[index];
            let kind = match event.event {
                MidiEventKind::SysEx(_) => Some(event.event),
                other => {
                    let kind = f(other);
                    // A SysEx payload must point into this buffer's pool.
                    debug_assert!(!matches!(kind, Some(MidiEventKind::SysEx(_))));
                    kind
                }
            };
            if let Some(kind) = kind {
                self.events[kept] = BufferedEvent(MidiEvent {
                    sample_offset: event.sample_offset,
                    event: kind,
                });
                kept += 1;
            }
        }
        self.len = kept;
    }

    /// Iterate over events in the buffer.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &MidiEvent<'_>> {
        self.as_slice().iter()
    }

    /// Get the events as a slice, with SysEx payloads borrowed from the
    /// buffer.
    #[inline]
    pub fn as_slice(&self) -> &[MidiEvent<'_>] {
        // SAFETY: The payloads point into the pool, which does not change
        // while `self` is borrowed.
        unsafe { unwrap_events(&self.events[..self.len]) }
    }

    /// Reorder the events or change their offsets (see [`MidiEventsMut`]).
    #[inline]
    pub fn events_mut(&mut self) -> MidiEventsMut<'_> {
        MidiEventsMut {
            events: &mut self.events[..self.len],
        }
    }

    /// The message of the SysEx event at `index` as it goes to a host: the
    /// payload framed by F0 and F7. `None` if that event is not a SysEx
    /// event or `index` is out of range.
    pub fn framed_sysex(&self, index: usize) -> Option<&[u8]> {
        match self.as_slice().get(index)?.event {
            MidiEventKind::SysEx(payload) => {
                // Every payload in the buffer was stored by `store_sysex`,
                // with an F0 right before it and an F7 right after it in the
                // pool. The message is taken from the pool itself, not from
                // the payload's pointer, which covers the payload only.
                let start = payload.as_ptr() as usize - self.sysex.as_ptr() as usize;
                Some(&self.sysex[start - 1..start + payload.len() + 1])
            }
            _ => None,
        }
    }
}

impl core::fmt::Debug for MidiBuffer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MidiBuffer")
            .field("events", &self.as_slice())
            .field("sysex_capacity", &self.sysex.len())
            .field("sysex_len", &self.sysex_len)
            .field("overflowed", &self.overflowed)
            .field("sysex_overflowed", &self.sysex_overflowed)
            .finish()
    }
}

// No Default impl: MidiBuffer is ~80KB and must not be stack-allocated
// accidentally via mem::take, Box::default(), etc. Use new_boxed().

/// The events of a [`MidiBuffer`] with access to reorder them and change
/// their offsets, for the format wrappers. It cannot put an event into the
/// buffer, so every SysEx payload stays in the buffer's own pool: events go
/// in through [`MidiBuffer::push`] only.
pub struct MidiEventsMut<'a> {
    events: &'a mut [BufferedEvent],
}

impl<'a> MidiEventsMut<'a> {
    /// The number of events.
    #[inline]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns true if there are no events.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// The events, with SysEx payloads borrowed from the buffer.
    #[inline]
    pub fn as_slice(&self) -> &[MidiEvent<'_>] {
        // SAFETY: The payloads point into the pool of the buffer, which
        // stays borrowed for `'a`.
        unsafe { unwrap_events(self.events) }
    }

    /// The events for the rest of the borrow of the buffer.
    #[inline]
    pub fn into_slice(self) -> &'a [MidiEvent<'a>] {
        // SAFETY: As in `as_slice`.
        unsafe { unwrap_events(self.events) }
    }

    /// A shorter borrow of the same events.
    #[inline]
    pub fn reborrow(&mut self) -> MidiEventsMut<'_> {
        MidiEventsMut { events: self.events }
    }

    /// The sample offset of the event at `index`, to change it.
    ///
    /// # Panics
    ///
    /// If `index` is out of range.
    #[inline]
    pub fn sample_offset_mut(&mut self, index: usize) -> &mut u32 {
        &mut self.events[index].0.sample_offset
    }

    /// The sample offsets of all events, to change them.
    #[inline]
    pub fn sample_offsets_mut(&mut self) -> impl Iterator<Item = &mut u32> {
        self.events.iter_mut().map(|event| &mut event.0.sample_offset)
    }

    /// Swap two events.
    ///
    /// # Panics
    ///
    /// If `a` or `b` is out of range.
    #[inline]
    pub fn swap(&mut self, a: usize, b: usize) {
        self.events.swap(a, b);
    }

    /// Split into the events before `mid` and those from `mid` on.
    ///
    /// # Panics
    ///
    /// If `mid` is greater than [`len`](Self::len).
    #[inline]
    pub fn split_at(self, mid: usize) -> (MidiEventsMut<'a>, MidiEventsMut<'a>) {
        let (first, rest) = self.events.split_at_mut(mid);
        (MidiEventsMut { events: first }, MidiEventsMut { events: rest })
    }

    /// Sort by sample offset, keeping the order of events at one offset,
    /// without allocating (see [`crate::sub_blocks::sort_by_offset`]).
    #[inline]
    pub fn sort_by_offset(&mut self) {
        crate::sub_blocks::sort_by_offset(self.events, |event| event.0.sample_offset);
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

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn note(offset: u32, pitch: u8) -> MidiEvent<'static> {
        MidiEvent::note_on(offset, 0, pitch, 0.8, pitch as NoteId, 0.0, 0)
    }

    fn payloads<'a>(events: &[MidiEvent<'a>]) -> Vec<&'a [u8]> {
        events
            .iter()
            .filter_map(|event| match event.event {
                MidiEventKind::SysEx(payload) => Some(payload),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn midi_buffer_new_boxed_is_empty() {
        let buf = MidiBuffer::new_boxed(0);
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
        assert!(!buf.has_overflowed());
        assert!(!buf.has_sysex_overflowed());
    }

    #[test]
    fn midi_buffer_new_boxed_push_and_iterate() {
        let mut buf = MidiBuffer::new_boxed(0);

        assert!(buf.push(note(0, 60)));
        assert_eq!(buf.len(), 1);

        let first = buf.iter().next().unwrap();
        assert_eq!(first.sample_offset, 0);
        if let MidiEventKind::NoteOn(n) = &first.event {
            assert_eq!(n.pitch, 60);
        } else {
            panic!("expected NoteOn");
        }
    }

    #[test]
    fn midi_buffer_new_boxed_clear() {
        let mut buf = MidiBuffer::new_boxed(16);
        buf.push(note(0, 60));
        buf.push(MidiEvent::sysex(0, &[0x7E; 20]));
        assert!(buf.has_sysex_overflowed());
        buf.clear();
        assert!(buf.is_empty());
        assert!(!buf.has_overflowed());
        assert!(!buf.has_sysex_overflowed());
        // The pool is empty again: a message that fills it fits.
        assert!(buf.push(MidiEvent::sysex(0, &[0x7E; 14])));
    }

    #[test]
    fn test_sysex_payload_is_copied_into_the_buffer() {
        let mut buf = MidiBuffer::new_boxed(64);
        {
            let message = vec![0x7E, 0x00, 0x06, 0x01];
            assert!(buf.push(MidiEvent::sysex(3, &message)));
        }
        // The message the event was made from is gone; the buffer has its own copy.
        assert_eq!(payloads(buf.as_slice()), [[0x7E, 0x00, 0x06, 0x01]]);
        assert_eq!(buf.as_slice()[0].sample_offset, 3);
        assert_eq!(buf.framed_sysex(0), Some(&[0xF0, 0x7E, 0x00, 0x06, 0x01, 0xF7][..]));
    }

    #[test]
    fn test_a_framed_payload_is_framed_once() {
        let mut buf = MidiBuffer::new_boxed(64);
        buf.push(MidiEvent::sysex(0, &[0xF0, 0x7E, 0x01, 0xF7]));
        assert_eq!(payloads(buf.as_slice()), [[0x7E, 0x01]]);
        assert_eq!(buf.framed_sysex(0), Some(&[0xF0, 0x7E, 0x01, 0xF7][..]));
    }

    #[test]
    fn test_framed_sysex_of_other_events_is_none() {
        let mut buf = MidiBuffer::new_boxed(64);
        buf.push(note(0, 60));
        buf.push(MidiEvent::sysex(0, &[]));
        assert_eq!(buf.framed_sysex(0), None);
        assert_eq!(buf.framed_sysex(1), Some(&[0xF0, 0xF7][..]));
        assert_eq!(buf.framed_sysex(2), None);
    }

    #[test]
    fn test_sysex_that_does_not_fit_is_dropped_whole() {
        // Room for one message of 4 bytes with its F0 and F7 plus one of 2.
        let mut buf = MidiBuffer::new_boxed(10);
        assert!(buf.push(MidiEvent::sysex(0, &[1, 2, 3, 4])));
        assert!(!buf.push(MidiEvent::sysex(1, &[5, 6, 7])));
        assert!(buf.has_sysex_overflowed());
        assert!(!buf.has_overflowed());
        // Other events and a smaller message still fit.
        assert!(buf.push(note(2, 60)));
        assert!(buf.push(MidiEvent::sysex(3, &[8, 9])));
        assert_eq!(payloads(buf.as_slice()), [&[1, 2, 3, 4][..], &[8, 9][..]]);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn test_a_buffer_without_a_pool_drops_sysex_only() {
        let mut buf = MidiBuffer::new_boxed(0);
        assert!(!buf.push(MidiEvent::sysex(0, &[])));
        assert!(buf.push(note(0, 60)));
        assert_eq!(buf.len(), 1);
        assert!(buf.has_sysex_overflowed());
    }

    #[test]
    fn test_events_beyond_the_maximum_are_dropped() {
        let mut buf = MidiBuffer::new_boxed(16);
        for _ in 0..MAX_MIDI_EVENTS {
            assert!(buf.push(note(0, 60)));
        }
        assert!(!buf.push(note(0, 61)));
        assert!(!buf.push(MidiEvent::sysex(0, &[1])));
        assert!(buf.has_overflowed());
        assert!(!buf.has_sysex_overflowed());
        assert_eq!(buf.len(), MAX_MIDI_EVENTS);
    }

    #[test]
    fn test_events_pass_from_buffer_to_buffer() {
        let mut input = MidiBuffer::new_boxed(64);
        let mut output = MidiBuffer::new_boxed(64);
        input.push(note(0, 60));
        input.push(MidiEvent::sysex(5, &[0x41, 0x10]));
        for event in input.as_slice() {
            assert!(output.push(*event));
        }
        input.clear();
        input.push(MidiEvent::sysex(0, &[0x7F, 0x7F, 0x7F]));
        // The output holds copies of its own.
        assert_eq!(payloads(output.as_slice()), [[0x41, 0x10]]);
        assert_eq!(output.framed_sysex(1), Some(&[0xF0, 0x41, 0x10, 0xF7][..]));
    }

    #[test]
    fn test_reordering_keeps_the_payloads() {
        let mut buf = MidiBuffer::new_boxed(64);
        buf.push(MidiEvent::sysex(30, &[3]));
        buf.push(note(10, 60));
        buf.push(MidiEvent::sysex(10, &[1]));
        buf.push(MidiEvent::sysex(20, &[2]));
        buf.events_mut().sort_by_offset();
        let offsets: Vec<u32> = buf.iter().map(|event| event.sample_offset).collect();
        assert_eq!(offsets, [10, 10, 20, 30]);
        assert_eq!(payloads(buf.as_slice()), [[1], [2], [3]]);
        assert_eq!(buf.framed_sysex(3), Some(&[0xF0, 3, 0xF7][..]));

        let mut events = buf.events_mut();
        events.swap(0, 3);
        *events.sample_offset_mut(1) = 7;
        let (first, rest) = events.split_at(2);
        assert_eq!(first.len(), 2);
        assert_eq!(payloads(rest.into_slice()), [[2]]);
        assert_eq!(buf.as_slice()[1].sample_offset, 7);
        assert_eq!(payloads(buf.as_slice()), [[3], [1], [2]]);
    }

    #[test]
    fn test_only_events_without_a_payload_become_static() {
        let kept: MidiEvent<'static> = {
            let event = note(4, 60);
            event.to_static().expect("a note borrows nothing")
        };
        assert_eq!(kept, note(4, 60));
        assert_eq!(MidiEvent::sysex(0, &[1]).to_static(), None);
    }

    #[test]
    fn test_long_texts_are_cut_after_a_whole_character() {
        // 22 characters of three bytes each: the 64th byte is inside the
        // 22nd character, which is left out.
        let text = "ア".repeat(22);
        let MidiEventKind::NoteExpressionText(expr) = MidiEvent::note_expression_text(0, 1, 0, &text).event else {
            panic!("a text event");
        };
        assert_eq!(expr.as_str(), "ア".repeat(21));

        let name = "é".repeat(20);
        let MidiEventKind::ChordInfo(chord) = MidiEvent::chord_info(0, 0, -1, 1, &format!("x{name}")).event else {
            panic!("a chord event");
        };
        assert_eq!(chord.name_str(), format!("x{}", "é".repeat(15)));
        let MidiEventKind::ScaleInfo(scale) = MidiEvent::scale_info(0, 0, 1, &format!("x{name}")).event else {
            panic!("a scale event");
        };
        assert_eq!(scale.name_str(), format!("x{}", "é".repeat(15)));
    }

    #[test]
    fn test_a_text_length_out_of_range_reads_as_empty() {
        let MidiEventKind::NoteExpressionText(mut expr) = MidiEvent::note_expression_text(0, 1, 0, "la").event else {
            panic!("a text event");
        };
        expr.text_len = MAX_EXPRESSION_TEXT_SIZE as u8 + 1;
        assert_eq!(expr.as_str(), "");
    }

    #[test]
    fn test_offsets_change_through_a_view() {
        let mut buf = MidiBuffer::new_boxed(64);
        buf.push(note(1, 60));
        buf.push(MidiEvent::sysex(2, &[7]));
        let mut events = buf.events_mut();
        for offset in events.reborrow().sample_offsets_mut() {
            *offset += 10;
        }
        assert_eq!(events.len(), 2);
        let offsets: Vec<u32> = buf.iter().map(|event| event.sample_offset).collect();
        assert_eq!(offsets, [11, 12]);
        assert_eq!(buf.framed_sysex(1), Some(&[0xF0, 7, 0xF7][..]));
    }

    #[test]
    fn test_scaling_matches_the_midi_2_0_upscaling() {
        // The values Apple's translation from MIDI 1.0 gives.
        assert_eq!(scale::upscale_7_to_32(64), 0x8000_0000);
        assert_eq!(scale::upscale_7_to_32(127), 0xFFFF_FFFF);
        assert_eq!(scale::upscale_7_to_32(33), 0x4200_0000);
        assert_eq!(scale::upscale_7_to_16(64), 0x8000);
        assert_eq!(scale::upscale_7_to_16(127), 0xFFFF);
        assert_eq!(scale::upscale_14_to_32(12 << 7), 0x1800_0000);
        assert_eq!(scale::upscale_14_to_32(100 << 7), 0xC802_4012);
        assert_eq!(scale::upscale_14_to_32((100 << 7) | 5), 0xC816_40B2);
        assert_eq!(scale::upscale_14_to_32(0x3FFF), 0xFFFF_FFFF);
    }

    #[test]
    fn test_every_value_scales_back_to_itself() {
        for value in 0..=127u8 {
            assert_eq!(scale::to_7bit(scale::from_7bit(value)), value);
            assert_eq!(scale::velocity_to_7bit(scale::velocity_from_7bit(value)), value);
            // A plugin's n / 127 gives byte n.
            assert_eq!(scale::to_7bit(value as f64 / 127.0), value);
            assert_eq!(scale::velocity_to_7bit(value as f32 / 127.0), value);
        }
        for value in 0..=0x3FFFu16 {
            assert_eq!(scale::to_14bit(scale::from_14bit(value)), value);
            assert_eq!(scale::bipolar_to_14bit(scale::bipolar_from_14bit(value)), value);
        }
        for value in [0, 1, 0x7FFF_FFFF, 0x8000_0000, 0x8000_0001, 0xFFFF_FFFE, 0xFFFF_FFFF] {
            assert_eq!(scale::unipolar_to_u32(scale::unipolar_from_u32(value)), value);
            assert_eq!(scale::bipolar_to_u32(scale::bipolar_from_u32(value)), value);
        }
    }

    #[test]
    fn test_scaling_reaches_both_ends_and_the_center() {
        assert_eq!(scale::from_7bit(0), 0.0);
        assert_eq!(scale::from_7bit(127), 1.0);
        assert_eq!(scale::bipolar_from_14bit(0), -1.0);
        assert_eq!(scale::bipolar_from_14bit(8192), 0.0);
        assert_eq!(scale::bipolar_from_14bit(0x3FFF), 1.0);
        assert_eq!(scale::velocity_from_7bit(127), 1.0);
        // A sustain pedal at 64 is on, at 63 off.
        assert!(scale::from_7bit(64) >= 0.5);
        assert!(scale::from_7bit(63) < 0.5);
    }

    #[test]
    fn test_midi1_bytes_scale_as_midi_2_0_does() {
        let kind = |status, data1, data2| MidiEvent::from_midi1_bytes(0, 2, status, 3, data1, data2).unwrap().event;
        let MidiEventKind::NoteOn(on) = kind(0x90, 60, 64) else { panic!() };
        assert_eq!(on.velocity, scale::velocity_from_u16(0x8000));
        assert_eq!(on.note_id, note_id(2, 3, 60));
        // Velocity 0: a note-off with the default release velocity.
        let MidiEventKind::NoteOff(off) = kind(0x90, 60, 0) else { panic!() };
        assert_eq!(off.velocity, scale::velocity_from_u16(0x8000));
        assert_eq!(off.note_id, on.note_id);
        let MidiEventKind::ControlChange(change) = kind(0xB0, 64, 127) else { panic!() };
        assert_eq!(change.value, 1.0);
        let MidiEventKind::PitchBend(bend) = kind(0xE0, 0x7F, 0x7F) else { panic!() };
        assert_eq!(bend.value, 1.0);
        let MidiEventKind::ProgramChange(program) = kind(0xC0, 5, 0) else { panic!() };
        assert_eq!(program.bank, None);
        assert!(MidiEvent::from_midi1_bytes(0, 0, 0xF0, 8, 0, 0).is_none());
    }

    #[test]
    fn test_note_ids_hold_group_channel_and_pitch() {
        assert_eq!(note_id(0, 0, 60), 60);
        assert_eq!(note_id(0, 1, 60), 128 + 60);
        assert_eq!(note_id_parts(note_id(15, 15, 127)), Some((15, 15, 127)));
        assert_eq!(note_id_parts(-1), None);
        assert_eq!(note_id_parts(1 << 15), None);
    }

    #[test]
    fn test_tuning_values_are_vst3_s() {
        assert_eq!(tuning_semitones(0.5), 0.0);
        assert_eq!(tuning_semitones(1.0), 120.0);
        assert_eq!(tuning_value(-120.0), 0.0);
        assert_eq!(tuning_value(12.0), 0.55);
    }

    #[test]
    fn test_map_kinds_replaces_and_removes_in_order() {
        let mut buffer = MidiBuffer::new_boxed(16);
        buffer.push(MidiEvent::control_change(1, 0, 1, 0.5));
        buffer.push(MidiEvent::sysex(2, &[0x7D]));
        buffer.push(MidiEvent::control_change(3, 0, 2, 0.5));
        buffer.map_kinds(|kind| match kind {
            MidiEventKind::ControlChange(change) if change.controller == 1 => None,
            MidiEventKind::ControlChange(change) => Some(MidiEventKind::PitchBend(PitchBend {
                channel: change.channel,
                value: change.value,
            })),
            other => Some(other),
        });
        let events = buffer.as_slice();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event, MidiEventKind::SysEx(&[0x7D]));
        assert_eq!(events[1].sample_offset, 3);
        assert!(matches!(events[1].event, MidiEventKind::PitchBend(_)));
    }

    #[test]
    fn test_remove_keeps_the_order() {
        let mut buf = MidiBuffer::new_boxed(64);
        buf.push(note(0, 60));
        buf.push(MidiEvent::sysex(1, &[1, 2]));
        buf.push(note(2, 62));
        buf.remove(0);
        assert_eq!(buf.len(), 2);
        assert_eq!(payloads(buf.as_slice()), [[1, 2]]);
        assert_eq!(buf.as_slice()[1].sample_offset, 2);
    }
}
