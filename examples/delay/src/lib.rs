//! Beamer Delay - Example tempo-synced delay plugin demonstrating the Beamer framework.
//!
//! # Two-Struct Pattern
//!
//! Beamer plugins use two structs:
//!
//! 1. **`DelayModel`** - What every thread shares, marked with
//!    `#[beamer::model]` and implementing `Model`: its parameters, how the
//!    host sees it and how it creates the processor.
//! 2. **`DelayProcessor`** - What the audio thread owns, created by
//!    `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - `EnumParameter` for sync mode (quarter, eighth, 16th, 32nd, free)
//! - Tempo information from `ProcessContext` for tempo-synced delays
//! - Ring buffer delay line implementation
//! - Parameter smoothing to avoid zipper noise
//! - A standalone `Smoother` that glides the delay time, read between
//!   samples, so that a new time bends the pitch instead of clicking
//! - A limiter in the feedback loop, so that repeats never grow
//! - Both simple stereo and ping-pong modes
//! - `Model::LAYOUTS` with stereo, mono and mono to stereo layouts
//! - A tail that follows the feedback and the time, infinite at 100%
//! - A `(SampleRate, MainOutputChannels)` setup: the sample rate for the
//!   delay lines, the output channels for the editor
//! - `set_active()` to clear the delay lines when the host activates the plugin
//! - Factory presets that set only some parameters
//!
//! # Editor
//!
//! The editor in `webview/` is plain HTML, CSS and JavaScript: no package
//! manager and no build step, the export macro embeds the folder as it is.
//! It works on `__BEAMER__` directly. The audio thread tells it what only
//! the processor knows, the host's tempo, through a value sender.

use beamer::prelude::*;
use beamer::serde::Serialize;

// =============================================================================
// Enum Types for Parameter Choices
// =============================================================================

/// Delay sync mode - determines how delay time is calculated.
#[derive(Copy, Clone, PartialEq, EnumParameter)]
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
#[derive(Copy, Clone, PartialEq, EnumParameter)]
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
// Model
// =============================================================================

/// What the editor needs to know of the processor, sent from the audio
/// thread.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(crate = "beamer::serde", rename_all = "camelCase")]
pub struct Status {
    /// The host's tempo in BPM, `None` (null in JavaScript) while the host
    /// reports none.
    pub tempo: Option<f64>,
    /// The channels of the main output: 1 on a mono track.
    pub output_channels: u32,
}

/// Delay plugin model.
///
/// Uses **declarative parameter definition**: all configuration is in
/// attributes. The `#[beamer::model]` attribute generates everything
/// including the `Default` implementation.
#[beamer::export]
#[beamer::model]
pub struct DelayModel {
    /// Sync mode selection (Free, 1/4, 1/8, 1/16, 1/32)
    #[parameter(id = "sync_mode", name = "Sync Mode")]
    pub sync_mode: EnumParameter<SyncMode>,

    /// Stereo mode selection (Stereo, Ping-Pong)
    #[parameter(id = "stereo_mode", name = "Stereo Mode")]
    pub stereo_mode: EnumParameter<StereoMode>,

    /// Delay time in milliseconds (only used when Sync Mode = Free)
    #[parameter(id = "time", name = "Time", default = 250.0, range = 1.0..=2000.0, kind = "ms")]
    pub time_ms: FloatParameter,

    /// Feedback amount (0% to 100%) - smoothed to avoid zipper noise
    #[parameter(id = "feedback", name = "Feedback", default = 0.4, range = 0.0..=1.0, kind = "percent", smoothing = "exp:5.0")]
    pub feedback: FloatParameter,

    /// Wet/dry mix (0% = dry, 100% = wet) - smoothed to avoid zipper noise
    #[parameter(id = "mix", name = "Mix", default = 0.5, range = 0.0..=1.0, kind = "percent", smoothing = "exp:5.0")]
    pub mix: FloatParameter,

    /// Events for the editor. The processor's status sender is made from
    /// it.
    pub events: WebViewEventQueue,
}

impl Model for DelayModel {
    // The sample rate sizes the delay lines, the output channels go to the
    // editor. See `beamer::setup` for all available types.
    type Setup = (SampleRate, MainOutputChannels);
    type Processor = DelayProcessor;
    // The factory presets, in the Factory Presets section below.
    const PRESETS: &'static [Preset<Self>] = FACTORY_PRESETS;
    // Stereo by default. On a mono track the delay runs mono or, as a
    // mono to stereo insert, spreads the mono input over both channels.
    const LAYOUTS: &'static [Layout] = &[Layout::STEREO, Layout::MONO, Layout::MONO_TO_STEREO];

