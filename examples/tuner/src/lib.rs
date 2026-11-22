//! Beamer Tuner - Example chromatic tuner from A0 to C8.
//!
//! # Two-Struct Pattern
//!
//! Beamer plugins use two structs:
//!
//! 1. **`TunerModel`** - What every thread shares, marked with
//!    `#[beamer::model]` and implementing `Model`: its parameters, how the
//!    host sees it and how it creates the processor.
//! 2. **`TunerProcessor`** - What the audio thread owns, created by
//!    `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - Analysis on the audio thread: a pitch detector ([`pitch`]), at most
//!   one analysis per block whatever its size, with every buffer made in
//!   `prepare()`, so `process()` allocates nothing
//! - Work done only while someone looks: the detector analyzes only while
//!   the editor is open (`events.is_editor_open()`); the cheap filtering
//!   runs always, so a reopened editor shows a pitch at once
//! - A value sender with a measurement (frequency, clarity, level), from
//!   which the editor works out the note and the cents
//! - A parameter only the editor reads: the reference pitch, saved with the
//!   project and shown in the host's generic view
//! - A mute that ramps over a few milliseconds (a standalone `Smoother`), so
//!   switching it never clicks
//! - `subcategories = ["tools"]` and `middle_c = "C4"` in `Config.toml`
//! - `Model::LAYOUTS` with a stereo and a mono layout
//!
//! # Editor
//!
//! The editor in `webview/` is Svelte, on the packages beamer-api,
//! beamer-controls and beamer-svelte. bun and Vite build it into
//! `webview/dist/`, which the export macro embeds. A needle on an arc
//! shows how far the pitch is from the nearest note, which is named under
//! it with the frequency and the cents beside it, over a strobe band and
//! the input level. It turns blue within 2 cents. The editor works out the
//! note and the cents from the frequency the processor sends and the
//! reference parameter, so it follows the reference dial at once. When no
//! pitch comes it holds the last note dimmed for 1.5 seconds, then rests.

pub mod pitch;

use beamer::prelude::*;

use pitch::{Pitch, PitchDetector};

/// The time the mute takes to fade the output out or in.
const MUTE_RAMP_MS: f64 = 10.0;

// =============================================================================
// Model
// =============================================================================

/// Plugin model: the reference pitch and the mute, the pitch the processor
/// sends and the queue it goes through.
#[beamer::export]
#[beamer::model]
pub struct TunerModel {
    /// The pitch of A4 the editor names the notes by. The processor does
    /// not read it: it measures a frequency, the editor works out the note
    /// and the cents, so it follows this parameter at once, also while the
    /// host does not call the processor. 415 Hz is baroque pitch, 440 Hz
    /// today's standard, orchestras tune up to 446 Hz.
    #[parameter(id = "reference", name = "Reference", default = 440.0,
                range = 400.0..=480.0, kind = "hz", step = 0.1)]
    pub reference: FloatParameter,

    /// Silences the output while tuning, as a pedal tuner does. The input
    /// is still measured.
    #[parameter(id = "mute", name = "Mute", default = false)]
    pub mute: BoolParameter,

    /// What the detector heard in its newest window, sent from the audio
    /// thread about 50 times a second while the editor is open.
    #[value(id = "pitch")]
    pub pitch: WebViewValue<Pitch>,

    /// The queue the sender above delivers through.
    pub events: WebViewEventQueue,
}

impl Model for TunerModel {
    type Setup = SampleRate;
    type Processor = TunerProcessor;
    // Stereo by default, mono on a mono track: a guitar comes in mono. The
    // detector hears the mean of the channels.
    const LAYOUTS: &'static [Layout] = &[Layout::STEREO, Layout::MONO];

    fn prepare(self: Arc<Self>, sample_rate: SampleRate) -> TunerProcessor {
        let mut mute_gain = Smoother::new(SmoothingStyle::Linear(MUTE_RAMP_MS));
        mute_gain.set_sample_rate(sample_rate.hz());
        // No ramp when the plugin starts muted.
        mute_gain.reset(gain_for(self.mute.get()));
        TunerProcessor {
            detector: PitchDetector::new(sample_rate.hz()),
            pitch_sender: self.pitch.sender(Pitch::SILENT),
            mute_gain,
            model: self,
        }
    }
}

/// The gain of the output for the state of the mute.
fn gain_for(muted: bool) -> f64 {
    if muted {
        0.0
    } else {
        1.0
    }
}

// =============================================================================
// Processor
// =============================================================================

/// The processor: measures the input, passes it through or mutes it.
pub struct TunerProcessor {
    model: Arc<TunerModel>,
    detector: PitchDetector,
    pitch_sender: WebViewValueSender<Pitch>,
    /// The gain of the output, which ramps when the mute changes.
    mute_gain: Smoother,
}

impl TunerProcessor {
    /// Generic processing for any sample type.
    fn process_generic<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        let num_samples = buffer.num_samples();
        let num_channels = buffer.num_channels();
        if num_channels == 0 {
            return;
        }

