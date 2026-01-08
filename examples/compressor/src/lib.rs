//! Beamer Compressor - Example compressor plugin demonstrating the Beamer framework.
//!
//! This plugin shows how to:
//! 1. Use `BypassHandler` with `CrossfadeCurve::EqualPower` for smooth bypass
//! 2. Implement `bypass_ramp_samples()` to report ramp duration to host
//! 3. Implement `set_active()` to reset DSP state on plugin activation
//! 4. Use `PowerMapper` via `kind = "db_log"` for logarithmic-feel dB mapping
//! 5. Use linear smoothing (`smoothing = "linear:50.0"`)
//! 6. Access sidechain input for external key signal
//! 7. Use `AudioSetup` config for sample-rate-dependent initialization
//!
//! ## DSP Overview
//!
//! Classic feed-forward compressor with dB-domain envelope processing:
//! - Envelope tracks overshoot above threshold
//! - Stereo-linked peak detection
//! - Soft/hard knee selection
//! - Dynamic auto makeup gain

use beamer::prelude::*;
use beamer::vst3_impl::vst3;
use beamer::{EnumParameter, HasParameters, Parameters};

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
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_sub_categories("Fx|Dynamics");

// =============================================================================
// Compression Ratio Enum
// =============================================================================

/// Compression ratio selection.
///
/// Discrete ratio values for predictable compression behavior.
#[derive(Copy, Clone, PartialEq, EnumParameter)]
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
#[derive(Parameters)]
pub struct CompressorParameters {
    // =========================================================================
    // Compression Controls
    // =========================================================================

    /// Threshold level in dB.
    #[parameter(
        id = "threshold",
        name = "Threshold",
        default = 0.0,
        range = -60.0..=0.0,
        kind = "db_log"
    )]
    pub threshold: FloatParameter,

    /// Compression ratio (discrete steps).
    #[parameter(id = "ratio", name = "Ratio")]
    pub ratio: EnumParameter<Ratio>,

    /// Attack time in milliseconds.
    #[parameter(
        id = "attack",
        name = "Attack",
        default = 10.0,
        range = 0.1..=200.0,
        kind = "ms",
        smoothing = "linear:50.0"
    )]
    pub attack: FloatParameter,

    /// Release time in milliseconds.
    #[parameter(
        id = "release",
        name = "Release",
        default = 100.0,
        range = 10.0..=2000.0,
        kind = "ms",
        smoothing = "linear:50.0"
    )]
    pub release: FloatParameter,

    /// Knee mode: soft (true) or hard (false).
    #[parameter(id = "knee", name = "Soft Knee", default = true)]
    pub soft_knee: BoolParameter,

    // =========================================================================
    // Gain Controls
    // =========================================================================

    /// Auto makeup gain toggle.
    #[parameter(id = "auto_makeup", name = "Auto Makeup", default = false)]
    pub auto_makeup: BoolParameter,

    /// Manual makeup gain in dB.
    #[parameter(
        id = "makeup",
        name = "Makeup Gain",
        default = 0.0,
        range = 0.0..=24.0,
        kind = "db"
    )]
    pub makeup_gain: FloatParameter,

    // =========================================================================
    // Bypass
    // =========================================================================

    /// Global bypass with smooth crossfade.
    #[parameter(id = "bypass", bypass)]
    pub bypass: BoolParameter,

    // =========================================================================
    // Sidechain
    // =========================================================================

    /// Use sidechain input for detection signal.
    #[parameter(id = "sidechain", name = "Sidechain", default = false)]
    pub use_sidechain: BoolParameter,
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

/// DC offset to prevent denormals in envelope follower.
const DC_OFFSET: f64 = 1e-25;

// =============================================================================
// Plugin (Unprepared State)
// =============================================================================

/// The compressor plugin in its unprepared state.
///
/// This struct holds the parameters before audio configuration is known.
/// When the host calls setupProcessing(), it is transformed into a
/// [`CompressorProcessor`] via the [`Plugin::prepare()`] method.
#[derive(Default, HasParameters)]
pub struct CompressorPlugin {
    /// Plugin parameters
    #[parameters]
    parameters: CompressorParameters,
}

