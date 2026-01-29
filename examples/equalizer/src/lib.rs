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
//! - Mono bus configuration via `input_bus_info()` / `output_bus_info()` overrides
//! - Biquad filters using standard bilinear transform mathematics
//! - Generic f32/f64 processing via `Sample` trait

use beamer::prelude::*;

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Shared plugin configuration (format-agnostic metadata)
pub static CONFIG: Config = Config::new("Beamer Equalizer")
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version(env!("CARGO_PKG_VERSION"));

/// VST3-specific configuration
#[cfg(feature = "vst3")]
pub static VST3_CONFIG: Vst3Config = Vst3Config::new("639DD3FC-D376-4023-BF19-CD57A01FCF4D")
    .with_categories("Fx|EQ");

/// AU-specific configuration
#[cfg(feature = "au")]
pub static AU_CONFIG: AuConfig = AuConfig::new(ComponentType::Effect, "Bmer", "eqlz")
    .with_tags(&["EQ"]);

/// Pi constant for filter calculations
const PI: f64 = std::f64::consts::PI;

/// Shelf slope parameter (S = 0.9 for gentle slope transition)
const SHELF_SLOPE: f64 = 0.9;

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

/// Shelf filter type for the shared calculation helper.
#[derive(Clone, Copy)]
enum ShelfType {
    Low,
    High,
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

    /// Calculate shelf filter coefficients (shared implementation).
    ///
    /// Derived from bilinear transform of analog shelf prototype.
    /// Uses `SHELF_SLOPE` constant for smooth transition.
    fn shelf(shelf_type: ShelfType, freq: f64, gain_db: f64, sample_rate: f64) -> Self {
        // Clamp frequency to prevent instability near Nyquist
        let freq = Self::clamp_frequency(freq, sample_rate);

        // Convert dB gain to linear amplitude
        // A = 10^(dB/40) for peaking/shelving filters
        let a = 10.0_f64.powf(gain_db / 40.0);

        // Angular frequency (radians per sample)
        let w0 = 2.0 * PI * freq / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();

        // Shelf slope parameter
        // alpha = sin(w0)/2 * sqrt((A + 1/A) * (1/S - 1) + 2)
        let alpha = sin_w0 / 2.0 * ((a + 1.0 / a) * (1.0 / SHELF_SLOPE - 1.0) + 2.0).sqrt();

        // Intermediate terms
        let a_plus_1 = a + 1.0;
        let a_minus_1 = a - 1.0;
        let two_sqrt_a_alpha = 2.0 * a.sqrt() * alpha;

        // The difference between low and high shelf is the sign applied to
        // a_minus_1 * cos_w0 terms in certain positions
        match shelf_type {
            ShelfType::Low => {
                // Normalizing coefficient for low shelf
                let a0 = a_plus_1 + a_minus_1 * cos_w0 + two_sqrt_a_alpha;

                Self {
                    b0: (a * (a_plus_1 - a_minus_1 * cos_w0 + two_sqrt_a_alpha)) / a0,
                    b1: (2.0 * a * (a_minus_1 - a_plus_1 * cos_w0)) / a0,
                    b2: (a * (a_plus_1 - a_minus_1 * cos_w0 - two_sqrt_a_alpha)) / a0,
                    a1: (-2.0 * (a_minus_1 + a_plus_1 * cos_w0)) / a0,
                    a2: (a_plus_1 + a_minus_1 * cos_w0 - two_sqrt_a_alpha) / a0,
                }
            }
            ShelfType::High => {
                // Normalizing coefficient for high shelf
                let a0 = a_plus_1 - a_minus_1 * cos_w0 + two_sqrt_a_alpha;

                Self {
                    b0: (a * (a_plus_1 + a_minus_1 * cos_w0 + two_sqrt_a_alpha)) / a0,
                    b1: (-2.0 * a * (a_minus_1 + a_plus_1 * cos_w0)) / a0,
                    b2: (a * (a_plus_1 + a_minus_1 * cos_w0 - two_sqrt_a_alpha)) / a0,
                    a1: (2.0 * (a_minus_1 - a_plus_1 * cos_w0)) / a0,
                    a2: (a_plus_1 - a_minus_1 * cos_w0 - two_sqrt_a_alpha) / a0,
                }
            }
        }
    }

    /// Calculate low shelf filter coefficients.
    ///
    /// Derived from bilinear transform of analog low shelf prototype.
    /// Shelf slope S = 0.9 provides smooth transition.
    /// Frequency is clamped to 49% of sample rate to prevent Nyquist instability.
    fn low_shelf(freq: f64, gain_db: f64, sample_rate: f64) -> Self {
        Self::shelf(ShelfType::Low, freq, gain_db, sample_rate)
    }

