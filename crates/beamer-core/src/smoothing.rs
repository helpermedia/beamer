//! Parameter smoothing for avoiding zipper noise during automation.
//!
//! This module provides [`Smoother`] for interpolating parameter values over time,
//! and [`SmoothingStyle`] for selecting the interpolation algorithm.
//!
//! # Usage
//!
//! Smoothers are typically used via [`FloatParameter::with_smoother()`](crate::FloatParameter::with_smoother),
//! but can also be used standalone for custom modulation.
//!
//! ```ignore
//! // Via FloatParameter (recommended)
//! let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
//!     .with_smoother(SmoothingStyle::Exponential(5.0));  // 5ms
//!
//! // Standalone usage
//! let mut smoother = Smoother::new(SmoothingStyle::Linear(10.0));
//! smoother.set_sample_rate(44100.0);
//! smoother.reset(1.0);
//! smoother.set_target(0.5);
//! let value = smoother.tick();  // Per-sample
//! ```
//!
//! # Thread Safety
//!
//! `Smoother` requires `&mut self` for advancing state and is intended for
//! single-threaded audio processing only.
//!
//! A `FloatParameter` keeps the state of its smoother in atomics, like its
//! value, so that the parameters can be shared between threads and still
//! be smoothed through `&self`. Its smoothing methods are meant for the
//! audio thread alone: the type system does not keep a second thread from
//! calling them, but two threads that advance one smoother disturb each
//! other's ramp.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Threshold for snapping to target value to avoid denormals and finish smoothing.
const SNAP_THRESHOLD: f64 = 1e-8;

/// Smoothing algorithm selection.
///
/// The `f64` parameter is the smoothing time in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SmoothingStyle {
    /// No smoothing - value changes instantly.
    #[default]
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

/// What the style and the sample rate decide about a smoother.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RampConfig {
    pub(crate) style: SmoothingStyle,
    /// Pole coefficient of the exponential and logarithmic styles.
    pub(crate) coefficient: f64,
    /// Length of a linear ramp in samples, at least 1.
    pub(crate) linear_steps: u32,
}

impl RampConfig {
    pub(crate) fn new(style: SmoothingStyle) -> Self {
        Self {
            style,
            coefficient: 0.0,
            linear_steps: 1,
        }
    }

    pub(crate) fn set_sample_rate(&mut self, sample_rate: f64) {
        match self.style {
            SmoothingStyle::None => {}
            SmoothingStyle::Linear(ms) => {
                self.linear_steps = ((ms * sample_rate / 1000.0) as u32).max(1);
            }
            SmoothingStyle::Exponential(ms) | SmoothingStyle::Logarithmic(ms) => {
                if sample_rate <= 0.0 {
                    return;
                }
                // One-pole coefficient: reaches ~63% in `ms` milliseconds.
                // coef = 1 - e^(-1 / (tau * sr)), where tau = ms / 1000.
                let tau = ms / 1000.0;
                let samples_per_tau = tau * sample_rate;
                if samples_per_tau > 0.0 {
                    self.coefficient = 1.0 - (-1.0 / samples_per_tau).exp();
                } else {
                    // Instant.
                    self.coefficient = 1.0;
                }
            }
        }
    }
}

/// Where a smoother is on its way to the target.
///
/// The math of every smoother lives here, on plain values. [`Smoother`]
/// keeps one in a field and [`AtomicSmoother`] keeps one in atomics.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ramp {
    pub(crate) current: f64,
    pub(crate) target: f64,
    /// Increment per sample of a linear ramp.
    pub(crate) step_size: f64,
    /// Samples until a linear ramp reaches its target.
    pub(crate) steps_remaining: u32,
}

impl Ramp {
    pub(crate) fn at(value: f64) -> Self {
        Self {
            current: value,
            target: value,
            step_size: 0.0,
            steps_remaining: 0,
        }
    }

    #[inline]
    pub(crate) fn set_target(&mut self, config: &RampConfig, target: f64) {
        if (self.target - target).abs() < 1e-10 {
            return;
        }
        self.target = target;

        match config.style {
            SmoothingStyle::None => {
                self.current = target;
            }
            SmoothingStyle::Linear(_) => {
                self.steps_remaining = config.linear_steps;
                self.step_size = (target - self.current) / self.steps_remaining as f64;
            }
            SmoothingStyle::Exponential(_) | SmoothingStyle::Logarithmic(_) => {
                // Coefficient already computed, just update target.
            }
        }
    }

