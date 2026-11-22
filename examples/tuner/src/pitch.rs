//! The pitch detector of the tuner.
//!
//! It finds the period of a mono signal in two steps:
//!
//! 1. The McLeod Pitch Method (MPM) on a decimated copy of the signal finds
//!    the period cheaply: the normalized square difference function, its
//!    key maxima and the first of them that comes close to the highest.
//!    McLeod and Wyvill, "A smarter way to find pitch" (ICMC 2005).
//! 2. The same function at the full rate, for the few lags around that
//!    period, measures it precisely. On the decimated grid a short period
//!    spans only a few samples.
//!
//! Plain Rust but for the derives that send [`Pitch`] to the editor, so it
//! reads and tests on its own.
//!
//! [`PitchDetector::push`] takes one sample at a time at a cost of a few
//! biquads. [`PitchDetector::analyze`] looks at the newest window: the
//! first step at the analysis rate of 14 to 28 kHz, the second over at most
//! 40 ms at the full rate for a few lags. In a release build one analysis
//! takes about 0.1 to 0.2 ms from 22.05 to 384 kHz. Every buffer is
//! allocated in [`PitchDetector::new`]: pushing and analyzing allocate
//! nothing.

use std::f64::consts::PI;

use beamer::prelude::TypeScript;
use beamer::serde::Serialize;

/// The ratio of a quarter tone, 50 cents.
const QUARTER_TONE: f64 = 1.029_302_236_643_492;

/// The lowest frequency named: A0 (27.5 Hz) less a quarter tone.
pub const LOWEST_HZ: f64 = 27.5 / QUARTER_TONE;

/// The highest frequency named: C8 (4186.009 Hz) plus a quarter tone.
pub const HIGHEST_HZ: f64 = 4186.009 * QUARTER_TONE;

/// The analysis runs at the sample rate divided by a whole number, at least
/// this rate.
const ANALYSIS_RATE_MIN: f64 = 14_000.0;

/// The low pass before decimation: above the highest frequency named,
/// below half the lowest analysis rate.
const LOW_PASS_HZ: f64 = 5_000.0;

/// The high pass against DC and rumble.
const HIGH_PASS_HZ: f64 = 20.0;

/// The length of the window the first step looks at. Its longest lag is
/// half of it, more than the period of the lowest frequency named.
const WINDOW_SECONDS: f64 = 0.09;

/// The time between two analyses.
const HOP_SECONDS: f64 = 0.02;

/// The longest the second step measures over.
const MEASURE_SECONDS_MAX: f64 = 0.04;

/// The first key maximum that reaches this share of the highest one is the
/// period. Lower picks a harmonic more readily, higher a multiple of the
/// period.
const KEY_MAXIMUM_SHARE: f64 = 0.9;

/// A window this periodic or more has a pitch.
const CLARITY_MIN: f64 = 0.85;

/// A window this loud or more has a pitch, in dB of its RMS level.
const GATE_DB: f64 = -60.0;

/// Values below this (-300 dB) are 0: far below anything audible, high
/// enough that neither an f32 nor the product of two is subnormal, which
/// some processors work through slowly.
const SMALLEST: f64 = 1e-15;

/// The lowest level reported, in dB.
pub const LEVEL_FLOOR_DB: f32 = -80.0;

/// What the detector heard in its newest window. The processor sends it to
/// the editor, which works out the note and the cents from it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, TypeScript)]
#[serde(crate = "beamer::serde", rename_all = "camelCase")]
pub struct Pitch {
    /// The fundamental in Hz, 0 when the window holds no clear pitch.
    pub frequency_hz: f32,
    /// How periodic the window is, from 0 (noise) to 1 (a pure tone).
    pub clarity: f32,
    /// The RMS level of the window in dB, at least -80 dB (the floor).
    pub level_db: f32,
}

impl Pitch {
    /// Nothing heard: no pitch, no level.
    pub const SILENT: Pitch = Pitch {
        frequency_hz: 0.0,
        clarity: 0.0,
        level_db: LEVEL_FLOOR_DB,
    };
}