impl Plugin for CompressorPlugin {
    type Config = AudioSetup; // Compressor needs sample rate for envelope coefficients
    type Processor = CompressorProcessor;

    fn prepare(mut self, config: AudioSetup) -> CompressorProcessor {
        // Set sample rate on parameters for smoothing calculations
        self.parameters.set_sample_rate(config.sample_rate);

        // Calculate bypass ramp samples based on sample rate
        let ramp_samples = (config.sample_rate * BYPASS_RAMP_MS * 0.001) as u32;

        CompressorProcessor {
            parameters: self.parameters,
            bypass_handler: BypassHandler::new(ramp_samples, CrossfadeCurve::EqualPower),
            state: CompressionState {
                env_db: DC_OFFSET,
                average_gr_db: 0.0,
            },
            sample_rate: config.sample_rate,
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
}

// =============================================================================
// Audio Processor (Prepared State)
// =============================================================================

/// The compressor processor, ready for audio processing.
///
/// This struct is created by [`CompressorPlugin::prepare()`] with valid
/// sample rate configuration. All DSP state is properly initialized.
#[derive(HasParameters)]
pub struct CompressorProcessor {
    /// Plugin parameters
    #[parameters]
    parameters: CompressorParameters,

    /// Bypass handler for smooth crossfade transitions
    bypass_handler: BypassHandler,

    /// Compression state
    state: CompressionState,

    /// Current sample rate (real value from start!)
    sample_rate: f64,
}

/// Compression state (envelope and gain reduction tracking).
///
/// Uses dB-domain envelope following:
/// - Envelope tracks overshoot above threshold
/// - Single stereo-linked envelope after max detection
/// - DC offset prevents denormals
struct CompressionState {
    /// Overshoot envelope in dB (with DC offset applied)
    env_db: f64,
    /// Smoothed average gain reduction in dB (for dynamic auto makeup)
    average_gr_db: f64,
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
            &mut self.parameters,
            &mut self.state,
            self.sample_rate,
        );
    }
}

