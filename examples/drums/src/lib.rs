//! Beamer Drums - Example drum synthesizer demonstrating multi-output buses.
//!
//! # Three-Struct Pattern
//!
//! Beamer plugins use three structs for clear separation of concerns:
//!
//! 1. **`DrumsParameters`** - Pure parameter definitions with `#[derive(Parameters)]`
//! 2. **`DrumsDescriptor`** - Plugin descriptor that holds parameters and implements `Descriptor`
//! 3. **`DrumsProcessor`** - Runtime processor created by `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - **Multi-output auxiliary buses**
//! - 4 mono output buses: Kick, Snare, Crash, Hi-Hat
//! - GM MIDI drum note mapping (notes 36, 38, 42, 49)
//! - 16-voice polyphony (4 voices per drum type)
//! - Individual synthesis algorithm per drum type
//! - Per-drum parameter groups
//! - Sample-accurate MIDI note triggering
//! - Velocity-sensitive response
//!
//! # MIDI Note Mapping (GM Standard)
//!
//! | MIDI Note | Name         | Drum Type | Output Bus |
//! |-----------|--------------|-----------|------------|
//! | 36 (C1)   | Kick Drum    | Kick      | Bus 0      |
//! | 38 (D1)   | Snare        | Snare     | Bus 1      |
//! | 42 (F#1)  | Closed Hat   | Hi-Hat    | Bus 2      |
//! | 49 (C#2)  | Crash Cymbal | Crash     | Bus 3      |
//!
//! # Multi-Output Bus Routing Pattern
//!
//! **CRITICAL**: Bus 0 is the main bus (accessed via `Buffer`), buses 1+ are auxiliary
//! (accessed via `AuxiliaryBuffers`).
//!
//! ```ignore
//! // Configuration
//! fn output_bus_count(&self) -> usize { 4 }
//! fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
//!     match index {
//!         0 => Some(BusInfo::mono("Kick")), // Main bus
//!         1 => Some(BusInfo::aux("Snare", 1)), // Aux bus 0
//!         2 => Some(BusInfo::aux("Hi-Hat", 1)), // Aux bus 1
//!         3 => Some(BusInfo::aux("Crash", 1)), // Aux bus 2
//!         _ => None,
//!     }
//! }
//!
//! // Processing
//! let kick_out = buffer.output(0); // Main bus
//! let snare_out = aux.output(0).unwrap().output(0); // Aux bus 0
//! let hihat_out = aux.output(1).unwrap().output(0); // Aux bus 1
//! let crash_out = aux.output(2).unwrap().output(0); // Aux bus 2
//! ```

use beamer::prelude::*;

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Shared plugin configuration (format-agnostic metadata)
pub static CONFIG: Config = Config::new("Beamer Drums", Category::Instrument)
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_subcategories(&[Subcategory::Drum]);

/// AU-specific configuration
/// Uses manufacturer code "Bmer" and subtype "drum" for identification
#[cfg(feature = "au")]
pub static AU_CONFIG: AuConfig = AuConfig::new("Bmer", "drum");

/// VST3-specific configuration
#[cfg(feature = "vst3")]
pub static VST3_CONFIG: Vst3Config = Vst3Config::new("D0A1B2C3-E4F5-A6B7-C8D9-E0F112233446");

/// Number of voices per drum type
const VOICES_PER_DRUM: usize = 4;

/// Pi constant for oscillator calculations
const PI: f64 = std::f64::consts::PI;

/// Two pi constant
const TWO_PI: f64 = 2.0 * PI;

/// Kick pitch envelope time constant (seconds)
const KICK_PITCH_ENV_TAU: f64 = 0.05;

/// Crash cymbal metallic oscillator frequencies (Hz).
const CRASH_METALLIC_FREQS: [f64; 6] = [4200.0, 5850.0, 7400.0, 9800.0, 12500.0, 15800.0];

// =============================================================================
// Enum Types
// =============================================================================