    fn prepare(
        self: Arc<Self>,
        (sample_rate, channels): (SampleRate, MainOutputChannels),
    ) -> DelayProcessor {
        // No tempo is known before the first block.
        let status = Status { tempo: None, output_channels: channels.0 };
        let mut delay_time = Smoother::new(SmoothingStyle::Exponential(GLIDE_MS));
        delay_time.set_sample_rate(sample_rate.hz());
        DelayProcessor {
            delay_l: DelayLine::new(sample_rate.hz()),
            delay_r: DelayLine::new(sample_rate.hz()),
            limiter_l: FeedbackLimiter::new(sample_rate.hz()),
            limiter_r: FeedbackLimiter::new(sample_rate.hz()),
            delay_time,
            delay_time_set: false,
            sample_rate: sample_rate.hz(),
            status_sender: self.events.value_sender("status", status),
            status,
            model: self,
        }
    }
}

// =============================================================================
// Factory Presets
// =============================================================================

/// The factory presets, in the order hosts list them.
///
/// A preset sets only the parameters it names, the others keep their
/// values. The synced presets leave the time alone, since a synced delay
/// does not use it.
const FACTORY_PRESETS: &[Preset<DelayModel>] = &[
    Preset::new("Slapback", |m| {
        m.sync_mode.set(SyncMode::Free);
        m.stereo_mode.set(StereoMode::Stereo);
        m.time_ms.set(80.0);
        m.feedback.set(0.2);
        m.mix.set(0.3);
    }),
    Preset::new("Eighth Note", |m| {
        m.sync_mode.set(SyncMode::Eighth);
        m.stereo_mode.set(StereoMode::Stereo);
        m.feedback.set(0.45);
        m.mix.set(0.4);
    }),
    Preset::new("Ping Pong", |m| {
        m.sync_mode.set(SyncMode::Quarter);
        m.stereo_mode.set(StereoMode::PingPong);
        m.feedback.set(0.5);
        m.mix.set(0.5);
    }),
    Preset::new("Ambient", |m| {
        m.sync_mode.set(SyncMode::Free);
        m.stereo_mode.set(StereoMode::Stereo);
        m.time_ms.set(750.0);
        m.feedback.set(0.7);
        m.mix.set(0.6);
    }),
];

// =============================================================================
// Delay Buffer
// =============================================================================

/// Maximum delay time in seconds (covers slow tempos and 2000ms free time).
/// The editor clamps to the same value.
const MAX_DELAY_SECONDS: f64 = 2.5;

/// The tempo a synced delay assumes while the host reports none. The
/// editor assumes the same.
const FALLBACK_TEMPO: f64 = 120.0;

/// How fast the delay time follows a new value: it covers about 63% of
/// the way in this many milliseconds. Large jumps in quick succession
/// sound like a tape being spun.
const GLIDE_MS: f64 = 100.0;

/// How fast the feedback limiter lets go after a peak, in milliseconds.
const LIMITER_RELEASE_MS: f64 = 100.0;

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

    /// Read from the delay line at a delay in samples that need not be
    /// whole: between two stored samples it interpolates linearly. A
    /// gliding delay time moves smoothly that way, without steps.
    fn read(&self, delay_samples: f64) -> f64 {
        if self.max_samples < 3 {
            return 0.0;
        }
        // At least one sample back, with room for the older neighbor.
        let delay = delay_samples.clamp(1.0, (self.max_samples - 2) as f64);
        let whole = delay.floor();
        let fraction = delay - whole;
        let newer = self.sample_at(whole as usize);
        let older = self.sample_at(whole as usize + 1);
        newer + (older - newer) * fraction
    }

    /// The sample written `delay` samples ago, `delay` from 1 to
    /// `max_samples - 1`.
    ///
    /// Calculates the read position using modular arithmetic:
    /// ```text
    /// read_pos = (write_pos + max_samples - delay) % max_samples
    /// ```
    fn sample_at(&self, delay: usize) -> f64 {
        self.buffer[(self.write_pos + self.max_samples - delay) % self.max_samples]
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

    /// Clear the buffer, so that no old echoes play.
    fn clear(&mut self) {
        self.buffer.fill(0.0);
    }
}

