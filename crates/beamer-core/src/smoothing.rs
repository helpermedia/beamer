//! Parameter smoothing for avoiding zipper noise during automation.
//!
//! This module provides [`Smoother`] for interpolating parameter values over time,
//! and [`SmoothingStyle`] for selecting the interpolation algorithm.
//!
//! # Usage
//!
//! Smoothers are typically used via [`FloatParam::with_smoother()`](crate::FloatParam::with_smoother),
//! but can also be used standalone for custom modulation.
//!
//! ```ignore
//! // Via FloatParam (recommended)
//! let gain = FloatParam::db("Gain", 0.0, -60.0..=12.0)
//!     .with_smoother(SmoothingStyle::Exponential(5.0));  // 5ms
//!
//! // Standalone usage
//! let mut smoother = Smoother::new(SmoothingStyle::Linear(10.0));
//! smoother.set_sample_rate(44100.0);
//! smoother.reset(1.0);
//! smoother.set_target(0.5);
//! let value = smoother.next();  // Per-sample
//! ```
//!
//! # Thread Safety
//!
//! `Smoother` requires `&mut self` for advancing state and is intended for
//! single-threaded audio processing only. The parent `FloatParam` uses atomic
//! storage for thread-safe parameter access from UI/host threads.

/// Threshold for snapping to target value to avoid denormals and finish smoothing.
const SNAP_THRESHOLD: f64 = 1e-8;

/// Smoothing algorithm selection.
///
/// The `f64` parameter is the smoothing time in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SmoothingStyle {
    /// No smoothing - value changes instantly.
    None,

    /// Linear interpolation over specified milliseconds.
    /// Reaches target exactly after the specified time.
    /// Good for: general purpose, predictable behavior.
    Linear(f64),

    /// Exponential (one-pole IIR) smoothing.
    /// Fast initial response, asymptotically approaches target.
    /// Reaches ~63% of target in the specified time (time constant).
    /// Good for: most musical parameters, can cross zero.
    Exponential(f64),

    /// Logarithmic smoothing for frequency and other positive-only values.
    /// Slow start, accelerating curve.
    /// CANNOT cross zero or handle negative values - use Exponential for dB parameters.
    /// Good for: filter frequencies (Hz), other always-positive parameters.
    Logarithmic(f64),
}

impl Default for SmoothingStyle {
    fn default() -> Self {
        Self::None
    }
}

/// A parameter value smoother.
///
/// Can be used standalone for custom modulation, or integrated
/// into [`FloatParam`](crate::FloatParam) via `.with_smoother()`.
///
/// # Thread Safety
///
/// `Smoother` is `Send` but not `Sync` - it requires `&mut self` for
/// advancing state. This is intentional for audio thread usage.
#[derive(Debug, Clone)]
pub struct Smoother {
    style: SmoothingStyle,
    sample_rate: f64,

    // Current state
    current: f64,
    target: f64,

    // Precomputed coefficients (style-dependent)
    coefficient: f64,     // For exponential: pole coefficient
    step_size: f64,       // For linear: increment per sample
    steps_remaining: u32, // For linear: samples until target reached
}

impl Smoother {
    /// Create a new smoother with the given style.
    ///
    /// Sample rate must be set before use via [`set_sample_rate()`](Self::set_sample_rate).
    pub fn new(style: SmoothingStyle) -> Self {
        Self {
            style,
            sample_rate: 0.0,
            current: 0.0,
            target: 0.0,
            coefficient: 0.0,
            step_size: 0.0,
            steps_remaining: 0,
        }
    }

    /// Create a smoother with no smoothing (pass-through).
    pub fn none() -> Self {
        Self::new(SmoothingStyle::None)
    }

    /// Get the smoothing style.
    pub fn style(&self) -> SmoothingStyle {
        self.style
    }