/// Drum type corresponding to MIDI note mapping.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum DrumType {
    Kick = 0,
    Snare = 1,
    HiHat = 2,
    Crash = 3,
}

/// Drum types in canonical order (matches voice array indices).
const DRUM_TYPES: [DrumType; 4] = [
    DrumType::Kick,
    DrumType::Snare,
    DrumType::HiHat,
    DrumType::Crash,
];

impl DrumType {
    /// Map GM MIDI drum note to drum type.
    fn from_midi_note(note: u8) -> Option<Self> {
        match note {
            36 => Some(DrumType::Kick),
            38 => Some(DrumType::Snare),
            42 => Some(DrumType::HiHat),
            49 => Some(DrumType::Crash),
            _ => None,
        }
    }
}

/// ADSR envelope stage (percussion uses only Attack-Decay).
#[derive(Copy, Clone, PartialEq, Eq)]
enum EnvelopeStage {
    Idle,
    Attack,
    Decay,
}

// =============================================================================
// Parameters
// =============================================================================

/// Plugin parameters - just output levels for each drum.
///
/// This example focuses on demonstrating multi-output buses, so synthesis
/// parameters are fixed internally. Only the output levels are exposed.
#[derive(Parameters)]
pub struct DrumsParameters {
    #[parameter(id = "kick_level", name = "Kick", default = 0.0,
                range = -60.0..=6.0, kind = "db")]
    pub kick_level: FloatParameter,

    #[parameter(id = "snare_level", name = "Snare", default = 0.0,
                range = -60.0..=6.0, kind = "db")]
    pub snare_level: FloatParameter,

    #[parameter(id = "hihat_level", name = "HiHat", default = 0.0,
                range = -60.0..=6.0, kind = "db")]
    pub hihat_level: FloatParameter,

    #[parameter(id = "crash_level", name = "Crash", default = 0.0,
                range = -60.0..=6.0, kind = "db")]
    pub crash_level: FloatParameter,
}

// =============================================================================
// Voice Architecture
// =============================================================================

/// Individual drum voice state.
#[derive(Copy, Clone)]
struct DrumVoice {
    // Voice state
    active: bool,
    note_id: i32,
    velocity: f32,
    trigger_time: u64,

    // Envelope state
    envelope_level: f64,
    envelope_stage: EnvelopeStage,

    // Synthesis state
    phase: f64, // Oscillator phase (0.0-1.0)
    metallic_phases: [f64; 6], // Hi-hat inharmonic oscillator phases
    noise_state: u32, // PRNG state for noise generation
    pitch_env_level: f64, // Kick pitch envelope level
    filter_state: f64, // One-pole filter state

    // Crash-specific state
    crash_phases: [f64; 6], // Crash cymbal metallic oscillator phases
}

impl DrumVoice {
    /// Create new voice with unique noise seed.
    fn new(drum_type: DrumType, voice_index: usize) -> Self {
        Self {
            active: false,
            note_id: -1,
            velocity: 0.0,
            trigger_time: 0,
            envelope_level: 0.0,
            envelope_stage: EnvelopeStage::Idle,
            phase: 0.0,
            metallic_phases: [0.0; 6],
            noise_state: ((drum_type as u32) << 16) | (voice_index as u32), // Unique seed
            pitch_env_level: 1.0,
            filter_state: 0.0,
            crash_phases: [0.0; 6],
        }
    }

    /// Trigger voice (soft retrigger to prevent clicks).
    fn trigger(&mut self, note_id: i32, velocity: f32, trigger_time: u64) {
        self.active = true;
        self.note_id = note_id;
        self.velocity = velocity;
        self.trigger_time = trigger_time;

        // Soft retrigger: don't reset envelope to zero (prevents clicks)
        if self.envelope_level < 0.001 {
            self.envelope_level = 0.0;
        }
        self.envelope_stage = EnvelopeStage::Attack;

        // Reset synthesis state
        self.phase = 0.0;
        self.pitch_env_level = 1.0;
        self.metallic_phases = [0.0; 6];
        self.crash_phases = [0.0; 6];
    }
}

