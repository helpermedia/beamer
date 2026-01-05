//! Beamer Compressor - Example compressor plugin demonstrating the Beamer framework.
//!
//! This plugin shows how to:
//! 1. Use `BypassHandler` with `CrossfadeCurve::EqualPower` for smooth bypass
//! 2. Implement `bypass_ramp_samples()` to report ramp duration to host
//! 3. Implement `set_active()` to reset DSP state on plugin activation
//! 4. Use `PowerMapper` via `kind = "db_log"` for logarithmic-feel dB mapping
//! 5. Use linear smoothing (`smoothing = "linear:50.0"`)
//! 6. Access sidechain input for external key signal
//!
//! ## DSP Overview
//!
//! Classic feed-forward compressor with:
//! - Peak envelope follower (stereo-linked)
//! - Soft/hard knee selection (6 dB fixed soft knee width)
//! - Auto makeup gain calculation

use beamer::prelude::*;
use beamer::vst3_impl::vst3;
use beamer::{EnumParam, Params};

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Component UID - unique identifier for the plugin
const COMPONENT_UID: vst3::Steinberg::TUID =
    vst3::uid(0xB1C2D3E4, 0xF5061728, 0x394A5B6C, 0x7D8E9F00);

/// Static plugin configuration
pub static CONFIG: PluginConfig = PluginConfig::new("Beamer Compressor", COMPONENT_UID)
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version("0.1.3")
    .with_sub_categories("Fx|Dynamics");

// =============================================================================
// Compression Ratio Enum
// =============================================================================

/// Compression ratio selection.
///
/// Discrete ratio values for predictable compression behavior.
#[derive(Copy, Clone, PartialEq, EnumParam)]
pub enum Ratio {
    /// 2:1 - Gentle compression
    #[name = "2:1"]
    Ratio2,
    /// 4:1 - Moderate compression (default)
    #[default]
    #[name = "4:1"]
    Ratio4,
    /// 8:1 - Strong compression
    #[name = "8:1"]
    Ratio8,
    /// 10:1 - Heavy compression
    #[name = "10:1"]
    Ratio10,
    /// 20:1 - Near-limiting
    #[name = "20:1"]
    Ratio20,
}

impl Ratio {
    /// Convert ratio enum to numeric value for calculations.
    fn to_value(self) -> f64 {
        match self {
            Ratio::Ratio2 => 2.0,
            Ratio::Ratio4 => 4.0,
            Ratio::Ratio8 => 8.0,
            Ratio::Ratio10 => 10.0,
            Ratio::Ratio20 => 20.0,
        }
    }
}

// =============================================================================
// Parameters
// =============================================================================

/// Parameter collection for the compressor plugin.
///
/// Uses **declarative parameter definition** with the new `db_log` kind
/// for threshold (power curve mapping - more resolution near 0 dB).
#[derive(Params)]
pub struct CompressorParams {
    // =========================================================================
    // Compression Controls
    // =========================================================================

    /// Threshold level in dB.
    /// Uses `db_log` for more resolution near 0 dB (power curve mapping).
    #[param(
        id = "threshold",
        name = "Threshold",
        default = -20.0,
        range = -60.0..=0.0,
        kind = "db_log"
    )]
    pub threshold: FloatParam,

    /// Compression ratio (discrete steps).
    #[param(id = "ratio", name = "Ratio")]
    pub ratio: EnumParam<Ratio>,

    /// Attack time in milliseconds.
    /// Uses **linear smoothing** to test this currently untested feature.
    #[param(
        id = "attack",
        name = "Attack",
        default = 10.0,
        range = 0.1..=100.0,
        kind = "ms",
        smoothing = "linear:50.0"
    )]
    pub attack: FloatParam,

    /// Release time in milliseconds.
    /// Uses **linear smoothing** to test this currently untested feature.
    #[param(
        id = "release",
        name = "Release",
        default = 100.0,
        range = 10.0..=1000.0,
        kind = "ms",
        smoothing = "linear:50.0"
    )]
    pub release: FloatParam,

    /// Knee mode: soft (true) or hard (false).
    /// Soft knee uses 6 dB fixed width.
    #[param(id = "knee", name = "Soft Knee", default = true)]
    pub soft_knee: BoolParam,

    // =========================================================================
    // Gain Controls
    // =========================================================================

    /// Manual makeup gain in dB.
    #[param(
        id = "makeup",
        name = "Makeup Gain",
        default = 0.0,
        range = 0.0..=24.0,
        kind = "db"
    )]
    pub makeup_gain: FloatParam,

    /// Auto makeup gain toggle.
    /// When enabled, automatically compensates for gain reduction.
    #[param(id = "auto_makeup", name = "Auto Makeup", default = false)]
    pub auto_makeup: BoolParam,

    // =========================================================================
    // Bypass
    // =========================================================================

    /// Global bypass with smooth crossfade.
    #[param(id = "bypass", bypass)]
    pub bypass: BoolParam,
}

