//! Range mapping for parameter normalization.
//!
//! This module provides traits and implementations for mapping between
//! plain parameter values (in natural units like Hz, dB, ms) and normalized
//! values (0.0 to 1.0) used for host communication.
//!
//! # Available Mappers
//!
//! - [`LinearMapper`] - Simple linear interpolation (most parameters)
//! - [`LogMapper`] - Logarithmic mapping for positive ranges (Hz)
//! - [`PowerMapper`] - Power curve for non-linear UI feel (dB thresholds)
//! - [`LogOffsetMapper`] - Logarithmic mapping for ranges including negatives
//!
//! # Example
//!
//! ```ignore
//! use beamer_core::parameter_range::{RangeMapper, LinearMapper, LogMapper, PowerMapper};
//!
//! // Linear mapping for most parameters
//! let linear = LinearMapper::new(0.0..=100.0);
//! assert_eq!(linear.normalize(50.0), 0.5);
//! assert_eq!(linear.denormalize(0.5), 50.0);
//!
//! // Logarithmic mapping for frequency parameters (positive ranges only)
//! let log = LogMapper::new(20.0..=20000.0);
//! // 632 Hz is roughly the geometric mean of 20 and 20000
//! assert!((log.denormalize(0.5) - 632.0).abs() < 1.0);
//!
//! // Power curve for threshold parameters (more resolution at max)
//! let power = PowerMapper::new(-60.0..=0.0, 2.0);
//! // With exponent 2.0, slider midpoint is closer to 0 dB than -30 dB
//! ```

use std::ops::RangeInclusive;

/// Trait for mapping between plain values and normalized values.
///
/// Implementations must be thread-safe (`Send + Sync`) for use in
/// audio plugin parameters.
pub trait RangeMapper: Send + Sync {
    /// Convert a plain value to normalized (0.0-1.0).
    ///
    /// Values outside the range are clamped.
    fn normalize(&self, plain: f64) -> f64;

    /// Convert a normalized value (0.0-1.0) to plain.
    ///
    /// Values outside 0.0-1.0 are clamped.
    fn denormalize(&self, normalized: f64) -> f64;

    /// Get the plain value range as (min, max).
    fn range(&self) -> (f64, f64);

    /// Get the default normalized value for a given plain default.
    fn default_normalized(&self, plain_default: f64) -> f64 {
        self.normalize(plain_default)
    }
}

/// Linear range mapping.
///
/// Maps values linearly between the range endpoints.
/// Suitable for most parameters where perceptual response is linear.
///
/// # Example
///
/// ```ignore
/// let mapper = LinearMapper::new(-60.0..=12.0);
/// assert_eq!(mapper.denormalize(0.0), -60.0);
/// assert_eq!(mapper.denormalize(1.0), 12.0);
/// ```
#[derive(Debug, Clone)]
pub struct LinearMapper {
    min: f64,
    max: f64,
}

impl LinearMapper {
    /// Create a new linear mapper with the given range.
    pub fn new(range: RangeInclusive<f64>) -> Self {
        Self {
            min: *range.start(),
            max: *range.end(),
        }
    }
}

