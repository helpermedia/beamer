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
    // SAFETY: Caller guarantees string points to valid null-terminated wide string.
    // We iterate until we find the null terminator.
    while unsafe { *string.add(len) } != 0 {
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

    // SAFETY: len_wstring requires a valid null-terminated wide string, which
    // the caller guarantees.
    let len = unsafe { len_wstring(string) };
    // SAFETY: Caller guarantees string is valid for at least `len` elements.
    let slice = unsafe { std::slice::from_raw_parts(string, len) };
    String::from_utf16(slice).ok()
}

/// Null-terminated wide strings (UTF-16) for a host, in memory allocated
/// once.
///
/// A stored string stays where it is, unchanged, until [`clear`]: a host may
/// keep its pointer until then. Storing and clearing never allocate.
///
/// [`clear`]: Self::clear
pub struct Utf16Pool {
    /// The units of the strings, each followed by a 0. Its length is set at
    /// creation and never changes, so the units do not move while the pool
    /// lives. It is written and read only through `as_mut_ptr` and
    /// `as_ptr`, which do not claim the whole allocation, so storing a new
    /// string leaves the pointers already handed out valid.
    units: Vec<TChar>,
    /// Units of `units` in use.
    len: usize,
    /// Set when a string did not fit since the last clear.
    overflowed: bool,
}

impl Utf16Pool {
    /// Create an empty pool of `units` units. Allocates; not for the audio
    /// thread.
    pub fn new(units: usize) -> Self {
        Self {
            units: vec![0; units],
            len: 0,
            overflowed: false,
        }
    }

    /// Remove every string. The pointers handed out before are no longer
    /// valid.
    pub fn clear(&mut self) {
        self.len = 0;
        self.overflowed = false;
    }

    /// Returns true if a string did not fit since the last clear.
    pub fn has_overflowed(&self) -> bool {
        self.overflowed
    }

    /// Copy `text` into the pool as UTF-16 followed by a 0. Returns a
    /// pointer to the copy and its length in units, without the 0.
    ///
    /// Returns `None` and notes the overflow when the string does not fit
    /// into what is left of the pool; a string is never cut.
    pub fn store(&mut self, text: &str) -> Option<(*const TChar, u32)> {
        let len = text.encode_utf16().count();
        let start = self.len;
        // The string and its 0.
        if len >= self.units.len() - start {
            self.overflowed = true;
            return None;
        }
        let base = self.units.as_mut_ptr();
        // SAFETY: `start + len + 1` is within the pool. The range lies past
        // every stored string, so no pointer handed out covers it.
        unsafe {
            for (index, unit) in text.encode_utf16().enumerate() {
                base.add(start + index).write(unit as TChar);
            }
            base.add(start + len).write(0);
        }
        self.len = start + len + 1;
        // SAFETY: `start` is within the pool, which never moves.
        Some((unsafe { self.units.as_ptr().add(start) }, len as u32))
    }
}
