//! Editor and GUI-related traits.

use crate::types::Size;

/// Size constraints for the plugin editor.
#[derive(Debug, Clone, Copy)]
pub struct EditorConstraints {
    /// Minimum size.
    pub min: Size,
    /// Maximum size.
    pub max: Size,
    /// Whether the editor is resizable.
    pub resizable: bool,
}

impl Default for EditorConstraints {
    fn default() -> Self {
        Self {
            min: Size::new(400, 300),
            max: Size::new(1600, 1200),
            resizable: true,
        }
    }
}

/// Trait for plugin editor/GUI callbacks.
///
/// Implement this trait to provide GUI-related configuration and callbacks.
/// The actual WebView creation and management is handled by the framework;
/// this trait just provides configuration and lifecycle hooks.
pub trait EditorDelegate: Send {
    /// Get the initial editor size.
    ///
    /// This is the size the editor window will have when first opened.
    /// Default is 800x600.
    fn editor_size(&self) -> Size {
        Size::new(800, 600)
    }

    /// Get the editor size constraints.
    ///
    /// These constraints determine the minimum and maximum sizes the editor
    /// can be resized to, and whether resizing is allowed at all.
    fn editor_constraints(&self) -> EditorConstraints {
        EditorConstraints::default()
    }

    /// Called when the editor is opened.
    ///
    /// Use this to initialize any editor-specific state.
    fn editor_opened(&mut self) {}

    /// Called when the editor is closed.
    ///
    /// Use this to clean up editor-specific state.
    fn editor_closed(&mut self) {}

    /// Called when the editor is resized.
    ///
    /// The new size has already been constrained to the editor constraints.
    fn editor_resized(&mut self, _new_size: Size) {}
}

/// Trait for plugins that don't need an editor.
///
/// Implement this for plugins that don't have a GUI. This is the default
/// for the basic `Processor` trait, but can be explicitly implemented
/// to opt out of editor support.
pub trait NoEditor {}
