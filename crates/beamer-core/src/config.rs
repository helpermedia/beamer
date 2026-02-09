//! Plugin configuration.
//!
//! This module provides unified plugin configuration that covers shared
//! metadata and format-specific identifiers (AU four-char codes, VST3 UIDs).
//!
//! # Example
//!
//! ```ignore
//! use beamer_core::{Config, config::Category};
//!
//! pub static CONFIG: Config = Config::new("My Plugin", Category::Effect, "Mfgr", "plgn")
//!     .with_vendor("My Company")
//!     .with_version("1.0.0");
//! ```

// =========================================================================
// FourCharCode
// =========================================================================

/// Four-character code (FourCC) for AU identifiers.
///
/// Used for manufacturer codes and subtype codes in AU registration.
/// Must be exactly 4 ASCII characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FourCharCode(pub [u8; 4]);

impl FourCharCode {
    /// Create a new FourCharCode from a 4-byte array.
    ///
    /// # Panics
    /// Debug builds will panic if any byte is not ASCII.
    pub const fn new(bytes: &[u8; 4]) -> Self {
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
/// use beamer_core::fourcc;
///
/// const MANUFACTURER: FourCharCode = fourcc!(b"Demo");
/// const SUBTYPE: FourCharCode = fourcc!(b"gain");
/// ```
#[macro_export]
macro_rules! fourcc {
    ($s:literal) => {{
        const BYTES: &[u8] = $s;
        const _: () = assert!(BYTES.len() == 4, "FourCC must be exactly 4 bytes");
        const _: () = assert!(BYTES[0].is_ascii(), "FourCC byte 0 must be ASCII");
        const _: () = assert!(BYTES[1].is_ascii(), "FourCC byte 1 must be ASCII");
        const _: () = assert!(BYTES[2].is_ascii(), "FourCC byte 2 must be ASCII");
        const _: () = assert!(BYTES[3].is_ascii(), "FourCC byte 3 must be ASCII");
        $crate::config::FourCharCode::new(&[BYTES[0], BYTES[1], BYTES[2], BYTES[3]])
    }};
}

// =========================================================================
// Subcategory
// =========================================================================

/// Plugin subcategory for more specific classification.
///
/// These map directly to VST3 subcategories and AU tags.
/// Use with `Config::with_subcategories()` to specify plugin characteristics.
///
/// # Example
///
/// ```ignore
/// pub static CONFIG: Config = Config::new("My Compressor", Category::Effect)
///     .with_subcategories(&[Subcategory::Dynamics]);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subcategory {
    // === Effect Subcategories ===
    /// Scope, FFT-Display, Loudness Processing
    Analyzer,
    /// Tools dedicated to Bass Guitar
    Bass,
    /// Channel strip tools
    ChannelStrip,
    /// Delay, Multi-tap Delay, Ping-Pong Delay
    Delay,
    /// Amp Simulator, Sub-Harmonic, SoftClipper
    Distortion,
    /// Tools dedicated to Drums
    Drums,
    /// Compressor, Expander, Gate, Limiter, Maximizer
    Dynamics,
    /// Equalization, Graphical EQ
    Eq,
    /// WahWah, ToneBooster, Specific Filter
    Filter,
    /// Tone Generator, Noise Generator
    Generator,
    /// Tools dedicated to Guitar
    Guitar,
    /// Dither, Noise Shaping
    Mastering,
    /// Tools dedicated to Microphone
    Microphone,
    /// Phaser, Flanger, Chorus, Tremolo, Vibrato, AutoPan
    Modulation,
    /// Network-based effects
    Network,
    /// Pitch Processing, Pitch Correction, Vocal Tuning
    PitchShift,
    /// Denoiser, Declicker
    Restoration,
    /// Reverberation, Room Simulation, Convolution Reverb
    Reverb,
    /// MonoToStereo, StereoEnhancer
    Spatial,
    /// LFE Splitter, Bass Manager
    Surround,
    /// Volume, Mixer, Tuner
    Tools,
    /// Tools dedicated to Vocals
    Vocals,

    // === Instrument Subcategories ===
    /// Instrument for Drum sounds
    Drum,
    /// External wrapped hardware
    External,
    /// Instrument for Piano sounds
    Piano,
    /// Instrument based on Samples
    Sampler,
    /// Instrument based on Synthesis
    Synth,

    // === Channel Configuration ===
    /// Mono only plug-in
    Mono,
    /// Stereo only plug-in
    Stereo,
    /// Ambisonics channel
    Ambisonics,
    /// Mixconverter, Up-Mixer, Down-Mixer
    UpDownMix,

    // === Processing Constraints ===
    /// Supports only realtime processing
    OnlyRealTime,
    /// Offline processing only
    OnlyOfflineProcess,
    /// Works as normal insert plug-in only (no offline)
    NoOfflineProcess,
}

impl Subcategory {
    /// Get the VST3 subcategory string.
    pub const fn to_vst3(&self) -> &'static str {
        match self {
            // Effect subcategories
            Subcategory::Analyzer => "Analyzer",
            Subcategory::Bass => "Bass",
            Subcategory::ChannelStrip => "Channel Strip",
            Subcategory::Delay => "Delay",
            Subcategory::Distortion => "Distortion",
            Subcategory::Drums => "Drums",
            Subcategory::Dynamics => "Dynamics",
            Subcategory::Eq => "EQ",
            Subcategory::Filter => "Filter",
            Subcategory::Generator => "Generator",
            Subcategory::Guitar => "Guitar",
            Subcategory::Mastering => "Mastering",
            Subcategory::Microphone => "Microphone",
            Subcategory::Modulation => "Modulation",
            Subcategory::Network => "Network",
            Subcategory::PitchShift => "Pitch Shift",
            Subcategory::Restoration => "Restoration",
            Subcategory::Reverb => "Reverb",
            Subcategory::Spatial => "Spatial",
            Subcategory::Surround => "Surround",
            Subcategory::Tools => "Tools",
            Subcategory::Vocals => "Vocals",
            // Instrument subcategories
            Subcategory::Drum => "Drum",
            Subcategory::External => "External",
            Subcategory::Piano => "Piano",
            Subcategory::Sampler => "Sampler",
            Subcategory::Synth => "Synth",
            // Channel configuration
            Subcategory::Mono => "Mono",
            Subcategory::Stereo => "Stereo",
            Subcategory::Ambisonics => "Ambisonics",
            Subcategory::UpDownMix => "Up-Downmix",
            // Processing constraints
            Subcategory::OnlyRealTime => "OnlyRT",
            Subcategory::OnlyOfflineProcess => "OnlyOfflineProcess",
            Subcategory::NoOfflineProcess => "NoOfflineProcess",
        }
    }