// =============================================================================
// Descriptor
// =============================================================================

/// Plugin descriptor implementing the Descriptor trait.
#[derive(Default, HasParameters)]
pub struct DrumsDescriptor {
    #[parameters]
    pub parameters: DrumsParameters,
}

impl Descriptor for DrumsDescriptor {
    type Setup = SampleRate;
    type Processor = DrumsProcessor;

    fn prepare(mut self, setup: SampleRate) -> DrumsProcessor {
        self.parameters.set_sample_rate(setup.hz());

        // Initialize voice pools (4 voices per drum type)
        let voices: [[DrumVoice; VOICES_PER_DRUM]; 4] = std::array::from_fn(|drum_idx| {
            std::array::from_fn(|voice_idx| DrumVoice::new(DRUM_TYPES[drum_idx], voice_idx))
        });

        DrumsProcessor {
            parameters: self.parameters,
            voices,
            sample_rate: setup.hz(),
            time_counter: 0,
            pending_events: Vec::with_capacity(64),
            render_buffers: std::array::from_fn(|_| vec![0.0; MAX_BUFFER_SIZE]),
        }
    }

    fn output_bus_count(&self) -> usize {
        4 // Kick, Snare, Hi-Hat, Crash
    }

    fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
        match index {
            0 => Some(BusInfo::mono("Kick")), // Main bus
            1 => Some(BusInfo::aux("Snare", 1)), // Aux bus 0 (mono)
            2 => Some(BusInfo::aux("Hi-Hat", 1)), // Aux bus 1 (mono)
            3 => Some(BusInfo::aux("Crash", 1)), // Aux bus 2 (mono)
            _ => None,
        }
    }

    fn input_bus_count(&self) -> usize {
        0 // No audio input
    }

    fn input_bus_info(&self, _index: usize) -> Option<BusInfo> {
        None
    }

    fn wants_midi(&self) -> bool {
        true
    }
}

// =============================================================================
// Processor
// =============================================================================

/// Maximum expected buffer size for pre-allocation (covers all common DAW configs).
const MAX_BUFFER_SIZE: usize = 8192;

/// Runtime processor implementing the Processor trait.
#[derive(HasParameters)]
pub struct DrumsProcessor {
    #[parameters]
    parameters: DrumsParameters,
    voices: [[DrumVoice; VOICES_PER_DRUM]; 4], // [drum_type][voice_index]
    sample_rate: f64,
    time_counter: u64,
    pending_events: Vec<MidiEvent>,
    render_buffers: [Vec<f64>; 4], // [kick, snare, hihat, crash]
}

impl DrumsProcessor {
    /// Handle MIDI note-on event.
    fn handle_note_on(&mut self, note_id: i32, pitch: u8, velocity: f32) {
        // Map MIDI note to drum type
        let drum_type = match DrumType::from_midi_note(pitch) {
            Some(dt) => dt,
            None => return, // Ignore unmapped notes
        };

        let drum_idx = drum_type as usize;
        let voices = &mut self.voices[drum_idx];

        // Voice allocation strategy (same as synthesizer example):
        // 1. Retrigger if same note_id is already active
        for voice in voices.iter_mut() {
            if voice.note_id == note_id && voice.active {
                voice.trigger(note_id, velocity, self.time_counter);
                self.time_counter += 1;
                return;
            }
        }

        // 2. Find free voice
        for voice in voices.iter_mut() {
            if !voice.active {
                voice.trigger(note_id, velocity, self.time_counter);
                self.time_counter += 1;
                return;
            }
        }

        // 3. Steal oldest voice
        let oldest_idx = voices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| v.trigger_time)
            .map(|(idx, _)| idx)
            .unwrap_or(0);

