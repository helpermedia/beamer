//! Beamer Delay - Example tempo-synced delay plugin demonstrating the Beamer framework.
//!
//! This plugin shows how to:
//! 1. Use `EnumParam` for sync mode (quarter, eighth, 16th, 32nd, free)
//! 2. Use `#[derive(HasParams)]` to eliminate params() boilerplate
//! 3. Use tempo information from `ProcessContext` for tempo-synced delays
//! 4. Implement a ring buffer delay line
//! 5. Apply parameter smoothing to avoid zipper noise
//! 6. Support both simple stereo and ping-pong modes
//! 7. Declare proper tail length for delay decay
//! 8. Use `AudioSetup` config for sample-rate-dependent initialization
//! 9. Implement `reset()` to clear internal state on playback restart

use beamer::prelude::*;
use beamer::vst3_impl::vst3;
use beamer::{EnumParam, HasParams, Params};

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Component UID - unique identifier for the plugin
const COMPONENT_UID: vst3::Steinberg::TUID =
    vst3::uid(0xA7B8C9D0, 0xE1F2A3B4, 0xC5D6E7F8, 0x09101112);

/// Static plugin configuration
pub static CONFIG: PluginConfig = PluginConfig::new("Beamer Delay", COMPONENT_UID)
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_sub_categories("Fx|Delay");

// =============================================================================
// Enum Types for Parameter Choices
// =============================================================================

/// Delay sync mode - determines how delay time is calculated.
#[derive(Copy, Clone, PartialEq, EnumParam)]
pub enum SyncMode {
    /// Free-running delay using time in milliseconds
    #[default]
    #[name = "Free"]
    Free,
    /// Quarter note (1/4)
    #[name = "1/4"]
    Quarter,
    /// Eighth note (1/8)
    #[name = "1/8"]
    Eighth,
    /// Sixteenth note (1/16)
    #[name = "1/16"]
    Sixteenth,
    /// Thirty-second note (1/32)
    #[name = "1/32"]
    ThirtySecond,
}

/// Stereo mode - determines how delay is applied to stereo channels.
#[derive(Copy, Clone, PartialEq, EnumParam)]
pub enum StereoMode {
    /// Same delay time on both channels
    #[default]
    #[name = "Stereo"]
    Stereo,
    /// Ping-pong: alternates between left and right channels
    #[name = "Ping-Pong"]
    PingPong,
}

// =============================================================================
// Parameters
// =============================================================================

/// Parameter collection for the delay plugin.
///
/// Uses **declarative parameter definition**: all configuration is in
/// attributes, and the `#[derive(Params)]` macro generates everything
/// including the `Default` implementation!
#[derive(Params)]
pub struct DelayParams {
    /// Sync mode selection (Free, 1/4, 1/8, 1/16, 1/32)
    #[param(id = "sync_mode", name = "Sync Mode")]
    pub sync_mode: EnumParam<SyncMode>,

    /// Stereo mode selection (Stereo, Ping-Pong)
    #[param(id = "stereo_mode", name = "Stereo Mode")]
    pub stereo_mode: EnumParam<StereoMode>,

    /// Delay time in milliseconds (only used when Sync Mode = Free)
    #[param(id = "time", name = "Time", default = 250.0, range = 1.0..=2000.0, kind = "ms")]
    pub time_ms: FloatParam,

    /// Feedback amount (0% to 100%) - smoothed to avoid zipper noise
    #[param(id = "feedback", name = "Feedback", default = 0.4, range = 0.0..=1.0, kind = "percent", smoothing = "exp:5.0")]
    pub feedback: FloatParam,

    /// Wet/dry mix (0% = dry, 100% = wet) - smoothed to avoid zipper noise
    #[param(id = "mix", name = "Mix", default = 0.5, range = 0.0..=1.0, kind = "percent", smoothing = "exp:5.0")]
    pub mix: FloatParam,
}

// =============================================================================
// Delay Buffer
// =============================================================================

/// Maximum delay time in seconds (covers slow tempos and 2000ms free time)
const MAX_DELAY_SECONDS: f64 = 2.5;

/// Ring buffer delay line implementation.
///
/// A ring buffer (circular buffer) is a fixed-size buffer that wraps around,
/// allowing efficient FIFO (first-in-first-out) operations without memory allocation.
///
/// ```text
/// Buffer layout (max_samples = 8, delay = 3):
///
///   write_pos
///      ↓
/// [4] [5] [6] [7] [0] [1] [2] [3]
///                  ↑
///               read_pos (write_pos - delay + max_samples) % max_samples
/// ```
///
/// Uses f64 internally for maximum precision, converts to/from
/// the processing sample type as needed.
struct DelayLine {
    buffer: Vec<f64>,
    write_pos: usize,
    max_samples: usize,
}