/// Finds the pitch of a mono signal pushed one sample at a time.
pub struct PitchDetector {
    sample_rate: f64,
    /// Every this many samples one goes into the decimated window.
    decimation: usize,
    /// Whether the analysis rate is too low for the highest notes, under
    /// three samples a period of C8: then `pick_period` checks for them.
    low_rate: bool,
    high_pass: Biquad,
    /// An 8th order Butterworth low pass as four biquads.
    low_pass: [Biquad; 4],
    /// The samples since the last one that went into `decimated`.
    decimation_phase: usize,
    /// The newest samples at the full rate, after both filters. Without
    /// the harmonics above the low pass the function at a high note's
    /// period is a cosine, which [`peak`] fits exactly.
    full: History,
    /// The newest samples at the analysis rate, after both filters.
    decimated: History,
    /// The normalized square difference function of the decimated window,
    /// one value per lag from 0 to half the window.
    differences: Vec<f32>,
    /// The longest the second step measures over, in samples.
    measure_max: usize,
    /// The samples between two analyses.
    hop: usize,
    /// The samples pushed since the last analysis.
    since_analysis: usize,
}

impl PitchDetector {
    /// A detector for a signal at `sample_rate`, with every buffer it needs.
    pub fn new(sample_rate: f64) -> Self {
        let decimation = ((sample_rate / ANALYSIS_RATE_MIN).floor() as usize).max(1);
        let analysis_rate = sample_rate / decimation as f64;
        // At least a few samples, also for a sample rate a host gets wrong
        // (0): the windows are never empty.
        let window = ((WINDOW_SECONDS * analysis_rate).ceil() as usize).max(8);
        let low_pass_hz = LOW_PASS_HZ.min(0.45 * sample_rate);
        let hop = ((HOP_SECONDS * sample_rate).round() as usize).max(1);
        Self {
            sample_rate,
            decimation,
            low_rate: analysis_rate < 3.0 * HIGHEST_HZ,
            high_pass: Biquad::high_pass(
                HIGH_PASS_HZ,
                std::f64::consts::FRAC_1_SQRT_2,
                sample_rate,
            ),
            low_pass: std::array::from_fn(|section| {
                Biquad::low_pass(low_pass_hz, butterworth_q(8, section), sample_rate)
            }),
            decimation_phase: 0,
            full: History::new(window * decimation),
            decimated: History::new(window),
            differences: vec![0.0; window / 2 + 1],
            measure_max: (MEASURE_SECONDS_MAX * sample_rate).round() as usize,
            hop,
            since_analysis: 0,
        }
    }

    /// Forgets the signal: the filters, the windows and the hop start over.
    pub fn reset(&mut self) {
        self.high_pass.reset();
        self.low_pass.iter_mut().for_each(Biquad::reset);
        self.decimation_phase = 0;
        self.full.clear();
        self.decimated.clear();
        self.since_analysis = 0;
    }

    /// Takes the next sample of the signal.
    pub fn push(&mut self, sample: f32) {
        let high_passed = self.high_pass.process(f64::from(sample));
        let mut low_passed = self
            .low_pass
            .iter_mut()
            .fold(high_passed, |signal, section| section.process(signal));
        // A sample that is no number (NaN, infinity) would stay in the
        // filters for good: they start over instead.
        if !low_passed.is_finite() {
            self.high_pass.reset();
            self.low_pass.iter_mut().for_each(Biquad::reset);
            low_passed = 0.0;
        }
        // A signal fading out leaves values whose products in the analysis
        // are too small for an f32 (subnormal).
        if low_passed.abs() < SMALLEST {
            low_passed = 0.0;
        }
        self.full.push(low_passed as f32);
        self.decimation_phase += 1;
        if self.decimation_phase == self.decimation {
            self.decimation_phase = 0;
            self.decimated.push(low_passed as f32);
        }
        self.since_analysis = self.since_analysis.saturating_add(1);
    }

    /// Whether a hop has passed since the last analysis.
    pub fn analysis_due(&self) -> bool {
        self.since_analysis >= self.hop
    }