        voices[oldest_idx].trigger(note_id, velocity, self.time_counter);
        self.time_counter += 1;
    }

    /// Render all voices of a specific drum type.
    fn render_drum_type(&mut self, drum_type: DrumType) -> f64 {
        let drum_idx = drum_type as usize;
        let sample_rate = self.sample_rate;

        let mut sum = 0.0;

        for voice in &mut self.voices[drum_idx] {
            if !voice.active {
                continue;
            }

            let sample = match drum_type {
                DrumType::Kick => synthesize_kick(voice, &self.parameters, sample_rate),
                DrumType::Snare => synthesize_snare(voice, &self.parameters, sample_rate),
                DrumType::Crash => synthesize_crash(voice, &self.parameters, sample_rate),
                DrumType::HiHat => synthesize_hihat(voice, &self.parameters, sample_rate),
            };

            sum += sample * voice.velocity as f64;
        }

        // Apply level parameter (convert dB to linear)
        let level_db = match drum_type {
            DrumType::Kick => self.parameters.kick_level.get(),
            DrumType::Snare => self.parameters.snare_level.get(),
            DrumType::Crash => self.parameters.crash_level.get(),
            DrumType::HiHat => self.parameters.hihat_level.get(),
        };

        let level_linear = if level_db <= -60.0 {
            0.0
        } else {
            10.0_f64.powf(level_db / 20.0)
        };

        sum * level_linear
    }

    /// Generic processing for any sample type.
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        aux: &mut AuxiliaryBuffers<S>,
        _context: &ProcessContext,
    ) {
        let num_samples = buffer.num_samples();

        // Clear pre-allocated render buffers (no heap allocation)
        for buf in &mut self.render_buffers {
            buf.resize(num_samples, 0.0);
            buf[..num_samples].fill(0.0);
        }

        let mut event_idx = 0;

        // Sample-accurate processing loop
        for sample_idx in 0..num_samples {
            // Process MIDI events at this sample offset
            while event_idx < self.pending_events.len() {
                let event = &self.pending_events[event_idx];
                if event.sample_offset as usize <= sample_idx {
                    if let MidiEventKind::NoteOn(note_on) = &event.event {
                        if note_on.velocity > 0.0 {
                            self.handle_note_on(note_on.note_id, note_on.pitch, note_on.velocity);
                        }
                    }
                    event_idx += 1;
                } else {
                    break;
                }
            }

            // Render each drum type (sum all voices of that type)
            let kick = self.render_drum_type(DrumType::Kick);
            let snare = self.render_drum_type(DrumType::Snare);
            let hihat = self.render_drum_type(DrumType::HiHat);
            let crash = self.render_drum_type(DrumType::Crash);
            self.render_buffers[0][sample_idx] = kick;
            self.render_buffers[1][sample_idx] = snare;
            self.render_buffers[2][sample_idx] = hihat;
            self.render_buffers[3][sample_idx] = crash;
        }

        // Write to output buses
        // Bus 0 (main) = Kick
        let kick_out = buffer.output(0);
        for (i, sample) in self.render_buffers[0][..num_samples].iter().enumerate() {
            kick_out[i] = S::from_f64(*sample);
        }

        // Bus 1 (aux 0) = Snare
        if let Some(mut snare_bus) = aux.output(0) {
            let snare_out = snare_bus.output(0);
            for (i, sample) in self.render_buffers[1][..num_samples].iter().enumerate() {
                snare_out[i] = S::from_f64(*sample);
            }
        }

        // Bus 2 (aux 1) = Hi-Hat
        if let Some(mut hihat_bus) = aux.output(1) {
            let hihat_out = hihat_bus.output(0);
            for (i, sample) in self.render_buffers[2][..num_samples].iter().enumerate() {
                hihat_out[i] = S::from_f64(*sample);
            }
        }

        // Bus 3 (aux 2) = Crash
        if let Some(mut crash_bus) = aux.output(2) {
            let crash_out = crash_bus.output(0);
            for (i, sample) in self.render_buffers[3][..num_samples].iter().enumerate() {
                crash_out[i] = S::from_f64(*sample);
            }
        }

        self.pending_events.clear();
    }
}

