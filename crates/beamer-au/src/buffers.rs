//! Core Audio's `AudioBufferList` as Rust types.
//!
//! `AudioBufferList` is a C-style structure with a variable number of
//! buffers. The actual buffer count is stored in `number_buffers`, which
//! needs pointer arithmetic to reach the buffers after the first one. This
//! module wraps that access. Beamer expects non-interleaved audio: each
//! `AudioBuffer` holds one channel.

use std::ffi::c_void;

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
        // SAFETY: Caller guarantees index < number_buffers. AudioBufferList is a
        // flexible array member struct where buffers follow the header contiguously.
        // The pointer arithmetic accesses valid memory within the allocated block.
        unsafe { &*buffers_ptr.add(index as usize) }
    }

    /// Get a mutable reference to the buffer at the given index.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `index < number_buffers`.
    #[inline]
    pub unsafe fn buffer_at_mut(&mut self, index: u32) -> &mut AudioBuffer {
        let buffers_ptr = self.buffers.as_mut_ptr();
        // SAFETY: Caller guarantees index < number_buffers. AudioBufferList is a
        // flexible array member struct where buffers follow the header contiguously.
        // The pointer arithmetic accesses valid memory within the allocated block.
        // Caller ensures exclusive mutable access to the buffer.
        unsafe { &mut *buffers_ptr.add(index as usize) }
    }
}