impl DelayLine {
    /// Create and allocate a delay line for the given sample rate.
    ///
    /// Buffer size is calculated as: `MAX_DELAY_SECONDS * sample_rate`
    fn new(sample_rate: f64) -> Self {
        let max_samples = (MAX_DELAY_SECONDS * sample_rate) as usize;
        Self {
            buffer: vec![0.0; max_samples],
            write_pos: 0,
            max_samples,
        }
    }

    /// Read from the delay line at the given delay in samples.
    ///
    /// Calculates read position using modular arithmetic:
    /// ```text
    /// read_pos = (write_pos + max_samples - delay) % max_samples
    /// ```
    fn read(&self, delay_samples: usize) -> f64 {
        if self.max_samples == 0 {
            return 0.0;
        }
        let delay_clamped = delay_samples.min(self.max_samples - 1);
        let read_pos = (self.write_pos + self.max_samples - delay_clamped) % self.max_samples;
        self.buffer[read_pos]
    }

    /// Write to the delay line and advance the write pointer.
    ///
    /// After writing, the write pointer advances by 1 and wraps around
    /// when it reaches the end of the buffer.
    fn write(&mut self, sample: f64) {
        if self.max_samples == 0 {
            return;
        }
        self.buffer[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) % self.max_samples;
    }

    /// Clear the buffer (e.g., on parameter changes that could cause feedback issues)
    #[allow(dead_code)]
    fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}

// =============================================================================
// Plugin (Unprepared State)
// =============================================================================

/// The delay plugin in its unprepared state.
///
/// This struct holds the parameters before audio configuration is known.
/// When the host calls setupProcessing(), it is transformed into a
/// [`DelayProcessor`] via the [`Plugin::prepare()`] method.
#[derive(Default, HasParams)]
pub struct DelayPlugin {
    /// Plugin parameters
    #[params]
    params: DelayParams,
}

impl Plugin for DelayPlugin {
    type Config = AudioSetup; // Delay needs sample rate for buffer allocation
    type Processor = DelayProcessor;

    fn prepare(mut self, config: AudioSetup) -> DelayProcessor {
        // Set sample rate on params for smoothing calculations
        self.params.set_sample_rate(config.sample_rate);

        DelayProcessor {
            params: self.params,
            delay_l: DelayLine::new(config.sample_rate),
            delay_r: DelayLine::new(config.sample_rate),
            sample_rate: config.sample_rate,
        }
    }
}

// =============================================================================
// Audio Processor (Prepared State)
// =============================================================================

/// The delay plugin processor, ready for audio processing.
///
/// This struct is created by [`DelayPlugin::prepare()`] with valid
/// sample rate configuration. All fields have real values from the start.
#[derive(HasParams)]
pub struct DelayProcessor {
    /// Plugin parameters
    #[params]
    params: DelayParams,
    /// Left channel delay line (allocated for current sample rate)
    delay_l: DelayLine,
    /// Right channel delay line (allocated for current sample rate)
    delay_r: DelayLine,
    /// Current sample rate
    sample_rate: f64,
}

impl DelayProcessor {
    /// Calculate delay time in samples based on sync mode and tempo.
    ///
    /// # Tempo Sync Calculation
    ///
    /// When synced to tempo, delay time is derived from the host's BPM:
    ///
    /// ```text
    /// samples_per_beat = sample_rate * 60 / tempo
    ///
    /// Note Division | Multiplier | At 120 BPM, 44.1kHz
    /// --------------|------------|--------------------
    /// 1/4 (quarter) | 1.0        | 22050 samples (500ms)
    /// 1/8 (eighth)  | 0.5        | 11025 samples (250ms)
    /// 1/16          | 0.25       | 5512 samples (125ms)
    /// 1/32          | 0.125      | 2756 samples (62.5ms)
    /// ```
    ///
    /// # Free Mode
    ///
    /// In free mode, delay time is simply:
    /// ```text
    /// delay_samples = time_ms / 1000 * sample_rate
    /// ```
    fn calculate_delay_samples(&self, context: &ProcessContext) -> usize {
        // samples_per_beat() returns samples directly (sample_rate * 60 / tempo)
        // Default fallback: 22050 samples = 500ms at 44.1kHz (120 BPM quarter note)
        let samples_per_beat = context.samples_per_beat().unwrap_or(22050.0);

        let delay_samples = match self.params.sync_mode.get() {
            SyncMode::Free => {
                // Convert milliseconds to samples
                self.params.time_ms.get() / 1000.0 * self.sample_rate
            }
            SyncMode::Quarter => samples_per_beat,           // 1 beat
            SyncMode::Eighth => samples_per_beat * 0.5,      // 1/2 beat
            SyncMode::Sixteenth => samples_per_beat * 0.25,  // 1/4 beat
            SyncMode::ThirtySecond => samples_per_beat * 0.125, // 1/8 beat
        };

        // Clamp to buffer size
        (delay_samples as usize).min(self.delay_l.max_samples.saturating_sub(1))
    }

