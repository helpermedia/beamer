//! Audio buffer conversion and validation for Audio Unit.
//!
//! This module provides types and functions for converting between Core Audio's
//! `AudioBufferList` and Beamer's `Buffer` types. It handles the complexity of
//! accessing flexible array member structs safely and validates buffer properties.
//!
//! # Buffer Format
//!
//! `AudioBufferList` is a C-style structure with a variable number of buffers.
//! The actual buffer count is stored in `number_buffers`, requiring pointer
//! arithmetic to access buffers beyond the first one. This module encapsulates
//! that unsafe access with proper bounds checking.
//!
//! # Supported Audio Formats
//!
//! Beamer expects non-interleaved float (f32) audio:
//! - Each `AudioBuffer` represents one channel
//! - `number_channels` must be 1 (otherwise the buffer is skipped)
//! - Data is validated for proper f32 alignment (4-byte) and size
//!
//! If interleaved audio is encountered, a one-time warning is logged and those
//! buffers are skipped. Interleaved audio support is not currently planned.
//!
//! # Safety
//!
//! Public functions are marked `unsafe` because they operate on C pointers and
//! assume the caller has validated lifetimes and accessibility. The callers
//! (typically the render callback) are responsible for ensuring buffers remain
//! valid for the entire processing operation.

use std::ffi::c_void;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};

// Static flag to ensure interleaved audio warning only logs once per session
static INTERLEAVED_WARNING_LOGGED: AtomicBool = AtomicBool::new(false);

/// Core Audio AudioBuffer structure.
///
/// Represents a single buffer of audio data.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioBuffer {
    /// Number of interleaved channels in the buffer.
    /// For non-interleaved audio, this is typically 1.
    pub number_channels: u32,
    /// Size of the buffer in bytes.
    pub data_byte_size: u32,
    /// Pointer to the audio data.
    pub data: *mut c_void,
}

/// Core Audio AudioBufferList structure.
///
/// Contains a variable number of AudioBuffer structures.
/// This is a flexible array member pattern - the actual size
/// depends on `number_buffers`.
#[repr(C)]
pub struct AudioBufferList {
    /// Number of buffers in the list.
    pub number_buffers: u32,
    /// First buffer (actual array continues beyond this).
    /// Use `buffer_at()` for safe access.
    pub buffers: [AudioBuffer; 1],
}

impl AudioBufferList {
    /// Get a reference to the buffer at the given index.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `index < number_buffers`.
    #[inline]
    pub unsafe fn buffer_at(&self, index: u32) -> &AudioBuffer {
        let buffers_ptr = self.buffers.as_ptr();
        &*buffers_ptr.add(index as usize)
    }

    /// Get a mutable reference to the buffer at the given index.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `index < number_buffers`.
    #[inline]
    pub unsafe fn buffer_at_mut(&mut self, index: u32) -> &mut AudioBuffer {
        let buffers_ptr = self.buffers.as_mut_ptr();
        &mut *buffers_ptr.add(index as usize)
    }
}

/// Validate buffer alignment and size for f32 data.
///
/// Returns true if the buffer is properly aligned and sized for f32 access.
#[inline]
fn validate_f32_buffer(data_ptr: *const c_void, byte_size: u32) -> bool {
    if data_ptr.is_null() {
        return false;
    }

    // Check f32 alignment (4 bytes)
    let align_offset = (data_ptr as usize) % std::mem::align_of::<f32>();
    if align_offset != 0 {
        log::warn!(
            "Buffer not aligned for f32 access (offset: {})",
            align_offset
        );
        return false;
    }

    // Check size is multiple of f32
    if !(byte_size as usize).is_multiple_of(std::mem::size_of::<f32>()) {
        log::warn!("Buffer size not multiple of f32 size");
        return false;
    }

    true
}

/// Convert an AudioBufferList to input channel slices.
///
/// # Safety
///
/// - `buffer_list` must be a valid pointer to an AudioBufferList
/// - The audio data must remain valid for the lifetime `'a`
/// - `num_samples` must not exceed the actual buffer sizes
///
/// # Returns
///
/// A vector of immutable f32 slices, one per channel.
/// Only non-interleaved buffers are supported. Interleaved buffers
/// are skipped with a warning.
#[inline]
pub unsafe fn input_buffer_list_to_slices<'a>(
    buffer_list: *const AudioBufferList,
    num_samples: usize,
) -> Vec<&'a [f32]> {
    if buffer_list.is_null() {
        return Vec::new();
    }

    let list = &*buffer_list;
    let mut channels = Vec::with_capacity(list.number_buffers as usize);

    for i in 0..list.number_buffers {
        let buffer = list.buffer_at(i);

        // Validate alignment and size
        if !validate_f32_buffer(buffer.data, buffer.data_byte_size) {
            continue;
        }

        if buffer.number_channels == 1 {
            // Non-interleaved: one channel per buffer (expected)
            let data_ptr = buffer.data as *const f32;
            let actual_samples =
                (buffer.data_byte_size as usize / std::mem::size_of::<f32>()).min(num_samples);
            channels.push(slice::from_raw_parts(data_ptr, actual_samples));
        } else if buffer.number_channels > 1 {
            // Interleaved audio: not supported
            // Log warning only once to avoid spam
            if !INTERLEAVED_WARNING_LOGGED.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "Interleaved audio buffer with {} channels - not supported, skipping. \
                     This warning will only appear once per session.",
                    buffer.number_channels
                );
            }
        }
    }

    channels
}

/// Convert an AudioBufferList to mutable output channel slices.
///
/// # Safety
///
/// - `buffer_list` must be a valid pointer to an AudioBufferList
/// - The audio data must remain valid for the lifetime `'a`
/// - `num_samples` must not exceed the actual buffer sizes
/// - The caller must ensure exclusive access to the buffer data
///
/// # Returns
///
/// A vector of mutable f32 slices, one per channel.
/// Only non-interleaved buffers are supported. Interleaved buffers
/// are skipped with a warning.
#[inline]
pub unsafe fn output_buffer_list_to_slices<'a>(
    buffer_list: *mut AudioBufferList,
    num_samples: usize,
) -> Vec<&'a mut [f32]> {
    if buffer_list.is_null() {
        return Vec::new();
    }

    let list = &mut *buffer_list;
    let mut channels = Vec::with_capacity(list.number_buffers as usize);

    for i in 0..list.number_buffers {
        let buffer = list.buffer_at_mut(i);

        // Validate alignment and size
        if !validate_f32_buffer(buffer.data, buffer.data_byte_size) {
            continue;
        }

        if buffer.number_channels == 1 {
            // Non-interleaved: one channel per buffer (expected)
            let data_ptr = buffer.data as *mut f32;
            let actual_samples =
                (buffer.data_byte_size as usize / std::mem::size_of::<f32>()).min(num_samples);
            channels.push(slice::from_raw_parts_mut(data_ptr, actual_samples));
        } else if buffer.number_channels > 1 {
            // Interleaved audio: not supported
            // Log warning only once to avoid spam
            if !INTERLEAVED_WARNING_LOGGED.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "Interleaved audio buffer with {} channels - not supported, skipping. \
                     This warning will only appear once per session.",
                    buffer.number_channels
                );
            }
        }
    }

    channels
}