    /// Get the AU tag string.
    ///
    /// AU tags are simpler and don't have all VST3 distinctions.
    /// Returns `None` for subcategories that don't map to AU tags.
    pub const fn to_au_tag(&self) -> Option<&'static str> {
        match self {
            Subcategory::Analyzer => Some("Analyzer"),
            Subcategory::Delay => Some("Delay"),
            Subcategory::Distortion => Some("Distortion"),
            Subcategory::Dynamics => Some("Dynamics"),
            Subcategory::Eq => Some("EQ"),
            Subcategory::Filter => Some("Filter"),
            Subcategory::Mastering => Some("Mastering"),
            Subcategory::Modulation => Some("Modulation"),
            Subcategory::PitchShift => Some("Pitch Shift"),
            Subcategory::Restoration => Some("Restoration"),
            Subcategory::Reverb => Some("Reverb"),
            Subcategory::Drum => Some("Drums"),
            Subcategory::Sampler => Some("Sampler"),
            Subcategory::Synth => Some("Synth"),
            Subcategory::Piano => Some("Piano"),
            Subcategory::Generator => Some("Generator"),
            // These don't have direct AU tag equivalents
            _ => None,
        }
    }
}

/// Plugin type - determines how hosts categorize and use the plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Audio effect (EQ, compressor, reverb, delay)
    Effect,
    /// Virtual instrument (synth, sampler, drum machine)
    Instrument,
    /// MIDI processor (arpeggiator, chord generator)
    MidiEffect,
    /// Audio generator (test tones, noise, file player)
    Generator,
}

