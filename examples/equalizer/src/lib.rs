//! Beamer Equalizer - Example 3-band parametric EQ demonstrating the Beamer framework.
//!
//! # Three-Struct Pattern
//!
//! Beamer plugins use three structs for clear separation of concerns:
//!
//! 1. **`EqualizerParameters`** - Pure parameter definitions with `#[derive(Parameters)]`
//! 2. **`EqualizerDescriptor`** - Plugin descriptor that holds parameters and implements `Descriptor`
//! 3. **`EqualizerProcessor`** - Runtime processor created by `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - `FloatParameter` with Hz scaling via `kind = "hz"` (LogMapper + frequency formatter)
//! - `FloatParameter` with dB scaling via `kind = "db"`
//! - Flat parameter groups via `group = "..."` attribute
//! - Stereo bus configuration via `input_bus_info()` / `output_bus_info()` overrides
//! - Biquad filters using standard bilinear transform mathematics
//! - Generic f32/f64 processing via `Sample` trait

use beamer::prelude::*;

/// Pi constant for filter calculations
const PI: f64 = std::f64::consts::PI;

// =============================================================================
// Biquad Filter
// =============================================================================

/// Biquad filter state (Direct Form II Transposed).
///
/// Stores the two delay elements needed for the biquad difference equation.
#[derive(Default, Clone, Copy)]
struct BiquadState {
    z1: f64,
    z2: f64,
}

impl BiquadState {
    /// Process a single sample through the filter.
    ///
    /// Uses Direct Form II Transposed structure:
    /// ```text
    /// y[n] = b0*x[n] + z1
    /// z1 = b1*x[n] - a1*y[n] + z2
    /// z2 = b2*x[n] - a2*y[n]
    /// ```
    #[inline]
    fn process(&mut self, input: f64, coeffs: &BiquadCoeffs) -> f64 {
        let output = coeffs.b0 * input + self.z1;
        self.z1 = coeffs.b1 * input - coeffs.a1 * output + self.z2;
        self.z2 = coeffs.b2 * input - coeffs.a2 * output;
        output
    }
}

/// Biquad filter coefficients.
///
/// Normalized coefficients where a0 = 1 (already divided out).
#[derive(Clone, Copy)]
struct BiquadCoeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Default for BiquadCoeffs {
    /// Default to passthrough (unity gain, no filtering).
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
        }
    }
}

impl BiquadCoeffs {
    /// Clamp frequency to prevent instability near Nyquist.
    ///
    /// When filter frequency approaches Nyquist (sample_rate / 2), the bilinear
    /// transform produces unstable or undefined coefficients. Clamping to 49%
    /// of sample rate provides a safe margin.
    #[inline]
    fn clamp_frequency(freq: f64, sample_rate: f64) -> f64 {
        freq.min(sample_rate * 0.49)
    }

    /// Calculate peaking (bell) filter coefficients.
    ///
    /// Derived from bilinear transform of analog parametric EQ prototype.
    /// Q controls bandwidth (higher Q = narrower peak).
    /// Frequency is clamped to 49% of sample rate to prevent Nyquist instability.
    /// Q is clamped to minimum 0.01 to prevent division by zero.
    fn peak(freq: f64, gain_db: f64, q: f64, sample_rate: f64) -> Self {
        // Clamp frequency to prevent instability near Nyquist
        let freq = Self::clamp_frequency(freq, sample_rate);

        // Clamp Q to prevent division by zero or near-zero values
        let q = q.max(0.01);

        let a = 10.0_f64.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();

        // Bandwidth parameter: alpha = sin(w0) / (2*Q)
        let alpha = sin_w0 / (2.0 * q);

        let a0 = 1.0 + alpha / a;

        Self {
            b0: (1.0 + alpha * a) / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

// =============================================================================
// Parameters
// =============================================================================

/// Equalizer plugin parameters.
///
/// 3-band parametric EQ with peak (bell) filters:
/// - Low band (20-2000 Hz)
/// - Mid band (200-8000 Hz)
/// - High band (2000-20000 Hz)
///
/// Each band has frequency, gain, and width controls.
/// Width controls bandwidth (higher = wider = more audible).
/// Parameters are organized into flat groups for DAW display.
#[derive(Parameters)]
pub struct EqualizerParameters {
    // =========================================================================
    // Low Band (Peak)
    // =========================================================================
    /// Low band center frequency
    #[parameter(
        id = "low_freq",
        name = "Low - Frequency",
        default = 200.0,
        range = 20.0..=2000.0,
        kind = "hz",
        group = "Low"
    )]
    pub low_freq: FloatParameter,

    /// Low band gain in dB
    #[parameter(
        id = "low_gain",
        name = "Low - Gain",
        default = 0.0,
        range = -12.0..=12.0,
        kind = "db",
        group = "Low"
    )]
    pub low_gain: FloatParameter,

    /// Low band width (higher = wider = more audible)
    #[parameter(
        id = "low_width",
        name = "Low - Width",
        default = 1.0,
        range = 0.1..=10.0,
        group = "Low"
    )]
    pub low_width: FloatParameter,

    // =========================================================================
    // Mid Band (Peak)
    // =========================================================================
    /// Mid band center frequency
    #[parameter(
        id = "mid_freq",
        name = "Mid - Frequency",
        default = 1000.0,
        range = 200.0..=8000.0,
        kind = "hz",
        group = "Mid"
    )]
    pub mid_freq: FloatParameter,

    /// Mid band gain in dB
    #[parameter(
        id = "mid_gain",
        name = "Mid - Gain",
        default = 0.0,
        range = -12.0..=12.0,
        kind = "db",
        group = "Mid"
    )]
    pub mid_gain: FloatParameter,

    /// Mid band width (higher = wider = more audible)
    #[parameter(
        id = "mid_width",
        name = "Mid - Width",
        default = 1.0,
        range = 0.1..=10.0,
        group = "Mid"
    )]
    pub mid_width: FloatParameter,

    // =========================================================================
    // High Band (Peak)
    // =========================================================================
    /// High band center frequency
    #[parameter(
        id = "high_freq",
        name = "High - Frequency",
        default = 4000.0,
        range = 2000.0..=20000.0,
        kind = "hz",
        group = "High"
    )]
    pub high_freq: FloatParameter,

    /// High band gain in dB
    #[parameter(
        id = "high_gain",
        name = "High - Gain",
        default = 0.0,
        range = -12.0..=12.0,
        kind = "db",
        group = "High"
    )]
    pub high_gain: FloatParameter,

    /// High band width (higher = wider = more audible)
    #[parameter(
        id = "high_width",
        name = "High - Width",
        default = 1.0,
        range = 0.1..=10.0,
        group = "High"
    )]
    pub high_width: FloatParameter,
}