impl RangeMapper for LinearMapper {
    fn normalize(&self, plain: f64) -> f64 {
        if (self.max - self.min).abs() < f64::EPSILON {
            return 0.5;
        }
        ((plain - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
    }

    fn denormalize(&self, normalized: f64) -> f64 {
        let normalized = normalized.clamp(0.0, 1.0);
        self.min + normalized * (self.max - self.min)
    }

    fn range(&self) -> (f64, f64) {
        (self.min, self.max)
    }
}

/// Logarithmic range mapping.
///
/// Maps values logarithmically between the range endpoints.
/// Suitable for frequency parameters where perceptual response is logarithmic.
///
/// # Panics
///
/// Panics if the range contains zero or negative values, as logarithm
/// is undefined for non-positive numbers.
///
/// # Example
///
/// ```ignore
/// let mapper = LogMapper::new(20.0..=20000.0);
/// // Geometric mean is at normalized 0.5
/// let mid = mapper.denormalize(0.5);
/// assert!((mid - 632.45).abs() < 1.0); // sqrt(20 * 20000)
/// ```
#[derive(Debug, Clone)]
pub struct LogMapper {
    min: f64,
    max: f64,
    min_log: f64,
    max_log: f64,
}

impl LogMapper {
    /// Create a new logarithmic mapper with the given range.
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - The range start is not positive (log requires positive values)
    /// - The range end is not greater than the range start
    pub fn new(range: RangeInclusive<f64>) -> Self {
        let min = *range.start();
        let max = *range.end();
        assert!(
            min > 0.0,
            "LogMapper requires positive range start, got min={}",
            min
        );
        assert!(
            max > min,
            "LogMapper requires max > min, got min={}, max={}",
            min, max
        );
        Self {
            min,
            max,
            min_log: min.ln(),
            max_log: max.ln(),
        }
    }
}

impl RangeMapper for LogMapper {
    fn normalize(&self, plain: f64) -> f64 {
        if (self.max_log - self.min_log).abs() < f64::EPSILON {
            return 0.5;
        }
        let plain = plain.max(self.min); // Clamp to positive
        let plain_log = plain.ln();
        ((plain_log - self.min_log) / (self.max_log - self.min_log)).clamp(0.0, 1.0)
    }

    fn denormalize(&self, normalized: f64) -> f64 {
        let normalized = normalized.clamp(0.0, 1.0);
        (self.min_log + normalized * (self.max_log - self.min_log)).exp()
    }

    fn range(&self) -> (f64, f64) {
        (self.min, self.max)
    }
}

/// Power curve range mapping.
///
/// Provides non-linear mapping using a power curve to give more resolution
/// at one end of the range. Works with any range (positive, negative, or mixed).
///
/// Common use cases:
/// - Threshold parameters where more precision is needed near 0 dB
/// - Any parameter where UI feel should be skewed toward one end
///
/// # Behavior
///
/// - `exponent > 1.0`: More resolution at the maximum (e.g., near 0 dB for threshold)
/// - `exponent < 1.0`: More resolution at the minimum
/// - `exponent = 1.0`: Linear (same as LinearMapper)
///
/// # Panics
///
/// Panics if the exponent is not positive, or if the range end is not
/// greater than the range start.
///
/// # Example
///
/// ```ignore
/// // More resolution near 0 dB (max), less at -60 dB (min)
/// let mapper = PowerMapper::new(-60.0..=0.0, 2.0);
///
/// // With exponent 2.0, normalized 0.5 maps to -15 dB (not -30 dB)
/// let mid = mapper.denormalize(0.5);
/// assert!((mid - -15.0).abs() < 0.1);
/// ```
#[derive(Debug, Clone)]
pub struct PowerMapper {
    min: f64,
    max: f64,
    inv_exponent: f64,
}

impl PowerMapper {
    /// Create a new power curve mapper.
    ///
    /// # Arguments
    ///
    /// * `range` - Value range (can be negative, positive, or mixed)
    /// * `exponent` - Power curve exponent (typical: 2.0-3.0)
    ///   - `exponent > 1.0`: More resolution at max
    ///   - `exponent < 1.0`: More resolution at min
    ///   - `exponent = 1.0`: Linear (same as LinearMapper)
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - `exponent <= 0.0` (must be positive)
    /// - `max <= min` (invalid range)
    pub fn new(range: RangeInclusive<f64>, exponent: f64) -> Self {
        let min = *range.start();
        let max = *range.end();

        assert!(
            max > min,
            "PowerMapper requires max > min, got min={}, max={}",
            min, max
        );
        assert!(
            exponent > 0.0,
            "PowerMapper requires positive exponent, got {}",
            exponent
        );

        Self {
            min,
            max,
            inv_exponent: 1.0 / exponent,
        }
    }
}

impl RangeMapper for PowerMapper {
    fn normalize(&self, plain: f64) -> f64 {
        if (self.max - self.min).abs() < f64::EPSILON {
            return 0.5;
        }

        // Linear normalize first
        let linear = ((plain - self.min) / (self.max - self.min)).clamp(0.0, 1.0);

        // Apply power curve (square for exponent=2.0)
        // This compresses the linear range so more slider travel is near max
        linear.powf(1.0 / self.inv_exponent)
    }

    fn denormalize(&self, normalized: f64) -> f64 {
        let normalized = normalized.clamp(0.0, 1.0);

        // Apply inverse power curve (square root for exponent=2.0)
        // This expands the normalized range back to linear
        let linear = normalized.powf(self.inv_exponent);

        // Linear denormalize
        self.min + linear * (self.max - self.min)
    }

    fn range(&self) -> (f64, f64) {
        (self.min, self.max)
    }
}

/// Logarithmic range mapping with offset for negative ranges.
///
/// Provides logarithmic mapping for ranges that include zero or negative values
/// by offsetting the range to positive values, applying log mapping, then
/// offsetting back.
///
/// Use this when you need true logarithmic behavior (geometric mean at midpoint)
/// for ranges that can't use [`LogMapper`].
///
/// # Panics
///
/// Panics if the range end is not greater than the range start.
///
/// # Example
///
/// ```ignore
/// // Logarithmic feel for threshold parameter
/// let mapper = LogOffsetMapper::new(-60.0..=0.0);
///
/// // The geometric mean (in offset space) is at normalized 0.5
/// let mid = mapper.denormalize(0.5);
/// // mid ≈ -53 dB (closer to min due to log curve)
/// ```
#[derive(Debug, Clone)]
pub struct LogOffsetMapper {
    min: f64,
    max: f64,
    offset: f64,
    min_log: f64,
    max_log: f64,
}

impl LogOffsetMapper {
    /// Create a new logarithmic offset mapper.
    ///
    /// The offset is automatically calculated to ensure all values
    /// are positive: `offset = abs(min) + 1.0`.
    ///
    /// # Panics
    ///
    /// Panics if `max <= min`.
    pub fn new(range: RangeInclusive<f64>) -> Self {
        let min = *range.start();
        let max = *range.end();

        assert!(
            max > min,
            "LogOffsetMapper requires max > min, got min={}, max={}",
            min, max
        );

        // Calculate offset to make all values positive
        // Add 1.0 to avoid ln(0)
        let offset = if min < 0.0 { min.abs() + 1.0 } else { 1.0 };

        let min_offset = min + offset;
        let max_offset = max + offset;

        Self {
            min,
            max,
            offset,
            min_log: min_offset.ln(),
            max_log: max_offset.ln(),
        }
    }
}

impl RangeMapper for LogOffsetMapper {
    fn normalize(&self, plain: f64) -> f64 {
        if (self.max_log - self.min_log).abs() < f64::EPSILON {
            return 0.5;
        }

        // Offset to positive, clamp to valid range
        let plain_offset = (plain + self.offset).max(self.min + self.offset);
        let plain_log = plain_offset.ln();

        ((plain_log - self.min_log) / (self.max_log - self.min_log)).clamp(0.0, 1.0)
    }

    fn denormalize(&self, normalized: f64) -> f64 {
        let normalized = normalized.clamp(0.0, 1.0);

        // Compute in offset (positive) space
        let plain_offset = (self.min_log + normalized * (self.max_log - self.min_log)).exp();

        // Remove offset to get original range
        plain_offset - self.offset
    }

    fn range(&self) -> (f64, f64) {
        (self.min, self.max)
    }
}