    /// Set the sample rate.
    ///
    /// Call this from `AudioProcessor::setup()`. Recomputes coefficients
    /// based on time constants.
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        self.recompute_coefficients();
    }

    /// Set a new target value.
    ///
    /// Call this when the parameter value changes (typically at start of process block).
    pub fn set_target(&mut self, target: f64) {
        if (self.target - target).abs() < 1e-10 {
            return;
        }
        self.target = target;

        match self.style {
            SmoothingStyle::None => {
                self.current = target;
            }
            SmoothingStyle::Linear(ms) => {
                let samples = (ms * self.sample_rate / 1000.0) as u32;
                self.steps_remaining = samples.max(1);
                self.step_size = (target - self.current) / self.steps_remaining as f64;
            }
            SmoothingStyle::Exponential(_) | SmoothingStyle::Logarithmic(_) => {
                // Coefficient already computed, just update target
            }
        }
    }

    /// Reset immediately to a value (no smoothing).
    ///
    /// Use when loading state or initializing to avoid ramps.
    pub fn reset(&mut self, value: f64) {
        self.current = value;
        self.target = value;
        self.steps_remaining = 0;
        self.step_size = 0.0;
    }

    /// Get the next smoothed value (per-sample).
    ///
    /// Call this once per sample in the audio loop.
    #[inline]
    pub fn next(&mut self) -> f64 {
        match self.style {
            SmoothingStyle::None => self.target,
            SmoothingStyle::Linear(_) => {
                if self.steps_remaining > 0 {
                    self.current += self.step_size;
                    self.steps_remaining -= 1;
                    if self.steps_remaining == 0 {
                        self.current = self.target;
                    }
                }
                self.current
            }
            SmoothingStyle::Exponential(_) => {
                // One-pole: y[n] = y[n-1] + coef * (target - y[n-1])
                self.current += self.coefficient * (self.target - self.current);

                // Snap when very close (avoid denormals, finish smoothing)
                if (self.current - self.target).abs() < SNAP_THRESHOLD {
                    self.current = self.target;
                }
                self.current
            }
            SmoothingStyle::Logarithmic(_) => {
                // Similar to exponential but in log domain
                // Only works for positive values
                if self.target > 0.0 && self.current > 0.0 {
                    let log_current = self.current.ln();
                    let log_target = self.target.ln();
                    let log_next = log_current + self.coefficient * (log_target - log_current);
                    self.current = log_next.exp();

                    if (self.current - self.target).abs() < SNAP_THRESHOLD {
                        self.current = self.target;
                    }
                } else {
                    self.current = self.target;
                }
                self.current
            }
        }
    }

    /// Get current smoothed value without advancing.
    #[inline]
    pub fn current(&self) -> f64 {
        match self.style {
            SmoothingStyle::None => self.target,
            _ => self.current,
        }
    }

    /// Get the target value.
    #[inline]
    pub fn target(&self) -> f64 {
        self.target
    }

    /// Skip forward by n samples (for block processing).
    ///
    /// This is equivalent to calling `next()` n times but may be optimized
    /// for some smoothing styles.
    pub fn skip(&mut self, samples: usize) {
        match self.style {
            SmoothingStyle::None => {}
            SmoothingStyle::Linear(_) => {
                let skip_count = (samples as u32).min(self.steps_remaining);
                if skip_count > 0 {
                    self.current += self.step_size * skip_count as f64;
                    self.steps_remaining -= skip_count;
                    if self.steps_remaining == 0 {
                        self.current = self.target;
                    }
                }
            }
            SmoothingStyle::Exponential(_) => {
                // Closed-form solution: after n samples of one-pole filter
                // current = target + (current - target) * (1 - coef)^n
                let decay = (1.0 - self.coefficient).powi(samples as i32);
                self.current = self.target + (self.current - self.target) * decay;

                if (self.current - self.target).abs() < SNAP_THRESHOLD {
                    self.current = self.target;
                }
            }
            SmoothingStyle::Logarithmic(_) => {
                // Closed-form in log domain (only for positive values)
                if self.target > 0.0 && self.current > 0.0 {
                    let log_current = self.current.ln();
                    let log_target = self.target.ln();
                    let decay = (1.0 - self.coefficient).powi(samples as i32);
                    let log_result = log_target + (log_current - log_target) * decay;
                    self.current = log_result.exp();

                    if (self.current - self.target).abs() < SNAP_THRESHOLD {
                        self.current = self.target;
                    }
                } else {
                    self.current = self.target;
                }
            }
        }
    }

    /// Fill a slice with smoothed values (f64).
    pub fn fill(&mut self, buffer: &mut [f64]) {
        for sample in buffer.iter_mut() {
            *sample = self.next();
        }
    }

    /// Fill a slice with smoothed values (f32).
    pub fn fill_f32(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.next() as f32;
        }
    }

    /// Returns true if still smoothing toward target.
    #[inline]
    pub fn is_smoothing(&self) -> bool {
        match self.style {
            SmoothingStyle::None => false,
            SmoothingStyle::Linear(_) => self.steps_remaining > 0,
            SmoothingStyle::Exponential(_) | SmoothingStyle::Logarithmic(_) => {
                (self.current - self.target).abs() > SNAP_THRESHOLD
            }
        }
    }

    fn recompute_coefficients(&mut self) {
        if self.sample_rate <= 0.0 {
            return;
        }

        match self.style {
            SmoothingStyle::None => {}
            SmoothingStyle::Linear(_) => {
                // Coefficients computed per set_target()
            }
            SmoothingStyle::Exponential(ms) | SmoothingStyle::Logarithmic(ms) => {
                // One-pole coefficient: reaches ~63% in `ms` milliseconds
                // coef = 1 - e^(-1 / (tau * sr))
                // where tau = ms / 1000
                let tau = ms / 1000.0;
                let samples_per_tau = tau * self.sample_rate;
                if samples_per_tau > 0.0 {
                    self.coefficient = 1.0 - (-1.0 / samples_per_tau).exp();
                } else {
                    self.coefficient = 1.0; // Instant
                }
            }
        }
    }
}

