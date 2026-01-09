//! AU-specific plugin configuration.
//!
//! This module provides Audio Unit-specific configuration that complements
//! the shared [`beamer_core::PluginConfig`].

/// AU component type (4-character code).
///
/// This determines how the host categorizes and uses the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentType {
    /// Audio effect (aufx) - processes audio, no MIDI input.
    /// Used for EQs, compressors, reverbs, delays, etc.
    Effect,

    /// Music device/instrument (aumu) - generates audio from MIDI.
    /// Used for synthesizers, samplers, drum machines, etc.
    MusicDevice,

    /// MIDI processor (aumi) - processes MIDI, may or may not process audio.
    /// Used for MIDI effects, arpeggiators, chord generators, etc.
    MidiProcessor,
}

impl ComponentType {
    /// Get the component type as a 32-bit FourCC value (big-endian).
    pub const fn as_u32(&self) -> u32 {
        match self {
            Self::Effect => u32::from_be_bytes(*b"aufx"),
            Self::MusicDevice => u32::from_be_bytes(*b"aumu"),
            Self::MidiProcessor => u32::from_be_bytes(*b"aumi"),
        }
    }

    /// Get the component type as a 4-character string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Effect => "aufx",
            Self::MusicDevice => "aumu",
            Self::MidiProcessor => "aumi",
        }
    }

    /// Check if this component type supports MIDI output.
    ///
    /// MIDI output is supported via `scheduleMIDIEventBlock` for:
    /// - `aumu` (Music Device/Instrument): Yes - instruments typically output MIDI
    /// - `aumi` (MIDI Processor): Yes - MIDI effects transform/generate MIDI
    /// - `aufx` (Effect): No - effects typically don't output MIDI
    ///
    /// Note: Even if a component type supports MIDI output, the host must
    /// provide the `scheduleMIDIEventBlock` for it to work. Some hosts may
    /// not provide this block even for supported types.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use beamer_au::ComponentType;
    ///
    /// let synth = ComponentType::MusicDevice;
    /// assert!(synth.supports_midi_output());
    ///
    /// let effect = ComponentType::Effect;
    /// assert!(!effect.supports_midi_output());
    /// ```
    pub const fn supports_midi_output(&self) -> bool {
        match self {
            Self::MusicDevice | Self::MidiProcessor => true,
            Self::Effect => false,
        }
    }
}

/// Four-character code (FourCC) for AU identifiers.
///
/// Used for manufacturer codes and subtype codes in AU registration.
/// Must be exactly 4 ASCII characters.
///
/// # Example
///
/// ```ignore
/// use beamer_au::{fourcc, FourCharCode};
///
/// // Using the macro (compile-time validated)
/// const MANUFACTURER: FourCharCode = fourcc!(b"Demo");
/// const SUBTYPE: FourCharCode = fourcc!(b"gain");
///
/// // Or manually
/// const MANUFACTURER2: FourCharCode = FourCharCode::new(b"Demo");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FourCharCode(pub [u8; 4]);

impl FourCharCode {
    /// Create a new FourCharCode from a 4-byte array.
    ///
    /// # Panics
    /// Debug builds will panic if any byte is not ASCII.
    pub const fn new(bytes: &[u8; 4]) -> Self {
        // Note: const fn can't use loops in older Rust versions
        // This works in Rust 1.46+
        debug_assert!(bytes[0].is_ascii(), "FourCC bytes must be ASCII");
        debug_assert!(bytes[1].is_ascii(), "FourCC bytes must be ASCII");
        debug_assert!(bytes[2].is_ascii(), "FourCC bytes must be ASCII");
        debug_assert!(bytes[3].is_ascii(), "FourCC bytes must be ASCII");
        Self(*bytes)
    }

    /// Get the FourCC as a 32-bit value (big-endian).
    pub const fn as_u32(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    /// Get the FourCC as a string slice.
    pub fn as_str(&self) -> &str {
        // Safe because we validate ASCII in new()
        std::str::from_utf8(&self.0).unwrap_or("????")
    }

    /// Get the raw bytes.
    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }
}

