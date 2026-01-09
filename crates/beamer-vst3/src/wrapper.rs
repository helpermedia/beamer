//! VST3-specific plugin configuration.
//!
//! This module provides VST3-specific configuration that complements
//! the shared [`beamer_core::PluginConfig`].

use vst3::Steinberg::TUID;

/// Default number of SysEx output slots per process block.
pub const DEFAULT_SYSEX_SLOTS: usize = 16;

/// Default SysEx buffer size in bytes per slot.
pub const DEFAULT_SYSEX_BUFFER_SIZE: usize = 512;

/// VST3-specific plugin configuration.
///
/// This struct holds VST3-specific metadata. Use in combination with
/// [`beamer_core::PluginConfig`] for complete plugin configuration.
///
/// # Example
///
/// ```ignore
/// use beamer_core::PluginConfig;
/// use beamer_vst3::{Vst3Config, vst3};
///
/// const COMPONENT_UID: vst3::Steinberg::TUID =
///     vst3::uid(0xDCDDB4BA, 0x2D6A4EC3, 0xA526D3E7, 0x244FAAE3);
///
/// pub static CONFIG: PluginConfig = PluginConfig::new("Beamer Gain")
///     .with_vendor("Beamer Framework")
///     .with_version(env!("CARGO_PKG_VERSION"));
///
/// pub static VST3_CONFIG: Vst3Config = Vst3Config::new(COMPONENT_UID);
///
/// export_vst3!(CONFIG, VST3_CONFIG, Vst3Processor<GainPlugin>);
/// ```
pub struct Vst3Config {
    /// Unique ID for the audio component class.
    pub component_uid: TUID,

    /// Optional unique ID for the controller class.
    /// When `None`, the plugin uses the combined component pattern.
    pub controller_uid: Option<TUID>,

    /// Number of SysEx output slots per process block.
    /// Higher values support more concurrent SysEx messages but use more memory.
    pub sysex_slots: usize,

    /// Maximum size of each SysEx message in bytes.
    /// Messages larger than this will be truncated.
    pub sysex_buffer_size: usize,
}

impl Vst3Config {
    /// Create a new VST3 configuration with default values.
    ///
    /// Uses combined component architecture (single class for processor + controller).
    pub const fn new(component_uid: TUID) -> Self {
        Self {
            component_uid,
            controller_uid: None,
            sysex_slots: DEFAULT_SYSEX_SLOTS,
            sysex_buffer_size: DEFAULT_SYSEX_BUFFER_SIZE,
        }
    }

    /// Set the controller class UID and enable split component/controller mode.
    pub const fn with_controller(mut self, controller_uid: TUID) -> Self {
        self.controller_uid = Some(controller_uid);
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

    /// Returns true if a dedicated controller class is registered.
    pub const fn has_controller(&self) -> bool {
        self.controller_uid.is_some()
    }
}
