//! AU error types.

use std::fmt;

/// Error type for AU operations.
#[derive(Debug)]
pub enum AuError {
    /// Invalid configuration (e.g., invalid sample rate, channel count).
    InvalidConfiguration(String),

    /// Resource allocation failed (e.g., buffer allocation).
    AllocationFailed(String),

    /// Audio processing error.
    ProcessingError(String),

    /// State serialization/deserialization error.
    StateError(String),

    /// Plugin not in expected state (e.g., processing before prepare).
    InvalidState(String),

    /// Objective-C runtime error.
    #[cfg(target_os = "macos")]
    ObjcError(String),
}

impl fmt::Display for AuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(msg) => write!(f, "Invalid configuration: {}", msg),
            Self::AllocationFailed(msg) => write!(f, "Allocation failed: {}", msg),
            Self::ProcessingError(msg) => write!(f, "Processing error: {}", msg),
            Self::StateError(msg) => write!(f, "State error: {}", msg),
            Self::InvalidState(msg) => write!(f, "Invalid state: {}", msg),
            #[cfg(target_os = "macos")]
            Self::ObjcError(msg) => write!(f, "Objective-C error: {}", msg),
        }
    }
}

impl std::error::Error for AuError {}

/// Result type alias for AU operations.
pub type AuResult<T> = Result<T, AuError>;

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