/// A peak limiter for the feedback loop.
///
/// It follows the level of what goes into a delay line and scales down
/// whatever would go above 1.0 (0 dBFS). With high feedback the repeats
/// add up with new input. Limited, they stay at full level at most, also
/// at 100% feedback, where the echoes hold instead of fading.
struct FeedbackLimiter {
    /// The level followed: up at once with a peak, down slowly after it,
    /// never below 1.0. Dividing by it leaves a quieter sample as it is.
    envelope: f64,
    /// How much of the envelope one sample keeps when the level falls.
    release: f64,
}

impl FeedbackLimiter {
    fn new(sample_rate: f64) -> Self {
        Self {
            envelope: 1.0,
            release: (-1.0 / (LIMITER_RELEASE_MS / 1000.0 * sample_rate)).exp(),
        }
    }

    /// The sample, scaled down while the envelope is above 1.0.
    fn process(&mut self, sample: f64) -> f64 {
        // The envelope is at least the level of this sample, so the result
        // never goes above 1.0.
        self.envelope = sample.abs().max(self.envelope * self.release).max(1.0);
        sample / self.envelope
    }

    /// Forget the level, with the delay lines cleared.
    fn reset(&mut self) {
        self.envelope = 1.0;
    }
}

// =============================================================================
// Processor
// =============================================================================

