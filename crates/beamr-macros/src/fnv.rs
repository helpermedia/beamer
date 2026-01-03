//! FNV-1a hash implementation for parameter ID hashing.
//!
//! This module provides a compile-time FNV-1a hash function for generating
//! stable u32 parameter IDs from string identifiers.

/// Compute FNV-1a 32-bit hash of a string.
///
/// This is a const fn, usable in const contexts and at compile time.
/// The FNV-1a hash provides good distribution and is simple to implement.
///
/// # Properties
///
/// - Deterministic: same input always produces same output
/// - Fast: simple byte-by-byte iteration
/// - No external dependencies
/// - ~4 billion possible values (2^32)
pub const fn fnv1a_32(s: &str) -> u32 {
    const FNV_OFFSET: u32 = 2166136261;
    const FNV_PRIME: u32 = 16777619;

    let bytes = s.as_bytes();
    let mut hash = FNV_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u32;
        hash = hash.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    hash
}