    /// Analyzes the newest window and starts the next hop.
    pub fn analyze(&mut self) -> Pitch {
        self.since_analysis = 0;
        let window = self.decimated.newest();
        let level_db = rms_db(window);
        normalized_square_difference(window, &mut self.differences);
        let Some((decimated_lag, clarity)) = pick_period(&self.differences, self.low_rate) else {
            return Pitch {
                level_db,
                ..Pitch::SILENT
            };
        };
        let clarity = clarity.clamp(0.0, 1.0);
        let unclear = Pitch {
            clarity: clarity as f32,
            level_db,
            ..Pitch::SILENT
        };
        if f64::from(level_db) < GATE_DB || clarity < CLARITY_MIN {
            return unclear;
        }
        let coarse_lag = decimated_lag * self.decimation as f64;
        let full = self.full.newest();
        // The coarse lag is within a decimated sample and one more.
        let reach = self.decimation + 1;
        let lag = match measure_period(full, coarse_lag, reach, self.measure_max) {
            // The coarse pass can take twice the period of a high note with
            // a weak fundamental: a few samples a period leave its true
            // peak lower than the next one. At the full rate both heights
            // are right. By the rule of the coarse pass the half is the
            // period when it comes close to the whole. Not at a low rate,
            // which is the analysis rate too: there the heights are as far
            // off as in the coarse pass. The half of a measured lag is
            // within a sample, so it looks two lags around it.
            Some((lag, height)) if !self.low_rate => {
                match measure_period(full, lag / 2.0, 2, self.measure_max) {
                    Some((half, half_height)) if half_height >= KEY_MAXIMUM_SHARE * height => half,
                    _ => lag,
                }
            }
            Some((lag, _)) => lag,
            None => coarse_lag,
        };
        let frequency_hz = self.sample_rate / lag;
        if !(LOWEST_HZ..=HIGHEST_HZ).contains(&frequency_hz) {
            return unclear;
        }
        Pitch {
            frequency_hz: frequency_hz as f32,
            ..unclear
        }
    }
}

/// The newest samples of a signal, readable as one slice in time order.
///
/// Each sample is written twice, half the buffer apart, so the newest
/// `len` samples always lie side by side.
struct History {
    samples: Vec<f32>,
    /// Where the next sample goes, which is also where the oldest starts.
    next: usize,
    len: usize,
}

impl History {
    fn new(len: usize) -> Self {
        Self {
            samples: vec![0.0; 2 * len],
            next: 0,
            len,
        }
    }

    fn clear(&mut self) {
        self.samples.fill(0.0);
        self.next = 0;
    }

    fn push(&mut self, sample: f32) {
        self.samples[self.next] = sample;
        self.samples[self.next + self.len] = sample;
        self.next = (self.next + 1) % self.len;
    }

    /// The newest `len` samples, the oldest first.
    fn newest(&self) -> &[f32] {
        &self.samples[self.next..self.next + self.len]
    }
}

/// A biquad filter in Direct Form II Transposed, coefficients from the
/// Audio EQ Cookbook.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn low_pass(frequency: f64, q: f64, sample_rate: f64) -> Self {
        let (cos, alpha) = Self::angle(frequency, q, sample_rate);
        Self::normalized((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0, cos, alpha)
    }

    fn high_pass(frequency: f64, q: f64, sample_rate: f64) -> Self {
        let (cos, alpha) = Self::angle(frequency, q, sample_rate);
        Self::normalized(
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            cos,
            alpha,
        )
    }

    /// The cosine of the frequency's angle and the cookbook's alpha.
    fn angle(frequency: f64, q: f64, sample_rate: f64) -> (f64, f64) {
        let omega = 2.0 * PI * frequency / sample_rate;
        (omega.cos(), omega.sin() / (2.0 * q))
    }

    fn normalized(b0: f64, b1: f64, b2: f64, cos: f64, alpha: f64) -> Self {
        let a0 = 1.0 + alpha;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }

    fn process(&mut self, input: f64) -> f64 {
        let output = self.b0 * input + self.z1;
        self.z1 = self.b1 * input - self.a1 * output + self.z2;
        self.z2 = self.b2 * input - self.a2 * output;
        // The state of a filter that fades out ends at 0, before it reaches
        // the subnormal values some processors work through slowly.
        if self.z1.abs() < SMALLEST {
            self.z1 = 0.0;
        }
        if self.z2.abs() < SMALLEST {
            self.z2 = 0.0;
        }
        output
    }
}

/// The Q of `section` (from 0) in a Butterworth filter of `order` built
/// from biquads.
fn butterworth_q(order: usize, section: usize) -> f64 {
    let angle = PI * (2 * section + 1) as f64 / (2 * order) as f64;
    1.0 / (2.0 * angle.cos())
}

/// The RMS level of `samples` in dB, at least [`LEVEL_FLOOR_DB`].
fn rms_db(samples: &[f32]) -> f32 {
    let power = samples
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / samples.len() as f64;
    let db = 10.0 * power.max(1e-20).log10();
    (db as f32).max(LEVEL_FLOOR_DB)
}

