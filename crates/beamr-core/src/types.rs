//! Common types used throughout the BEAMR framework.

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
