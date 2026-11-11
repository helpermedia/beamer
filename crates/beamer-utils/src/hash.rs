//! Hash functions for stable ID generation.
//!
//! This module provides hash functions used throughout the Beamer framework
//! for generating stable parameter IDs from string identifiers.

/// Compute FNV-1a 32-bit hash of a string.
///
/// This is a simple, fast hash function that produces consistent results
/// across platforms and compiler versions. It is used to generate VST3
/// parameter IDs from string identifiers.
///
/// # Properties
///
/// - **Deterministic**: Same input always produces the same output
/// - **Fast**: Simple byte-by-byte iteration with no memory allocations
/// - **Const-friendly**: Can be evaluated at compile time
/// - **No dependencies**: Pure Rust implementation
/// - **32-bit output**: ~4 billion possible values (2^32)
///
/// # Algorithm
///
/// FNV-1a (Fowler-Noll-Vo) is a non-cryptographic hash function:
///
/// 1. Start with FNV offset basis: `2166136261`
/// 2. For each byte: XOR with hash, then multiply by FNV prime `16777619`
/// 3. Return final hash value
///
/// # Examples
///
/// ```
/// use beamer_utils::fnv1a_32;
///
/// // Runtime usage
/// let id = fnv1a_32("gain");
///
/// // Compile-time usage
/// const GAIN_ID: u32 = fnv1a_32("gain");
/// ```
///
/// # Collision Handling
///
/// Two different strings can have the same hash. `#[beamer::model]`
/// and `#[beamer::group]` report that at compile time for the
/// parameter ids of one struct. The format wrappers check the ids of the
/// whole model when a plugin instance is created.
///
/// # VST3 Integration
///
/// VST3 parameter IDs are `uint32` values. This function maps the path of a
/// parameter (like `"cutoff"` or `"filter/resonance"`) to the numeric ID
/// that VST3 expects.
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

/// FNV-1a 32-bit hash of `s`, kept to 31 bits so that it fits a positive
/// `i32`.
///
/// Parameter group ids use it: VST3 unit ids are `int32` values, kept
/// positive since -1 means "no parent" for a VST3 unit.
#[inline]
pub const fn fnv1a_31(s: &str) -> i32 {
    (fnv1a_32(s) & 0x7FFF_FFFF) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fnv1a_empty() {
        // FNV-1a of empty string is the offset basis
        assert_eq!(fnv1a_32(""), 2166136261);
    }

    #[test]
    fn test_fnv1a_single_char() {
        // Known test vector for "a"
        assert_eq!(fnv1a_32("a"), 0xe40c292c);
    }

    #[test]
    fn test_fnv1a_const() {
        // Ensure it works in const context
        const HASH: u32 = fnv1a_32("test");
        assert_eq!(HASH, fnv1a_32("test"));
    }

    #[test]
    fn test_fnv1a_consistency() {
        // Same input produces same output
        let id1 = fnv1a_32("cutoff");
        let id2 = fnv1a_32("cutoff");
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_fnv1a_different_strings() {
        // Different inputs produce different outputs
        let id1 = fnv1a_32("gain");
        let id2 = fnv1a_32("frequency");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_fnv1a_31_drops_the_top_bit() {
        // Hosts know groups by these values, so they must never change.
        // The 32-bit hashes of "Filter" and "envelope" have the top bit set.
        assert_eq!(fnv1a_32("Filter"), 0xf4a9c097);
        assert_eq!(fnv1a_31("Filter"), 0x74a9c097);
        assert_eq!(fnv1a_31("envelope"), 0x7a10a755);
        assert_eq!(fnv1a_31("first/envelope"), 0x2d8a4bfa);
        assert_eq!(fnv1a_31(""), 0x011c9dc5);
    }
}
