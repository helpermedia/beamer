//! Error handling helpers for Audio Unit implementation.
//!
//! This module provides centralized error handling utilities including:
//! - Default/fallback constants
//! - Error code mapping
//!
//! In the hybrid ObjC/Rust architecture, NSError creation is handled by the
//! Objective-C wrapper. This module provides Rust-side error code conversion.

use crate::error::{os_status, PluginError};

// =============================================================================
// Default/Fallback Constants
// =============================================================================

/// Default sample rate when AU format query fails.
pub const DEFAULT_SAMPLE_RATE: f64 = 44100.0;

/// Default maximum frames per render when AU property query fails.
pub const DEFAULT_MAX_FRAMES: u32 = 1024;

/// Default channel count for fallback configurations.
pub const DEFAULT_CHANNEL_COUNT: usize = 2;

// =============================================================================
// Error Code Conversion
// =============================================================================

/// Convert PluginError to OS status code.
///
/// Maps Beamer's error types to appropriate AU error codes.
pub fn plugin_error_to_os_status(err: &PluginError) -> i32 {
    match err {
        PluginError::InitializationFailed(_) => os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED,
        PluginError::ProcessingError(_) => os_status::K_AUDIO_UNIT_ERR_RENDER,
        PluginError::StateError(_) => os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
        PluginError::EditorError(_) => os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
        PluginError::PlatformError(_) => os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
        PluginError::WebViewError(_) => os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(DEFAULT_SAMPLE_RATE, 44100.0);
        assert_eq!(DEFAULT_MAX_FRAMES, 1024);
        assert_eq!(DEFAULT_CHANNEL_COUNT, 2);
    }

    #[test]
    fn test_error_code_mapping() {
        let err = PluginError::InitializationFailed("test".to_string());
        assert_eq!(
            plugin_error_to_os_status(&err),
            os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED
        );

        let err = PluginError::ProcessingError("test".to_string());
        assert_eq!(plugin_error_to_os_status(&err), os_status::K_AUDIO_UNIT_ERR_RENDER);
    }
}
