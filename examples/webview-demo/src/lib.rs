//! WebView demo plugin.
//!
//! An example demonstrating React + Vite + Tailwind webview GUI support
//! in a Beamer plugin. Web assets are built with bun and embedded at
//! compile time. The plugin is a simple gain effect with an interactive GUI.
//!
//! It also shows state beyond parameters: the pan law is a `State` field.
//! The web UI reads and sets it through `__BEAMER__.state`, the processor
//! reads it on the audio thread and the host saves it with the project.
//!
//! And it shows an event from Rust to the web UI: the plugin tells the
//! editor the sample rate whenever the host prepares it.

use beamer::prelude::*;
use beamer::serde::{Deserialize, Serialize};

// =============================================================================
// Parameters
// =============================================================================

/// How the pan control distributes the signal over the two channels.
///
/// This is a setting and not a parameter: a host does not automate it. The
/// plugin keeps it in a `State` field.
#[derive(Debug, Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(crate = "beamer::serde", rename_all = "snake_case")]
pub enum PanLaw {
    /// Equal loudness across the stereo field (-3 dB at the center).
    #[default]
    ConstantPower,
    /// A plain crossfade (-6 dB at the center).
    Linear,
}

/// Parameters for the WebView demo plugin.
#[derive(Parameters)]
pub struct WebViewDemoParameters {
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
    /// while it is open.
    pub events: WebViewEvents,
}

// =============================================================================
// Descriptor
// =============================================================================

/// Plugin descriptor with WebView GUI support.
///
/// The `#[beamer::export]` macro scans `webview/dist/` at compile time
/// and embeds all built assets via `include_bytes!()`.
#[beamer::export]
#[derive(Default, HasParameters)]
pub struct WebViewDemoDescriptor {
    #[parameters]
    pub parameters: Arc<WebViewDemoParameters>,
}

impl Descriptor for WebViewDemoDescriptor {
    type Setup = SampleRate;
    type Processor = WebViewDemoProcessor;

    fn prepare(&self, setup: SampleRate) -> WebViewDemoProcessor {
        // Keep the sample rate for an editor that opens later and tell the
        // one that is open. A host calls this on one of its own threads,
        // never on the audio thread, so emitting is fine here.
        let sample_rate = setup.hz();
        self.parameters.sample_rate.set(sample_rate);
        self.parameters.events.emit("sampleRate", &sample_rate);

        WebViewDemoProcessor {
            parameters: Arc::clone(&self.parameters),
            // The audio thread's end of the state field.
            pan_law: self.parameters.pan_law.reader(),
        }
    }

    fn webview_handler(&self) -> Option<Arc<dyn WebViewHandler>> {
        // The handler shares the parameters, so that it can answer with the
        // sample rate. The state field needs no handler.
        Some(Arc::new(DemoHandler {
            parameters: Arc::clone(&self.parameters),
        }))
    }
}

// =============================================================================
// WebView Handler (invoke demo)
// =============================================================================

/// Handles `__BEAMER__.invoke()` calls from JavaScript.
struct DemoHandler {
    parameters: Arc<WebViewDemoParameters>,
}

impl WebViewHandler for DemoHandler {
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
            "getSampleRate" => Ok(serde_json::json!(*self.parameters.sample_rate.get())),
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

// =============================================================================
// Processor
// =============================================================================

/// Audio processor with simple gain.
pub struct WebViewDemoProcessor {
    pub parameters: Arc<WebViewDemoParameters>,
    /// The pan law as the audio thread sees it.
    pan_law: HandoverReader<PanLaw>,
}

impl Processor for WebViewDemoProcessor {
    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        debug_assert!(buffer.num_output_channels() <= 2, "pan assumes stereo");

        let gain = self.parameters.gain.as_linear() as f32;

        // The pan position from 0.0 (left) to 1.0 (right).
        let position = ((self.parameters.pan.get() + 1.0) * 0.5) as f32;

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

        for (ch, (input, output)) in buffer.zip_channels().enumerate() {
            let g = pan_gains[ch.min(1)];
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * g;
            }
        }
    }
}