    /// Calculate high shelf filter coefficients.
    ///
    /// Derived from bilinear transform of analog high shelf prototype.
    /// Shelf slope S = 0.9 provides smooth transition.
    /// Frequency is clamped to 49% of sample rate to prevent Nyquist instability.
    fn high_shelf(freq: f64, gain_db: f64, sample_rate: f64) -> Self {
        Self::shelf(ShelfType::High, freq, gain_db, sample_rate)
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
/// 3-band parametric EQ with:
/// - Low shelf (20-2000 Hz)
/// - Mid peak with adjustable Q (200-8000 Hz)
/// - High shelf (2000-20000 Hz)
///
/// Parameters are organized into flat groups for DAW display.
#[derive(Parameters)]
pub struct EqualizerParameters {
    // =========================================================================
    // Low Band (Low Shelf)
    // =========================================================================
    /// Low shelf center frequency
    #[parameter(
        id = "low_freq",
        name = "Frequency",
        default = 200.0,
        range = 20.0..=2000.0,
        kind = "hz",
        group = "Low"
    )]
    pub low_freq: FloatParameter,

    /// Low shelf gain in dB
    #[parameter(
        id = "low_gain",
        name = "Gain",
        default = 0.0,
        range = -12.0..=12.0,
        kind = "db",
        group = "Low"
    )]
    pub low_gain: FloatParameter,

    // =========================================================================
    // Mid Band (Peak/Bell)
    // =========================================================================
    /// Mid peak center frequency
    #[parameter(
        id = "mid_freq",
        name = "Frequency",
        default = 1000.0,
        range = 200.0..=8000.0,
        kind = "hz",
        group = "Mid"
    )]
    pub mid_freq: FloatParameter,

    /// Mid peak gain in dB
    #[parameter(
        id = "mid_gain",
        name = "Gain",
        default = 0.0,
        range = -12.0..=12.0,
        kind = "db",
        group = "Mid"
    )]
    pub mid_gain: FloatParameter,

    /// Mid peak Q (bandwidth control)
    #[parameter(
        id = "mid_q",
        name = "Q",
        default = 1.0,
        range = 0.1..=10.0,
        group = "Mid"
    )]
    pub mid_q: FloatParameter,

    // =========================================================================
    // High Band (High Shelf)
    // =========================================================================
    /// High shelf center frequency
    #[parameter(
        id = "high_freq",
        name = "Frequency",
        default = 4000.0,
        range = 2000.0..=20000.0,
        kind = "hz",
        group = "High"
    )]
    pub high_freq: FloatParameter,

    /// High shelf gain in dB
    #[parameter(
        id = "high_gain",
        name = "Gain",
        default = 0.0,
        range = -12.0..=12.0,
        kind = "db",
        group = "High"
    )]
    pub high_gain: FloatParameter,
}

// =============================================================================
// Descriptor
// =============================================================================

/// Equalizer plugin descriptor (unprepared state).
///
/// Configures mono I/O buses and transforms into processor when prepared.
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
            low_state: BiquadState::default(),
            mid_state: BiquadState::default(),
            high_state: BiquadState::default(),
            low_coeffs: BiquadCoeffs::default(),
            mid_coeffs: BiquadCoeffs::default(),
            high_coeffs: BiquadCoeffs::default(),
        }
    }

    // =========================================================================
    // Mono Bus Configuration
    // =========================================================================

    fn input_bus_count(&self) -> usize {
        1
    }

    fn output_bus_count(&self) -> usize {
        1
    }

    fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
        (index == 0).then(|| BusInfo::mono("Input"))
    }

    fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
        (index == 0).then(|| BusInfo::mono("Output"))
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

    /// Filter states (mono, one per band)
    low_state: BiquadState,
    mid_state: BiquadState,
    high_state: BiquadState,

    /// Filter coefficients (recalculated when parameters change)
    low_coeffs: BiquadCoeffs,
    mid_coeffs: BiquadCoeffs,
    high_coeffs: BiquadCoeffs,
}

impl EqualizerProcessor {
    /// Update filter coefficients from current parameter values.
    fn update_coefficients(&mut self) {
        self.low_coeffs = BiquadCoeffs::low_shelf(
            self.parameters.low_freq.get(),
            self.parameters.low_gain.get(),
            self.sample_rate,
        );

        self.mid_coeffs = BiquadCoeffs::peak(
            self.parameters.mid_freq.get(),
            self.parameters.mid_gain.get(),
            self.parameters.mid_q.get(),
            self.sample_rate,
        );

        self.high_coeffs = BiquadCoeffs::high_shelf(
            self.parameters.high_freq.get(),
            self.parameters.high_gain.get(),
            self.sample_rate,
        );
    }

    /// Generic processing implementation for both f32 and f64.
    fn process_generic<S: Sample>(&mut self, buffer: &mut Buffer<S>) {
        // Update coefficients at the start of each block
        self.update_coefficients();

        // Process mono buffer: chain filters low -> mid -> high
        for (input, output) in buffer.zip_channels() {
            for (in_sample, out_sample) in input.iter().zip(output.iter_mut()) {
                let sample = in_sample.to_f64();

                // Chain the three EQ bands
                let sample = self.low_state.process(sample, &self.low_coeffs);
                let sample = self.mid_state.process(sample, &self.mid_coeffs);
                let sample = self.high_state.process(sample, &self.high_coeffs);

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

// =============================================================================
// Plugin Exports
// =============================================================================

#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, EqualizerDescriptor);

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, EqualizerDescriptor);
