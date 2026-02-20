//! GUI-related traits.

use crate::types::Size;

/// Size constraints for the plugin GUI.
#[derive(Debug, Clone, Copy)]
pub struct GuiConstraints {
    /// Minimum size.
    pub min: Size,
    /// Maximum size.
    pub max: Size,
    /// Whether the GUI is resizable.
    pub resizable: bool,
}

impl Default for GuiConstraints {
    fn default() -> Self {
        Self {
            min: Size::new(400, 300),
            max: Size::new(1600, 1200),
            resizable: true,
        }
    }
}

/// Trait for plugin GUI callbacks.
///
/// Implement this trait to provide GUI-related configuration and callbacks.
/// The actual WebView creation and management is handled by the framework;
/// this trait just provides configuration and lifecycle hooks.
pub trait GuiDelegate: Send + Sync {
    /// Get the initial GUI size.
    ///
    /// This is the size the plugin window will have when first opened.
    fn gui_size(&self) -> Size;

    /// Get the GUI size constraints.
    ///
    /// These constraints determine the minimum and maximum sizes the GUI
    /// can be resized to and whether resizing is allowed at all.
    fn gui_constraints(&self) -> GuiConstraints {
        GuiConstraints::default()
    }

    /// Called when the GUI is opened.
    ///
    /// Use this to initialize any GUI-specific state.
    fn gui_opened(&mut self) {}

    /// Called when the GUI is closed.
    ///
    /// Use this to clean up GUI-specific state.
    fn gui_closed(&mut self) {}

    /// Called when the GUI is resized.
    ///
    /// The new size has already been constrained to the GUI constraints.
    fn gui_resized(&mut self, _new_size: Size) {}
}

/// Trait for plugins that don't need a GUI.
///
/// Implement this for plugins that don't have a GUI. This is the default
/// for the basic `Processor` trait, but can be explicitly implemented
/// to opt out of GUI support.
pub trait NoGui {}
