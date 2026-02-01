//! Shared plugin configuration.
//!
//! This module provides format-agnostic plugin metadata that is shared
//! across all plugin formats (AU, VST3).
//!
//! Format-specific configurations (UIDs, FourCC codes, etc.) are defined
//! in their respective crates.
//!
//! # Example
//!
//! ```ignore
//! use beamer_core::{Config, Category};
//!
//! pub static CONFIG: Config = Config::new("My Plugin", Category::Effect)
//!     .with_vendor("My Company")
//!     .with_version("1.0.0");
//! ```

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

/// Format-agnostic plugin configuration.
///
/// Contains metadata shared across all plugin formats. Format-specific
/// configurations (like VST3 UIDs or AU FourCC codes) are defined separately.
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
}

impl Config {
    /// Create a new plugin configuration with default values.
    ///
    /// # Example
    ///
    /// ```ignore
    /// pub static CONFIG: Config = Config::new("My Plugin", Category::Effect)
    ///     .with_vendor("My Company")
    ///     .with_version(env!("CARGO_PKG_VERSION"));
    /// ```
    pub const fn new(name: &'static str, category: Category) -> Self {
        Self {
            name,
            category,
            vendor: "Unknown Vendor",
            url: "",
            email: "",
            version: "1.0.0",
            has_editor: false,
            subcategories: &[],
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
    ///
    /// # Example
    ///
    /// ```ignore
    /// pub static CONFIG: Config = Config::new("My Compressor", Category::Effect)
    ///     .with_subcategories(&[Subcategory::Dynamics]);
    /// ```
    pub const fn with_subcategories(mut self, subcategories: &'static [Subcategory]) -> Self {
        self.subcategories = subcategories;
        self
    }

    /// Build the VST3 subcategories string.
    ///
    /// Combines the main category with subcategories using pipe separators.
    /// For example: `Category::Effect` with `[Subcategory::Dynamics]` becomes `"Fx|Dynamics"`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let config = Config::new("My Plugin", Category::Effect)
    ///     .with_subcategories(&[Subcategory::Dynamics, Subcategory::Eq]);
    /// assert_eq!(config.vst3_subcategories(), "Fx|Dynamics|EQ");
    /// ```
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
    ///
    /// # Example
    ///
    /// ```ignore
    /// let config = Config::new("My Plugin", Category::Effect)
    ///     .with_subcategories(&[Subcategory::Dynamics, Subcategory::Eq]);
    /// assert_eq!(config.au_tags(), vec!["Dynamics", "EQ"]);
    /// ```
    pub fn au_tags(&self) -> Vec<&'static str> {
        self.subcategories
            .iter()
            .filter_map(|sub| sub.to_au_tag())
            .collect()
    }
}
