//! Plugin wrapper configuration and metadata.

use vst3::Steinberg::TUID;

/// Default number of SysEx output slots per process block.
pub const DEFAULT_SYSEX_SLOTS: usize = 16;

/// Default SysEx buffer size in bytes per slot.
pub const DEFAULT_SYSEX_BUFFER_SIZE: usize = 512;

/// Configuration for a VST3 plugin.
///
/// This struct holds all the metadata needed to create a VST3 plugin instance.
/// Uses combined component architecture (single class for processor + controller).
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

    /// Unique ID for the audio component class.
    pub component_uid: TUID,

    /// Optional unique ID for the controller class.
    /// When `None`, the plugin uses the combined component pattern.
    pub controller_uid: Option<TUID>,

    /// Plugin category (e.g., "Fx", "Instrument").
    pub category: &'static str,

    /// Sub-categories (e.g., "Dynamics", "EQ").
    pub sub_categories: &'static str,

    /// Whether this plugin has an editor/GUI.
    pub has_editor: bool,

    /// Number of SysEx output slots per process block.
    /// Higher values support more concurrent SysEx messages but use more memory.
    pub sysex_slots: usize,

    /// Maximum size of each SysEx message in bytes.
    /// Messages larger than this will be truncated.
    pub sysex_buffer_size: usize,
}

impl PluginConfig {
    /// Create a new plugin configuration with default values.
    pub const fn new(name: &'static str, component_uid: TUID) -> Self {
        Self {
            name,
            vendor: "Unknown Vendor",
            url: "",
            email: "",
            version: "1.0.0",
            component_uid,
            controller_uid: None,
            category: "Fx",
            sub_categories: "",
            has_editor: false,
            sysex_slots: DEFAULT_SYSEX_SLOTS,
            sysex_buffer_size: DEFAULT_SYSEX_BUFFER_SIZE,
        }
    }

    /// Set the controller class UID and enable split component/controller mode.
    pub const fn with_controller(mut self, controller_uid: TUID) -> Self {
        self.controller_uid = Some(controller_uid);
        self
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

    /// Set the plugin category.
    pub const fn with_category(mut self, category: &'static str) -> Self {
        self.category = category;
        self
    }

    /// Set the sub-categories.
    pub const fn with_sub_categories(mut self, sub_categories: &'static str) -> Self {
        self.sub_categories = sub_categories;
        self
    }

    /// Enable the editor/GUI.
    pub const fn with_editor(mut self) -> Self {
        self.has_editor = true;
        self
    }

    /// Set the number of SysEx output slots per process block.
    ///
    /// Higher values allow more concurrent SysEx messages but use more memory.
    /// Default is 16 slots. For sample dumps or large property exchanges,
    /// consider increasing to 64 or more.
    pub const fn with_sysex_slots(mut self, slots: usize) -> Self {
        self.sysex_slots = slots;
        self
    }

    /// Set the maximum size of each SysEx message in bytes.
    ///
    /// Messages larger than this will be truncated.
    /// Default is 512 bytes. For large SysEx payloads, consider 2048 or 4096.
    pub const fn with_sysex_buffer_size(mut self, size: usize) -> Self {
        self.sysex_buffer_size = size;
        self
    }
}

impl PluginConfig {
    /// Returns true if a dedicated controller class is registered.
    pub const fn has_controller(&self) -> bool {
        self.controller_uid.is_some()
    }
}