// =============================================================================
// DSP Helper Functions
// =============================================================================

/// Convert time in milliseconds to one-pole filter coefficient.
///
/// The coefficient determines how fast the envelope follower responds.
/// Larger time = smaller coefficient = slower response.
#[inline]
fn time_to_coeff(time_ms: f64, sample_rate: f64) -> f64 {
    if time_ms <= 0.0 {
        1.0 // Instant response
    } else {
        // One-pole time constant formula
        // After `time_ms`, reaches ~63% of target
        1.0 - (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
    }
}

/// Update envelope follower with attack/release behavior.
///
/// Uses attack coefficient when input is above envelope (signal rising),
/// release coefficient when input is below envelope (signal falling).
#[inline]
fn update_envelope(current: f64, input_level: f64, attack_coeff: f64, release_coeff: f64) -> f64 {
    let coeff = if input_level > current {
        attack_coeff
    } else {
        release_coeff
    };
    current + coeff * (input_level - current)
}

/// Compute gain reduction in dB using soft/hard knee.
///
/// # Arguments
/// * `input_db` - Input level in dB
/// * `threshold_db` - Threshold level in dB
/// * `ratio` - Compression ratio (e.g., 4.0 for 4:1)
/// * `knee_width_db` - Knee width in dB (0 for hard knee)
///
/// # Returns
/// Gain reduction amount in dB (always <= 0)
#[inline]
fn compute_gain_reduction(
    input_db: f64,
    threshold_db: f64,
    ratio: f64,
    knee_width_db: f64,
) -> f64 {
    let overshoot = input_db - threshold_db;

    if overshoot <= -knee_width_db / 2.0 {
        // Below knee - no compression
        0.0
    } else if knee_width_db <= 0.0 || overshoot >= knee_width_db / 2.0 {
        // Above knee (or hard knee) - full compression
        // Gain reduction = overshoot * (1 - 1/ratio)
        -overshoot * (1.0 - 1.0 / ratio)
    } else {
        // In soft knee region - quadratic interpolation
        let x = overshoot + knee_width_db / 2.0;
        -(x * x) / (2.0 * knee_width_db) * (1.0 - 1.0 / ratio)
    }
}

/// Convert linear amplitude to dB with floor.
#[inline]
fn linear_to_db(linear: f64) -> f64 {
    if linear <= 0.0 {
        -96.0 // Floor
    } else {
        20.0 * linear.log10()
    }
}

/// Convert dB to linear amplitude.
#[inline]
fn db_to_linear(db: f64) -> f64 {
    10.0_f64.powf(db / 20.0)
}

// =============================================================================
// Bypass Handler Constants
// =============================================================================

/// Bypass crossfade duration in milliseconds.
const BYPASS_RAMP_MS: f64 = 10.0;

/// Fixed soft knee width in dB.
const SOFT_KNEE_WIDTH_DB: f64 = 6.0;

// =============================================================================
// Audio Processor
// =============================================================================

/// The compressor plugin processor.
///
/// Implements a classic feed-forward compressor with:
/// - Peak-based envelope follower
/// - Stereo linking (max of L/R envelopes)
/// - Smooth bypass crossfade using `BypassHandler`
pub struct CompressorProcessor {
    /// Plugin parameters
    params: CompressorParams,

    /// Bypass handler for smooth crossfade transitions.
    /// Uses `CrossfadeCurve::EqualPower` for constant loudness.
    bypass_handler: BypassHandler,

    /// Compression state (envelope followers)
    state: CompressionState,

    /// Current sample rate
    sample_rate: f64,

    /// Cached bypass ramp samples (updated in setup)
    ramp_samples: u32,
}

/// Compression state (envelope followers).
struct CompressionState {
    envelope_l: f64,
    envelope_r: f64,
}

impl CompressorProcessor {
    /// Process compression on a buffer.
    fn process_compression<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        aux: &mut AuxiliaryBuffers<S>,
    ) {
        process_compression_inner(
            buffer,
            aux,
            &mut self.params,
            &mut self.state,
            self.sample_rate,
        );
    }
}

