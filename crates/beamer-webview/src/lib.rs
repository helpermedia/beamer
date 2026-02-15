//! WebView GUI support for Beamer audio plugins.
//!
//! Provides platform-native WebView embedding for VST3 plugin editors.

mod error;
mod platform;
mod view;

use beamer_core::{EditorConstraints, EditorDelegate, Size};

pub use error::{Result, WebViewError};
pub use view::WebViewPlugView;

/// Configuration for a WebView editor.
pub struct WebViewConfig {
    /// HTML content to load.
    pub html: &'static str,
    /// Whether to enable developer tools.
    pub dev_tools: bool,
}

/// Simple `EditorDelegate` backed by fixed size and constraints.
///
/// Used when the plugin doesn't provide its own delegate (the common case
/// for Config-driven editor setup).
pub struct StaticEditorDelegate {
    size: Size,
    constraints: EditorConstraints,
}

impl StaticEditorDelegate {
    /// Create a new static delegate with the given size and constraints.
    pub fn new(size: Size, constraints: EditorConstraints) -> Self {
        Self { size, constraints }
    }
}

impl EditorDelegate for StaticEditorDelegate {
    fn editor_size(&self) -> Size {
        self.size
    }

    fn editor_constraints(&self) -> EditorConstraints {
        self.constraints
    }
}
