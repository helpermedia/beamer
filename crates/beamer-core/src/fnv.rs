//! FNV-1a hash function for parameter IDs.
//!
//! This is used to generate stable u32 IDs from string parameter identifiers.

/// Compute FNV-1a 32-bit hash of a string.
///
/// This is a simple, fast hash function that produces consistent results
/// across platforms. Used to generate VST3 parameter IDs from string names.
#[inline]
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