// =============================================================================
// Descriptor
// =============================================================================

/// Equalizer plugin descriptor (unprepared state).
///
/// Configures stereo I/O buses and transforms into processor when prepared.
#[beamer::export]
#[derive(Default, HasParameters)]
pub struct EqualizerDescriptor {
    #[parameters]
    pub parameters: EqualizerParameters,
}

impl Descriptor for EqualizerDescriptor {
    type Setup = SampleRate;
    type Processor = EqualizerProcessor;

    fn prepare(self, sample_rate: SampleRate) -> EqualizerProcessor {
        EqualizerProcessor {
            parameters: self.parameters,
            sample_rate: sample_rate.hz(),
            low_state: [BiquadState::default(); 2],
            mid_state: [BiquadState::default(); 2],
            high_state: [BiquadState::default(); 2],
            low_coeffs: BiquadCoeffs::default(),
            mid_coeffs: BiquadCoeffs::default(),
            high_coeffs: BiquadCoeffs::default(),
        }
    }

    // =========================================================================
    // Stereo Bus Configuration
    // =========================================================================

    fn input_bus_count(&self) -> usize {
        1
    }

    fn output_bus_count(&self) -> usize {
        1
    }

    fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
        (index == 0).then(|| BusInfo::stereo("Input"))
    }

    fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
        (index == 0).then(|| BusInfo::stereo("Output"))
    }
}

// =============================================================================
// Processor
// =============================================================================

/// Equalizer plugin processor (prepared state).
///
/// Contains filter state and coefficients for all three bands.
#[derive(HasParameters)]
pub struct EqualizerProcessor {
    #[parameters]
    pub parameters: EqualizerParameters,

    /// Sample rate in Hz
    sample_rate: f64,

    /// Filter states (stereo, one per band per channel)
    low_state: [BiquadState; 2],
    mid_state: [BiquadState; 2],
    high_state: [BiquadState; 2],

    /// Filter coefficients (recalculated when parameters change)
    low_coeffs: BiquadCoeffs,
    mid_coeffs: BiquadCoeffs,
    high_coeffs: BiquadCoeffs,
}

impl EqualizerProcessor {
    /// Update filter coefficients from current parameter values.
    ///
    /// Width is converted to Q via `Q = 1/width`, so higher width = lower Q = wider band.
    fn update_coefficients(&mut self) {
        self.low_coeffs = BiquadCoeffs::peak(
            self.parameters.low_freq.get(),
            self.parameters.low_gain.get(),
            1.0 / self.parameters.low_width.get(),
            self.sample_rate,
        );

        self.mid_coeffs = BiquadCoeffs::peak(
            self.parameters.mid_freq.get(),
            self.parameters.mid_gain.get(),
            1.0 / self.parameters.mid_width.get(),
            self.sample_rate,
        );

        self.high_coeffs = BiquadCoeffs::peak(
            self.parameters.high_freq.get(),
            self.parameters.high_gain.get(),
            1.0 / self.parameters.high_width.get(),
            self.sample_rate,
        );
    }

    /// Generic processing implementation for both f32 and f64.
    fn process_generic<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        // Update coefficients at the start of each block
        self.update_coefficients();

        // Process stereo buffer: chain filters low -> mid -> high per channel
        for (ch, (input, output)) in buffer.zip_channels().enumerate() {
            for (in_sample, out_sample) in input.iter().zip(output.iter_mut()) {
                let sample = in_sample.to_f64();

                // Chain the three EQ bands (using per-channel state)
                let sample = self.low_state[ch].process(sample, &self.low_coeffs);
                let sample = self.mid_state[ch].process(sample, &self.mid_coeffs);
                let sample = self.high_state[ch].process(sample, &self.high_coeffs);

                *out_sample = S::from_f64(sample);
            }
        }
    }
}

impl Processor for EqualizerProcessor {
    type Descriptor = EqualizerDescriptor;

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &ProcessContext,
    ) {
        self.process_generic(buffer);
    }

    fn supports_double_precision(&self) -> bool {
        true
    }

    fn process_f64(
        &mut self,
        buffer: &mut Buffer<f64>,
        _aux: &mut AuxiliaryBuffers<f64>,
        _context: &ProcessContext,
    ) {
        self.process_generic(buffer);
    }
}
