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
use beamer::{HasParameters, Parameters, Presets}; // Import the derive macros

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Component UID - unique identifier for the plugin (VST3 only)
#[cfg(feature = "vst3")]
const COMPONENT_UID: beamer::vst3::Steinberg::TUID =
    beamer::vst3::uid(0xDCDDB4BA, 0x2D6A4EC3, 0xA526D3E7, 0x244FAAE3);

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
#[cfg(feature = "vst3")]
pub static VST3_CONFIG: Vst3Config = Vst3Config::new(COMPONENT_UID);

/// AU-specific configuration
/// Uses manufacturer code "Bmer" and subtype "gain" for identification
#[cfg(feature = "au")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    fourcc!(b"Bmer"),
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

// =============================================================================
// Factory Presets
// =============================================================================

/// Factory presets for the gain plugin.
///
/// These presets demonstrate the sparse preset feature - each preset only
/// specifies the parameters it wants to change. The `#[derive(Presets)]` macro
/// generates the `FactoryPresets` trait implementation.
#[derive(Presets)]
#[preset(parameters = GainParameters)]
pub enum GainPresets {
    /// Unity gain - signal passes through unchanged
    #[preset(name = "Unity", values(gain = 0.0))]
    Unity,

    /// Quiet - reduce volume by 12 dB (quarter amplitude)
    #[preset(name = "Quiet", values(gain = -12.0))]
    Quiet,

    /// Boost - increase volume by 6 dB (double amplitude)
    #[preset(name = "Boost", values(gain = 6.0))]
    Boost,
}

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
    type Setup = (); // Simple gain doesn't need sample rate
    type Processor = GainProcessor;

    fn prepare(self, _: ()) -> GainProcessor {
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

#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, Vst3Processor<GainPlugin, GainPresets>);

// =============================================================================
// Audio Unit Export
// =============================================================================

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, GainPlugin, GainPresets);

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use beamer::prelude::FactoryPresets;

    #[test]
    fn factory_presets_count() {
        // GainPresets has 3 presets: Unity, Quiet, Boost
        assert_eq!(GainPresets::count(), 3);
    }

    #[test]
    fn factory_presets_info_unity() {
        let info = GainPresets::info(0);
        assert!(info.is_some());
        assert_eq!(info.unwrap().name, "Unity");
    }

    #[test]
    fn factory_presets_info_quiet() {
        let info = GainPresets::info(1);
        assert!(info.is_some());
        assert_eq!(info.unwrap().name, "Quiet");
    }

    #[test]
    fn factory_presets_info_boost() {
        let info = GainPresets::info(2);
        assert!(info.is_some());
        assert_eq!(info.unwrap().name, "Boost");
    }

    #[test]
    fn factory_presets_info_out_of_bounds() {
        assert!(GainPresets::info(3).is_none());
        assert!(GainPresets::info(100).is_none());
    }

    #[test]
    fn factory_presets_apply_unity() {
        let params = GainParameters::default();
        // Set gain to something other than 0.0 first to verify apply works
        params.gain.set_normalized(0.5);

        let result = GainPresets::apply(0, &params);
        assert!(result);
        // Unity preset sets gain to 0.0 dB
        let gain_value = params.gain.get();
        assert!(
            (gain_value - 0.0).abs() < 0.001,
            "Expected gain ~0.0, got {}",
            gain_value
        );
    }

    #[test]
    fn factory_presets_apply_quiet() {
        let params = GainParameters::default();

        let result = GainPresets::apply(1, &params);
        assert!(result);
        // Quiet preset sets gain to -12.0 dB
        let gain_value = params.gain.get();
        assert!(
            (gain_value - (-12.0)).abs() < 0.001,
            "Expected gain ~-12.0, got {}",
            gain_value
        );
    }

    #[test]
    fn factory_presets_apply_boost() {
        let params = GainParameters::default();

        let result = GainPresets::apply(2, &params);
        assert!(result);
        // Boost preset sets gain to 6.0 dB
        let gain_value = params.gain.get();
        assert!(
            (gain_value - 6.0).abs() < 0.001,
            "Expected gain ~6.0, got {}",
            gain_value
        );
    }

    #[test]
    fn factory_presets_apply_out_of_bounds() {
        let params = GainParameters::default();
        // Store original value
        let original = params.gain.get();

        // apply() with invalid index should return false
        let result = GainPresets::apply(3, &params);
        assert!(!result);

        // Parameter should be unchanged
        assert!(
            (params.gain.get() - original).abs() < 0.001,
            "Parameter should not change on invalid preset index"
        );
    }
}

