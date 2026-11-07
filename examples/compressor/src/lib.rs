//! Beamer Compressor - Example compressor plugin demonstrating the Beamer framework.
//!
//! # Two-Struct Pattern
//!
//! Beamer plugins use two structs:
//!
//! 1. **`CompressorModel`** - What every thread shares, marked with
//!    `#[beamer::model]` and implementing `Model`: its parameters, how the
//!    host sees it and how it creates the processor.
//! 2. **`CompressorProcessor`** - What the audio thread owns, created by
//!    `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - `BypassHandler` with `CrossfadeCurve::Linear` for smooth bypass, its
//!   dry signal delayed by the lookahead and a warm-up after a bypass
//! - A lookahead parameter: `latency_samples()` follows it at runtime
//! - `bypass_ramp_samples()` to report ramp duration to host
//! - `set_active()` to reset DSP state on plugin activation
//! - `PowerMapper` via `kind = "db_log"` for logarithmic-feel dB mapping
//! - Linear smoothing (`smoothing = "linear:50.0"`)
//! - Sidechain input for external key signal, off until the host switches it
//!   on, with `ActiveBuses` telling `prepare()` whether it is on
//! - `Model::LAYOUTS` with mono and stereo main buses and sidechains
//! - `SampleRate` setup for sample-rate-dependent initialization
//! - `context.report_value()` for the gain reduction in a diagnostics build
//!
//! # DSP Overview
//!
//! Classic feed-forward compressor with dB-domain envelope processing:
//! - Envelope tracks overshoot above the start of the knee
//! - Stereo-linked peak detection
//! - A soft knee centered on the threshold or a hard knee
//! - Dynamic auto makeup gain
//! - Optional lookahead: the audio is delayed, the detector is not

use beamer::prelude::*;

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
// Model
// =============================================================================

/// Compressor plugin model.
///
/// All configuration is in attributes. The `#[beamer::model]` attribute
/// generates everything including the `Default` implementation.
#[beamer::export]
#[beamer::model]
pub struct CompressorModel {
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

    /// Lookahead in milliseconds: the audio is delayed by it, so that the
    /// envelope reacts before a peak reaches the output. It is the
    /// plugin's latency.
    #[parameter(
        id = "lookahead",
        name = "Lookahead",
        default = 0.0,
        range = 0.0..=10.0,
        kind = "ms"
    )]
    pub lookahead: FloatParameter,

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

impl Model for CompressorModel {
    // Compressor needs sample rate for envelope coefficients and the
    // active buses for its sidechain. See `beamer::setup` for all available
    // types.
    type Setup = (SampleRate, MaxBufferSize, MainInputChannels, ActiveBuses);
    type Processor = CompressorProcessor;
    // Stereo in and out with a stereo sidechain as input bus 1 by default.
    // The main buses and the sidechain can each be mono as well; the
    // detector links the channels it gets.
    const LAYOUTS: &'static [Layout] = &[
        Layout {
            inputs: &[BusLayout::STEREO, BusLayout::STEREO],
            outputs: &[BusLayout::STEREO],
        },
        Layout {
            inputs: &[BusLayout::STEREO, BusLayout::MONO],
            outputs: &[BusLayout::STEREO],
        },
        Layout {
            inputs: &[BusLayout::MONO, BusLayout::MONO],
            outputs: &[BusLayout::MONO],
        },
        Layout {
            inputs: &[BusLayout::MONO, BusLayout::STEREO],
            outputs: &[BusLayout::MONO],
        },
    ];

    fn prepare(
        self: Arc<Self>,
        (sample_rate, max_buffer_size, input_channels, buses): (
            SampleRate,
            MaxBufferSize,
            MainInputChannels,
            ActiveBuses,
        ),
    ) -> CompressorProcessor {
        // Calculate bypass ramp samples based on sample rate
        let ramp_samples = (sample_rate.hz() * BYPASS_RAMP_MS * 0.001) as u32;
        // The longest lookahead is the largest latency.
        let max_lookahead_samples = ms_to_samples(MAX_LOOKAHEAD_MS, sample_rate.hz());

        let mut processor = CompressorProcessor {
            // The handler keeps the dry signal, so it needs the channel
            // count, the longest block and the largest latency.
            bypass_handler: BypassHandler::new(
                ramp_samples,
                CrossfadeCurve::Linear,
                input_channels.0 as usize,
                max_buffer_size.0,
                max_lookahead_samples as u32,
            ),
            state: CompressionState {
                env_db: DC_OFFSET,
                average_gr_db: 0.0,
            },
            lookahead: Lookahead::new(max_lookahead_samples),
            lookahead_samples: 0,
            max_lookahead_samples,
            sample_rate: sample_rate.hz(),
            // Input bus 1 is the sidechain. A host switches it on when the
            // user routes a key signal to it.
            has_sidechain: buses.is_input_on(1),
            model: self,
        };
        // The latency a host asks for before the first block.
        processor.update_timing();
        processor
    }

    // =========================================================================
    // Multi-Bus Configuration (Sidechain)
    // =========================================================================

    fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
        match index {
            0 => Some(BusInfo::main("Input")),
            // The sidechain, off until the host switches it on.
            1 => Some(BusInfo::aux("Sidechain")),
            _ => None,
        }
    }
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