        // The detector hears the mean of the channels.
        let scale = 1.0 / num_channels as f64;
        for index in 0..num_samples {
            let sum: f64 = buffer
                .channels()
                .map(|channel| channel[index].to_f64())
                .sum();
            self.detector.push((sum * scale) as f32);
        }

        // At most one analysis per call, on the newest window: a long block
        // spans several hops but costs one analysis. None while no editor
        // shows it.
        if self.model.events.is_editor_open() && self.detector.analysis_due() {
            self.pitch_sender.send(&self.detector.analyze());
        }

        self.mute_gain.set_target(gain_for(self.model.mute.get()));
        if self.mute_gain.is_smoothing() {
            for index in 0..num_samples {
                let gain = S::from_f64(self.mute_gain.tick());
                for channel in buffer.channels_mut() {
                    channel[index] = channel[index] * gain;
                }
            }
        } else if self.mute_gain.current() == 0.0 {
            buffer.clear();
        }
        // Otherwise the audio passes as it came in.
    }
}

impl Processor for TunerProcessor {
    /// A new start: the detector forgets the signal, the mute holds its
    /// state without a ramp.
    fn set_active(&mut self, active: bool) {
        if active {
            self.detector.reset();
            self.mute_gain.reset(gain_for(self.model.mute.get()));
        }
    }

