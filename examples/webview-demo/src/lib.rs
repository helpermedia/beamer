//! WebView demo plugin.
//!
//! An example demonstrating React + Vite + Tailwind webview GUI support
//! in a Beamer plugin. Web assets are built with bun and embedded at
//! compile time. The plugin is a simple gain effect with an interactive GUI.

use std::sync::Arc;

use beamer::prelude::*;

// =============================================================================
// Parameters
// =============================================================================

/// Simple gain parameter for demonstration.
#[derive(Parameters)]
pub struct WebViewDemoParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

// =============================================================================
// Descriptor (unprepared state)
// =============================================================================

/// Plugin descriptor with WebView GUI support.
///
/// The `#[beamer::export]` macro scans `webview/dist/` at compile time
/// and embeds all built assets via `include_bytes!()`.
#[beamer::export]
#[derive(Default, HasParameters)]
pub struct WebViewDemoDescriptor {
    #[parameters]
    pub parameters: WebViewDemoParameters,
}

impl Descriptor for WebViewDemoDescriptor {
    type Setup = ();
    type Processor = WebViewDemoProcessor;

    fn prepare(self, _: ()) -> WebViewDemoProcessor {
        WebViewDemoProcessor {
            parameters: self.parameters,
        }
    }

    fn webview_handler(&self) -> Option<Arc<dyn WebViewHandler>> {
        Some(Arc::new(DemoHandler))
    }
}

// =============================================================================
// WebView Handler (invoke/event demo)
// =============================================================================

/// Handles `__BEAMER__.invoke()` calls from JavaScript.
struct DemoHandler;

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
            _ => Err(format!("unknown method: {method}")),
        }
    }
}

// =============================================================================
// Processor (prepared state)
// =============================================================================

/// Audio processor with simple gain.
#[derive(HasParameters)]
pub struct WebViewDemoProcessor {
    #[parameters]
    pub parameters: WebViewDemoParameters,
}

impl Processor for WebViewDemoProcessor {
    type Descriptor = WebViewDemoDescriptor;

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        let gain = self.parameters.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}
