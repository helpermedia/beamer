//! BR Gain - Example gain plugin demonstrating the BEAMR framework.
//!
//! This plugin shows how to:
//! 1. Implement the `AudioProcessor` trait for DSP
//! 2. Implement the `Parameters` trait for host communication
//! 3. Combine them with the `Plugin` trait
//! 4. Export using `Vst3Processor<T>` wrapper
//! 5. Use multi-bus support for sidechain ducking
//! 6. Access transport info via ProcessContext

use std::sync::atomic::{AtomicU64, Ordering};

use beamr::prelude::*;
use beamr::vst3_impl::vst3;

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
pub static CONFIG: PluginConfig = PluginConfig::new("BR Gain", COMPONENT_UID)
    .with_vendor("BEAMR Framework")
    .with_url("https://github.com/helpermedia/beamr")
    .with_email("support@example.com")
    .with_version("1.0.0")
    .with_sub_categories("Fx|Dynamics");

// =============================================================================
// Parameter IDs
// =============================================================================

const PARAM_GAIN: u32 = 0;

// =============================================================================
// Parameters
// =============================================================================

/// Parameter collection for the gain plugin.
///
/// Uses `AtomicU64` for lock-free parameter storage, enabling safe access
/// from both the audio thread and UI/host threads.
pub struct GainParams {
    /// Gain value stored as atomic bits (normalized 0.0 to 1.0)
    gain: AtomicU64,
    /// Parameter metadata
    gain_info: ParamInfo,
}

impl GainParams {
    /// Create a new parameter collection with default values.
    pub fn new() -> Self {
        Self {
            // Default: 0.5 normalized = 0 dB (unity gain)
            gain: AtomicU64::new(0.5f64.to_bits()),
            gain_info: ParamInfo {
                id: PARAM_GAIN,
                name: "Gain",
                short_name: "Gain",
                units: "dB",
                default_normalized: 0.5,
                step_count: 0, // Continuous
                flags: ParamFlags {
                    can_automate: true,
                    is_readonly: false,
                    is_bypass: false,
                },
            },
        }
    }

    /// Get the gain as a linear multiplier (0.0 to 2.0).
    ///
    /// - normalized 0.0 → linear 0.0 (silence)
    /// - normalized 0.5 → linear 1.0 (unity gain, 0 dB)
    /// - normalized 1.0 → linear 2.0 (+6 dB)
    #[inline]
    pub fn gain_linear(&self) -> f32 {
        let normalized = f64::from_bits(self.gain.load(Ordering::Relaxed));
        (normalized * 2.0) as f32
    }
}

impl Default for GainParams {
    fn default() -> Self {
        Self::new()
    }
}

impl Parameters for GainParams {
    fn count(&self) -> usize {
        1
    }

    fn info(&self, index: usize) -> Option<&ParamInfo> {
        match index {
            0 => Some(&self.gain_info),
            _ => None,
        }
    }

    fn get_normalized(&self, id: u32) -> f64 {
        match id {
            PARAM_GAIN => f64::from_bits(self.gain.load(Ordering::Relaxed)),
            _ => 0.0,
        }
    }

    fn set_normalized(&self, id: u32, value: f64) {
        if id == PARAM_GAIN {
            self.gain
                .store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }

    fn normalized_to_string(&self, id: u32, normalized: f64) -> String {
        match id {
            PARAM_GAIN => {
                // Convert normalized (0-1) to dB display
                let linear = normalized * 2.0;
                let db = if linear < 0.0001 {
                    -100.0
                } else {
                    20.0 * linear.log10()
                };
                format!("{:.1} dB", db)
            }
            _ => String::new(),
        }
    }

    fn string_to_normalized(&self, id: u32, string: &str) -> Option<f64> {
        match id {
            PARAM_GAIN => {
                // Parse dB value to normalized
                let trimmed = string
                    .trim()
                    .trim_end_matches(" dB")
                    .trim_end_matches("dB")
                    .trim();
                let db: f64 = trimmed.parse().ok()?;
                let linear = 10.0f64.powf(db / 20.0);
                Some((linear / 2.0).clamp(0.0, 1.0))
            }
            _ => None,
        }
    }

    fn normalized_to_plain(&self, id: u32, normalized: f64) -> f64 {
        match id {
            PARAM_GAIN => normalized * 2.0, // 0-1 → 0-2 linear
            _ => 0.0,
        }
    }

    fn plain_to_normalized(&self, id: u32, plain: f64) -> f64 {
        match id {
            PARAM_GAIN => (plain / 2.0).clamp(0.0, 1.0), // 0-2 linear → 0-1
            _ => 0.0,
        }
    }
}

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

impl AudioProcessor for GainProcessor {
    fn setup(&mut self, _sample_rate: f64, _max_buffer_size: usize) {
        // No sample-rate dependent state for a simple gain plugin
    }

    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
        let gain = self.params.gain_linear();

        // Example: Access transport info from host
        // tempo is available when the DAW provides it (most do)
        let _tempo = context.transport.tempo.unwrap_or(120.0);
        let _is_playing = context.transport.is_playing;

        // You could use tempo for tempo-synced effects:
        // let samples_per_beat = context.samples_per_beat().unwrap_or(22050.0);
        // let delay_samples = samples_per_beat * 0.25; // 16th note delay

        // Calculate sidechain level for ducking (if sidechain is connected)
        // Using the new AuxInput::rms() helper for cleaner code
        let sidechain_level = aux
            .sidechain()
            .map(|sc| {
                // Average RMS across all sidechain channels
                let mut sum = 0.0f32;
                for ch in 0..sc.num_channels() {
                    sum += sc.rms(ch);
                }
                if sc.num_channels() > 0 {
                    sum / sc.num_channels() as f32
                } else {
                    0.0
                }
            })
            .unwrap_or(0.0);

        // Simple ducking: reduce gain when sidechain has signal
        // Ducking amount: 0 = no ducking, 1 = full ducking
        let duck_amount = (sidechain_level * 4.0).min(1.0);
        let effective_gain = gain * (1.0 - duck_amount * 0.8); // Max 80% reduction

        // Process using zip_channels() iterator for cleaner code
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * effective_gain;
            }
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

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        // State format: version byte + gain value as f64 bytes
        let mut data = Vec::with_capacity(9);
        data.push(1); // Version 1
        let gain = self.params.get_normalized(PARAM_GAIN);
        data.extend_from_slice(&gain.to_le_bytes());
        Ok(data)
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        if data.is_empty() {
            return Ok(());
        }

        // Check version
        let version = data[0];
        if version == 1 && data.len() >= 9 {
            let bytes: [u8; 8] = data[1..9].try_into().unwrap();
            let gain = f64::from_le_bytes(bytes);
            self.params.set_normalized(PARAM_GAIN, gain.clamp(0.0, 1.0));
        }
        // Unknown versions are silently ignored (forward compatibility)

        Ok(())
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

    fn create() -> Self {
        Self {
            params: GainParams::new(),
        }
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

// Export VST3 entry points using the generic wrapper
export_vst3!(CONFIG, Vst3Processor<GainProcessor>);
