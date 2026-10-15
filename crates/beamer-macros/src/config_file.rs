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
    /// Explicit VST3 UUID override (format: "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX").
    pub vst3_id: Option<String>,
    /// Explicit VST3 controller UUID for split component/controller architecture (format: "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX").
    pub vst3_controller_id: Option<String>,
    /// Whether the plugin has a GUI.
    pub has_gui: Option<bool>,
    /// Initial GUI size as [width, height] in pixels.
    pub gui_size: Option<(u32, u32)>,
    /// Number of SysEx output slots per process block (default: 16).
    #[serde(default)]
    pub sysex_slots: Option<usize>,
    /// Maximum size of each SysEx message in bytes (default: 512).
    #[serde(default)]
    pub sysex_buffer_size: Option<usize>,
    /// Background color shown while web content loads (hex string, e.g. "#1a1a2e").
    #[serde(default)]
    pub gui_background_color: Option<String>,
    /// How much the plugin logs to the system log: "error", "warn", "info",
    /// "debug" or "trace". Without it the plugin installs no logger.
    #[serde(default)]
    pub log_level: Option<String>,
}

/// The `LogLevel` variant for a `log_level` value.
pub fn log_level_variant(level: &str) -> Result<&'static str, String> {
    match level {
        "error" => Ok("Error"),
        "warn" => Ok("Warn"),
        "info" => Ok("Info"),
        "debug" => Ok("Debug"),
        "trace" => Ok("Trace"),
        _ => Err(format!(
            "log_level must be \"error\", \"warn\", \"info\", \"debug\" or \"trace\", got {level:?}"
        )),
    }
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

/// Category names in Config.toml and the `Category` variant each one selects.
pub(crate) const CATEGORIES: &[(&str, &str)] = &[
    ("effect", "Effect"),
    ("instrument", "Instrument"),
    ("midi_effect", "MidiEffect"),
    ("generator", "Generator"),
];

/// Look up the `Category` variant for a category name from Config.toml.
pub(crate) fn category_variant(category: &str) -> Result<&'static str, String> {
    match CATEGORIES.iter().find(|(name, _)| *name == category) {
        Some((_, variant)) => Ok(variant),
        None => {
            let names: Vec<&str> = CATEGORIES.iter().map(|(name, _)| *name).collect();
            Err(format!("category must be one of {:?}, got {:?}", names, category))
        }
    }
}

fn validate_uuid(uuid: &str, field: &str) -> Result<(), String> {
    if uuid.len() != 36 {
        return Err(format!(
            "{field} must be a UUID in format XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX, got {:?}",
            uuid
        ));
    }
    for (i, c) in uuid.chars().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if c != '-' {
                    return Err(format!(
                        "{field} has invalid character at position {i}: expected '-', got '{c}'"
                    ));
                }
            }
            _ => {
                if !c.is_ascii_hexdigit() {
                    return Err(format!(
                        "{field} has invalid character at position {i}: expected hex digit, got '{c}'"
                    ));
                }
            }
        }
    }
    Ok(())
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
        category_variant(&self.category)?;
        if let Some(ref level) = self.log_level {
            log_level_variant(level)?;
        }
        if let Some(ref id) = self.vst3_id {
            validate_uuid(id, "vst3_id")?;
        }
        if let Some(ref id) = self.vst3_controller_id {
            validate_uuid(id, "vst3_controller_id")?;
        }
        if self.has_gui == Some(true) && self.gui_size.is_none() {
            return Err(
                "gui_size is required when has_gui is true (e.g. gui_size = [400, 300])"
                    .to_string(),
            );
        }
        Ok(())
    }
}
