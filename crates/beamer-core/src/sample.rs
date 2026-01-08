//! Sample type abstraction for f32/f64 audio processing.
//!
//! Enables zero-cost generic buffer processing through monomorphization.

use std::ops::{Add, Div, Mul, Sub};

/// Trait for audio sample types (f32, f64).
///
/// Designed for zero-cost abstraction - all methods inline for monomorphization.
/// Only includes operations commonly needed in audio DSP inner loops.
///
/// # Design Philosophy
///
/// - **Minimal but complete**: Only essential operations
/// - **Inline everything**: Trust LLVM to optimize
/// - **Includes Div**: For cleaner RMS/average calculations
///
/// # Example: Generic Gain Plugin
///
/// ```ignore
/// fn process_generic<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
///     let gain = S::from_f32(self.parameters.gain_linear());
///     for (input, output) in buffer.zip_channels() {
///         for (i, o) in input.iter().zip(output.iter_mut()) {
///             *o = *i * gain;
///         }
///     }
/// }
/// ```
pub trait Sample:
    Copy
    + Default
    + Send
    + Sync
    + 'static
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + PartialOrd
{
    /// Zero value (0.0).
    const ZERO: Self;

    /// Unit value (1.0).
    const ONE: Self;

    /// Convert from f32.
    fn from_f32(value: f32) -> Self;

    /// Convert to f32.
    fn to_f32(self) -> f32;

    /// Convert from f64.
    fn from_f64(value: f64) -> Self;

    /// Convert to f64.
    fn to_f64(self) -> f64;

    /// Absolute value.
    fn abs(self) -> Self;

    /// Square root.
    fn sqrt(self) -> Self;

    /// Sine.
    fn sin(self) -> Self;

    /// Cosine.
    fn cos(self) -> Self;

    /// Minimum of two values.
    fn min(self, other: Self) -> Self;

    /// Maximum of two values.
    fn max(self, other: Self) -> Self;

    /// Clamp value between min and max.
    fn clamp(self, min: Self, max: Self) -> Self {
        self.max(min).min(max)
    }
}

impl Sample for f32 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;

    #[inline(always)]
    fn from_f32(value: f32) -> Self {
        value
    }

    #[inline(always)]
    fn to_f32(self) -> f32 {
        self
    }

    #[inline(always)]
    fn from_f64(value: f64) -> Self {
        value as f32
    }

    #[inline(always)]
    fn to_f64(self) -> f64 {
        self as f64
    }

    #[inline(always)]
    fn abs(self) -> Self {
        f32::abs(self)
    }

    #[inline(always)]
    fn sqrt(self) -> Self {
        f32::sqrt(self)
    }

    #[inline(always)]
    fn sin(self) -> Self {
        f32::sin(self)
    }

    #[inline(always)]
    fn cos(self) -> Self {
        f32::cos(self)
    }

    #[inline(always)]
    fn min(self, other: Self) -> Self {
        f32::min(self, other)
    }

    #[inline(always)]
    fn max(self, other: Self) -> Self {
        f32::max(self, other)
    }

    #[inline(always)]
    fn clamp(self, min: Self, max: Self) -> Self {
        f32::clamp(self, min, max)
    }
}

impl Sample for f64 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;

    #[inline(always)]
    fn from_f32(value: f32) -> Self {
        value as f64
    }

    #[inline(always)]
    fn to_f32(self) -> f32 {
        self as f32
    }

    #[inline(always)]
    fn from_f64(value: f64) -> Self {
        value
    }

    #[inline(always)]
    fn to_f64(self) -> f64 {
        self
    }

    #[inline(always)]
    fn abs(self) -> Self {
        f64::abs(self)
    }

    #[inline(always)]
    fn sqrt(self) -> Self {
        f64::sqrt(self)
    }

    #[inline(always)]
    fn sin(self) -> Self {
        f64::sin(self)
    }

    #[inline(always)]
    fn cos(self) -> Self {
        f64::cos(self)
    }

    #[inline(always)]
    fn min(self, other: Self) -> Self {
        f64::min(self, other)
    }

    #[inline(always)]
    fn max(self, other: Self) -> Self {
        f64::max(self, other)
    }

    #[inline(always)]
    fn clamp(self, min: Self, max: Self) -> Self {
        f64::clamp(self, min, max)
    }
}
