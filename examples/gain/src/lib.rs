//! Beamer Gain - Example gain plugin demonstrating the Beamer framework.
//!
//! This plugin shows how to:
//! 1. Use `#[derive(Parameters)]` macro for automatic trait implementations
//! 2. Use `#[derive(HasParameters)]` to eliminate parameters() boilerplate
//! 3. Implement the two-phase Plugin → AudioProcessor lifecycle
//! 4. Export using `Vst3Processor<T>` wrapper
//! 5. Use multi-bus support for sidechain ducking
//! 6. Access transport info via ProcessContext
//! 7. Use the `FloatParameter` type for cleaner parameter storage

use beamer::prelude::*;
use beamer::vst3_impl::vst3;
use beamer::{HasParameters, Parameters}; // Import the derive macros

#[cfg(target_os = "macos")]
use beamer_au::{export_au, AuConfig, ComponentType, fourcc};

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Component UID - unique identifier for the plugin
const COMPONENT_UID: vst3::Steinberg::TUID =
    vst3::uid(0xDCDDB4BA, 0x2D6A4EC3, 0xA526D3E7, 0x244FAAE3);

/// Shared plugin configuration (format-agnostic metadata)
pub static CONFIG: PluginConfig = PluginConfig::new("Beamer Gain")
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_sub_categories("Fx|Dynamics");

/// VST3-specific configuration
/// Note: No .with_controller() - this is a simple plugin without custom GUI.
/// The host will use its generic parameter UI. For plugins with WebView GUI,
/// you would add .with_controller(CONTROLLER_UID)
pub static VST3_CONFIG: Vst3Config = Vst3Config::new(COMPONENT_UID);

/// AU-specific configuration
/// Uses manufacturer code "Demo" and subtype "gain" for identification
#[cfg(target_os = "macos")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    fourcc!(b"Demo"),
    fourcc!(b"gain"),
);

// =============================================================================
// Parameters
// =============================================================================

/// Parameter collection for the gain plugin.
///
/// Uses **declarative parameter definition**: all configuration is in
/// attributes, and the `#[derive(Parameters)]` macro generates everything
/// including the `Default` implementation!
///
/// The macro generates:
/// - `Parameters` trait (count, iter, by_id, save_state, load_state)
/// - `ParameterStore` trait (host integration)
/// - `Default` trait (from attribute values)
/// - Compile-time hash collision detection
#[derive(Parameters)]
pub struct GainParameters {
    /// Gain parameter using declarative attribute syntax.
    /// - Default: 0 dB (unity gain)
    /// - Range: -60 dB to +12 dB
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

// No manual `new()` or `Default` impl needed - the macro generates everything!

impl GainParameters {
    /// Get the gain as a linear multiplier for DSP calculations.
    ///
    /// Converts the dB value to a linear amplitude multiplier using the formula:
    ///
    /// ```text
    /// linear = 10^(dB / 20)
    /// ```
    ///
    /// # Returns
    /// Linear amplitude multiplier (always positive)
    ///
    /// # Examples
    /// | dB Value | Linear Multiplier |
    /// |----------|-------------------|
    /// | 0 dB     | 1.0 (unity gain)  |
    /// | -6 dB    | ~0.501 (half)     |
    /// | +6 dB    | ~1.995 (double)   |
    /// | -12 dB   | ~0.251 (quarter)  |
    /// | -∞ dB    | 0.0 (silence)     |
    #[inline]
    pub fn gain_linear(&self) -> f32 {
        self.gain.as_linear() as f32
    }
}

// =============================================================================
// Plugin (Unprepared State)
// =============================================================================

/// The gain plugin in its unprepared state.
///
/// This struct holds the parameters before audio configuration is known.
/// When the host calls setupProcessing(), it is transformed into a
/// [`GainProcessor`] via the [`Plugin::prepare()`] method.
///
/// The `#[derive(HasParameters)]` macro automatically implements `parameters()` and
/// `parameters_mut()` by looking for the field marked with `#[parameters]`.
#[derive(Default, HasParameters)]
pub struct GainPlugin {
    /// Plugin parameters
    #[parameters]
    parameters: GainParameters,
}

impl Plugin for GainPlugin {
    type Config = NoConfig; // Simple gain doesn't need sample rate
    type Processor = GainProcessor;

    fn prepare(self, _config: NoConfig) -> GainProcessor {
        GainProcessor {
            parameters: self.parameters,
        }
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
}

// =============================================================================
// Audio Processor (Prepared State)
// =============================================================================

/// The gain plugin processor, ready for audio processing.
///
/// This struct is created by [`GainPlugin::prepare()`] and contains
/// everything needed for real-time audio processing.
///
/// The `#[derive(HasParameters)]` macro automatically implements `parameters()` and
/// `parameters_mut()` by looking for the field marked with `#[parameters]`.
#[derive(HasParameters)]
pub struct GainProcessor {
    /// Plugin parameters
    #[parameters]
    parameters: GainParameters,
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
        let gain = S::from_f32(self.parameters.gain_linear());

        // Example: Access transport info from host
        // tempo is available when the DAW provides it (most do)
        let _tempo = context.transport.tempo.unwrap_or(120.0);
        let _is_playing = context.transport.is_playing;

        // You could use tempo for tempo-synced effects:
        // let samples_per_beat = context.samples_per_beat().unwrap_or(22050.0);
        // let delay_samples = samples_per_beat * 0.25; // 16th note delay

        // =================================================================
        // Sidechain Ducking
        // =================================================================
        // Calculate average RMS level across sidechain channels.
        // RMS (Root Mean Square) measures the "power" of the signal:
        //   RMS = sqrt(sum(samples²) / N)
        //
        // This gives a more musical/perceptual level than peak detection.
        let sidechain_level: S = aux
            .sidechain()
            .map(|sc| {
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

        // Simple ducking formula:
        //   duck_amount = clamp(sidechain_level * sensitivity, 0, 1)
        //   effective_gain = gain * (1 - duck_amount * max_reduction)
        //
        // With sensitivity=4.0 and max_reduction=0.8:
        // - Sidechain at 0.0 → no ducking (gain unchanged)
        // - Sidechain at 0.25 → full ducking (80% gain reduction)
        let duck_amount = (sidechain_level * S::from_f32(4.0)).min(S::ONE);
        let effective_gain = gain * (S::ONE - duck_amount * S::from_f32(0.8));

        // Process using zip_channels() iterator for cleaner code
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * effective_gain;
            }
        }
    }
}

impl AudioProcessor for GainProcessor {
    type Plugin = GainPlugin;

    fn unprepare(self) -> GainPlugin {
        GainPlugin {
            parameters: self.parameters,
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &ProcessContext,
    ) {
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
    // State Persistence
    // =========================================================================

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        // Delegate to the macro-generated save_state which uses string-based IDs
        Ok(self.parameters.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        // Delegate to the macro-generated load_state
        self.parameters.load_state(data).map_err(PluginError::StateError)
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

// Export VST3 entry points using the generic wrapper
export_vst3!(CONFIG, VST3_CONFIG, Vst3Processor<GainPlugin>);

// =============================================================================
// Audio Unit Export
// =============================================================================

#[cfg(target_os = "macos")]
export_au!(CONFIG, AU_CONFIG, GainPlugin);