impl Category {
    /// Convert to AU component type code (FourCC as u32, big-endian)
    pub const fn to_au_component_type(&self) -> u32 {
        match self {
            Category::Effect => u32::from_be_bytes(*b"aufx"),
            Category::Instrument => u32::from_be_bytes(*b"aumu"),
            Category::MidiEffect => u32::from_be_bytes(*b"aumi"),
            Category::Generator => u32::from_be_bytes(*b"augn"),
        }
    }

    /// Convert to VST3 base category string
    pub const fn to_vst3_category(&self) -> &'static str {
        match self {
            Category::Effect | Category::MidiEffect => "Fx",
            Category::Instrument => "Instrument",
            Category::Generator => "Generator",
        }
    }

    /// Check if this type accepts MIDI input
    pub const fn accepts_midi(&self) -> bool {
        matches!(self, Category::Instrument | Category::MidiEffect)
    }

    /// Check if this type can produce MIDI output
    pub const fn produces_midi(&self) -> bool {
        matches!(self, Category::Instrument | Category::MidiEffect)
    }
}

/// Default number of SysEx output slots per process block.
pub const DEFAULT_SYSEX_SLOTS: usize = 16;

/// Default SysEx buffer size in bytes per slot.
pub const DEFAULT_SYSEX_BUFFER_SIZE: usize = 512;

/// Unified plugin configuration.
///
/// Contains all plugin metadata: shared fields (name, vendor, category),
/// plugin identity (AU four-char codes), and VST3-specific settings.
/// The VST3 component UID is derived automatically from the AU codes
/// via FNV-1a hash unless explicitly overridden.
///
/// # Example
///
/// ```ignore
/// use beamer_core::{Config, config::Category};
///
/// pub static CONFIG: Config = Config::new("My Plugin", Category::Effect, "Mfgr", "plgn")
///     .with_vendor("My Company")
///     .with_version(env!("CARGO_PKG_VERSION"));
/// ```
#[derive(Debug, Clone)]
pub struct Config {
    /// Plugin name displayed in the DAW.
    pub name: &'static str,

    /// Plugin category (effect, instrument, etc.)
    pub category: Category,

    /// Vendor/company name.
    pub vendor: &'static str,

    /// Vendor URL.
    pub url: &'static str,

    /// Vendor email.
    pub email: &'static str,

    /// Plugin version string.
    pub version: &'static str,

    /// Whether this plugin has an editor/GUI.
    pub has_editor: bool,

    /// Plugin subcategories for more specific classification.
    pub subcategories: &'static [Subcategory],

    /// Manufacturer code (4-character identifier for your company/brand).
    pub manufacturer: FourCharCode,

    /// Subtype code (4-character identifier for this specific plugin).
    pub subtype: FourCharCode,

    /// Explicit VST3 component UID override. When `None`, the UID is
    /// derived from the manufacturer and subtype codes via FNV-1a hash.
    pub vst3_id: Option<[u32; 4]>,

    /// Explicit VST3 controller UID. When `None`, the plugin uses the
    /// combined component pattern (processor + controller in one object).
    pub vst3_controller_id: Option<[u32; 4]>,

    /// Number of SysEx output slots per process block (VST3).
    pub vst3_sysex_slots: usize,

    /// Maximum size of each SysEx message in bytes (VST3).
    pub vst3_sysex_buffer_size: usize,
}