// =============================================================================
// AU-specific MIDI Tests
// =============================================================================

/// Tests for MIDI Program Change → Factory Preset mapping in the AU wrapper.
///
/// This feature automatically maps MIDI PC events to factory presets:
/// - PC 0 → Preset 0, PC 1 → Preset 1, etc.
/// - PC events within preset range are applied and filtered out
/// - PC events outside preset range pass through to the plugin
#[cfg(all(test, feature = "au", target_os = "macos"))]
mod au_midi_tests {
    use super::*;
    use beamer::prelude::{AuProcessor, BusType, MidiEvent, MidiEventKind};
    use beamer::au_impl::{AuPluginInstance, CachedBusConfig, CachedBusInfo};
    use beamer::au_impl::render::MidiBuffer;

    /// Helper to create a prepared AuProcessor for testing.
    fn create_prepared_processor() -> AuProcessor<GainPlugin, GainPresets> {
        let mut processor = AuProcessor::<GainPlugin, GainPresets>::new();

        // Simple stereo config
        let bus_config = CachedBusConfig::new(
            vec![
                CachedBusInfo::new(2, BusType::Main),  // Main stereo input
                CachedBusInfo::new(2, BusType::Aux),   // Sidechain
            ],
            vec![CachedBusInfo::new(2, BusType::Main)], // Main stereo output
        );

        processor
            .allocate_render_resources(44100.0, 512, &bus_config)
            .expect("Failed to prepare processor");

        processor
    }

    // Normalized values for gain presets (range: -60..=12 dB, 72 dB total)
    // Unity (0 dB):   (0 - (-60)) / 72 = 60/72 ≈ 0.833
    // Quiet (-12 dB): (-12 - (-60)) / 72 = 48/72 ≈ 0.667
    // Boost (6 dB):   (6 - (-60)) / 72 = 66/72 ≈ 0.917
    const NORM_UNITY: f64 = 60.0 / 72.0;  // ~0.833
    const NORM_QUIET: f64 = 48.0 / 72.0;  // ~0.667
    const NORM_BOOST: f64 = 66.0 / 72.0;  // ~0.917

    /// Helper to get the gain parameter's normalized value.
    /// Uses info(0) to get the first parameter's ID, then reads its normalized value.
    fn get_gain_normalized(processor: &AuProcessor<GainPlugin, GainPresets>) -> f64 {
        let params = processor.parameter_store().unwrap();
        let gain_id = params.info(0).unwrap().id;
        params.get_normalized(gain_id)
    }

    /// Helper to set the gain parameter's normalized value.
    fn set_gain_normalized(processor: &mut AuProcessor<GainPlugin, GainPresets>, value: f64) {
        let params = processor.parameter_store().unwrap();
        let gain_id = params.info(0).unwrap().id;
        processor.parameter_store_mut().unwrap().set_normalized(gain_id, value);
    }