    /// Generic processing implementation for both f32 and f64.
    ///
    /// # Signal Flow
    ///
    /// ```text
    /// Stereo Mode:
    ///   in_L ──┬──→ out_L = dry + wet_L
    ///          │
    ///          └──→ delay_L ──→ wet_L ──┐
    ///                    ↑              │
    ///                    └── feedback ──┘
    ///
    /// Ping-Pong Mode:
    ///   in_L + in_R ──→ mono ──→ delay_L ──→ delay_R ──→ delay_L ...
    ///                              │           │
    ///                              ↓           ↓
    ///                           out_L       out_R
    /// ```
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        _aux: &mut AuxiliaryBuffers<S>,
        context: &ProcessContext,
    ) {
        let delay_samples = self.calculate_delay_samples(context);
        let stereo_mode = self.params.stereo_mode.get();

        let num_samples = buffer.num_samples();
        let num_channels = buffer.num_output_channels().min(2);

        if num_channels == 0 {
            return;
        }

        for sample_idx in 0..num_samples {
            // Get smoothed parameter values (advances smoother each sample)
            // This prevents "zipper noise" when automating parameters
            let feedback = self.params.feedback.tick_smoothed();
            let mix = self.params.mix.tick_smoothed();

            // Read input samples
            let in_l = buffer.input(0)[sample_idx].to_f64();
            let in_r = if num_channels > 1 {
                buffer.input(1)[sample_idx].to_f64()
            } else {
                in_l
            };

            // Read from delay lines (the "wet" signal)
            let wet_l = self.delay_l.read(delay_samples);
            let wet_r = self.delay_r.read(delay_samples);

            // Wet/dry mix formula:
            //   output = dry * (1 - mix) + wet * mix
            //
            // mix=0.0 → 100% dry (no delay)
            // mix=0.5 → 50% dry + 50% wet
            // mix=1.0 → 100% wet (delay only)
            let out_l = in_l * (1.0 - mix) + wet_l * mix;
            let out_r = in_r * (1.0 - mix) + wet_r * mix;

            // Write to delay lines based on stereo mode
            match stereo_mode {
                StereoMode::Stereo => {
                    // Simple stereo: each channel feeds back to itself
                    // Feedback formula: delay_input = input + wet * feedback
                    //
                    // feedback=0.0 → single echo
                    // feedback=0.5 → decaying repeats
                    // feedback=1.0 → infinite repeats (careful!)
                    self.delay_l.write(in_l + wet_l * feedback);
                    self.delay_r.write(in_r + wet_r * feedback);
                }
                StereoMode::PingPong => {
                    // True ping-pong: sum to mono, alternate sides
                    // Creates a bouncing stereo effect:
                    //   L → R → L → R → ...
                    let mono_in = (in_l + in_r) * 0.5;

                    // Mono input goes to L buffer, L feeds R, R feeds L
                    self.delay_l.write(mono_in + wet_r * feedback);
                    self.delay_r.write(wet_l * feedback);
                }
            }

            // Write output
            buffer.output(0)[sample_idx] = S::from_f64(out_l);
            if num_channels > 1 {
                buffer.output(1)[sample_idx] = S::from_f64(out_r);
            }
        }
    }
}

impl AudioProcessor for DelayProcessor {
    type Plugin = DelayPlugin;

    fn unprepare(self) -> DelayPlugin {
        // Return just the params; delay buffers are discarded
        // They'll be reallocated with correct size on next prepare()
        DelayPlugin {
            params: self.params,
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &ProcessContext,
    ) {
        self.process_generic(buffer, aux, context);
    }

    fn supports_double_precision(&self) -> bool {
        true
    }

    fn process_f64(
        &mut self,
        buffer: &mut Buffer<f64>,
        aux: &mut AuxiliaryBuffers<f64>,
        context: &ProcessContext,
    ) {
        self.process_generic(buffer, aux, context);
    }

    fn set_active(&mut self, active: bool) {
        if active {
            // Clear delay buffers when activated (e.g., after deactivation/reactivation)
            // This ensures no stale audio bleeds into the new playback position
            self.delay_l.clear();
            self.delay_r.clear();
        }
    }

    fn tail_samples(&self) -> u32 {
        // Return the maximum delay buffer size as tail length
        // This ensures the host knows the plugin has audio tail
        self.delay_l.max_samples as u32
    }

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(self.params.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        self.params.load_state(data).map_err(PluginError::StateError)
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

export_vst3!(CONFIG, Vst3Processor<DelayPlugin>);