/// Delay plugin processor.
///
/// Ready for audio processing. Created by `DelayModel::prepare()`
/// with valid sample rate configuration. All fields have real values from the start.
pub struct DelayProcessor {
    /// The plugin's model, shared with the host and the UI.
    model: Arc<DelayModel>,
    /// Left channel delay line (allocated for current sample rate)
    delay_l: DelayLine,
    /// Right channel delay line (allocated for current sample rate)
    delay_r: DelayLine,
    /// The limiter of what goes into the left delay line.
    limiter_l: FeedbackLimiter,
    /// The limiter of what goes into the right delay line.
    limiter_r: FeedbackLimiter,
    /// The delay time in samples, gliding to a new time.
    delay_time: Smoother,
    /// Whether the delay time starts from where it should be. False after
    /// `prepare()` and `set_active()`: the first block jumps to the time
    /// instead of gliding to it from a guess.
    delay_time_set: bool,
    /// Current sample rate
    sample_rate: f64,
    /// Sends the status to the editor.
    status_sender: WebViewValueSender<Status>,
    /// The status last sent.
    status: Status,
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
    fn calculate_delay_samples(&self, context: &ProcessContext) -> f64 {
        // samples_per_beat() returns samples directly (sample_rate * 60 / tempo).
        // Without a tempo from the host a beat lasts as long as at 120 BPM.
        let samples_per_beat = context
            .samples_per_beat()
            .unwrap_or(self.sample_rate * 60.0 / FALLBACK_TEMPO);

        let delay_samples = match self.model.sync_mode.get() {
            SyncMode::Free => {
                // Convert milliseconds to samples
                self.model.time_ms.get() / 1000.0 * self.sample_rate
            }
            SyncMode::Quarter => samples_per_beat,           // 1 beat
            SyncMode::Eighth => samples_per_beat * 0.5,      // 1/2 beat
            SyncMode::Sixteenth => samples_per_beat * 0.25,  // 1/4 beat
            SyncMode::ThirtySecond => samples_per_beat * 0.125, // 1/8 beat
        };

        // A whole number of samples: reading between two samples dulls the
        // sound a little, which would add up over the repeats. Only the
        // glide reads between samples. Clamped to the buffer size, with
        // room for the older neighbor that `read` interpolates with.
        delay_samples
            .round()
            .min(self.delay_l.max_samples.saturating_sub(2) as f64)
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
        // The delay time glides to the time of this block. A jump would
        // click, as the read position would skip to other samples.
        let delay_samples = self.calculate_delay_samples(context);
        if self.delay_time_set {
            self.delay_time.set_target(delay_samples);
        } else {
            self.delay_time.reset(delay_samples);
            self.delay_time_set = true;
        }
        let stereo_mode = self.model.stereo_mode.get();

        // Tell the editor when the host's tempo changes. Sending never
        // waits or allocates. While the tempo stays, nothing is sent.
        let tempo = context.transport.tempo;
        if tempo != self.status.tempo {
            self.status.tempo = tempo;
            self.status_sender.send(&self.status);
        }

        let num_samples = buffer.num_samples();
        let num_channels = buffer.num_channels().min(2);
        // A mono input (also with a stereo output) feeds both delay lines.
        let stereo_input = buffer.num_input_channels() > 1;

        if num_channels == 0 {
            return;
        }

        for sample_idx in 0..num_samples {
            // Get smoothed parameter values (advances smoother each sample)
            // This prevents "zipper noise" when automating parameters
            let feedback = self.model.feedback.tick_smoothed();
            let mix = self.model.mix.tick_smoothed();
            let delay = self.delay_time.tick();

            // Read input samples. The buffer is processed in place, so they
            // are read before the output is written over them.
            let in_l = buffer.channel(0)[sample_idx].to_f64();
            let in_r = if stereo_input {
                buffer.channel(1)[sample_idx].to_f64()
            } else {
                in_l
            };

            // Read from delay lines (the "wet" signal)
            let wet_l = self.delay_l.read(delay);
            let wet_r = self.delay_r.read(delay);
            // Ping-pong echoes alternate between the lines. A single
            // channel plays both, so that no echo is lost.
            let wet_out_l = if num_channels == 1 && stereo_mode == StereoMode::PingPong {
                wet_l + wet_r
            } else {
                wet_l
            };

            // Wet/dry mix formula:
            //   output = dry * (1 - mix) + wet * mix
            //
            // mix=0.0 → 100% dry (no delay)
            // mix=0.5 → 50% dry + 50% wet
            // mix=1.0 → 100% wet (delay only)
            let out_l = in_l * (1.0 - mix) + wet_out_l * mix;
            let out_r = in_r * (1.0 - mix) + wet_r * mix;

            // Write to delay lines based on stereo mode
            match stereo_mode {
                StereoMode::Stereo => {
                    // Simple stereo: each channel feeds back to itself.
                    // Feedback formula: delay_input = input + wet * feedback
                    //
                    // feedback=0.0: a single echo.
                    // feedback=0.5: decaying repeats.
                    // feedback=1.0: repeats that hold, kept at full level
                    // at most by the limiter.
                    self.delay_l.write(self.limiter_l.process(in_l + wet_l * feedback));
                    self.delay_r.write(self.limiter_r.process(in_r + wet_r * feedback));
                }
                StereoMode::PingPong => {
                    // True ping-pong: sum to mono, alternate sides
                    // Creates a bouncing stereo effect:
                    //   L → R → L → R → ...
                    let mono_in = (in_l + in_r) * 0.5;

                    // Mono input goes to L buffer, L feeds R, R feeds L
                    self.delay_l.write(self.limiter_l.process(mono_in + wet_r * feedback));
                    self.delay_r.write(self.limiter_r.process(wet_l * feedback));
                }
            }

            // Write output
            buffer.channel(0)[sample_idx] = S::from_f64(out_l);
            if num_channels > 1 {
                buffer.channel(1)[sample_idx] = S::from_f64(out_r);
            }
        }
    }
}

impl Processor for DelayProcessor {
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
            self.limiter_l.reset();
            self.limiter_r.reset();
            // The next block starts at its delay time, without a glide.
            self.delay_time_set = false;
        }
    }

    fn tail_samples(&self) -> u32 {
        // At 100% feedback the repeats hold: the tail never ends.
        let feedback = self.model.feedback.get();
        if feedback >= 1.0 {
            return u32::MAX;
        }
        // Otherwise the repeats fall below -60 dB after this many delays,
        // the first echo included. Rounded up to whole seconds, so that
        // small moves of a dial mostly leave the tail as it is: an Audio
        // Unit host hears of each change.
        let repeats = if feedback > 0.0 {
            (0.001_f64.ln() / feedback.ln()).ceil() + 1.0
        } else {
            1.0
        };
        // Hosts ask before the first block, which sets the delay time:
        // until then the longest delay stands in for it.
        let delay = if self.delay_time_set {
            self.delay_time.target()
        } else {
            self.delay_l.max_samples as f64
        };
        let seconds = (repeats * delay / self.sample_rate).ceil();
        (seconds * self.sample_rate).min((u32::MAX - 1) as f64) as u32
    }
}
