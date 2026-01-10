//! AU error types.
//!
//! This module re-exports `PluginError` and `PluginResult` from `beamer_core`
//! for consistency across plugin formats. It also provides AU-specific OS status
//! code constants and conversion utilities.

// Re-export core error types for use throughout beamer-au
pub use beamer_core::{PluginError, PluginResult};

// OSStatus error codes commonly used in Audio Unit
#[cfg(target_os = "macos")]
pub mod os_status {
    /// No error.
    pub const NO_ERR: i32 = 0;

    /// Unspecified error.
    pub const K_AUDIO_UNIT_ERR_INVALID_PROPERTY: i32 = -10879;

    /// Invalid property value.
    pub const K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE: i32 = -10851;

    /// Invalid parameter.
    pub const K_AUDIO_UNIT_ERR_INVALID_PARAMETER: i32 = -10878;

    /// Property not writable.
    pub const K_AUDIO_UNIT_ERR_PROPERTY_NOT_WRITABLE: i32 = -10850;

    /// Uninitialized.
    pub const K_AUDIO_UNIT_ERR_UNINITIALIZED: i32 = -10867;

    /// Cannot do in current context.
    pub const K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT: i32 = -10863;

    /// Render operation failed.
    pub const K_AUDIO_UNIT_ERR_RENDER: i32 = -10877;

    /// Too many frames to process.
    pub const K_AUDIO_UNIT_ERR_TOO_MANY_FRAMES_TO_PROCESS: i32 = -10874;

    /// Invalid file.
    pub const K_AUDIO_UNIT_ERR_INVALID_FILE: i32 = -10871;

    /// Unknown file type.
    pub const K_AUDIO_UNIT_ERR_UNKNOWN_FILE_TYPE: i32 = -10870;

    /// File not specified.
    pub const K_AUDIO_UNIT_ERR_FILE_NOT_SPECIFIED: i32 = -10869;

    /// Format not supported.
    pub const K_AUDIO_UNIT_ERR_FORMAT_NOT_SUPPORTED: i32 = -10868;

    /// Invalid element.
    pub const K_AUDIO_UNIT_ERR_INVALID_ELEMENT: i32 = -10877;

    /// Invalid scope.
    pub const K_AUDIO_UNIT_ERR_INVALID_SCOPE: i32 = -10866;
}
