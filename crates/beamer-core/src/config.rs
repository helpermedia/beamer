//! Shared plugin configuration.
//!
//! This module provides format-agnostic plugin metadata that is shared
//! across all plugin formats (VST3, AU, CLAP, etc.).
//!
//! Format-specific configurations (UIDs, FourCC codes, etc.) are defined
//! in their respective crates.
//!
//! # Example
//!
//! ```ignore
//! use beamer_core::PluginConfig;
//!
//! pub static CONFIG: PluginConfig = PluginConfig::new("My Plugin")
//!     .with_vendor("My Company")
//!     .with_version("1.0.0")
//!     .with_sub_categories("Fx|Dynamics");
//! ```

/// Format-agnostic plugin configuration.
///
/// Contains metadata shared across all plugin formats. Format-specific
/// configurations (like VST3 UIDs or AU FourCC codes) are defined separately.
#[derive(Debug, Clone)]
pub struct PluginConfig {
    /// Plugin name displayed in the DAW.
    pub name: &'static str,

    /// Vendor/company name.
    pub vendor: &'static str,

    /// Vendor URL.
    pub url: &'static str,

    /// Vendor email.
    pub email: &'static str,

    /// Plugin version string.
    pub version: &'static str,

    /// Whether this plugin has an editor/GUI.
    pub has_editor: bool,
}

impl PluginConfig {
    /// Create a new plugin configuration with default values.
    ///
    /// # Example
    ///
    /// ```ignore
    /// pub static CONFIG: PluginConfig = PluginConfig::new("My Plugin")
    ///     .with_vendor("My Company")
    ///     .with_version(env!("CARGO_PKG_VERSION"));
    /// ```
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            vendor: "Unknown Vendor",
            url: "",
            email: "",
            version: "1.0.0",
            has_editor: false,
        }
    }

    /// Set the vendor name.
    pub const fn with_vendor(mut self, vendor: &'static str) -> Self {
        self.vendor = vendor;
        self
    }

    /// Set the vendor URL.
    pub const fn with_url(mut self, url: &'static str) -> Self {
        self.url = url;
        self
    }

    /// Set the vendor email.
    pub const fn with_email(mut self, email: &'static str) -> Self {
        self.email = email;
        self
    }

    /// Set the version string.
    pub const fn with_version(mut self, version: &'static str) -> Self {
        self.version = version;
        self
    }

    /// Enable the editor/GUI.
    pub const fn with_editor(mut self) -> Self {
        self.has_editor = true;
        self
    }
}
