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
//! - `Model::LAYOUTS` with a stereo and a mono layout
//! - Generic f32/f64 processing via `Sample` trait

use beamer::prelude::*;

// =============================================================================
// Model
// =============================================================================

/// Gain plugin model.
///
/// The `#[beamer::model]` attribute generates:
/// - `Parameters` trait (count, iter, by_id, groups, smoothing), which also
///   makes the model a `ParameterStore` for the host
/// - `Saved` trait (parameters and `State` fields, saving and loading)
/// - `WebViewEvents` trait (where the event queue of the web UI is)
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
    // Stereo by default, mono on a mono track. The gain works on any
    // number of channels.
    const LAYOUTS: &'static [Layout] = &[Layout::STEREO, Layout::MONO];

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

        for channel in buffer.channels_mut() {
            for sample in channel {
                *sample = *sample * gain;
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

#[cfg(test)]
mod tests {
    use super::*;
    use beamer::allocation_check;

    #[test]
    fn test_process_neither_allocates_nor_frees() {
        let mut processor = Arc::new(GainModel::default()).prepare(());
        let mut left = [0.5f32; 64];
        let mut right = [0.5f32; 64];
        let mut buffer = Buffer::new([&mut left[..], &mut right[..]], std::iter::empty(), 2, 64);
        let mut aux = AuxiliaryBuffers::empty();
        let context = ProcessContext::with_empty_transport(48_000.0, 64);

        let (_, counts) = allocation_check::count_in_process(|| {
            processor.process(&mut buffer, &mut aux, &context);
        });
        // The check runs in debug builds with the `allocation-check` feature.
        if allocation_check::is_active() {
            assert!(counts.is_empty(), "{counts:?}");
        }
    }
}