impl std::fmt::Display for FourCharCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Macro for creating FourCharCode at compile time with validation.
///
/// # Example
///
/// ```ignore
/// use beamer_au::fourcc;
///
/// const MANUFACTURER: FourCharCode = fourcc!(b"Demo");
/// const SUBTYPE: FourCharCode = fourcc!(b"gain");
/// ```
///
/// # Compile-time Errors
///
/// The macro will fail to compile if the input is not exactly 4 ASCII bytes.
#[macro_export]
macro_rules! fourcc {
    ($s:literal) => {{
        const BYTES: &[u8] = $s;
        const _: () = assert!(BYTES.len() == 4, "FourCC must be exactly 4 bytes");
        const _: () = assert!(BYTES[0].is_ascii(), "FourCC byte 0 must be ASCII");
        const _: () = assert!(BYTES[1].is_ascii(), "FourCC byte 1 must be ASCII");
        const _: () = assert!(BYTES[2].is_ascii(), "FourCC byte 2 must be ASCII");
        const _: () = assert!(BYTES[3].is_ascii(), "FourCC byte 3 must be ASCII");
        $crate::FourCharCode::new(&[BYTES[0], BYTES[1], BYTES[2], BYTES[3]])
    }};
}

/// AU-specific plugin configuration.
///
/// This struct holds Audio Unit-specific metadata. Use in combination with
/// [`beamer_core::PluginConfig`] for complete plugin configuration.
///
/// # Example
///
/// ```ignore
/// use beamer_core::PluginConfig;
/// use beamer_au::{AuConfig, ComponentType, fourcc};
///
/// pub static CONFIG: PluginConfig = PluginConfig::new("Beamer Gain")
///     .with_vendor("Beamer Framework")
///     .with_version(env!("CARGO_PKG_VERSION"));
///
/// pub static AU_CONFIG: AuConfig = AuConfig::new(
///     ComponentType::Effect,
///     fourcc!(b"Demo"),  // Manufacturer
///     fourcc!(b"gain"),  // Subtype
/// );
///
/// export_au!(CONFIG, AU_CONFIG, GainPlugin);
/// ```
#[derive(Debug)]
pub struct AuConfig {
    /// Component type (aufx, aumu, aumi).
    pub component_type: ComponentType,

    /// Manufacturer code (4-character identifier for your company/brand).
    /// Should be unique across all AU developers.
    /// Apple recommends registering codes with them.
    pub manufacturer: FourCharCode,

    /// Subtype code (4-character identifier for this specific plugin).
    /// Should be unique within your manufacturer namespace.
    pub subtype: FourCharCode,

    /// Optional tags for additional categorization.
    /// Used in macOS 10.11+ for search and filtering.
    pub tags: &'static [&'static str],
}

impl AuConfig {
    /// Create a new AU configuration.
    ///
    /// # Arguments
    ///
    /// * `component_type` - The AU component type (Effect, MusicDevice, MidiProcessor)
    /// * `manufacturer` - Your 4-character manufacturer code
    /// * `subtype` - Your 4-character plugin subtype code
    pub const fn new(
        component_type: ComponentType,
        manufacturer: FourCharCode,
        subtype: FourCharCode,
    ) -> Self {
        Self {
            component_type,
            manufacturer,
            subtype,
            tags: &[],
        }
    }

    /// Add tags for additional categorization.
    ///
    /// Tags help users find plugins in hosts that support AU tag searching.
    /// Common tags include: "Delay", "Reverb", "Compressor", "EQ", "Filter",
    /// "Distortion", "Synth", "Sampler", etc.
    pub const fn with_tags(mut self, tags: &'static [&'static str]) -> Self {
        self.tags = tags;
        self
    }

    /// Get the full AudioComponentDescription type as a u32.
    pub const fn component_type_u32(&self) -> u32 {
        self.component_type.as_u32()
    }

    /// Get the manufacturer code as a u32.
    pub const fn manufacturer_u32(&self) -> u32 {
        self.manufacturer.as_u32()
    }

    /// Get the subtype code as a u32.
    pub const fn subtype_u32(&self) -> u32 {
        self.subtype.as_u32()
    }
}