    #[inline]
    pub(crate) fn tick(&mut self, config: &RampConfig) -> f64 {
        match config.style {
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
                // One-pole: y[n] = y[n-1] + coef * (target - y[n-1]).
                self.current += config.coefficient * (self.target - self.current);

                // Snap when very close (avoid denormals, finish smoothing).
                if (self.current - self.target).abs() < SNAP_THRESHOLD {
                    self.current = self.target;
                }
                self.current
            }
            SmoothingStyle::Logarithmic(_) => {
                // Similar to exponential but in log domain.
                // Only works for positive values.
                if self.target > 0.0 && self.current > 0.0 {
                    let log_current = self.current.ln();
                    let log_target = self.target.ln();
                    let log_next = log_current + config.coefficient * (log_target - log_current);
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

    #[inline]
    pub(crate) fn current(&self, config: &RampConfig) -> f64 {
        match config.style {
            SmoothingStyle::None => self.target,
            _ => self.current,
        }
    }

    pub(crate) fn skip(&mut self, config: &RampConfig, samples: usize) {
        match config.style {
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
                // Closed-form solution: after n samples of one-pole filter,
                // current = target + (current - target) * (1 - coef)^n.
                let decay = (1.0 - config.coefficient).powi(samples as i32);
                self.current = self.target + (self.current - self.target) * decay;

                if (self.current - self.target).abs() < SNAP_THRESHOLD {
                    self.current = self.target;
                }
            }
            SmoothingStyle::Logarithmic(_) => {
                // Closed-form in log domain (only for positive values).
                if self.target > 0.0 && self.current > 0.0 {
                    let log_current = self.current.ln();
                    let log_target = self.target.ln();
                    let decay = (1.0 - config.coefficient).powi(samples as i32);
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

    #[inline]
    pub(crate) fn is_smoothing(&self, config: &RampConfig) -> bool {
        match config.style {
            SmoothingStyle::None => false,
            SmoothingStyle::Linear(_) => self.steps_remaining > 0,
            SmoothingStyle::Exponential(_) | SmoothingStyle::Logarithmic(_) => {
                (self.current - self.target).abs() > SNAP_THRESHOLD
            }
        }
    }
}

/// A parameter value smoother.
///
/// Can be used standalone for custom modulation, or integrated
/// into [`FloatParameter`](crate::FloatParameter) via `.with_smoother()`.
///
/// # Thread Safety
///
/// `Smoother` requires `&mut self` for advancing state, so one thread
/// owns it. This is intentional for audio thread usage.
#[derive(Debug, Clone)]
pub struct Smoother {
    config: RampConfig,
    ramp: Ramp,
}

impl Smoother {
    /// Create a new smoother with the given style.
    ///
    /// Sample rate must be set before use via [`set_sample_rate()`](Self::set_sample_rate).
    pub fn new(style: SmoothingStyle) -> Self {
        Self {
            config: RampConfig::new(style),
            ramp: Ramp::at(0.0),
        }
    }

    /// Create a smoother with no smoothing (pass-through).
    pub fn none() -> Self {
        Self::new(SmoothingStyle::None)
    }

    /// Get the smoothing style.
    pub fn style(&self) -> SmoothingStyle {
        self.config.style
    }

    /// Set the sample rate.
    ///
    /// Call this before processing starts. Recomputes coefficients
    /// based on time constants.
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.config.set_sample_rate(sample_rate);
    }

    /// Set a new target value.
    ///
    /// Call this when the parameter value changes (typically at start of process block).
    pub fn set_target(&mut self, target: f64) {
        self.ramp.set_target(&self.config, target);
    }

    /// Reset immediately to a value (no smoothing).
    ///
    /// Use when loading state or initializing to avoid ramps.
    pub fn reset(&mut self, value: f64) {
        self.ramp = Ramp::at(value);
    }

    /// Advance by one sample and return the smoothed value.
    ///
    /// Call this once per sample in the audio loop.
    #[inline]
    pub fn tick(&mut self) -> f64 {
        self.ramp.tick(&self.config)
    }

    /// Get current smoothed value without advancing.
    #[inline]
    pub fn current(&self) -> f64 {
        self.ramp.current(&self.config)
    }

    /// Get the target value.
    #[inline]
    pub fn target(&self) -> f64 {
        self.ramp.target
    }

    /// Skip forward by n samples (for block processing).
    ///
    /// This is equivalent to calling `tick()` n times but may be optimized
    /// for some smoothing styles.
    pub fn skip(&mut self, samples: usize) {
        self.ramp.skip(&self.config, samples);
    }

    /// Fill a slice with smoothed values (f64).
    pub fn fill(&mut self, buffer: &mut [f64]) {
        for sample in buffer.iter_mut() {
            *sample = self.tick();
        }
    }

    /// Fill a slice with smoothed values (f32).
    pub fn fill_f32(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.tick() as f32;
        }
    }

    /// Returns true if still smoothing toward target.
    #[inline]
    pub fn is_smoothing(&self) -> bool {
        self.ramp.is_smoothing(&self.config)
    }
}

impl Default for Smoother {
    fn default() -> Self {
        Self::none()
    }
}

/// The smoother inside a [`FloatParameter`](crate::FloatParameter).
///
/// It keeps its state in atomics, so that a parameter can be advanced
/// through a shared reference. One thread at a time advances it (the audio
/// thread): each call loads the state, computes on local values and stores
/// the result. Calls from two threads at once are memory-safe, but one of
/// them loses its step. Every access is relaxed, because the fields are
/// only ever combined by the thread that wrote them.
pub(crate) struct AtomicSmoother {
    style: SmoothingStyle,
    coefficient: AtomicU64,
    linear_steps: AtomicU32,
    /// Bits of the normalized parameter value the target was taken from.
    seen_bits: AtomicU64,
    current: AtomicU64,
    target: AtomicU64,
    step_size: AtomicU64,
    steps_remaining: AtomicU32,
}

impl AtomicSmoother {
    /// Create a smoother that rests at `value`.
    ///
    /// `value_bits` are the bits of the normalized parameter value that
    /// `value` was mapped from.
    pub(crate) fn new(style: SmoothingStyle, value_bits: u64, value: f64) -> Self {
        let config = RampConfig::new(style);
        Self {
            style,
            coefficient: AtomicU64::new(config.coefficient.to_bits()),
            linear_steps: AtomicU32::new(config.linear_steps),
            seen_bits: AtomicU64::new(value_bits),
            current: AtomicU64::new(value.to_bits()),
            target: AtomicU64::new(value.to_bits()),
            step_size: AtomicU64::new(0.0_f64.to_bits()),
            steps_remaining: AtomicU32::new(0),
        }
    }

    #[inline]
    fn config(&self) -> RampConfig {
        RampConfig {
            style: self.style,
            coefficient: f64::from_bits(self.coefficient.load(Ordering::Relaxed)),
            linear_steps: self.linear_steps.load(Ordering::Relaxed),
        }
    }

    #[inline]
    fn ramp(&self) -> Ramp {
        Ramp {
            current: f64::from_bits(self.current.load(Ordering::Relaxed)),
            target: f64::from_bits(self.target.load(Ordering::Relaxed)),
            step_size: f64::from_bits(self.step_size.load(Ordering::Relaxed)),
            steps_remaining: self.steps_remaining.load(Ordering::Relaxed),
        }
    }

    #[inline]
    fn store_ramp(&self, ramp: &Ramp) {
        self.current.store(ramp.current.to_bits(), Ordering::Relaxed);
        self.target.store(ramp.target.to_bits(), Ordering::Relaxed);
        self.step_size
            .store(ramp.step_size.to_bits(), Ordering::Relaxed);
        self.steps_remaining
            .store(ramp.steps_remaining, Ordering::Relaxed);
    }

    /// Load the state, follow the parameter value, run `advance` and store
    /// the state.
    ///
    /// `denormalize` maps the normalized value to the plain one. It only
    /// runs when the value changed since the last call.
    #[inline]
    fn update<R>(
        &self,
        value_bits: u64,
        denormalize: impl FnOnce(f64) -> f64,
        advance: impl FnOnce(&RampConfig, &mut Ramp) -> R,
    ) -> R {
        let config = self.config();
        let mut ramp = self.ramp();
        if self.seen_bits.load(Ordering::Relaxed) != value_bits {
            self.seen_bits.store(value_bits, Ordering::Relaxed);
            ramp.set_target(&config, denormalize(f64::from_bits(value_bits)));
        }
        let result = advance(&config, &mut ramp);
        self.store_ramp(&ramp);
        result
    }

    /// Set the sample rate and recompute the coefficients.
    pub(crate) fn set_sample_rate(
        &self,
        sample_rate: f64,
        value_bits: u64,
        denormalize: impl FnOnce(f64) -> f64,
    ) {
        let mut config = self.config();
        config.set_sample_rate(sample_rate);
        self.coefficient
            .store(config.coefficient.to_bits(), Ordering::Relaxed);
        self.linear_steps
            .store(config.linear_steps, Ordering::Relaxed);
        self.update(value_bits, denormalize, |_, _| ());
    }

    /// Jump to `value` (no ramp).
    pub(crate) fn reset(&self, value_bits: u64, value: f64) {
        self.seen_bits.store(value_bits, Ordering::Relaxed);
        self.store_ramp(&Ramp::at(value));
    }

    /// Get the smoothed value without advancing.
    #[inline]
    pub(crate) fn current(&self) -> f64 {
        f64::from_bits(self.current.load(Ordering::Relaxed))
    }

    /// Advance by one sample and return the smoothed value.
    #[inline]
    pub(crate) fn tick(&self, value_bits: u64, denormalize: impl FnOnce(f64) -> f64) -> f64 {
        self.update(value_bits, denormalize, |config, ramp| ramp.tick(config))
    }

    /// Skip forward by `samples` samples.
    pub(crate) fn skip(
        &self,
        value_bits: u64,
        denormalize: impl FnOnce(f64) -> f64,
        samples: usize,
    ) {
        self.update(value_bits, denormalize, |config, ramp| {
            ramp.skip(config, samples)
        });
    }

    /// Call `write` with each of the next `count` smoothed values.
    #[inline]
    pub(crate) fn fill(
        &self,
        value_bits: u64,
        denormalize: impl FnOnce(f64) -> f64,
        count: usize,
        mut write: impl FnMut(usize, f64),
    ) {
        self.update(value_bits, denormalize, |config, ramp| {
            for index in 0..count {
                write(index, ramp.tick(config));
            }
        });
    }

    /// Returns true if still smoothing toward target.
    pub(crate) fn is_smoothing(&self) -> bool {
        self.ramp().is_smoothing(&self.config())
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
        assert!((s.tick() - 1.0).abs() < 1e-10);
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
            s.tick();
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
            s.tick();
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

    /// Drive a smoother through a fixed sequence and collect the bits of
    /// what it returns.
    fn golden_run(style: SmoothingStyle) -> Vec<u64> {
        let mut values = Vec::new();
        let mut smoother = Smoother::new(style);
        smoother.set_sample_rate(1000.0);
        smoother.reset(1.0);
        smoother.set_target(8.0);
        for _ in 0..3 {
            values.push(smoother.tick().to_bits());
        }
        smoother.skip(2);
        values.push(smoother.current().to_bits());
        smoother.set_target(0.5);
        for _ in 0..2 {
            values.push(smoother.tick().to_bits());
        }
        // The logarithmic style jumps to a target that is not positive.
        smoother.set_target(-2.0);
        values.push(smoother.tick().to_bits());
        values
    }

    /// The exact values of every style, so that a change to the math does
    /// not go unnoticed. They were recorded before the math moved into
    /// `Ramp`.
    #[test]
    fn test_styles_give_the_recorded_values() {
        assert_eq!(
            golden_run(SmoothingStyle::None),
            [
                0x4020000000000000,
                0x4020000000000000,
                0x4020000000000000,
                0x4020000000000000,
                0x3fe0000000000000,
                0x3fe0000000000000,
                0xc000000000000000,
            ]
        );
        assert_eq!(
            golden_run(SmoothingStyle::Linear(10.0)),
            [
                0x3ffb333333333333,
                0x4003333333333333,
                0x4008cccccccccccc,
                0x4012000000000000,
                0x4010666666666666,
                0x400d999999999999,
                0x40090a3d70a3d70a,
            ]
        );
        assert_eq!(
            golden_run(SmoothingStyle::Exponential(5.0)),
            [
                0x400226ad0958d5a4,
                0x400a764ab4b7ba60,
                0x4010a21e41dcbbb9,
                0x4015b30a484c5758,
                0x401220e45edf9303,
                0x400e68e6de76f0a5,
                0x4005ff427bf953d8,
            ]
        );
        assert_eq!(
            golden_run(SmoothingStyle::Logarithmic(5.0)),
            [
                0x3ff75335eae61017,
                0x3fffc2005311f18b,
                0x400471845f75e6b2,
                0x400dc8279a07fcd2,
                0x4004b265c4cbdee1,
                0x3ffeba686ffd96a9,
                0xc000000000000000,
            ]
        );
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
