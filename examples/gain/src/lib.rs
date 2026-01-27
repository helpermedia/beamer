//! Beamer Gain - Example gain plugin demonstrating the simplest Beamer pattern.
//!
//! This plugin shows the **minimal single-struct pattern** where one struct
//! serves as both Plugin and Processor. This works for plugins without
//! preparation-time state (like delay buffers that need sample rate).
//!
//! Key points:
//! 1. `#[derive(Parameters)]` generates all parameter traits including `Default`
//! 2. Single struct implements both `Plugin` and `AudioProcessor`
//! 3. `type Processor = Self` and `fn prepare(self, _: ()) -> Self { self }`

use beamer::prelude::*;

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Shared plugin configuration (format-agnostic metadata)
pub static CONFIG: PluginConfig = PluginConfig::new("Beamer Gain")
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version(env!("CARGO_PKG_VERSION"));

/// VST3-specific configuration
#[cfg(feature = "vst3")]
pub static VST3_CONFIG: Vst3Config = Vst3Config::new("DCDDB4BA-2D6A-4EC3-A526-D3E7244FAAE3")
    .with_categories("Fx|Dynamics");

/// AU-specific configuration
#[cfg(feature = "au")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    "Bmer",
    "gain",
);

// =============================================================================
// Gain Plugin (Single-Struct Pattern)
// =============================================================================

/// A simple gain plugin demonstrating the minimal Beamer pattern.
///
/// This struct is both the Plugin (unprepared state) and the AudioProcessor
/// (prepared state). For simple plugins without DSP state that depends on
/// sample rate or buffer size, this single-struct pattern is recommended.
///
/// The `#[derive(Parameters)]` macro generates:
/// - `Parameters` trait (count, iter, by_id, save_state, load_state)
/// - `ParameterStore` trait (host integration)
/// - `HasParameters` trait (parameters() and parameters_mut())
/// - `Default` trait (from attribute values)
#[derive(Parameters)]
pub struct Gain {
    /// Gain parameter: -60 dB to +12 dB, default 0 dB (unity gain)
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

impl Gain {
    /// Get the gain as a linear multiplier for DSP calculations.
    ///
    /// Converts dB to linear amplitude: `linear = 10^(dB / 20)`
    #[inline]
    pub fn gain_linear(&self) -> f32 {
        self.gain.as_linear() as f32
    }

    /// Generic processing implementation for both f32 and f64.
    fn process_generic<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        let gain = S::from_f32(self.gain_linear());

        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

impl Plugin for Gain {
    type Setup = ();
    type Processor = Self;

    fn prepare(self, _: ()) -> Self {
        self
    }
}

impl AudioProcessor for Gain {
    type Plugin = Self;

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        self.process_generic(buffer);
    }

    fn supports_double_precision(&self) -> bool {
        true
    }

    fn process_f64(
        &mut self,
        buffer: &mut Buffer<f64>,
        _aux: &mut AuxiliaryBuffers<f64>,
        _context: &ProcessContext,
    ) {
        self.process_generic(buffer);
    }
}

// =============================================================================
// Plugin Exports
// =============================================================================

#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, Gain);

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, Gain);
