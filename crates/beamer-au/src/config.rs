//! AU-specific plugin configuration.
//!
//! This module provides Audio Unit-specific configuration that complements
//! the shared [`beamer_core::Config`].

/// Four-character code (FourCC) for AU identifiers.
///
/// Used for manufacturer codes and subtype codes in AU registration.
/// Must be exactly 4 ASCII characters.
///
/// # Example
///
/// ```ignore
/// use beamer_au::{fourcc, FourCharCode};
///
/// // Using the macro (compile-time validated)
/// const MANUFACTURER: FourCharCode = fourcc!(b"Demo");
/// const SUBTYPE: FourCharCode = fourcc!(b"gain");
///
/// // Or manually
/// const MANUFACTURER2: FourCharCode = FourCharCode::new(b"Demo");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FourCharCode(pub [u8; 4]);

impl FourCharCode {
    /// Create a new FourCharCode from a 4-byte array.
    ///
    /// # Panics
    /// Debug builds will panic if any byte is not ASCII.
    pub const fn new(bytes: &[u8; 4]) -> Self {
        // Note: const fn can't use loops in older Rust versions
        // This works in Rust 1.46+
        debug_assert!(bytes[0].is_ascii(), "FourCC bytes must be ASCII");
        debug_assert!(bytes[1].is_ascii(), "FourCC bytes must be ASCII");
        debug_assert!(bytes[2].is_ascii(), "FourCC bytes must be ASCII");
        debug_assert!(bytes[3].is_ascii(), "FourCC bytes must be ASCII");
        Self(*bytes)
    }

    /// Get the FourCC as a 32-bit value (big-endian).
    pub const fn as_u32(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    /// Get the FourCC as a string slice.
    pub fn as_str(&self) -> &str {
        // Safe because we validate ASCII in new()
        std::str::from_utf8(&self.0).unwrap_or("????")
    }

    /// Get the raw bytes.
    pub const fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }
}

impl std::fmt::Display for FourCharCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Macro for creating FourCharCode at compile time with validation.
///
/// # Example
///
/// ```ignore
/// use beamer_au::fourcc;
///
/// const MANUFACTURER: FourCharCode = fourcc!(b"Demo");
/// const SUBTYPE: FourCharCode = fourcc!(b"gain");
/// ```
///
/// # Compile-time Errors
///
/// The macro will fail to compile if the input is not exactly 4 ASCII bytes.
#[macro_export]
macro_rules! fourcc {
    ($s:literal) => {{
        const BYTES: &[u8] = $s;
        const _: () = assert!(BYTES.len() == 4, "FourCC must be exactly 4 bytes");
        const _: () = assert!(BYTES[0].is_ascii(), "FourCC byte 0 must be ASCII");
        const _: () = assert!(BYTES[1].is_ascii(), "FourCC byte 1 must be ASCII");
        const _: () = assert!(BYTES[2].is_ascii(), "FourCC byte 2 must be ASCII");
        const _: () = assert!(BYTES[3].is_ascii(), "FourCC byte 3 must be ASCII");
        $crate::FourCharCode::new(&[BYTES[0], BYTES[1], BYTES[2], BYTES[3]])
    }};
}

/// AU-specific plugin configuration.
///
/// This struct holds Audio Unit-specific metadata. Use in combination with
/// [`beamer_core::Config`] for complete plugin configuration.
///
/// # Example
///
/// ```ignore
/// use beamer_core::{Config, Category};
/// use beamer_au::AuConfig;
///
/// pub static CONFIG: Config = Config::new("Beamer Gain", Category::Effect)
///     .with_vendor("Beamer Framework")
///     .with_version(env!("CARGO_PKG_VERSION"));
///
/// pub static AU_CONFIG: AuConfig = AuConfig::new(
///     "Demo",  // Manufacturer
///     "gain",  // Subtype
/// );
///
/// export_au!(CONFIG, AU_CONFIG, GainPlugin);
/// ```
#[derive(Debug)]
pub struct AuConfig {
    /// Manufacturer code (4-character identifier for your company/brand).
    /// Should be unique across all AU developers.
    /// Apple recommends registering codes with them.
    pub manufacturer: FourCharCode,

    /// Subtype code (4-character identifier for this specific plugin).
    /// Should be unique within your manufacturer namespace.
    pub subtype: FourCharCode,
}

/// Helper to convert a string literal to a 4-byte array at compile time.
///
/// # Panics
/// Panics at compile time if the string is not exactly 4 bytes.
const fn str_to_four_bytes(s: &str) -> [u8; 4] {
    let bytes = s.as_bytes();
    assert!(bytes.len() == 4, "FourCC string must be exactly 4 bytes");
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

impl AuConfig {
    /// Create a new AU configuration.
    ///
    /// # Arguments
    ///
    /// * `manufacturer` - Your 4-character manufacturer code (e.g., `"Demo"`)
    /// * `subtype` - Your 4-character plugin subtype code (e.g., `"gain"`)
    ///
    /// # Panics
    /// Panics at compile time if codes are not exactly 4 ASCII characters.
    ///
    /// # Example
    ///
    /// ```ignore
    /// AuConfig::new("Bmer", "gain")
    /// ```
    pub const fn new(manufacturer: &str, subtype: &str) -> Self {
        Self {
            manufacturer: FourCharCode::new(&str_to_four_bytes(manufacturer)),
            subtype: FourCharCode::new(&str_to_four_bytes(subtype)),
        }
    }

    /// Get the manufacturer code as a u32.
    pub const fn manufacturer_u32(&self) -> u32 {
        self.manufacturer.as_u32()
    }

    /// Get the subtype code as a u32.
    pub const fn subtype_u32(&self) -> u32 {
        self.subtype.as_u32()
    }
}