/// Convert a time in milliseconds to samples.
#[inline]
fn ms_to_samples(time_ms: f64, sample_rate: f64) -> usize {
    (time_ms * 0.001 * sample_rate).round() as usize
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

/// The gain reduction in dB (0 or negative) for a level `over_db` above
/// the start of the knee.
///
/// Nothing below the knee, then a quadratic knee centered on the threshold
/// and `knee_db` wide, then the full ratio. A hard knee is 0 dB wide. The
/// knee meets the ratio without a step in level or slope, so the output
/// rises with the input everywhere.
#[inline]
fn gain_reduction_db(over_db: f64, ratio: f64, knee_db: f64) -> f64 {
    let slope = 1.0 - 1.0 / ratio;
    if over_db <= 0.0 {
        0.0
    } else if over_db < knee_db {
        -slope * over_db * over_db / (2.0 * knee_db)
    } else {
        -slope * (over_db - knee_db / 2.0)
    }
}

// =============================================================================
// Bypass Handler Constants
// =============================================================================

/// Bypass crossfade duration in milliseconds.
const BYPASS_RAMP_MS: f64 = 10.0;

/// The longest lookahead in milliseconds, the top of the parameter's
/// range.
const MAX_LOOKAHEAD_MS: f64 = 10.0;

/// The warm-up after a bypass in attack times: three bring the envelope
/// within 5 percent of where it settles.
const WARMUP_ATTACK_TIMES: f64 = 3.0;

/// Fixed soft knee width in dB, centered on the threshold.
const SOFT_KNEE_WIDTH_DB: f64 = 6.0;

/// DC offset to prevent denormals in envelope follower.
const DC_OFFSET: f64 = 1e-25;

// =============================================================================
// Processor
// =============================================================================

/// Compressor plugin processor.
///
/// Ready for audio processing. Created by `CompressorModel::prepare()`
/// with valid sample rate configuration. All DSP state is properly initialized.
pub struct CompressorProcessor {
    /// The plugin's model, shared with the host and the UI.
    model: Arc<CompressorModel>,

    /// Bypass handler for smooth crossfade transitions
    bypass_handler: BypassHandler,

    /// Compression state
    state: CompressionState,

    /// The delay of the audio for the lookahead.
    lookahead: Lookahead,

    /// The lookahead of the current block in samples: the latency.
    lookahead_samples: usize,

    /// The longest lookahead in samples, which the delay holds.
    max_lookahead_samples: usize,

    /// Current sample rate (real value from start!)
    sample_rate: f64,

    /// Whether the plugin was prepared with the sidechain on. Without it
    /// the detector keys on the main input.
    has_sidechain: bool,
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

/// The delay of the audio for the lookahead: one line per processed
/// channel (at most two), long enough for the longest lookahead.
struct Lookahead {
    /// The recent input of each channel, as a ring.
    lines: [Vec<f64>; 2],
    /// Where the next sample goes.
    write: usize,
}

impl Lookahead {
    fn new(max_samples: usize) -> Self {
        Self {
            lines: [vec![0.0; max_samples + 1], vec![0.0; max_samples + 1]],
            write: 0,
        }
    }

    /// Clear the delay, as on activation.
    fn clear(&mut self) {
        for line in &mut self.lines {
            line.fill(0.0);
        }
        self.write = 0;
    }

    /// Put `input` into the line of `channel` and return the sample
    /// `delay` samples before it. Call [`advance()`](Self::advance) once
    /// all channels of a sample are done.
    #[inline]
    fn delay(&mut self, channel: usize, input: f64, delay: usize) -> f64 {
        let line = &mut self.lines[channel];
        let len = line.len();
        line[self.write] = input;
        line[(self.write + len - delay) % len]
    }

    /// Move on to the next sample.
    #[inline]
    fn advance(&mut self) {
        self.write = (self.write + 1) % self.lines[0].len();
    }
}

/// What a block of compression did, for the diagnostics log.
struct CompressionReport {
    /// The largest gain reduction of the block in dB, as a positive number.
    largest_reduction_db: f64,
    /// Whether the detector keyed on the sidechain.
    keyed_on_sidechain: bool,
}

impl CompressorProcessor {
    /// Start the envelope and the lookahead from silence, as on
    /// activation.
    fn clear_dsp(&mut self) {
        self.state.env_db = DC_OFFSET;
        self.state.average_gr_db = 0.0;
        self.lookahead.clear();
    }

    /// Take the lookahead of this block (the latency) and set the
    /// warm-up after a bypass: the lookahead plus three times the attack
    /// (its target value), so that the cleared envelope settles while the
    /// dry signal plays.
    fn update_timing(&mut self) {
        self.lookahead_samples = ms_to_samples(self.model.lookahead.get(), self.sample_rate)
            .min(self.max_lookahead_samples);
        let attack_samples = ms_to_samples(self.model.attack.get() * WARMUP_ATTACK_TIMES, self.sample_rate);
        self.bypass_handler
            .set_warmup_samples((self.lookahead_samples + attack_samples) as u32);
    }

    /// Process compression on a buffer and report what it did to a
    /// diagnostics build (`cargo xtask bundle compressor --diagnostics`).
    /// Other builds ignore the reports.
    fn process_compression<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        aux: &mut AuxiliaryBuffers<S>,
        context: &ProcessContext,
    ) {
        let report = process_compression_inner(
            buffer,
            aux,
            &self.model,
            &mut self.state,
            &mut self.lookahead,
            self.lookahead_samples,
            self.sample_rate,
            self.has_sidechain,
        );
        context.report_value("gain reduction dB", report.largest_reduction_db as f32);
        context.report_value("keyed on sidechain", if report.keyed_on_sidechain { 1.0 } else { 0.0 });
    }
}

/// Inner compression processing function.
///
/// Processing steps:
/// 1. Rectify input and stereo-link (max of L/R)
/// 2. Convert to dB and compute overshoot above the start of the knee
/// 3. Run attack/release envelope on the overshoot
/// 4. Compute gain reduction from smoothed overshoot
/// 5. Apply it to the audio delayed by the lookahead
#[allow(clippy::too_many_arguments)]
fn process_compression_inner<S: Sample>(
    buffer: &mut Buffer<S>,
    aux: &mut AuxiliaryBuffers<S>,
    model: &CompressorModel,
    state: &mut CompressionState,
    lookahead: &mut Lookahead,
    lookahead_samples: usize,
    sample_rate: f64,
    has_sidechain: bool,
) -> CompressionReport {
    let num_samples = buffer.num_samples();
    let num_channels = buffer.num_channels().min(2);
    let mut report = CompressionReport { largest_reduction_db: 0.0, keyed_on_sidechain: false };

    if num_channels == 0 || num_samples == 0 {
        return report;
    }

    // Get parameter values
    let threshold_db = model.threshold.get();
    let ratio = model.ratio.get().to_value();
    let knee_db = if model.soft_knee.get() {
        SOFT_KNEE_WIDTH_DB
    } else {
        0.0
    };
    // The knee is centered on the threshold: compression starts half its
    // width below it.
    let knee_start_db = threshold_db - knee_db / 2.0;

    let manual_makeup_db = model.makeup_gain.get();

    // Only use the sidechain when the parameter asks for it and the host
    // switched it on. An Audio Unit sidechain without a source has no
    // samples for a call, so the buffer is checked too.
    let use_sidechain = model.use_sidechain.get() && has_sidechain && aux.sidechain().is_some();
    report.keyed_on_sidechain = use_sidechain;

    // Pre-calculate envelope coefficients from smoothed attack/release values.
    // The times are read once per block, so the smoothers advance by a whole
    // block here.
    let attack_ms = model.attack.smoothed();
    let release_ms = model.release.smoothed();
    model.attack.skip_smoothing(num_samples);
    model.release.skip_smoothing(num_samples);
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
            let in_l = buffer.channel(0)[sample_idx].to_f64().abs();
            let in_r = if num_channels > 1 {
                buffer.channel(1)[sample_idx].to_f64().abs()
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

        // Compute overshoot above the start of the knee (clamped to >= 0).
        let over_db = (key_db - knee_start_db).max(0.0);

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
        let reduction_db = gain_reduction_db(smoothed_over_db, ratio, knee_db);

        // =====================================================================
        // Step 5: Auto makeup and final gain
        // =====================================================================
        // Update smoothed average gain reduction
        state.average_gr_db += gr_smooth_coeff * (reduction_db - state.average_gr_db);
        report.largest_reduction_db = report.largest_reduction_db.max(-reduction_db);

        let auto_makeup_db = if model.auto_makeup.get() {
            -state.average_gr_db
        } else {
            0.0
        };

        let total_gain_linear = db_to_linear(reduction_db + auto_makeup_db + manual_makeup_db);

        // Apply the gain to the audio delayed by the lookahead. The buffer
        // is processed in place: this sample still holds the input.
        for channel in 0..num_channels {
            let sample = &mut buffer.channel(channel)[sample_idx];
            let delayed = lookahead.delay(channel, sample.to_f64(), lookahead_samples);
            *sample = S::from_f64(delayed * total_gain_linear);
        }
        lookahead.advance();
    }

    report
}

impl Processor for CompressorProcessor {
    /// Called when plugin is activated/deactivated.
    fn set_active(&mut self, active: bool) {
        if active {
            self.clear_dsp();
            self.bypass_handler.reset();
            // The latency a host asks for after activating.
            self.update_timing();
        }
    }

    /// The lookahead is the latency.
    fn latency_samples(&self) -> u32 {
        self.lookahead_samples as u32
    }

    /// The longest lookahead.
    fn max_latency_samples(&self) -> u32 {
        self.max_lookahead_samples as u32
    }

    /// Report bypass ramp duration to host.
    fn bypass_ramp_samples(&self) -> u32 {
        self.bypass_handler.ramp_samples()
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &ProcessContext,
    ) {
        let is_bypassed = self.model.bypass.get();
        self.update_timing();
        let latency = self.latency_samples();

        // Split API: begin() returns what action to take
        match self.bypass_handler.begin(is_bypassed, latency, buffer) {
            BypassAction::Passthrough => {
                // Fully bypassed: the buffer holds the input, delayed by the latency.
            }
            BypassAction::Process => {
                // Normal processing - no crossfade needed
                self.process_compression(buffer, aux, context);
            }
            BypassAction::ProcessAndCrossfade => {
                // Process first, then apply crossfade
                self.process_compression(buffer, aux, context);
                self.bypass_handler.finish(buffer);
            }
            BypassAction::Resume => {
                // The envelope and the lookahead did not follow the input
                // while bypassed: start them clean, then process. The
                // handler holds the dry signal for the warm-up.
                self.clear_dsp();
                self.process_compression(buffer, aux, context);
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
        context: &ProcessContext,
    ) {
        let is_bypassed = self.model.bypass.get();
        self.update_timing();
        let latency = self.latency_samples();

        // Split API: same pattern for f64 processing
        match self.bypass_handler.begin(is_bypassed, latency, buffer) {
            BypassAction::Passthrough => {}
            BypassAction::Process => {
                self.process_compression(buffer, aux, context);
            }
            BypassAction::ProcessAndCrossfade => {
                self.process_compression(buffer, aux, context);
                self.bypass_handler.finish(buffer);
            }
            BypassAction::Resume => {
                self.clear_dsp();
                self.process_compression(buffer, aux, context);
                self.bypass_handler.finish(buffer);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamer::allocation_check;

    const SAMPLE_RATE: f64 = 48_000.0;
    const BLOCK: usize = 64;

    fn prepare(model: &Arc<CompressorModel>) -> CompressorProcessor {
        let buses = ActiveBuses::default_for(&**model);
        Arc::clone(model).prepare((SampleRate(SAMPLE_RATE), MaxBufferSize(BLOCK), MainInputChannels(2), buses))
    }

    /// Process one stereo block of `input` in place.
    fn process(processor: &mut CompressorProcessor, left: &mut [f32], right: &mut [f32]) {
        let mut buffer = Buffer::new([left, right], std::iter::empty(), 2, BLOCK);
        let mut aux = AuxiliaryBuffers::empty();
        let context = ProcessContext::with_empty_transport(SAMPLE_RATE, BLOCK);
        processor.process(&mut buffer, &mut aux, &context);
    }

    #[test]
    fn test_bypass_keeps_the_audio_delayed_by_the_lookahead() {
        // A quiet input below the threshold: the compressor is a delay of
        // its lookahead, active and bypassed. Bypassed for blocks 10 to 19,
        // then back through the warm-up and the ramp.
        let model = Arc::new(CompressorModel::default());
        model.lookahead.set(5.0);
        let mut processor = prepare(&model);
        let latency = processor.latency_samples() as usize;
        assert_eq!(latency, 240);

        // Long enough for the warm-up (240 + 3 x 480) and the ramp (480).
        let input: Vec<f32> = (0..BLOCK * 64).map(|n| 0.001 * (n % 50) as f32).collect();
        let mut output = Vec::new();
        for (index, block) in input.chunks(BLOCK).enumerate() {
            model.bypass.set((10..20).contains(&index));
            let mut left = block.to_vec();
            let mut right = block.to_vec();
            process(&mut processor, &mut left, &mut right);
            assert_eq!(left, right);
            output.extend_from_slice(&left);
        }
        for (index, sample) in output.iter().enumerate() {
            let expected = if index < latency { 0.0 } else { input[index - latency] };
            assert!((sample - expected).abs() < 1e-6, "sample {index}: {sample} is not {expected}");
        }
        assert_eq!(processor.bypass_handler.state(), BypassState::Active);
    }

    #[test]
    fn test_warmup_lets_the_envelope_settle_before_the_ramp() {
        // A steady input 14 dB above the threshold. After a bypass the
        // dry signal holds while the cleared envelope attacks again, so
        // the output after the ramp is as compressed as before the bypass.
        let model = Arc::new(CompressorModel::default());
        model.threshold.set(-20.0);
        model.lookahead.set(5.0);
        let mut processor = prepare(&model);
        let mut output = Vec::new();
        for index in 0..140 {
            model.bypass.set((60..70).contains(&index));
            let mut left = [0.5f32; BLOCK];
            let mut right = [0.5f32; BLOCK];
            process(&mut processor, &mut left, &mut right);
            output.extend_from_slice(&left);
        }
        let compressed = output[60 * BLOCK - 1];
        assert!(compressed < 0.2, "{compressed}");
        // Lookahead 240, warm-up 240 + 3 x 480, ramp 480 from block 70 on.
        let settled = 70 * BLOCK + 240 + 1440 + 480 + 1;
        for (index, sample) in output.iter().enumerate().skip(settled) {
            assert!((sample / compressed - 1.0).abs() < 0.02, "sample {index}: {sample}, compressed {compressed}");
        }
    }

    #[test]
    fn test_lookahead_change_while_playing_moves_the_delay() {
        // A quiet input: no compression, so the output is the input
        // delayed by the lookahead, 0 for 10 blocks, then the full 10 ms.
        let model = Arc::new(CompressorModel::default());
        let mut processor = prepare(&model);
        let input: Vec<f32> = (0..BLOCK * 30).map(|n| 0.001 * (n % 50) as f32).collect();
        let mut output = Vec::new();
        for (index, block) in input.chunks(BLOCK).enumerate() {
            model.lookahead.set(if index < 10 { 0.0 } else { 10.0 });
            let mut left = block.to_vec();
            let mut right = block.to_vec();
            process(&mut processor, &mut left, &mut right);
            output.extend_from_slice(&left);
        }
        assert_eq!(processor.latency_samples(), 480);
        assert_eq!(output[..10 * BLOCK], input[..10 * BLOCK]);
        for index in 10 * BLOCK..output.len() {
            let expected = if index < 480 { 0.0 } else { input[index - 480] };
            assert!((output[index] - expected).abs() < 1e-6, "sample {index}: {} is not {expected}", output[index]);
        }
    }

    #[test]
    fn test_latency_follows_the_lookahead() {
        let model = Arc::new(CompressorModel::default());
        let mut processor = prepare(&model);
        assert_eq!(processor.latency_samples(), 0);
        model.lookahead.set(2.0);
        process(&mut processor, &mut [0.0; BLOCK], &mut [0.0; BLOCK]);
        assert_eq!(processor.latency_samples(), 96);
        // Above the range the parameter keeps its top, 10 ms.
        model.lookahead.set(20.0);
        process(&mut processor, &mut [0.0; BLOCK], &mut [0.0; BLOCK]);
        assert_eq!(processor.latency_samples(), 480);
    }

    #[test]
    fn test_soft_knee_output_rises_with_the_input() {
        // From 10 dB below the start of the knee to 10 dB above its end,
        // for every ratio.
        for ratio in [2.0, 4.0, 8.0, 10.0, 20.0] {
            let mut last = f64::NEG_INFINITY;
            for step in 0..=260 {
                let over_db = -10.0 + step as f64 * 0.1;
                let output = over_db + gain_reduction_db(over_db, ratio, SOFT_KNEE_WIDTH_DB);
                assert!(output > last, "ratio {ratio}, {over_db} dB over the knee's start: {output} after {last}");
                last = output;
            }
        }
    }

    #[test]
    fn test_soft_knee_is_centered_on_the_threshold() {
        let ratio = 4.0;
        let knee = SOFT_KNEE_WIDTH_DB;
        let slope = 1.0 - 1.0 / ratio;
        // Nothing at the start of the knee, half its width below the
        // threshold.
        assert_eq!(gain_reduction_db(0.0, ratio, knee), 0.0);
        // At the threshold an eighth of the knee's width times the slope.
        let at_threshold = gain_reduction_db(knee / 2.0, ratio, knee);
        assert!((at_threshold + slope * knee / 8.0).abs() < 1e-12, "{at_threshold}");
        // Above the knee the hard knee's curve: the full ratio from the
        // threshold.
        for over_threshold in [3.0, 6.0, 20.0] {
            let soft = gain_reduction_db(over_threshold + knee / 2.0, ratio, knee);
            let hard = gain_reduction_db(over_threshold, ratio, 0.0);
            assert!((soft - hard).abs() < 1e-12, "{over_threshold} dB over the threshold: {soft} is not {hard}");
        }
    }

    #[test]
    fn test_process_neither_allocates_nor_frees() {
        let model = Arc::new(CompressorModel::default());
        model.lookahead.set(5.0);
        let mut processor = prepare(&model);
        let mut left = [0.5f32; BLOCK];
        let mut right = [0.5f32; BLOCK];
        let (_, counts) = allocation_check::count_in_process(|| {
            process(&mut processor, &mut left, &mut right);
        });
        // The check runs in debug builds with the `allocation-check` feature.
        if allocation_check::is_active() {
            assert!(counts.is_empty(), "{counts:?}");
        }
    }
}
