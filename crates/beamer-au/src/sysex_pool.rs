//! Pre-allocated SysEx output buffer pool for real-time safety.
//!
//! Mirrors VST3's SysExOutputPool - provides stable memory for SysEx messages
//! that need to be passed to the AU host during render.

/// Pre-allocated pool for SysEx output messages.
///
/// Avoids heap allocation during audio processing by pre-allocating
/// a fixed number of buffer slots at initialization time.
pub struct SysExOutputPool {
    /// Pre-allocated buffer slots for SysEx data
    buffers: Vec<Vec<u8>>,
    /// Length of valid data in each slot
    lengths: Vec<usize>,
    /// Maximum number of slots
    max_slots: usize,
    /// Maximum buffer size per slot
    max_buffer_size: usize,
    /// Next available slot index
    next_slot: usize,
    /// Set to true when an allocation fails due to pool exhaustion
    overflowed: bool,
    /// Heap-backed fallback buffer for overflow (only when feature enabled).
    #[cfg(feature = "sysex-heap-fallback")]
    fallback: Vec<Vec<u8>>,
}

impl SysExOutputPool {
    /// Default number of SysEx slots per process block
    pub const DEFAULT_SLOTS: usize = 16;
    /// Default maximum size per SysEx message
    pub const DEFAULT_BUFFER_SIZE: usize = 512;

    /// Create a new pool with default capacity.
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_SLOTS, Self::DEFAULT_BUFFER_SIZE)
    }

    /// Create a new pool with the specified capacity.
    pub fn with_capacity(slots: usize, buffer_size: usize) -> Self {
        let mut buffers = Vec::with_capacity(slots);
        for _ in 0..slots {
            buffers.push(vec![0u8; buffer_size]);
        }
        let lengths = vec![0usize; slots];

        Self {
            buffers,
            lengths,
            max_slots: slots,
            max_buffer_size: buffer_size,
            next_slot: 0,
            overflowed: false,
            #[cfg(feature = "sysex-heap-fallback")]
            fallback: Vec::new(),
        }
    }

    /// Clear the pool for reuse. O(1) operation.
    #[inline]
    pub fn clear(&mut self) {
        self.next_slot = 0;
        self.overflowed = false;
    }

    /// Allocate a slot and copy SysEx data into it.
    ///
    /// Returns `Some((pointer, length))` on success, `None` if pool exhausted.
    /// The pointer is stable until `clear()` is called.
    pub fn allocate(&mut self, data: &[u8]) -> Option<(*const u8, usize)> {
        if self.next_slot >= self.max_slots {
            self.overflowed = true;

            #[cfg(feature = "sysex-heap-fallback")]
            {
                let copy_len = data.len().min(self.max_buffer_size);
                self.fallback.push(data[..copy_len].to_vec());
            }

            return None;
        }

        let slot = self.next_slot;
        self.next_slot += 1;

        let copy_len = data.len().min(self.max_buffer_size);
        self.buffers[slot][..copy_len].copy_from_slice(&data[..copy_len]);
        self.lengths[slot] = copy_len;

        Some((self.buffers[slot].as_ptr(), copy_len))
    }

    /// Allocate and return a slice reference instead of raw pointer.
    pub fn allocate_slice(&mut self, data: &[u8]) -> Option<&[u8]> {
        if self.next_slot >= self.max_slots {
            self.overflowed = true;

            #[cfg(feature = "sysex-heap-fallback")]
            {
                let copy_len = data.len().min(self.max_buffer_size);
                self.fallback.push(data[..copy_len].to_vec());
            }

            return None;
        }

        let slot = self.next_slot;
        self.next_slot += 1;

        let copy_len = data.len().min(self.max_buffer_size);
        self.buffers[slot][..copy_len].copy_from_slice(&data[..copy_len]);
        self.lengths[slot] = copy_len;

        Some(&self.buffers[slot][..copy_len])
    }

    /// Check if the pool overflowed during this block.
    #[inline]
    pub fn has_overflowed(&self) -> bool {
        self.overflowed
    }

    /// Get the pool's slot capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.max_slots
    }

    /// Get number of slots currently used.
    #[inline]
    pub fn used(&self) -> usize {
        self.next_slot
    }

    /// Check if fallback buffer has pending messages (feature-gated).
    #[cfg(feature = "sysex-heap-fallback")]
    #[inline]
    pub fn has_fallback(&self) -> bool {
        !self.fallback.is_empty()
    }

    /// Take ownership of fallback messages (feature-gated).
    #[cfg(feature = "sysex-heap-fallback")]
    #[inline]
    pub fn take_fallback(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.fallback)
    }
}

impl Default for SysExOutputPool {
    fn default() -> Self {
        Self::new()
    }
}