/// The normalized square difference function of `window`, one value per
/// lag in `out`: 2 r(lag) / m(lag), with r the autocorrelation over the
/// overlap and m the energy of both parts of it.
fn normalized_square_difference(window: &[f32], out: &mut [f32]) {
    let len = window.len();
    let mut energy = 2.0
        * window
            .iter()
            .map(|sample| f64::from(*sample).powi(2))
            .sum::<f64>();
    for (lag, value) in out.iter_mut().enumerate() {
        let correlation = f64::from(dot(&window[..len - lag], &window[lag..]));
        *value = if energy > 0.0 {
            (2.0 * correlation / energy) as f32
        } else {
            0.0
        };
        // The next lag's overlap loses the last sample of the first part
        // and the first sample of the second.
        energy -= f64::from(window[len - 1 - lag]).powi(2) + f64::from(window[lag]).powi(2);
    }
}

/// The dot product of two slices of the same length, in eight sums the
/// compiler can keep in vector registers.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let (a_chunks, a_tail) = a.as_chunks::<8>();
    let (b_chunks, b_tail) = b.as_chunks::<8>();
    let tail: f32 = a_tail.iter().zip(b_tail).map(|(x, y)| x * y).sum();
    let mut sums = [0.0f32; 8];
    for (x, y) in a_chunks.iter().zip(b_chunks) {
        for lane in 0..8 {
            sums[lane] += x[lane] * y[lane];
        }
    }
    sums.iter().sum::<f32>() + tail
}

/// Calls `visit` with the index of each key maximum of the function: the
/// highest value in each region where it is positive, after the region
/// around lag 0. A region still open at the end counts when its highest
/// value is not the last one.
fn key_maxima(values: &[f32], mut visit: impl FnMut(usize)) {
    let first_negative = values
        .iter()
        .position(|value| *value <= 0.0)
        .unwrap_or(values.len());
    let mut highest: Option<usize> = None;
    for (index, value) in values.iter().enumerate().skip(first_negative) {
        if *value > 0.0 {
            if highest.is_none_or(|best| *value > values[best]) {
                highest = Some(index);
            }
        } else if let Some(best) = highest.take() {
            visit(best);
        }
    }
    if let Some(best) = highest {
        if best + 1 < values.len() {
            visit(best);
        }
    }
}

/// The lag of the period in the normalized square difference function,
/// between samples, with the function's height there: the first key
/// maximum that reaches [`KEY_MAXIMUM_SHARE`] of the highest one. Heights
/// are those of the peaks through the samples, not of the samples: at
/// short periods a sample can lie far below its peak. With `low_rate`
/// (below three samples a period of C8) `None` when the first key maximum
/// is a clear one (from 0.25) with a period under three samples: a tone
/// that high cannot be measured at this rate. A later key maximum on a
/// whole number of its periods would read as a wrong pitch. At higher
/// rates no note in the range has so short a period. There such a first
/// key maximum is a side lobe of a bright tone's peak at lag 0.
fn pick_period(values: &[f32], low_rate: bool) -> Option<(f64, f64)> {
    let peak_at = |index: usize| {
        let (offset, height) = peak(
            f64::from(values[index - 1]),
            f64::from(values[index]),
            f64::from(values[index + 1]),
        );
        (index as f64 + offset, height)
    };
    let mut highest = None::<f64>;
    let mut first_too_short = None;
    key_maxima(values, |index| {
        let (before, middle, after) = (values[index - 1], values[index], values[index + 1]);
        // A cosine of -0.5 or less through the three values: under three
        // samples a period, see `peak`.
        first_too_short.get_or_insert(low_rate && middle >= 0.25 && before + after <= -middle);
        let (_, height) = peak_at(index);
        highest = Some(highest.map_or(height, |best| best.max(height)));
    });
    if first_too_short == Some(true) {
        return None;
    }
    let threshold = KEY_MAXIMUM_SHARE * highest?;
    let mut chosen = None;
    key_maxima(values, |index| {
        let found = peak_at(index);
        if chosen.is_none() && found.1 >= threshold {
            chosen = Some(found);
        }
    });
    chosen
}

