//! Beamer Gain - Example gain plugin demonstrating the Beamer framework.
//!
//! # Two-Struct Pattern
//!
//! Beamer plugins use two structs:
//!
//! 1. **`GainModel`** - What every thread shares, marked with
//!    `#[beamer::model]` and implementing `Model`: its parameters, how the
//!    host sees it and how it creates the processor.
//! 2. **`GainProcessor`** - What the audio thread owns, created by
//!    `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - `FloatParameter` with dB scaling via `kind = "db"`
//! - `()` setup for plugins without sample-rate-dependent state
//! - Generic f32/f64 processing via `Sample` trait

use beamer::prelude::*;

// =============================================================================
// Model
// =============================================================================

/// Gain plugin model.
///
/// The `#[beamer::model]` attribute generates:
/// - `Parameters` trait (count, iter, by_id, groups, smoothing)
/// - `ModelFields` trait (`State` fields, the event queue, saving and loading)
/// - `ParameterStore` trait (host integration)
/// - `Default` trait (from attribute values)
///
/// It lives as long as the plugin instance and creates the `GainProcessor`
/// via `prepare()` once the audio configuration is known.
#[beamer::export]
#[beamer::model]
pub struct GainModel {
    /// Gain parameter: -60 dB to +12 dB, default 0 dB (unity gain)
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

impl GainModel {
    /// Get the gain as a linear multiplier for DSP calculations.
    ///
    /// Converts dB to linear amplitude: `linear = 10^(dB / 20)`
    #[inline]
    pub fn gain_linear(&self) -> f32 {
        self.gain.as_linear() as f32
    }
}

impl Model for GainModel {
    type Setup = ();
    type Processor = GainProcessor;

    fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
        GainProcessor { model: self }
    }
}

// =============================================================================
// Processor
// =============================================================================

/// Gain plugin processor.
///
/// Ready for audio processing. Created by `GainModel::prepare()`.
pub struct GainProcessor {
    pub model: Arc<GainModel>,
}

impl GainProcessor {
    /// Generic processing implementation for both f32 and f64.
    fn process_generic<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        let gain = S::from_f32(self.model.gain_linear());

        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

impl Processor for GainProcessor {
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