impl Processor for DrumsProcessor {
    type Descriptor = DrumsDescriptor;

    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
        self.process_generic(buffer, aux, context);
    }

    fn process_f64(&mut self, buffer: &mut Buffer<f64>, aux: &mut AuxiliaryBuffers<f64>, context: &ProcessContext) {
        self.process_generic(buffer, aux, context);
    }

    fn process_midi(&mut self, input: &[MidiEvent], _output: &mut MidiBuffer) {
        self.pending_events.extend_from_slice(input);
    }
}

// =============================================================================
// DSP Helper Functions
// =============================================================================

/// Process attack-decay envelope, deactivating the voice when done.
fn process_envelope(voice: &mut DrumVoice, attack_ms: f64, decay_ms: f64, sample_rate: f64) {
    let attack_samples = (attack_ms / 1000.0 * sample_rate).max(1.0);
    let decay_samples = (decay_ms / 1000.0 * sample_rate).max(1.0);

    match voice.envelope_stage {
        EnvelopeStage::Idle => {}
        EnvelopeStage::Attack => {
            voice.envelope_level += 1.0 / attack_samples;
            if voice.envelope_level >= 1.0 {
                voice.envelope_level = 1.0;
                voice.envelope_stage = EnvelopeStage::Decay;
            }
        }
        EnvelopeStage::Decay => {
            let decay_coeff = (-9.21 / decay_samples).exp(); // Reach -80dB in decay_samples
            voice.envelope_level *= decay_coeff;
            if voice.envelope_level < 0.0001 {
                voice.envelope_level = 0.0;
                voice.active = false;
                voice.envelope_stage = EnvelopeStage::Idle;
            }
        }
    }
}

/// Synthesize kick drum.
fn synthesize_kick(voice: &mut DrumVoice, _params: &DrumsParameters, sample_rate: f64) -> f64 {
    let pitch_hz = 55.0;
    let pitch_env_amount = 0.5;
    let tone = 0.4;

    process_envelope(voice, 1.0, 120.0, sample_rate);

    // Pitch envelope (exponential decay from high to low)
    let pitch_env_tau_samples = KICK_PITCH_ENV_TAU * sample_rate;
    let pitch_decay_coeff = (-1.0 / pitch_env_tau_samples).exp();
    voice.pitch_env_level *= pitch_decay_coeff;

    // Current frequency with pitch envelope
    let freq = pitch_hz * (1.0 + voice.pitch_env_level * 6.0 * pitch_env_amount);
    let phase_inc = freq / sample_rate;

    // Sine wave oscillator
    let osc = (voice.phase * TWO_PI).sin();
    voice.phase += phase_inc;
    if voice.phase >= 1.0 {
        voice.phase -= 1.0;
    }

    // Click transient (fades with pitch envelope)
    let click = xorshift32(&mut voice.noise_state) * voice.pitch_env_level * 0.3;

    // One-pole lowpass filter for tone control
    let cutoff = 80.0 + tone * 300.0;
    let filtered = one_pole_lowpass(osc * 0.9 + click * 0.1, &mut voice.filter_state, cutoff, sample_rate);

    filtered * voice.envelope_level
}

/// Synthesize snare drum.
fn synthesize_snare(voice: &mut DrumVoice, _params: &DrumsParameters, sample_rate: f64) -> f64 {
    let tune_hz = 180.0;
    let tone = 0.6;
    let snap = 0.5;

    process_envelope(voice, 1.0, 80.0, sample_rate);

    // Body: triangle wave
    let phase_inc = tune_hz / sample_rate;
    let triangle = 4.0 * (voice.phase - 0.5).abs() - 1.0;
    voice.phase += phase_inc;
    if voice.phase >= 1.0 {
        voice.phase -= 1.0;
    }

    // Noise component
    let noise = xorshift32(&mut voice.noise_state);

    // Tone filter on noise (bandpass simulation with lowpass)
    let cutoff = 2000.0 + tone * 6000.0;
    let filtered_noise = one_pole_lowpass(noise, &mut voice.filter_state, cutoff, sample_rate);

    // Mix body and noise based on snap parameter
    let mixed = triangle * (1.0 - snap) + filtered_noise * snap;

    mixed * voice.envelope_level
}