/// The lag of the period at the full rate, measured around `coarse_lag`:
/// the normalized square difference function from the coarse lag towards
/// its highest value nearby, up to `reach` lags away, then the peak through
/// it. The coarse lag is close, so this takes a few lags. Measured over whole periods, as many
/// as fit in `samples` up to `measure_max`, so the function of a pure tone
/// has its peak at the period. Returns the lag with the function's height
/// there, `None` when the highest value is at an end of the reach.
fn measure_period(
    samples: &[f32],
    coarse_lag: f64,
    reach: usize,
    measure_max: usize,
) -> Option<(f64, f64)> {
    let center = coarse_lag.round() as usize;
    let lowest_lag = center.saturating_sub(reach).max(1);
    let highest_lag = center + reach;
    let available = samples.len().checked_sub(highest_lag)?;
    let limit = available.min(measure_max);
    let periods = (limit as f64 / coarse_lag).floor();
    let length = if periods >= 1.0 {
        (periods * coarse_lag).round() as usize
    } else {
        limit
    };
    // Every lag compares the same samples, those that leave room for the
    // longest lag.
    let start = samples.len() - highest_lag - length;
    let first = &samples[start..start + length];
    let first_energy: f64 = first.iter().map(|sample| f64::from(*sample).powi(2)).sum();
    let at = |lag: usize| {
        let second = &samples[start + lag..][..length];
        let mut correlation = 0.0;
        let mut second_energy = 0.0;
        for (x, y) in first.iter().zip(second) {
            correlation += f64::from(*x) * f64::from(*y);
            second_energy += f64::from(*y).powi(2);
        }
        let energy = first_energy + second_energy;
        if energy > 0.0 {
            2.0 * correlation / energy
        } else {
            0.0
        }
    };
    // Climb from the coarse lag to the highest value: each step is higher
    // than the one before, so it ends.
    let mut lag = center.clamp(lowest_lag + 1, highest_lag - 1);
    let (mut before, mut middle, mut after) = (at(lag - 1), at(lag), at(lag + 1));
    loop {
        if before > middle && lag - 1 > lowest_lag {
            lag -= 1;
            (after, middle) = (middle, before);
            before = at(lag - 1);
        } else if after > middle && lag + 1 < highest_lag {
            lag += 1;
            (before, middle) = (middle, after);
            after = at(lag + 1);
        } else {
            break;
        }
    }
    if before > middle || after > middle {
        return None;
    }
    let (offset, height) = peak(before, middle, after);
    Some((lag as f64 + offset, height))
}