/// Inner compression processing function.
fn process_compression_inner<S: Sample>(
    buffer: &mut Buffer<S>,
    aux: &mut AuxiliaryBuffers<S>,
    params: &mut CompressorParams,
    state: &mut CompressionState,
    sample_rate: f64,
) {
    let num_samples = buffer.num_samples();
    let num_channels = buffer.num_output_channels().min(2);

    if num_channels == 0 || num_samples == 0 {
        return;
    }

    // Get parameter values
    let threshold_db = params.threshold.get();
    let ratio = params.ratio.get().to_value();
    let knee_width = if params.soft_knee.get() {
        SOFT_KNEE_WIDTH_DB
    } else {
        0.0
    };

    // Calculate auto makeup gain
    let auto_makeup_db = if params.auto_makeup.get() {
        // Formula: makeup_db = threshold_db * (1 - 1/ratio)
        // This compensates for the average gain reduction at threshold
        -threshold_db * (1.0 - 1.0 / ratio)
    } else {
        0.0
    };

    let manual_makeup_db = params.makeup_gain.get();
    let total_makeup_linear = db_to_linear(auto_makeup_db + manual_makeup_db);

    // Check if sidechain is connected
    let use_sidechain = aux.sidechain().is_some();

    // Pre-calculate envelope coefficients from smoothed attack/release values.
    // We use block-based smoothing (one value per buffer) rather than per-sample
    // since attack/release change slowly and coefficient calculation is expensive.
    let attack_ms = params.attack.smoothed();
    let release_ms = params.release.smoothed();
    let attack_coeff = time_to_coeff(attack_ms, sample_rate);
    let release_coeff = time_to_coeff(release_ms, sample_rate);

    // Process sample by sample
    for sample_idx in 0..num_samples {

        // Determine detection signal source
        let (detect_l, detect_r) = if use_sidechain {
            // Use sidechain input for detection
            if let Some(sc) = aux.sidechain() {
                let sc_l = if sc.num_channels() > 0 {
                    sc.channel(0)[sample_idx].to_f64().abs()
                } else {
                    0.0
                };
                let sc_r = if sc.num_channels() > 1 {
                    sc.channel(1)[sample_idx].to_f64().abs()
                } else {
                    sc_l
                };
                (sc_l, sc_r)
            } else {
                (0.0, 0.0)
            }
        } else {
            // Use main input for detection
            let in_l = buffer.input(0)[sample_idx].to_f64().abs();
            let in_r = if num_channels > 1 {
                buffer.input(1)[sample_idx].to_f64().abs()
            } else {
                in_l
            };
            (in_l, in_r)
        };

        // Update envelope followers
        state.envelope_l =
            update_envelope(state.envelope_l, detect_l, attack_coeff, release_coeff);
        state.envelope_r =
            update_envelope(state.envelope_r, detect_r, attack_coeff, release_coeff);

        // Stereo link: use max of both channels for consistent stereo image
        let envelope_max = state.envelope_l.max(state.envelope_r);
        let envelope_db = linear_to_db(envelope_max);

        // Calculate gain reduction
        let gain_reduction_db =
            compute_gain_reduction(envelope_db, threshold_db, ratio, knee_width);
        let gain_linear = db_to_linear(gain_reduction_db) * total_makeup_linear;

        // Apply gain to output
        let gain = S::from_f64(gain_linear);

        buffer.output(0)[sample_idx] = buffer.input(0)[sample_idx] * gain;
        if num_channels > 1 {
            buffer.output(1)[sample_idx] = buffer.input(1)[sample_idx] * gain;
        }
    }
}