/// Inner compression processing function.
///
/// Processing steps:
/// 1. Rectify input and stereo-link (max of L/R)
/// 2. Convert to dB and compute overshoot above threshold
/// 3. Run attack/release envelope on the overshoot
/// 4. Compute gain reduction from smoothed overshoot
fn process_compression_inner<S: Sample>(
    buffer: &mut Buffer<S>,
    aux: &mut AuxiliaryBuffers<S>,
    parameters: &mut CompressorParameters,
    state: &mut CompressionState,
    sample_rate: f64,
) {
    let num_samples = buffer.num_samples();
    let num_channels = buffer.num_output_channels().min(2);

    if num_channels == 0 || num_samples == 0 {
        return;
    }

    // Get parameter values
    let threshold_db = parameters.threshold.get();
    let ratio = parameters.ratio.get().to_value();
    let knee_width = if parameters.soft_knee.get() {
        SOFT_KNEE_WIDTH_DB
    } else {
        0.0
    };

    let manual_makeup_db = parameters.makeup_gain.get();

    // Only use sidechain when explicitly enabled by parameter and buffer exists
    let use_sidechain = parameters.use_sidechain.get() && aux.sidechain().is_some();

    // Pre-calculate envelope coefficients from smoothed attack/release values
    let attack_ms = parameters.attack.smoothed();
    let release_ms = parameters.release.smoothed();
    let attack_coeff = time_to_coeff(attack_ms, sample_rate);
    let release_coeff = time_to_coeff(release_ms, sample_rate);

    // Coefficient for smoothing average gain reduction (1 second time constant)
    let gr_smooth_coeff = time_to_coeff(1000.0, sample_rate);

    // Process sample by sample
    for sample_idx in 0..num_samples {
        // =====================================================================
        // Step 1: Get detection signal and stereo-link
        // =====================================================================
        let detect_linked = if use_sidechain {
            // Use sidechain input for detection
            if let Some(sc) = aux.sidechain() {
                let sc_l = sc.sample(0, sample_idx).to_f64().abs();
                let sc_r = if sc.num_channels() > 1 {
                    sc.sample(1, sample_idx).to_f64().abs()
                } else {
                    sc_l
                };
                sc_l.max(sc_r)
            } else {
                0.0
            }
        } else {
            // Use main input for detection
            let in_l = buffer.input(0)[sample_idx].to_f64().abs();
            let in_r = if num_channels > 1 {
                buffer.input(1)[sample_idx].to_f64().abs()
            } else {
                in_l
            };
            in_l.max(in_r) // Stereo link: use max of L/R
        };

        // =====================================================================
        // Step 2: Convert to dB and compute overshoot
        // =====================================================================
        // Add DC offset before log to avoid log(0)
        let key_db = linear_to_db(detect_linked + DC_OFFSET);

        // Compute overshoot above threshold (clamped to >= 0)
        let over_db = (key_db - threshold_db).max(0.0);

        // =====================================================================
        // Step 3: Run attack/release envelope on overshoot (dB domain)
        // =====================================================================
        // Add DC offset to prevent denormals
        let over_db_offset = over_db + DC_OFFSET;

        // Run one-pole envelope on the overshoot
        state.env_db = update_envelope(state.env_db, over_db_offset, attack_coeff, release_coeff);

        // Remove DC offset to get actual overshoot
        let smoothed_over_db = state.env_db - DC_OFFSET;

        // =====================================================================
        // Step 4: Compute gain reduction with soft/hard knee
        // =====================================================================
        let gain_reduction_db = if knee_width <= 0.0 || smoothed_over_db >= knee_width / 2.0 {
            // Hard knee or above knee region: full compression
            -smoothed_over_db * (1.0 - 1.0 / ratio)
        } else if smoothed_over_db <= 0.0 {
            // Below threshold: no compression
            0.0
        } else {
            // In soft knee region: quadratic interpolation
            -(smoothed_over_db * smoothed_over_db) / (knee_width / 2.0) * (1.0 - 1.0 / ratio)
        };

        // =====================================================================
        // Step 5: Auto makeup and final gain
        // =====================================================================
        // Update smoothed average gain reduction
        state.average_gr_db += gr_smooth_coeff * (gain_reduction_db - state.average_gr_db);

        let auto_makeup_db = if parameters.auto_makeup.get() {
            -state.average_gr_db
        } else {
            0.0
        };

        let total_gain_linear = db_to_linear(gain_reduction_db + auto_makeup_db + manual_makeup_db);

        // Apply gain to output
        let gain = S::from_f64(total_gain_linear);

        buffer.output(0)[sample_idx] = buffer.input(0)[sample_idx] * gain;
        if num_channels > 1 {
            buffer.output(1)[sample_idx] = buffer.input(1)[sample_idx] * gain;
        }
    }
}

impl AudioProcessor for CompressorProcessor {
    type Plugin = CompressorPlugin;

    fn unprepare(self) -> CompressorPlugin {
        // Return just the parameters; DSP state is discarded
        // It'll be reallocated on next prepare()
        CompressorPlugin {
            parameters: self.parameters,
        }
    }

    /// Called when plugin is activated/deactivated.
    fn set_active(&mut self, active: bool) {
        if active {
            self.state.env_db = DC_OFFSET;
            self.state.average_gr_db = 0.0;
        }
    }

    /// Report bypass ramp duration to host.
    fn bypass_ramp_samples(&self) -> u32 {
        self.bypass_handler.ramp_samples()
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        let is_bypassed = self.parameters.bypass.get();

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
        let is_bypassed = self.parameters.bypass.get();

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
    // State Persistence
    // =========================================================================

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(self.parameters.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        self.parameters.load_state(data).map_err(PluginError::StateError)
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

export_vst3!(CONFIG, Vst3Processor<CompressorPlugin>);
