//! String conversion utilities for VST3 interfaces.
//!
//! VST3 uses a mix of C-strings and wide strings (UTF-16). These utilities
//! handle the conversions safely.

use std::ffi::{c_char, CString};
use vst3::Steinberg::Vst::TChar;

/// Copy a Rust string to a C-string buffer.
///
/// Truncates if the string is too long, ensuring null-termination.
pub fn copy_cstring(src: &str, dst: &mut [c_char]) {
    if dst.is_empty() {
        return;
    }

    let c_string = CString::new(src).unwrap_or_else(|_| CString::default());
    let bytes = c_string.as_bytes_with_nul();

    for (src, dst) in bytes.iter().zip(dst.iter_mut()) {
        *dst = *src as c_char;
    }

    // Ensure null-termination if truncated
    if bytes.len() > dst.len() {
        if let Some(last) = dst.last_mut() {
            *last = 0;
        }
    }
}

/// Copy a Rust string to a wide string (UTF-16) buffer.
///
/// Truncates if the string is too long, ensuring null-termination.
pub fn copy_wstring(src: &str, dst: &mut [TChar]) {
    if dst.is_empty() {
        return;
    }

    let mut len = 0;
    for (src_char, dst_char) in src.encode_utf16().zip(dst.iter_mut()) {
        *dst_char = src_char as TChar;
        len += 1;
    }

    // Add null-terminator
    if len < dst.len() {
        dst[len] = 0;
    } else if let Some(last) = dst.last_mut() {
        *last = 0;
    }
}

/// Get the length of a null-terminated wide string.
///
/// # Safety
/// The pointer must point to a valid null-terminated wide string.
pub unsafe fn len_wstring(string: *const TChar) -> usize {
    if string.is_null() {
        return 0;
    }

    let mut len = 0;
    while *string.add(len) != 0 {
        len += 1;
    }
    len
}

/// Convert a wide string to a Rust String.
///
/// # Safety
/// The pointer must point to a valid null-terminated wide string.
pub unsafe fn wstring_to_string(string: *const TChar) -> Option<String> {
    if string.is_null() {
        return None;
    }

    let len = len_wstring(string);
    let slice = std::slice::from_raw_parts(string, len);
    String::from_utf16(slice).ok()
}