/// Synthesize crash cymbal.
fn synthesize_crash(voice: &mut DrumVoice, _params: &DrumsParameters, sample_rate: f64) -> f64 {
    let tone = 0.6;

    process_envelope(voice, 2.0, 1500.0, sample_rate);

    // Noise component
    let noise = xorshift32(&mut voice.noise_state);

    // Metallic character: 6 inharmonic oscillators
    let metallic = metallic_oscillators(&mut voice.crash_phases, &CRASH_METALLIC_FREQS, sample_rate);

    // Mix noise and metallic (noise-heavy for smoother wash)
    let mixed = noise * 0.6 + metallic * 0.4;

    // Tone filter
    let cutoff = 4000.0 + tone * 6000.0;
    let filtered = one_pole_lowpass(mixed, &mut voice.filter_state, cutoff, sample_rate);

    filtered * voice.envelope_level * 0.5
}

/// Synthesize hi-hat.
fn synthesize_hihat(voice: &mut DrumVoice, _params: &DrumsParameters, sample_rate: f64) -> f64 {
    let tone = 0.8;

    process_envelope(voice, 0.5, 40.0, sample_rate);

    // White noise
    let noise = xorshift32(&mut voice.noise_state);

    // Metallic character: 6 inharmonic square waves
    const HIHAT_METALLIC_FREQS: [f64; 6] = [8000.0, 10100.0, 12700.0, 14300.0, 16900.0, 19200.0];
    let metallic = metallic_oscillators(&mut voice.metallic_phases, &HIHAT_METALLIC_FREQS, sample_rate);

    // Mix noise and metallic
    let mixed = noise * 0.7 + metallic * 0.3;

    // High-pass effect (subtract heavily filtered version)
    let hp_cutoff = 7000.0 + tone * 8000.0;
    let lowpassed = one_pole_lowpass(mixed, &mut voice.filter_state, hp_cutoff, sample_rate);

    // High-pass = input - lowpass (simplified)
    let highpassed = mixed - lowpassed * 0.5;

    highpassed * voice.envelope_level
}

// =============================================================================
// Low-Level DSP Primitives
// =============================================================================

/// Sum 6 inharmonic square-wave oscillators for metallic cymbal character.
#[inline]
fn metallic_oscillators(phases: &mut [f64; 6], freqs: &[f64; 6], sample_rate: f64) -> f64 {
    let mut sum = 0.0;
    for (i, &freq) in freqs.iter().enumerate() {
        let phase_inc = freq / sample_rate;
        phases[i] += phase_inc;
        if phases[i] >= 1.0 {
            phases[i] -= 1.0;
        }
        sum += if phases[i] < 0.5 { 1.0 } else { -1.0 };
    }
    sum / 6.0
}

/// Simple xorshift32 PRNG for white noise generation.
#[inline]
fn xorshift32(state: &mut u32) -> f64 {
    let mut x = *state;
    if x == 0 {
        x = 1; // Prevent zero state
    }
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    // Convert to -1.0 to 1.0
    (x as f64 / u32::MAX as f64) * 2.0 - 1.0
}

/// One-pole lowpass filter.
#[inline]
fn one_pole_lowpass(input: f64, state: &mut f64, cutoff: f64, sample_rate: f64) -> f64 {
    let omega = TWO_PI * cutoff / sample_rate;
    let alpha = omega / (1.0 + omega);
    *state += alpha * (input - *state);
    *state
}

// =============================================================================
// Plugin Exports
// =============================================================================

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, DrumsDescriptor);

#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, DrumsDescriptor);