/// Helper to convert a string literal to a 4-byte array at compile time.
const fn str_to_four_bytes(s: &str) -> [u8; 4] {
    let bytes = s.as_bytes();
    assert!(bytes.len() == 4, "FourCC string must be exactly 4 bytes");
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

// =========================================================================
// VST3 UUID derivation (FNV-1a 128-bit)
// =========================================================================

/// Beamer namespace salt for VST3 UID derivation.
const BEAMER_VST3_NAMESPACE: &[u8; 15] = b"beamer-vst3-uid";

/// Derive VST3 UID parts from a namespace and FourCC codes.
const fn derive_vst3_uid(namespace: &[u8], manufacturer: &[u8; 4], subtype: &[u8; 4]) -> [u32; 4] {
    // Build input: namespace + manufacturer + subtype
    // Use a fixed-size buffer large enough for any namespace we use (max 16 bytes + 8)
    let ns_len = namespace.len();
    let total_len = ns_len + 8;
    let mut data = [0u8; 24]; // max size
    let mut i = 0;
    while i < ns_len {
        data[i] = namespace[i];
        i += 1;
    }
    data[ns_len] = manufacturer[0];
    data[ns_len + 1] = manufacturer[1];
    data[ns_len + 2] = manufacturer[2];
    data[ns_len + 3] = manufacturer[3];
    data[ns_len + 4] = subtype[0];
    data[ns_len + 5] = subtype[1];
    data[ns_len + 6] = subtype[2];
    data[ns_len + 7] = subtype[3];

    // Hash only the relevant bytes
    let hash = fnv1a_128_len(&data, total_len);
    [
        (hash >> 96) as u32,
        (hash >> 64) as u32,
        (hash >> 32) as u32,
        hash as u32,
    ]
}

/// FNV-1a 128-bit hash with explicit length (for fixed-size buffer usage in const fn).
const fn fnv1a_128_len(data: &[u8], len: usize) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb0142_62b821756295c58d;
    const PRIME: u128 = 0x0000000001000000_000000000000013B;
    let mut hash = OFFSET;
    let mut i = 0;
    while i < len {
        hash ^= data[i] as u128;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

// =========================================================================
// UUID string parsing (compile-time)
// =========================================================================

/// Parse a hex character to its numeric value.
const fn hex_to_u8(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'A'..=b'F' => c - b'A' + 10,
        b'a'..=b'f' => c - b'a' + 10,
        _ => panic!("Invalid hex character in UUID"),
    }
}

/// Parse 8 hex digits (skipping dashes) into a u32.
const fn parse_uuid_u32(bytes: &[u8], start: usize) -> u32 {
    let mut result: u32 = 0;
    let mut i = 0;
    let mut hex_count = 0;
    while hex_count < 8 {
        let c = bytes[start + i];
        if c != b'-' {
            result = (result << 4) | (hex_to_u8(c) as u32);
            hex_count += 1;
        }
        i += 1;
    }
    result
}

/// Parse a UUID string ("XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX") into [u32; 4].
const fn parse_uuid(uuid: &str) -> [u32; 4] {
    let bytes = uuid.as_bytes();
    [
        parse_uuid_u32(bytes, 0),
        parse_uuid_u32(bytes, 9),
        parse_uuid_u32(bytes, 19),
        parse_uuid_u32(bytes, 28),
    ]
}

impl Config {
    /// Create a new plugin configuration.
    ///
    /// # Arguments
    ///
    /// * `name` - Plugin name displayed in the DAW
    /// * `category` - Plugin category (effect, instrument, etc.)
    /// * `manufacturer` - 4-character manufacturer code (e.g., "Bmer")
    /// * `subtype` - 4-character plugin subtype code (e.g., "gain")
    ///
    /// # Panics
    /// Panics at compile time if manufacturer or subtype are not exactly 4 ASCII characters.
    ///
    /// # Example
    ///
    /// ```ignore
    /// pub static CONFIG: Config = Config::new("My Plugin", Category::Effect, "Mfgr", "plgn")
    ///     .with_vendor("My Company")
    ///     .with_version(env!("CARGO_PKG_VERSION"));
    /// ```
    pub const fn new(name: &'static str, category: Category, manufacturer: &str, subtype: &str) -> Self {
        Self {
            name,
            category,
            vendor: "Unknown Vendor",
            url: "",
            email: "",
            version: "1.0.0",
            has_editor: false,
            subcategories: &[],
            manufacturer: FourCharCode::new(&str_to_four_bytes(manufacturer)),
            subtype: FourCharCode::new(&str_to_four_bytes(subtype)),
            vst3_id: None,
            vst3_controller_id: None,
            vst3_sysex_slots: DEFAULT_SYSEX_SLOTS,
            vst3_sysex_buffer_size: DEFAULT_SYSEX_BUFFER_SIZE,
        }
    }

    /// Set the vendor name.
    pub const fn with_vendor(mut self, vendor: &'static str) -> Self {
        self.vendor = vendor;
        self
    }

    /// Set the vendor URL.
    pub const fn with_url(mut self, url: &'static str) -> Self {
        self.url = url;
        self
    }

    /// Set the vendor email.
    pub const fn with_email(mut self, email: &'static str) -> Self {
        self.email = email;
        self
    }

    /// Set the version string.
    pub const fn with_version(mut self, version: &'static str) -> Self {
        self.version = version;
        self
    }

    /// Enable the editor/GUI.
    pub const fn with_editor(mut self) -> Self {
        self.has_editor = true;
        self
    }

    /// Set the plugin subcategories.
    ///
    /// Subcategories provide more specific classification beyond the main category.
    /// They are used for VST3 subcategory strings and AU tags.
    pub const fn with_subcategories(mut self, subcategories: &'static [Subcategory]) -> Self {
        self.subcategories = subcategories;
        self
    }

    /// Override the auto-derived VST3 component UID with an explicit UUID.
    ///
    /// By default, the VST3 UID is derived from the manufacturer and subtype
    /// codes. Use this only when you need a specific UUID (e.g., matching an
    /// existing shipped plugin).
    ///
    /// # Arguments
    ///
    /// * `uuid` - UUID string in format "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
    pub const fn with_vst3_id(mut self, uuid: &'static str) -> Self {
        self.vst3_id = Some(parse_uuid(uuid));
        self
    }

    /// Set an explicit VST3 controller UID to enable split component/controller mode.
    ///
    /// By default, plugins use the combined component pattern (processor and
    /// controller in one object). Use this for split architecture.
    ///
    /// # Arguments
    ///
    /// * `uuid` - UUID string in format "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
    pub const fn with_vst3_controller_id(mut self, uuid: &'static str) -> Self {
        self.vst3_controller_id = Some(parse_uuid(uuid));
        self
    }

    /// Set the number of SysEx output slots per process block (VST3).
    ///
    /// Higher values allow more concurrent SysEx messages but use more memory.
    /// Default is 16 slots.
    pub const fn with_vst3_sysex_slots(mut self, slots: usize) -> Self {
        self.vst3_sysex_slots = slots;
        self
    }

    /// Set the maximum size of each SysEx message in bytes (VST3).
    ///
    /// Messages larger than this will be truncated. Default is 512 bytes.
    pub const fn with_vst3_sysex_buffer_size(mut self, size: usize) -> Self {
        self.vst3_sysex_buffer_size = size;
        self
    }

    /// Get VST3 component UID as [u32; 4].
    ///
    /// Returns the explicit override if set via `with_vst3_id()`, otherwise
    /// derives a UID from the manufacturer and subtype codes via FNV-1a hash.
    pub const fn vst3_uid_parts(&self) -> [u32; 4] {
        match self.vst3_id {
            Some(parts) => parts,
            None => derive_vst3_uid(
                BEAMER_VST3_NAMESPACE.as_slice(),
                self.manufacturer.as_bytes(),
                self.subtype.as_bytes(),
            ),
        }
    }

    /// Get VST3 controller UID as [u32; 4], if split component/controller mode is enabled.
    pub const fn vst3_controller_uid_parts(&self) -> Option<[u32; 4]> {
        self.vst3_controller_id
    }

    /// Get the manufacturer code as a u32.
    pub const fn manufacturer_u32(&self) -> u32 {
        self.manufacturer.as_u32()
    }

    /// Get the subtype code as a u32.
    pub const fn subtype_u32(&self) -> u32 {
        self.subtype.as_u32()
    }

    /// Build the VST3 subcategories string.
    ///
    /// Combines the main category with subcategories using pipe separators.
    /// For example: `Category::Effect` with `[Subcategory::Dynamics]` becomes `"Fx|Dynamics"`.
    pub fn vst3_subcategories(&self) -> String {
        let mut result = String::from(self.category.to_vst3_category());
        for sub in self.subcategories {
            result.push('|');
            result.push_str(sub.to_vst3());
        }
        result
    }

    /// Get AU tags derived from subcategories.
    ///
    /// Returns tags for subcategories that have AU equivalents.
    /// Subcategories without AU mappings are skipped.
    pub fn au_tags(&self) -> Vec<&'static str> {
        self.subcategories
            .iter()
            .filter_map(|sub| sub.to_au_tag())
            .collect()
    }
}
