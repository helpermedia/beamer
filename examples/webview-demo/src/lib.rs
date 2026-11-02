//! WebView demo plugin.
//!
//! An example demonstrating React + Vite + Tailwind webview editor support
//! in a Beamer plugin. Web assets are built with bun and embedded at
//! compile time. The plugin is a simple gain effect with an interactive editor.
//!
//! It also shows state beyond parameters: the pan law is a `State` field.
//! The web UI reads and sets it through `__BEAMER__.state`, the processor
//! reads it on the audio thread and the host saves it with the project.
//!
//! And it shows an event from Rust to the web UI: the plugin tells the
//! editor the sample rate whenever the host prepares it.
//!
//! And data from the audio thread: an output meter needs only the newest
//! level (a value sender). A scrolling level history needs every point in
//! order (a stream sender).
//!
//! The plugin is two structs. Every thread shares the model; the audio
//! thread owns the processor.

use beamer::prelude::*;
use beamer::serde::{Deserialize, Serialize};

// =============================================================================
// Model
// =============================================================================

/// How the pan control distributes the signal over the two channels.
///
/// This is a setting and not a parameter: a host does not automate it. The
/// model keeps it in a `State` field.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(crate = "beamer::serde", rename_all = "snake_case")]
pub enum PanLaw {
    /// Equal loudness across the stereo field (-3 dB at the center).
    #[default]
    ConstantPower,
    /// A plain crossfade (-6 dB at the center).
    Linear,
}

/// The output level as the meter shows it.
#[derive(Debug, Default, Clone, Copy, Serialize)]
#[serde(crate = "beamer::serde")]
pub struct Levels {
    /// The held peak of each channel, linear. Not in dB: the infinity of
    /// silence would reach JavaScript as null.
    pub peak: [f32; 2],
}

/// Everything the plugin knows, shared by the host, the web UI and the
/// audio thread for the life of the instance.
///
/// `#[beamer::model]` makes this struct the model: the host sees the
/// parameters, the state fields are saved with the project and the events
/// reach the editor. The `#[beamer::export]` macro scans `webview/dist/`
/// at compile time and embeds all built assets via `include_bytes!()`.
#[beamer::export]
#[beamer::model]
pub struct WebViewDemoModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,

    #[parameter(id = "pan", name = "Pan", default = 0.0, kind = "pan")]
    pub pan: FloatParameter,

    /// Saved with the plugin state under the id "pan_law".
    #[state(id = "pan_law")]
    pub pan_law: State<PanLaw>,

    /// The sample rate the host runs at, 0 until it prepared the plugin.
    /// Not saved and not a parameter: the web UI shows it.
    pub sample_rate: Handover<f64>,

    /// Events for the web UI. What is emitted here reaches the editor
    /// while it is open. The processor's senders are made from it too.
    pub events: WebViewEventQueue,
}

impl Model for WebViewDemoModel {
    type Setup = SampleRate;
    type Processor = WebViewDemoProcessor;

    fn prepare(self: Arc<Self>, setup: SampleRate) -> WebViewDemoProcessor {
        // Keep the sample rate for an editor that opens later and tell the
        // one that is open. A host calls this on one of its own threads,
        // never on the audio thread, so emitting is fine here.
        let sample_rate = setup.hz();
        self.sample_rate.set(sample_rate);
        self.events.emit("sampleRate", &sample_rate);

        WebViewDemoProcessor {
            // The audio thread's end of the state field. Taken before the
            // model moves into the processor.
            pan_law: self.pan_law.reader(),
            // The meter shows the newest level only.
            levels: self.events.value_sender("levels", Levels::default()),
            // The history needs every point. About two arrive between two
            // ticks of the editor. Room for 64 is 640 ms: an editor whose main
            // thread is busy for a moment (a project that loads, a heavy
            // redraw) loses nothing.
            history: self.events.stream_sender("history", 64),
            peaks: [0.0; 2],
            // A held peak falls by 20 dB a second.
            release_per_sample: 10f32.powf(-1.0 / sample_rate as f32),
            window_len: (sample_rate / HISTORY_POINTS_PER_SECOND).round().max(1.0) as usize,
            window_peak: 0.0,
            window_filled: 0,
            model: self,
        }
    }
}