    // The f64 path is native: a host that processes in f64 gets
    // `process_f64()` without a conversion.
    fn supports_double_precision(&self) -> bool {
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        self.process_generic(buffer);
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

    const SAMPLE_RATE: f64 = 48_000.0;

    // The processor of `model` at 48 kHz, with or without an editor open.
    // The queue is claimed, as a plugin instance does, so that its sender
    // works.
    fn prepared(model: &Arc<TunerModel>, editor_open: bool) -> TunerProcessor {
        model.events.claim();
        if editor_open {
            model.events.editor_opened();
        }
        Arc::clone(model).prepare(SampleRate(SAMPLE_RATE))
    }

    // A sine at `frequency` with an amplitude of 0.5, from sample `start` on.
    fn sine(frequency: f64, start: usize, samples: usize) -> Vec<f32> {
        (start..start + samples)
            .map(|index| {
                (0.5 * (2.0 * std::f64::consts::PI * frequency * index as f64 / SAMPLE_RATE).sin())
                    as f32
            })
            .collect()
    }

    // Process `channels` in place in one block.
    fn render(processor: &mut TunerProcessor, channels: &mut [Vec<f32>]) {
        let samples = channels[0].len();
        let count = channels.len();
        let mut buffer = Buffer::new(
            channels.iter_mut().map(|channel| &mut channel[..]),
            std::iter::empty(),
            count,
            samples,
        );
        let context = ProcessContext::with_empty_transport(SAMPLE_RATE, samples);
        processor.process(&mut buffer, &mut AuxiliaryBuffers::empty(), &context);
    }

    // Feed `seconds` of a stereo sine at `frequency` in blocks of `block`.
    fn feed(processor: &mut TunerProcessor, frequency: f64, seconds: f64, block: usize) {
        let total = (seconds * SAMPLE_RATE) as usize;
        for start in (0..total).step_by(block) {
            let tone = sine(frequency, start, block.min(total - start));
            render(processor, &mut [tone.clone(), tone]);
        }
    }

    // The frequency in the newest pitch the editor got, `None` when none
    // went out.
    fn sent_frequency(model: &TunerModel) -> Option<f64> {
        let script = model.events.take_script()?;
        let value = script.rsplit(r#""frequencyHz":"#).next()?;
        let end = value.find(|c: char| c != '.' && c != '-' && !c.is_ascii_digit())?;
        value[..end].parse().ok()
    }

    fn cents(measured: f64, expected: f64) -> f64 {
        1200.0 * (measured / expected).log2()
    }

    #[test]
    fn test_the_audio_passes_unchanged() {
        let model = Arc::new(TunerModel::default());
        let mut processor = prepared(&model, true);
        let left = sine(440.0, 0, 4800);
        let right = sine(660.0, 0, 4800);
        let mut channels = [left.clone(), right.clone()];
        render(&mut processor, &mut channels);
        assert_eq!(channels, [left, right]);
    }

    #[test]
    fn test_the_mute_ramps_to_silence_and_back_without_a_step() {
        let model = Arc::new(TunerModel::default());
        let mut processor = prepared(&model, false);
        // A constant input shows the gain as it is.
        let ones = || vec![vec![1.0f32; 960]; 2];
        model.mute.set(true);
        let mut channels = ones();
        render(&mut processor, &mut channels);
        // 10 ms at 48 kHz.
        let ramp = 480;
        let gains = &channels[0];
        assert!(
            gains[0] > 0.99,
            "the ramp starts at full gain: {}",
            gains[0]
        );
        assert!(
            gains
                .windows(2)
                .all(|pair| pair[1] <= pair[0] && pair[0] - pair[1] < 0.01),
            "it falls in small steps"
        );
        assert!(
            gains[ramp..].iter().all(|gain| *gain == 0.0),
            "silent after 10 ms"
        );
        let mut channels = ones();
        render(&mut processor, &mut channels);
        assert!(
            channels.iter().flatten().all(|sample| *sample == 0.0),
            "it stays silent"
        );

        model.mute.set(false);
        let mut channels = ones();
        render(&mut processor, &mut channels);
        let gains = &channels[1];
        assert!(gains[0] < 0.01, "the ramp starts at silence: {}", gains[0]);
        assert!(
            gains
                .windows(2)
                .all(|pair| pair[1] >= pair[0] && pair[1] - pair[0] < 0.01),
            "it rises in small steps"
        );
        assert!(
            gains[ramp..].iter().all(|gain| *gain == 1.0),
            "full gain after 10 ms"
        );
    }

    #[test]
    fn test_a_plugin_that_starts_muted_starts_silent() {
        let model = Arc::new(TunerModel::default());
        model.mute.set(true);
        let mut processor = prepared(&model, false);
        let mut channels = [vec![1.0f32; 64]];
        render(&mut processor, &mut channels);
        assert!(channels[0].iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn test_no_analysis_while_the_editor_is_closed_a_pitch_once_it_opens() {
        let model = Arc::new(TunerModel::default());
        let mut processor = prepared(&model, false);
        // The sender's first value goes out once, editor or not.
        assert_eq!(
            sent_frequency(&model),
            Some(0.0),
            "the first value is silent"
        );
        feed(&mut processor, 440.0, 0.2, 512);
        assert!(
            processor.detector.analysis_due(),
            "nothing analyzed the hops that passed"
        );
        assert_eq!(model.events.take_script(), None, "nothing goes out");

        // The editor opens: the first block after it sends a pitch, from the
        // window the detector kept filling.
        model.events.editor_opened();
        feed(&mut processor, 440.0, 64.0 / SAMPLE_RATE, 64);
        let frequency = sent_frequency(&model).expect("a pitch goes out");
        assert!(cents(frequency, 440.0).abs() < 0.1, "{frequency} Hz");
    }

    #[test]
    fn test_every_block_size_reads_the_same_pitch() {
        for block in [1, 32, 512, 4096] {
            let model = Arc::new(TunerModel::default());
            let mut processor = prepared(&model, true);
            feed(&mut processor, 196.0, 12_288.0 / SAMPLE_RATE, block);
            let frequency = sent_frequency(&model).expect("a pitch goes out");
            assert!(
                cents(frequency, 196.0).abs() < 0.01,
                "block of {block}: {frequency} Hz"
            );
        }
    }

    #[test]
    fn test_a_mono_track_and_one_side_of_a_stereo_track_read_the_pitch() {
        let model = Arc::new(TunerModel::default());
        let mut processor = prepared(&model, true);
        render(&mut processor, &mut [sine(82.41, 0, 9600)]);
        let frequency = sent_frequency(&model).expect("a pitch goes out");
        assert!(cents(frequency, 82.41).abs() < 0.1, "mono: {frequency} Hz");

        let model = Arc::new(TunerModel::default());
        let mut processor = prepared(&model, true);
        render(&mut processor, &mut [sine(82.41, 0, 9600), vec![0.0; 9600]]);
        let frequency = sent_frequency(&model).expect("a pitch goes out");
        assert!(
            cents(frequency, 82.41).abs() < 0.1,
            "left only: {frequency} Hz"
        );
    }

    #[test]
    fn test_process_neither_allocates_nor_frees() {
        let model = Arc::new(TunerModel::default());
        // With the editor open, so the analysis runs inside the check.
        let mut processor = prepared(&model, true);
        feed(&mut processor, 440.0, 0.1, 512);
        let mut channels = [sine(440.0, 0, 1024), sine(440.0, 0, 1024)];
        let [left, right] = &mut channels;
        let mut buffer = Buffer::new([&mut left[..], &mut right[..]], std::iter::empty(), 2, 1024);
        let mut aux = AuxiliaryBuffers::empty();
        let context = ProcessContext::with_empty_transport(SAMPLE_RATE, 1024);
        model.mute.set(true);
        let (_, counts) = allocation_check::count_in_process(|| {
            processor.process(&mut buffer, &mut aux, &context);
        });
        assert!(!processor.detector.analysis_due(), "the analysis ran");
        // The check runs in debug builds with the `allocation-check` feature.
        if allocation_check::is_active() {
            assert!(counts.is_empty(), "{counts:?}");
        }
    }

    #[test]
    fn test_the_reference_shows_and_reads_back_in_hz() {
        let model = TunerModel::default();
        let reference = &model.reference;
        assert_eq!(reference.get(), 440.0);
        assert_eq!(reference.normalized_to_plain(0.0), 400.0);
        assert_eq!(reference.normalized_to_plain(1.0), 480.0);
        assert_eq!(reference.info().units, "Hz");
        // The decimals of the step, 0.1 Hz.
        assert_eq!(
            reference.display_normalized(reference.get_normalized()),
            "440.0"
        );
        reference.set(442.5);
        assert_eq!(
            reference.display_normalized(reference.get_normalized()),
            "442.5"
        );
    }
}
