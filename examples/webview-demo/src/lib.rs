//! WebView demo plugin.
//!
//! An example demonstrating React + Vite + Tailwind webview GUI support
//! in a Beamer plugin. Web assets are built with bun and embedded at
//! compile time. The plugin is a simple gain effect with an interactive GUI.
//!
//! It also shows state beyond parameters: the pan law is a `State` field.
//! The web UI sets it through the WebView handler, the processor reads it
//! on the audio thread and the host saves it with the project.

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
    type Setup = ();
    type Processor = WebViewDemoProcessor;

    fn prepare(&self, _: ()) -> WebViewDemoProcessor {
        WebViewDemoProcessor {
            parameters: Arc::clone(&self.parameters),
            // The audio thread's end of the state field.
            pan_law: self.parameters.pan_law.reader(),
        }
    }

    fn webview_handler(&self) -> Option<Arc<dyn WebViewHandler>> {
        // The handler shares the parameters, so that the web UI can read
        // and set the state field.
        Some(Arc::new(DemoHandler {
            parameters: Arc::clone(&self.parameters),
        }))
    }
}

// =============================================================================
// WebView Handler (invoke/event demo)
// =============================================================================

/// Handles `__BEAMER__.invoke()` calls from JavaScript.
struct DemoHandler {
    parameters: Arc<WebViewDemoParameters>,
}

impl WebViewHandler for DemoHandler {
    fn on_invoke(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        match method {
            "getInfo" => Ok(serde_json::json!({
                "name": "Beamer WebView Demo",
                "version": env!("CARGO_PKG_VERSION"),
                "framework": "Beamer",
            })),
            // The pan law as "constant_power" or "linear".
            "getPanLaw" => serde_json::to_value(*self.parameters.pan_law.get())
                .map_err(|error| error.to_string()),
            "setPanLaw" => {
                let law = args.first().cloned().ok_or("setPanLaw needs a pan law")?;
                let law: PanLaw = serde_json::from_value(law).map_err(|error| error.to_string())?;
                self.parameters.pan_law.set(law);
                Ok(serde_json::Value::Null)
            }
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
