//! Beamer Gain - Example gain plugin demonstrating the Beamer framework.
//!
//! This plugin shows how to:
//! 1. Use `#[derive(Params)]` macro for automatic trait implementations
//! 2. Implement the `AudioProcessor` trait for DSP
//! 3. Combine them with the `Plugin` trait
//! 4. Export using `Vst3Processor<T>` wrapper
//! 5. Use multi-bus support for sidechain ducking
//! 6. Access transport info via ProcessContext
//! 7. Use the new `FloatParam` type for cleaner parameter storage

use beamer::prelude::*;
use beamer::vst3_impl::vst3;
use beamer::Params; // Import the derive macro

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Component UID - unique identifier for the plugin
const COMPONENT_UID: vst3::Steinberg::TUID =
    vst3::uid(0xDCDDB4BA, 0x2D6A4EC3, 0xA526D3E7, 0x244FAAE3);

/// Static plugin configuration
/// Note: No .with_controller() - this is a simple plugin without custom GUI.
/// The host will use its generic parameter UI. For plugins with WebView GUI,
/// you would add .with_controller(CONTROLLER_UID).with_editor()
pub static CONFIG: PluginConfig = PluginConfig::new("Beamer Gain", COMPONENT_UID)
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version("0.0.1")
    .with_sub_categories("Fx|Dynamics");

// =============================================================================
// Parameters
// =============================================================================

/// Parameter collection for the gain plugin.
///
/// Uses **declarative parameter definition**: all configuration is in
/// attributes, and the `#[derive(Params)]` macro generates everything
/// including the `Default` implementation!
///
/// The macro generates:
/// - `Params` trait (count, iter, by_id, save_state, load_state)
/// - `Parameters` trait (VST3 integration)
/// - `Default` trait (from attribute values)
/// - Compile-time hash collision detection
#[derive(Params)]
pub struct GainParams {
    /// Gain parameter using declarative attribute syntax.
    /// - Default: 0 dB (unity gain)
    /// - Range: -60 dB to +12 dB
    #[param(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParam,
}

// No manual `new()` or `Default` impl needed - the macro generates everything!

impl GainParams {
    /// Get the gain as a linear multiplier.
    ///
    /// FloatParam::db() stores the value as linear amplitude internally,
    /// so we can use it directly in DSP without conversion.
    ///
    /// - 0 dB → linear 1.0 (unity gain)
    /// - -6 dB → linear ~0.5
    /// - +6 dB → linear ~2.0
    #[inline]
    pub fn gain_linear(&self) -> f32 {
        self.gain.as_linear() as f32
    }
}

// The #[derive(Params)] macro automatically generates:
// - impl Params for GainParams { ... }
// - impl Parameters for GainParams { ... }
// - impl Default for GainParams { ... } (when using declarative attrs)
// - const PARAM_GAIN_VST3_ID: u32 = fnv1a("gain")
// - Compile-time collision detection

// =============================================================================
// Audio Processor
// =============================================================================

/// The gain plugin processor.
///
/// This struct holds the plugin state and implements the DSP logic.
pub struct GainProcessor {
    /// Plugin parameters
    params: GainParams,
}

impl GainProcessor {
    /// Generic processing implementation for both f32 and f64.
    ///
    /// This demonstrates the recommended pattern: write your DSP once
    /// using the Sample trait, then delegate from both process() and
    /// process_f64() to avoid code duplication.
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        aux: &mut AuxiliaryBuffers<S>,
        context: &ProcessContext,
    ) {
        let gain = S::from_f32(self.params.gain_linear());

        // Example: Access transport info from host
        // tempo is available when the DAW provides it (most do)
        let _tempo = context.transport.tempo.unwrap_or(120.0);
        let _is_playing = context.transport.is_playing;

        // You could use tempo for tempo-synced effects:
        // let samples_per_beat = context.samples_per_beat().unwrap_or(22050.0);
        // let delay_samples = samples_per_beat * 0.25; // 16th note delay

        // Calculate sidechain level for ducking (if sidechain is connected)
        // Using the new AuxInput::rms() helper for cleaner code
        let sidechain_level: S = aux
            .sidechain()
            .map(|sc| {
                // Average RMS across all sidechain channels
                let mut sum = S::ZERO;
                for ch in 0..sc.num_channels() {
                    sum = sum + sc.rms(ch);
                }
                if sc.num_channels() > 0 {
                    sum / S::from_f32(sc.num_channels() as f32)
                } else {
                    S::ZERO
                }
            })
            .unwrap_or(S::ZERO);

        // Simple ducking: reduce gain when sidechain has signal
        // Ducking amount: 0 = no ducking, 1 = full ducking
        let duck_amount = (sidechain_level * S::from_f32(4.0)).min(S::ONE);
        let effective_gain = gain * (S::ONE - duck_amount * S::from_f32(0.8)); // Max 80% reduction

        // Process using zip_channels() iterator for cleaner code
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * effective_gain;
            }
        }
    }
}

impl AudioProcessor for GainProcessor {
    fn setup(&mut self, _sample_rate: f64, _max_buffer_size: usize) {
        // No sample-rate dependent state for a simple gain plugin
    }

    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
        // Delegate to generic implementation
        self.process_generic(buffer, aux, context);
    }

    // =========================================================================
    // 64-bit Processing Support
    // =========================================================================

    fn supports_double_precision(&self) -> bool {
        true // This plugin supports native f64 processing
    }

    fn process_f64(
        &mut self,
        buffer: &mut Buffer<f64>,
        aux: &mut AuxiliaryBuffers<f64>,
        context: &ProcessContext,
    ) {
        // Delegate to generic implementation - same code works for both f32 and f64!
        self.process_generic(buffer, aux, context);
    }

    // =========================================================================
    // Multi-Bus Configuration
    // =========================================================================

    fn input_bus_count(&self) -> usize {
        2 // Main stereo input + Sidechain input
    }

    fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
        match index {
            0 => Some(BusInfo::stereo("Input")),
            1 => Some(BusInfo::aux("Sidechain", 2)), // Stereo sidechain
            _ => None,
        }
    }

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        // Delegate to the macro-generated save_state which uses string-based IDs
        Ok(self.params.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        // Delegate to the macro-generated load_state
        self.params.load_state(data).map_err(|e| PluginError::StateError(e))
    }
}

// =============================================================================
// Plugin Trait Implementation
// =============================================================================

impl Plugin for GainProcessor {
    type Params = GainParams;

    fn params(&self) -> &Self::Params {
        &self.params
    }

    fn params_mut(&mut self) -> &mut Self::Params {
        &mut self.params
    }

    fn create() -> Self {
        Self {
            params: GainParams::default(),
        }
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

// Export VST3 entry points using the generic wrapper
export_vst3!(CONFIG, Vst3Processor<GainProcessor>);
