//! Beamer Gain - Example gain plugin demonstrating the Beamer framework.
//!
//! # Three-Struct Pattern
//!
//! Beamer plugins use three structs for clear separation of concerns:
//!
//! 1. **`GainParameters`** - Pure parameter definitions with `#[derive(Parameters)]`
//! 2. **`GainDescriptor`** - Plugin descriptor that holds parameters and implements `Descriptor`
//! 3. **`GainProcessor`** - Runtime processor created by `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - `FloatParameter` with dB scaling via `kind = "db"`
//! - `()` setup for plugins without sample-rate-dependent state
//! - Generic f32/f64 processing via `Sample` trait

use beamer::prelude::*;

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Shared plugin configuration (format-agnostic metadata)
pub static CONFIG: Config = Config::new("Beamer Gain")
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
// Parameters
// =============================================================================

/// Gain plugin parameters.
///
/// The `#[derive(Parameters)]` macro generates:
/// - `Parameters` trait (count, iter, by_id, save_state, load_state)
/// - `ParameterStore` trait (host integration)
/// - `Default` trait (from attribute values)
#[derive(Parameters)]
pub struct GainParameters {
    /// Gain parameter: -60 dB to +12 dB, default 0 dB (unity gain)
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

impl GainParameters {
    /// Get the gain as a linear multiplier for DSP calculations.
    ///
    /// Converts dB to linear amplitude: `linear = 10^(dB / 20)`
    #[inline]
    pub fn gain_linear(&self) -> f32 {
        self.gain.as_linear() as f32
    }
}

// =============================================================================
// Descriptor
// =============================================================================

/// Gain plugin descriptor (unprepared state).
///
/// Holds parameters and describes the plugin to the host before audio
/// configuration is known. Transforms into `GainProcessor` via `prepare()`.
#[derive(Default, HasParameters)]
pub struct GainDescriptor {
    #[parameters]
    pub parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    type Setup = ();
    type Processor = GainProcessor;

    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor {
            parameters: self.parameters,
        }
    }
}

// =============================================================================
// Processor
// =============================================================================

/// Gain plugin processor (prepared state).
///
/// Ready for audio processing. Created by `GainDescriptor::prepare()`.
#[derive(HasParameters)]
pub struct GainProcessor {
    #[parameters]
    pub parameters: GainParameters,
}

impl GainProcessor {
    /// Generic processing implementation for both f32 and f64.
    fn process_generic<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        let gain = S::from_f32(self.parameters.gain_linear());

        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

impl Processor for GainProcessor {
    type Descriptor = GainDescriptor;

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
export_vst3!(CONFIG, VST3_CONFIG, GainDescriptor);

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, GainDescriptor);
