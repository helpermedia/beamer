//! Serde structs for Config.toml and Presets.toml.

use std::collections::HashMap;

use serde::Deserialize;

/// Plugin configuration from Config.toml.
#[derive(Deserialize)]
pub struct ConfigFile {
    /// Plugin display name.
    pub name: String,
    /// Plugin category: "effect", "instrument", "midi_effect", "generator".
    pub category: String,
    /// 4-character manufacturer code.
    pub manufacturer_code: String,
    /// 4-character plugin code.
    pub plugin_code: String,
    /// Vendor display name.
    pub vendor: Option<String>,
    /// Plugin URL.
    pub url: Option<String>,
    /// Support email.
    pub email: Option<String>,
    /// Subcategory strings (e.g., ["dynamics", "eq"]).
    pub subcategories: Option<Vec<String>>,
    /// Explicit Vst3 UUID override (format: "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX").
    pub vst3_id: Option<String>,
    /// Explicit Vst3 controller UUID for split component/controller architecture (format: "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX").
    pub vst3_controller_id: Option<String>,
    /// Whether the plugin has a GUI editor.
    pub has_editor: Option<bool>,
    /// Number of SysEx output slots per process block (default: 16).
    #[serde(default)]
    pub sysex_slots: Option<usize>,
    /// Maximum size of each SysEx message in bytes (default: 512).
    #[serde(default)]
    pub sysex_buffer_size: Option<usize>,
}

/// Presets file from Presets.toml.
#[derive(Deserialize)]
pub struct PresetsFile {
    /// List of preset definitions.
    pub preset: Vec<PresetEntry>,
}

/// A single preset definition.
#[derive(Deserialize)]
pub struct PresetEntry {
    /// Display name shown in the DAW's preset browser.
    pub name: String,
    /// Parameter values (parameter_id -> plain value).
    #[serde(flatten)]
    pub values: HashMap<String, toml::Value>,
}

impl ConfigFile {
    /// Validate the config file contents.
    pub fn validate(&self) -> Result<(), String> {
        if self.manufacturer_code.len() != 4 || !self.manufacturer_code.is_ascii() {
            return Err(format!(
                "manufacturer_code must be exactly 4 ASCII characters, got {:?}",
                self.manufacturer_code
            ));
        }
        if self.plugin_code.len() != 4 || !self.plugin_code.is_ascii() {
            return Err(format!(
                "plugin_code must be exactly 4 ASCII characters, got {:?}",
                self.plugin_code
            ));
        }
        let valid_categories = ["effect", "instrument", "midi_effect", "generator"];
        if !valid_categories.contains(&self.category.as_str()) {
            return Err(format!(
                "category must be one of {:?}, got {:?}",
                valid_categories, self.category
            ));
        }
        Ok(())
    }
}