impl Default for Smoother {
    fn default() -> Self {
        Self::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_smoothing() {
        let mut s = Smoother::new(SmoothingStyle::None);
        s.set_sample_rate(44100.0);
        s.reset(0.0);
        s.set_target(1.0);
        assert!((s.next() - 1.0).abs() < 1e-10);
        assert!(!s.is_smoothing());
    }

    #[test]
    fn test_linear_reaches_target() {
        let mut s = Smoother::new(SmoothingStyle::Linear(10.0)); // 10ms
        s.set_sample_rate(1000.0); // 1 sample per ms
        s.reset(0.0);
        s.set_target(1.0);

        // Should take 10 samples to reach target
        for _ in 0..10 {
            s.next();
        }
        assert!((s.current() - 1.0).abs() < 1e-10);
        assert!(!s.is_smoothing());
    }

    #[test]
    fn test_exponential_approaches_target() {
        let mut s = Smoother::new(SmoothingStyle::Exponential(5.0)); // 5ms time constant
        s.set_sample_rate(44100.0);
        s.reset(0.0);
        s.set_target(1.0);

        // After many samples, should be very close to target
        for _ in 0..10000 {
            s.next();
        }
        assert!((s.current() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_skip_linear() {
        let mut s = Smoother::new(SmoothingStyle::Linear(10.0));
        s.set_sample_rate(1000.0);
        s.reset(0.0);
        s.set_target(1.0);

        s.skip(5);
        assert!((s.current() - 0.5).abs() < 1e-10);

        s.skip(5);
        assert!((s.current() - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_fill_f32() {
        let mut s = Smoother::new(SmoothingStyle::Linear(10.0));
        s.set_sample_rate(1000.0);
        s.reset(0.0);
        s.set_target(1.0);

        let mut buffer = [0.0f32; 10];
        s.fill_f32(&mut buffer);

        // First value should be ~0.1, last should be 1.0
        assert!(buffer[0] > 0.0);
        assert!((buffer[9] - 1.0).abs() < 1e-5);
    }
}
