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
/// The derive macro (`#[derive(Parameters)]`) performs compile-time collision
/// detection across all parameter IDs in a struct. If two different string
/// IDs produce the same hash, the macro will emit a compilation error with
/// the colliding identifiers.
///
/// # VST3 Integration
///
/// VST3 parameter IDs are `uint32` values. This function maps human-readable
/// string IDs (like `"cutoff"` or `"resonance"`) to the numeric IDs that
/// VST3 expects.
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
}