/// The peak through three values one lag apart, the middle one the
/// highest: its offset from the middle one and its height. It fits a
/// cosine, which the function of a tone is once only its fundamental is
/// left, as at the short periods after the low pass. There the samples can
/// miss the peak by far (C8 at 14.7 kHz has 3.5 samples per period), where
/// a parabola would be off. At long periods it gives what a parabola gives.
/// The middle value itself when the three make no peak or when they would
/// mean fewer than three samples per period: three values do not pin a
/// cosine down there, its height could grow without bound and noise would
/// read as a pitch. At analysis rates of 14 kHz and up a period in the
/// range has at least 3.25 samples; below that the highest notes read as
/// none (see `pick_period`). The height is at most 1, as the function's
/// is.
fn peak(before: f64, middle: f64, after: f64) -> (f64, f64) {
    if middle <= 0.0 {
        return (0.0, middle);
    }
    // For values A cos(w (lag - peak)): before + after = 2 middle cos(w).
    let cos = (before + after) / (2.0 * middle);
    // A cosine of -0.5 is a period of three samples.
    if !(-0.5 < cos && cos < 1.0) {
        return (0.0, middle);
    }
    let omega = cos.acos();
    // And after - before = 2 A sin(w) sin(w offset), middle = A cos(w offset).
    let phase = ((after - before) / (2.0 * middle * omega.sin())).atan();
    (
        (phase / omega).clamp(-1.0, 1.0),
        (middle / phase.cos()).min(1.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATES: [f64; 6] = [22_050.0, 44_100.0, 48_000.0, 96_000.0, 192_000.0, 384_000.0];

    /// MIDI note numbers: A0, E1, B1, E2, A2, A4, C6, C7 and C8.
    const NOTES: [u8; 9] = [21, 28, 35, 40, 45, 69, 84, 96, 108];

    fn note_hz(note: u8) -> f64 {
        440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0)
    }

    fn cents(measured: f32, expected: f64) -> f64 {
        1200.0 * (f64::from(measured) / expected).log2()
    }

    /// The pitch of `seconds` of `signal`, a function of the time in
    /// seconds, at `sample_rate`.
    fn measure(sample_rate: f64, seconds: f64, signal: impl Fn(f64) -> f64) -> Pitch {
        let mut detector = PitchDetector::new(sample_rate);
        for index in 0..(seconds * sample_rate) as usize {
            detector.push(signal(index as f64 / sample_rate) as f32);
        }
        detector.analyze()
    }

    fn sine(frequency: f64, amplitude: f64) -> impl Fn(f64) -> f64 {
        move |time| amplitude * (2.0 * PI * frequency * time).sin()
    }

    /// A tone of harmonics with these amplitudes, the first the
    /// fundamental, leaving out those at or above `limit_hz`.
    fn harmonics(frequency: f64, amplitudes: Vec<f64>, limit_hz: f64) -> impl Fn(f64) -> f64 {
        move |time| {
            (1..)
                .map(|k| f64::from(k) * frequency)
                .zip(&amplitudes)
                .take_while(|(harmonic, _)| *harmonic < limit_hz)
                .map(|(harmonic, amplitude)| amplitude * (2.0 * PI * harmonic * time).sin())
                .sum()
        }
    }

    /// A sawtooth without aliasing: harmonics at 1/k up to 12 kHz.
    fn sawtooth(frequency: f64) -> impl Fn(f64) -> f64 {
        let count = (12_000.0 / frequency) as usize;
        harmonics(
            frequency,
            (1..=count).map(|k| 0.3 / k as f64).collect(),
            12_000.0,
        )
    }

    /// White noise from a fixed seed, from -0.5 to 0.5.
    fn noise() -> impl FnMut() -> f64 {
        noise_from(0x2545_f491)
    }

    /// White noise from `seed`, from -0.5 to 0.5.
    fn noise_from(seed: u32) -> impl FnMut() -> f64 {
        let mut state = seed;
        move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            f64::from(state) / f64::from(u32::MAX) - 0.5
        }
    }

    #[test]
    fn test_sines_from_a0_to_c8_read_their_frequency() {
        for sample_rate in RATES {
            for note in NOTES {
                let expected = note_hz(note);
                let pitch = measure(sample_rate, 0.2, sine(expected, 0.5));
                let error = cents(pitch.frequency_hz, expected);
                assert!(
                    error.abs() < 0.1,
                    "note {note} at {sample_rate} Hz: {} Hz, {error:.3} cents off",
                    pitch.frequency_hz
                );
            }
        }
    }

    #[test]
    fn test_harmonic_tones_read_their_fundamental() {
        let tones: [(&str, &[f64]); 2] = [
            ("a strong second harmonic", &[0.1, 0.4, 0.2, 0.1, 0.05]),
            ("no fundamental", &[0.0, 0.4, 0.3, 0.2, 0.1, 0.05]),
        ];
        for sample_rate in [44_100.0, 96_000.0] {
            for note in NOTES {
                let expected = note_hz(note);
                let pitch = measure(sample_rate, 0.2, sawtooth(expected));
                let error = cents(pitch.frequency_hz, expected);
                assert!(
                    error.abs() < 0.25,
                    "sawtooth, note {note} at {sample_rate} Hz: {error:.3} cents off"
                );
                for (name, amplitudes) in tones {
                    // Without its fundamental a tone keeps its period only
                    // while two harmonics pass the low pass: up to C6. A
                    // C7 without one is a C8 after it, but high notes never
                    // lack their fundamental; low ones do, through small
                    // speakers or on a bass.
                    if amplitudes[0] == 0.0 && note > 84 {
                        continue;
                    }
                    let tone = harmonics(expected, amplitudes.to_vec(), sample_rate / 2.0);
                    let pitch = measure(sample_rate, 0.2, tone);
                    let error = cents(pitch.frequency_hz, expected);
                    assert!(
                        error.abs() < 0.25,
                        "{name}, note {note} at {sample_rate} Hz: {error:.3} cents off"
                    );
                }
            }
        }
    }

    #[test]
    fn test_a_plucked_tone_reads_its_fundamental() {
        // A burst of noise, then harmonics that fade, the high ones faster.
        let sample_rate = 48_000.0;
        for note in [28, 40, 45, 52, 64, 69] {
            let expected = note_hz(note);
            let mut detector = PitchDetector::new(sample_rate);
            let mut next_noise = noise();
            let count = (12_000.0 / expected) as usize;
            for index in 0..(0.3 * sample_rate) as usize {
                let time = index as f64 / sample_rate;
                let burst = if time < 0.005 {
                    0.3 * next_noise()
                } else {
                    0.0
                };
                let tone: f64 = (1..=count)
                    .map(|k| {
                        let k = k as f64;
                        0.3 / k
                            * (-time * (2.0 + 3.0 * k)).exp()
                            * (2.0 * PI * k * expected * time).sin()
                    })
                    .sum();
                detector.push((burst + tone) as f32);
            }
            let pitch = detector.analyze();
            let error = cents(pitch.frequency_hz, expected);
            assert!(
                error.abs() < 0.5,
                "note {note}: {} Hz, {error:.3} cents off",
                pitch.frequency_hz
            );
        }
    }

    #[test]
    fn test_detuned_notes_read_back() {
        for note in [45, 69] {
            for detune in [-30.0, -10.0, 10.0, 30.0] {
                let expected = note_hz(note) * 2f64.powf(detune / 1200.0);
                let pitch = measure(48_000.0, 0.2, sine(expected, 0.5));
                let error = cents(pitch.frequency_hz, expected);
                assert!(
                    error.abs() < 0.1,
                    "note {note} {detune:+} cents: {error:.3} cents off"
                );
            }
        }
    }

    #[test]
    fn test_silence_quiet_tones_and_noise_have_no_pitch() {
        let silent = measure(48_000.0, 0.2, |_| 0.0);
        assert_eq!(silent, Pitch::SILENT);

        // About -73 dB RMS, below the gate.
        let quiet = measure(48_000.0, 0.2, sine(440.0, 0.0003));
        assert_eq!(quiet.frequency_hz, 0.0);
        assert!(quiet.level_db < GATE_DB as f32);

        let mut detector = PitchDetector::new(48_000.0);
        let mut next_noise = noise();
        for _ in 0..9_600 {
            detector.push(next_noise() as f32);
        }
        let noisy = detector.analyze();
        assert_eq!(noisy.frequency_hz, 0.0, "noise: clarity {}", noisy.clarity);
        assert!(noisy.level_db > GATE_DB as f32);
    }

    #[test]
    fn test_tones_outside_the_range_have_no_pitch() {
        // 25 Hz: below A0 less a quarter tone, but with a period within the
        // longest lag, so only the range keeps it out.
        for frequency in [25.0, 5_000.0] {
            let pitch = measure(48_000.0, 0.2, sine(frequency, 0.5));
            assert_eq!(pitch.frequency_hz, 0.0, "{frequency} Hz");
        }
    }

    #[test]
    fn test_noise_never_reads_as_a_pitch() {
        // Five seconds of noise from each of four seeds, every window
        // analyzed: about 1000 windows a rate.
        for (sample_rate, seed) in [44_100.0, 48_000.0].into_iter().flat_map(|rate| {
            [0x2545_f491, 0x9e37_79b9, 0x85eb_ca6b, 0xc2b2_ae35].map(|seed| (rate, seed))
        }) {
            let mut detector = PitchDetector::new(sample_rate);
            let mut next_noise = noise_from(seed);
            for index in 0..(5.0 * sample_rate) as usize {
                detector.push((0.5 * next_noise()) as f32);
                if index > 9_600 && detector.analysis_due() {
                    let pitch = detector.analyze();
                    assert_eq!(
                        pitch.frequency_hz, 0.0,
                        "{pitch:?} at {sample_rate} Hz, seed {seed:x}, sample {index}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_bright_tones_read_their_fundamental() {
        // Harmonics of equal strength up to 12 kHz, as a pulse or a buzz:
        // the function's peak at lag 0 is narrow, its side lobes a period of
        // about three samples at the analysis rate.
        let pulse = |frequency: f64| {
            let count = (12_000.0 / frequency) as usize;
            harmonics(frequency, vec![0.5 / count as f64; count], 12_000.0)
        };
        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            for note in [41, 45, 57, 69, 81, 82] {
                let expected = note_hz(note);
                let pitch = measure(sample_rate, 0.2, pulse(expected));
                let error = cents(pitch.frequency_hz, expected);
                assert!(
                    error.abs() < 0.25,
                    "pulse, note {note} at {sample_rate} Hz: {pitch:?}"
                );
            }
        }
        // A strong second harmonic from C6 to C8: never an octave off, within
        // a cent (the high notes at 32 kHz come closest). At 12 kHz, a low
        // rate without decimation, up to A#6 and only the right note: a few
        // cents off are its limit. Higher notes read as none.
        for (sample_rate, highest, bound) in [
            (12_000.0, 94, 50.0),
            (32_000.0, 108, 1.0),
            (44_100.0, 108, 1.0),
            (48_000.0, 108, 1.0),
            (88_200.0, 108, 1.0),
            (96_000.0, 108, 1.0),
            (192_000.0, 108, 1.0),
        ] {
            for note in 84..=highest {
                let expected = note_hz(note);
                let tone = harmonics(expected, vec![0.1, 0.4, 0.2, 0.1, 0.05], sample_rate / 2.0);
                let pitch = measure(sample_rate, 0.2, tone);
                let error = cents(pitch.frequency_hz, expected);
                assert!(
                    error.abs() < bound,
                    "second harmonic, note {note} at {sample_rate} Hz: {pitch:?}"
                );
            }
        }
    }

    #[test]
    fn test_notes_too_high_for_a_low_rate_read_as_none() {
        // Under three samples a period at 11.025 and 8 kHz.
        for (sample_rate, frequency) in [
            (11_025.0, 4186.009),
            (11_025.0, 3729.31),
            (8_000.0, 2959.96),
            (8_000.0, 3322.44),
        ] {
            let pitch = measure(sample_rate, 0.2, sine(frequency, 0.5));
            assert_eq!(
                pitch.frequency_hz, 0.0,
                "{frequency} Hz at {sample_rate} Hz: {pitch:?}"
            );
        }
        // Lower notes still read.
        let pitch = measure(11_025.0, 0.2, sine(440.0, 0.5));
        assert!(cents(pitch.frequency_hz, 440.0).abs() < 0.1, "{pitch:?}");
    }

    #[test]
    fn test_the_peak_of_three_values_stays_within_the_function() {
        // A cosine through these would have a period under three samples
        // and a height of 17.9.
        assert_eq!(peak(-0.5999, 0.2, 0.2), (0.0, 0.2));
        // Uneven values of a real peak never exceed 1.
        let (_, height) = peak(0.9, 0.99, 0.2);
        assert!(height <= 1.0, "{height}");
        // A cosine of period 4 sampled half a lag off its top: exact.
        let at = |lag: f64| (std::f64::consts::FRAC_PI_2 * (lag - 0.5)).cos();
        let (offset, height) = peak(at(-1.0), at(0.0), at(1.0));
        assert!(
            (offset - 0.5).abs() < 1e-12 && (height - 1.0).abs() < 1e-12,
            "{offset} {height}"
        );
    }

    #[test]
    fn test_a_sample_that_is_no_number_does_not_stop_the_detector() {
        for bad in [f32::NAN, f32::INFINITY] {
            let mut detector = PitchDetector::new(48_000.0);
            detector.push(bad);
            let tone = sine(440.0, 0.5);
            for index in 0..9_600 {
                detector.push(tone(f64::from(index) / 48_000.0) as f32);
            }
            let pitch = detector.analyze();
            assert!(
                cents(pitch.frequency_hz, 440.0).abs() < 0.1,
                "after {bad}: {pitch:?}"
            );
        }
    }

    #[test]
    fn test_an_analysis_is_due_after_each_hop() {
        let mut detector = PitchDetector::new(48_000.0);
        // 20 ms at 48 kHz.
        for _ in 0..959 {
            detector.push(0.0);
        }
        assert!(!detector.analysis_due());
        detector.push(0.0);
        assert!(detector.analysis_due());
        detector.analyze();
        assert!(!detector.analysis_due());
    }

    #[test]
    fn test_reset_forgets_the_signal() {
        let mut detector = PitchDetector::new(48_000.0);
        let tone = sine(440.0, 0.5);
        for index in 0..9_600 {
            detector.push(tone(f64::from(index) / 48_000.0) as f32);
        }
        assert!(detector.analyze().frequency_hz > 0.0);
        detector.reset();
        assert_eq!(detector.analyze(), Pitch::SILENT);
    }

    /// The time of one analysis, the worst over the notes:
    /// `cargo test --release -p tuner analysis_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn analysis_cost() {
        for sample_rate in RATES {
            let mut worst = std::time::Duration::ZERO;
            for note in NOTES {
                let mut detector = PitchDetector::new(sample_rate);
                let tone = sawtooth(note_hz(note));
                for index in 0..(0.2 * sample_rate) as usize {
                    detector.push(tone(index as f64 / sample_rate) as f32);
                }
                let start = std::time::Instant::now();
                for _ in 0..100 {
                    std::hint::black_box(detector.analyze());
                }
                worst = worst.max(start.elapsed() / 100);
            }
            println!("{sample_rate} Hz: {worst:?} per analysis at most");
        }
    }
}
