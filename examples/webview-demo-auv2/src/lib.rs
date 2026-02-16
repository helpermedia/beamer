//! WebView demo plugin.
//!
//! A minimal example demonstrating WebView GUI support in a Beamer plugin.
//! The plugin is a simple gain effect with a static HTML editor loaded
//! via WKWebView (macOS). Windows support is planned but not yet implemented.

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

/// Plugin descriptor with WebView editor support.
///
/// The `#[beamer::export]` macro auto-detects `webview/index.html` and
/// embeds it as the editor HTML content.
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
