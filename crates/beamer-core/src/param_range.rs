//! Range mapping for parameter normalization.
//!
//! This module provides traits and implementations for mapping between
//! plain parameter values (in natural units like Hz, dB, ms) and normalized
//! values (0.0 to 1.0) used for host communication.
//!
//! # Example
//!
//! ```ignore
//! use beamer_core::param_range::{RangeMapper, LinearMapper, LogMapper};
//!
//! // Linear mapping for most parameters
//! let linear = LinearMapper::new(0.0..=100.0);
//! assert_eq!(linear.normalize(50.0), 0.5);
//! assert_eq!(linear.denormalize(0.5), 50.0);
//!
//! // Logarithmic mapping for frequency parameters
//! let log = LogMapper::new(20.0..=20000.0);
//! // 632 Hz is roughly the geometric mean of 20 and 20000
//! assert!((log.denormalize(0.5) - 632.0).abs() < 1.0);
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
