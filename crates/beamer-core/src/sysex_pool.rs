//! Pre-allocated SysEx output buffer pool for real-time safety.
//!
//! This module provides `SysExOutputPool`, which pre-allocates buffer slots
//! to avoid heap allocation during audio processing.

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
    /// Default number of SysEx slots per process block.
    pub const DEFAULT_SLOTS: usize = 16;
    /// Default maximum size per SysEx message.
    pub const DEFAULT_BUFFER_SIZE: usize = 512;

    /// Create a new pool with default capacity.
    pub fn new() -> Self {
        Self::with_capacity(Self::DEFAULT_SLOTS, Self::DEFAULT_BUFFER_SIZE)
    }

    /// Create a new pool with the specified capacity.
    ///
    /// Pre-allocates all buffers to avoid heap allocation during process().
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
    ///
    /// Note: This does NOT clear the fallback buffer, which is drained separately
    /// at the start of the next process block.
    #[inline]
    pub fn clear(&mut self) {
        self.next_slot = 0;
        self.overflowed = false;
    }

    /// Allocate a slot and copy SysEx data into it.
    ///
    /// Returns `Some((pointer, length))` on success, `None` if pool exhausted.
    /// The pointer is stable until `clear()` is called.
    ///
    /// Sets the overflow flag when the pool is exhausted.
    /// With `sysex-heap-fallback` feature: overflow messages are stored in a
    /// heap-backed fallback buffer instead of being dropped.
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
    ///
    /// Safer API for contexts that don't need raw pointers.
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
    ///
    /// These messages should be emitted at the start of the current process block.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_pool() {
        let pool = SysExOutputPool::new();
        assert_eq!(pool.capacity(), SysExOutputPool::DEFAULT_SLOTS);
        assert_eq!(pool.used(), 0);
        assert!(!pool.has_overflowed());
    }

    #[test]
    fn test_allocate() {
        let mut pool = SysExOutputPool::with_capacity(2, 64);
        let data = [0xF0, 0x41, 0x10, 0xF7];

        let result = pool.allocate(&data);
        assert!(result.is_some());
        assert_eq!(pool.used(), 1);

        let (ptr, len) = result.unwrap();
        assert_eq!(len, 4);
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        assert_eq!(slice, &data);
    }

    #[test]
    fn test_allocate_slice() {
        let mut pool = SysExOutputPool::with_capacity(2, 64);
        let data = [0xF0, 0x41, 0x10, 0xF7];

        let result = pool.allocate_slice(&data);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &data);
        assert_eq!(pool.used(), 1);
    }

    #[test]
    fn test_overflow() {
        let mut pool = SysExOutputPool::with_capacity(1, 64);
        let data = [0xF0, 0xF7];

        assert!(pool.allocate(&data).is_some());
        assert!(!pool.has_overflowed());

        assert!(pool.allocate(&data).is_none());
        assert!(pool.has_overflowed());
    }

    #[test]
    fn test_clear() {
        let mut pool = SysExOutputPool::with_capacity(1, 64);
        let data = [0xF0, 0xF7];

        pool.allocate(&data);
        pool.allocate(&data); // Overflow
        assert!(pool.has_overflowed());
        assert_eq!(pool.used(), 1);

        pool.clear();
        assert!(!pool.has_overflowed());
        assert_eq!(pool.used(), 0);
    }

    #[test]
    fn test_truncation() {
        let mut pool = SysExOutputPool::with_capacity(1, 4);
        let data = [0xF0, 0x41, 0x10, 0x42, 0x00, 0xF7]; // 6 bytes

        let result = pool.allocate_slice(&data);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 4); // Truncated to buffer size
    }
}
