//! Error handling helpers for Audio Unit implementation.
//!
//! This module provides centralized error handling utilities including:
//! - NSError creation for Objective-C interop
//! - Default/fallback constants
//! - Error code mapping
//!
//! Consolidates error handling patterns that were previously duplicated
//! across multiple files.

use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2::{class, msg_send};
use objc2_foundation::{NSError, NSString};

use crate::error::{os_status, AuError};

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
// NSError Creation
// =============================================================================

/// Create an NSError from an error code and message.
///
/// This is used to return errors to the AU host via Objective-C methods.
///
/// # Arguments
///
/// * `code` - OS status error code
/// * `message` - Human-readable error description
///
/// # Returns
///
/// A retained NSError object, or None if creation fails.
///
/// # Example
///
/// ```ignore
/// let error = create_ns_error(
///     os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED,
///     "Plugin not initialized"
/// );
/// ```
pub fn create_ns_error(code: i32, message: &str) -> Option<Retained<NSError>> {
    // SAFETY: All msg_send! calls are to Objective-C methods with correct signatures.
    // NSString::from_str and class! are safe to call from any thread. The dictionary
    // and error objects are created and returned in a single unsafe block, ensuring
    // no invalid state escapes. We create a full NSError with userInfo before returning.
    unsafe {
        let domain = NSString::from_str("com.beamer.audiounit");
        let description_key = NSString::from_str("NSLocalizedDescriptionKey");
        let description_value = NSString::from_str(message);

        // Create userInfo dictionary
        let dict_class: &AnyClass = class!(NSMutableDictionary);
        let user_info: Retained<objc2::runtime::AnyObject> = msg_send![dict_class, new];
        let _: () = msg_send![
            &user_info,
            setObject: &*description_value,
            forKey: &*description_key
        ];

        // Create NSError
        let error_class: &AnyClass = class!(NSError);
        msg_send![
            error_class,
            errorWithDomain: &*domain,
            code: code as isize,
            userInfo: &*user_info
        ]
    }
}

/// Helper to set an NSError pointer and return Bool::NO.
///
/// This pattern is used extensively in AU allocation methods.
///
/// # Safety
///
/// The caller must ensure that `error` is either null or points to a valid
/// mutable pointer that can receive the NSError.
///
/// # Example
///
/// ```ignore
/// if plugin.is_none() {
///     return set_error_and_fail(
///         error,
///         os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED,
///         "Plugin not initialized"
///     );
/// }
/// ```
pub unsafe fn set_error_and_fail(
    error: *mut *mut NSError,
    code: i32,
    message: &str,
) -> objc2::runtime::Bool {
    if !error.is_null() {
        *error = create_ns_error(code, message)
            .map(Retained::into_raw)
            .unwrap_or(std::ptr::null_mut());
    }
    objc2::runtime::Bool::NO
}

// =============================================================================
// Error Code Conversion
// =============================================================================

/// Convert AuError to OS status code.
///
/// Maps Beamer's error types to appropriate AU error codes.
impl AuError {
    pub fn to_os_status(&self) -> i32 {
        match self {
            AuError::InvalidConfiguration(_) => os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
            AuError::AllocationFailed(_) => os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED,
            AuError::ProcessingError(_) => os_status::K_AUDIO_UNIT_ERR_RENDER,
            AuError::StateError(_) => os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
            AuError::InvalidState(_) => os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED,
            #[cfg(target_os = "macos")]
            AuError::ObjcError(_) => os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
        }
    }

    /// Convert AuError to NSError.
    ///
    /// Convenience method that combines error code mapping and NSError creation.
    pub fn to_ns_error(&self) -> Option<Retained<NSError>> {
        create_ns_error(self.to_os_status(), &self.to_string())
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
        let err = AuError::InvalidConfiguration("test".to_string());
        assert_eq!(
            err.to_os_status(),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY
        );

        let err = AuError::ProcessingError("test".to_string());
        assert_eq!(err.to_os_status(), os_status::K_AUDIO_UNIT_ERR_RENDER);
    }

    #[test]
    fn test_create_ns_error() {
        let error = create_ns_error(os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED, "Test error");
        assert!(error.is_some());
    }
}