impl WebViewHandler for WebViewDemoModel {
    /// Handles `__BEAMER__.invoke()` calls from JavaScript.
    fn on_invoke(
        &self,
        method: &str,
        _args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        match method {
            "getInfo" => Ok(serde_json::json!({
                "name": "Beamer WebView Demo",
                "version": env!("CARGO_PKG_VERSION"),
                "framework": "Beamer",
            })),
            // The sample rate of the host, 0 before it prepared the plugin.
            // An open editor then follows it through the "sampleRate" event.
            "getSampleRate" => Ok(serde_json::json!(*self.sample_rate.get())),
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

// =============================================================================
// Processor
// =============================================================================

/// How many points of the level history a second has.
const HISTORY_POINTS_PER_SECOND: f64 = 100.0;

/// The level where the meter's scale ends (-60 dB, linear). A held peak
/// below it is at rest.
const METER_FLOOR: f32 = 0.001;

/// Audio processor with simple gain.
pub struct WebViewDemoProcessor {
    model: Arc<WebViewDemoModel>,
    /// The pan law as the audio thread sees it.
    pan_law: HandoverReader<PanLaw>,
    /// The meter's level, of which the web UI gets the newest.
    levels: WebViewValueSender<Levels>,
    /// The points of the level history, of which the web UI gets each.
    history: WebViewStreamSender<f32>,
    /// The held peak of each channel, linear.
    peaks: [f32; 2],
    /// What a held peak keeps of itself per sample.
    release_per_sample: f32,
    /// How many samples one point of the history covers.
    window_len: usize,
    /// The loudest sample of the point the history is at.
    window_peak: f32,
    /// How many samples of that point were measured.
    window_filled: usize,
}

impl WebViewDemoProcessor {
    /// Measure the output for the meter and the history.
    fn measure(&mut self, buffer: &Buffer) {
        let mut channels = buffer.channels();
        let Some(left) = channels.next() else {
            return;
        };
        // A mono plugin shows its one channel on both sides of the meter.
        let right = channels.next().unwrap_or(left);

        // The held peaks fall for the length of the block, then rise to
        // the loudest sample in it. Below the floor a peak is at rest.
        let before = self.peaks;
        let fall = self.release_per_sample.powi(left.len() as i32);
        for (peak, channel) in self.peaks.iter_mut().zip([left, right]) {
            let loudest = channel.iter().fold(0.0f32, |loudest, sample| loudest.max(sample.abs()));
            *peak = (*peak * fall).max(loudest);
            if *peak < METER_FLOOR {
                *peak = 0.0;
            }
        }
        // A value sender delivers what it is sent. A meter at rest sends
        // nothing, so the web UI has nothing to do.
        if self.peaks != before {
            self.levels.send(&Levels { peak: self.peaks });
        }

        // One point of the history per window: its loudest sample of
        // both channels.
        for (left, right) in left.iter().zip(right) {
            self.window_peak = self.window_peak.max(left.abs()).max(right.abs());
            self.window_filled += 1;
            if self.window_filled == self.window_len {
                self.history.push(self.window_peak);
                self.window_peak = 0.0;
                self.window_filled = 0;
            }
        }
    }
}

impl Processor for WebViewDemoProcessor {
    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        debug_assert!(buffer.num_channels() <= 2, "pan assumes stereo");

        let gain = self.model.gain.as_linear() as f32;

        // The pan position from 0.0 (left) to 1.0 (right).
        let position = ((self.model.pan.get() + 1.0) * 0.5) as f32;

        // Take over a pan law that the UI or a loaded state set since the
        // last block. This takes no lock and does not allocate.
        let pan_gains = match *self.pan_law.latest() {
            PanLaw::ConstantPower => {
                // The angle maps the position to [0, PI/2].
                let angle = position * std::f32::consts::FRAC_PI_2;
                [gain * angle.cos(), gain * angle.sin()]
            }
            PanLaw::Linear => [gain * (1.0 - position), gain * position],
        };

        for (ch, channel) in buffer.channels_mut().enumerate() {
            let g = pan_gains[ch.min(1)];
            for sample in channel {
                *sample *= g;
            }
        }

        // Measuring costs time, so only while the editor is open.
        if self.model.events.is_editor_open() {
            self.measure(buffer);
        } else if self.peaks != [0.0; 2] {
            // The meter comes to rest with the next block the host
            // processes, so that an editor that opens later does not show a
            // peak from before. Sending takes no lock and does not
            // allocate.
            self.peaks = [0.0; 2];
            self.window_peak = 0.0;
            self.window_filled = 0;
            self.levels.send(&Levels { peak: self.peaks });
        }
    }
}