impl AudioProcessor for CompressorProcessor {
    fn setup(&mut self, sample_rate: f64, _max_buffer_size: usize) {
        self.sample_rate = sample_rate;
        self.params.set_sample_rate(sample_rate);

        // Calculate bypass ramp samples based on sample rate
        self.ramp_samples = (sample_rate * BYPASS_RAMP_MS * 0.001) as u32;
        self.bypass_handler.set_ramp_samples(self.ramp_samples);
    }

    /// Called when plugin is activated/deactivated.
    ///
    /// Resets envelope followers to avoid clicks on activation.
    /// This tests the `set_active()` method which was previously untested.
    fn set_active(&mut self, active: bool) {
        if active {
            // Reset envelope state to avoid clicks
            self.state.envelope_l = 0.0;
            self.state.envelope_r = 0.0;
        }
    }

    /// Report bypass ramp duration to host.
    ///
    /// This tests the `bypass_ramp_samples()` method which was previously untested.
    fn bypass_ramp_samples(&self) -> u32 {
        self.bypass_handler.ramp_samples()
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        let is_bypassed = self.params.bypass.get();

        // Split API: begin() returns what action to take
        match self.bypass_handler.begin(is_bypassed) {
            BypassAction::Passthrough => {
                // Fully bypassed - just copy input to output
                buffer.copy_to_output();
            }
            BypassAction::Process => {
                // Normal processing - no crossfade needed
                self.process_compression(buffer, aux);
            }
            BypassAction::ProcessAndCrossfade => {
                // Process first, then apply crossfade
                self.process_compression(buffer, aux);
                self.bypass_handler.finish(buffer);
            }
        }
    }

    // =========================================================================
    // 64-bit Processing Support
    // =========================================================================

    fn supports_double_precision(&self) -> bool {
        true
    }

    fn process_f64(
        &mut self,
        buffer: &mut Buffer<f64>,
        aux: &mut AuxiliaryBuffers<f64>,
        _context: &ProcessContext,
    ) {
        let is_bypassed = self.params.bypass.get();

        // Split API: same pattern for f64 processing
        match self.bypass_handler.begin(is_bypassed) {
            BypassAction::Passthrough => {
                buffer.copy_to_output();
            }
            BypassAction::Process => {
                self.process_compression(buffer, aux);
            }
            BypassAction::ProcessAndCrossfade => {
                self.process_compression(buffer, aux);
                self.bypass_handler.finish(buffer);
            }
        }
    }

    // =========================================================================
    // Multi-Bus Configuration (Sidechain)
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

    // =========================================================================
    // State Persistence
    // =========================================================================

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(self.params.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        self.params.load_state(data).map_err(PluginError::StateError)
    }
}

// =============================================================================
// Plugin Trait Implementation
// =============================================================================

impl Plugin for CompressorProcessor {
    type Params = CompressorParams;

    fn params(&self) -> &Self::Params {
        &self.params
    }

    fn params_mut(&mut self) -> &mut Self::Params {
        &mut self.params
    }

    fn create() -> Self {
        Self {
            params: CompressorParams::default(),
            // Initialize with EqualPower curve for constant loudness during bypass
            bypass_handler: BypassHandler::new(64, CrossfadeCurve::EqualPower),
            state: CompressionState {
                envelope_l: 0.0,
                envelope_r: 0.0,
            },
            sample_rate: 44100.0,
            ramp_samples: 64,
        }
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

export_vst3!(CONFIG, Vst3Processor<CompressorProcessor>);