    #[test]
    fn midi_pc_applies_preset_and_filters() {
        let mut processor = create_prepared_processor();

        // Set gain to max (normalized 1.0 = 12 dB)
        set_gain_normalized(&mut processor, 1.0);

        // Send PC 1 (Quiet preset: gain = -12 dB)
        let input = vec![MidiEvent::program_change(0, 0, 1)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // PC event should be filtered out (preset was applied)
        assert_eq!(output.len(), 0, "PC event should be filtered out");

        // Verify the preset was applied (Quiet = -12 dB ≈ 0.667 normalized)
        let norm_value = get_gain_normalized(&processor);
        assert!(
            (norm_value - NORM_QUIET).abs() < 0.01,
            "Quiet preset should set normalized gain to ~{}, got {}",
            NORM_QUIET, norm_value
        );
    }

    #[test]
    fn midi_pc_out_of_range_passes_through() {
        let mut processor = create_prepared_processor();

        // Set initial gain to something known
        set_gain_normalized(&mut processor, 0.5);
        let initial_norm = get_gain_normalized(&processor);

        // Send PC 10 (out of range - only 3 presets)
        let input = vec![MidiEvent::program_change(0, 0, 10)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // Out-of-range PC should pass through
        assert_eq!(output.len(), 1, "Out-of-range PC should pass through");

        // Verify it's a PC event with correct program number
        if let MidiEventKind::ProgramChange(pc) = &output.iter().next().unwrap().event {
            assert_eq!(pc.program, 10);
        } else {
            panic!("Expected ProgramChange event");
        }

        // Parameters should be unchanged
        let final_norm = get_gain_normalized(&processor);
        assert!(
            (final_norm - initial_norm).abs() < 0.001,
            "Out-of-range PC should not change parameters"
        );
    }

    #[test]
    fn midi_other_events_pass_through() {
        let mut processor = create_prepared_processor();

        // Send control change events (simpler than notes)
        let input = vec![
            MidiEvent::control_change(0, 0, 1, 0.5),   // Mod wheel
            MidiEvent::control_change(10, 0, 7, 0.8),  // Volume
            MidiEvent::control_change(20, 0, 10, 0.5), // Pan
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // All events should pass through
        assert_eq!(output.len(), 3, "Non-PC events should pass through");
    }

    #[test]
    fn midi_mixed_events_filters_only_valid_pc() {
        let mut processor = create_prepared_processor();

        // Mix of events: CC, valid PC, CC, invalid PC, CC
        let input = vec![
            MidiEvent::control_change(0, 0, 1, 0.5),
            MidiEvent::program_change(0, 0, 2),  // Boost preset (valid)
            MidiEvent::control_change(10, 0, 7, 0.8),
            MidiEvent::program_change(20, 0, 50), // Invalid (out of range)
            MidiEvent::control_change(30, 0, 10, 0.5),
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // 5 input events, but valid PC (program 2) should be filtered
        // Remaining: CC, CC, PC(50), CC = 4 events
        assert_eq!(
            output.len(),
            4,
            "Valid PC should be filtered, others pass through"
        );

        // Verify Boost preset was applied
        let norm_value = get_gain_normalized(&processor);
        assert!(
            (norm_value - NORM_BOOST).abs() < 0.01,
            "Boost preset should set normalized gain to ~{}, got {}",
            NORM_BOOST, norm_value
        );
    }

    #[test]
    fn midi_pc_zero_applies_first_preset() {
        let mut processor = create_prepared_processor();

        // Set gain to something other than Unity
        set_gain_normalized(&mut processor, 0.3);

        // Send PC 0 (Unity preset: gain = 0 dB)
        let input = vec![MidiEvent::program_change(0, 0, 0)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // PC should be filtered
        assert_eq!(output.len(), 0);

        // Verify Unity preset was applied
        let norm_value = get_gain_normalized(&processor);
        assert!(
            (norm_value - NORM_UNITY).abs() < 0.01,
            "Unity preset should set normalized gain to ~{}, got {}",
            NORM_UNITY, norm_value
        );
    }

    #[test]
    fn midi_multiple_pc_events_last_wins() {
        let mut processor = create_prepared_processor();

        // Send multiple PC events - last valid one should win
        let input = vec![
            MidiEvent::program_change(0, 0, 0),  // Unity (0 dB)
            MidiEvent::program_change(10, 0, 1), // Quiet (-12 dB)
            MidiEvent::program_change(20, 0, 2), // Boost (6 dB) - last, should win
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // All PC events should be filtered
        assert_eq!(output.len(), 0);

        // Last preset (Boost) should be applied
        let norm_value = get_gain_normalized(&processor);
        assert!(
            (norm_value - NORM_BOOST).abs() < 0.01,
            "Last PC (Boost) should set normalized gain to ~{}, got {}",
            NORM_BOOST, norm_value
        );
    }
}
