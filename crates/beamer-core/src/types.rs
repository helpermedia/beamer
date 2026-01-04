//! Common types used throughout the Beamer framework.

// =============================================================================
// Audio Buffer Limits
// =============================================================================
//
// These constants define upper bounds for audio buffer storage. They serve as
// compile-time ceilings for worst-case memory bounds while allowing plugins to
// use only what they need through plugin-driven allocation.
//
// - 32 channels: Supports 22.2 surround and Dolby Atmos 9.1.6 (16 channels)
//                with generous headroom for future immersive formats
// - 16 buses: Main + sidechain + 14 aux (generous for multi-out instruments)
// - 15 aux buses: Total buses minus main bus
//
// Note: Plugins that declare configurations exceeding these limits will fail
// gracefully with clear error messages during setupProcessing().
// =============================================================================

/// Maximum number of audio channels per bus.
///
/// Set to 32 to support immersive audio formats:
/// - 22.2 surround (24 channels)
/// - Dolby Atmos 9.1.6 (16 channels)
/// - NHK 22.2 (24 channels)
///
/// Provides headroom for future formats. Plugins declaring more than 32
/// channels per bus will fail during initialization with a clear error.
pub const MAX_CHANNELS: usize = 32;

/// Maximum number of audio buses (main + auxiliary).
///
/// Set to 16 to support complex multi-bus configurations:
/// - Multi-out instruments (e.g., 16 individual drum outputs)
/// - Complex routing with multiple sidechains and aux sends
///
/// Plugins declaring more than 16 buses will fail during initialization.
pub const MAX_BUSES: usize = 16;

/// Maximum number of auxiliary buses (total buses minus main bus).
///
/// Equal to `MAX_BUSES - 1`. Used for auxiliary bus storage arrays.
pub const MAX_AUX_BUSES: usize = MAX_BUSES - 1;

/// Size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    /// Create a new size.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// Rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    /// Create a new rectangle.
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self { left, top, right, bottom }
    }

    /// Get the width of the rectangle.
    pub fn width(&self) -> u32 {
        (self.right - self.left).max(0) as u32
    }

    /// Get the height of the rectangle.
    pub fn height(&self) -> u32 {
        (self.bottom - self.top).max(0) as u32
    }

    /// Convert to a Size.
    pub fn size(&self) -> Size {
        Size {
            width: self.width(),
            height: self.height(),
        }
    }

    /// Create from origin (0, 0) and size.
    pub fn from_size(size: Size) -> Self {
        Self {
            left: 0,
            top: 0,
            right: size.width as i32,
            bottom: size.height as i32,
        }
    }
}

/// Parameter identifier.
pub type ParamId = u32;

/// Parameter value (normalized 0.0 to 1.0).
pub type ParamValue = f64;
