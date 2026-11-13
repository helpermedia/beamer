//! Beamer Drums - Example drum synthesizer demonstrating multi-output buses.
//!
//! # Two-Struct Pattern
//!
//! Beamer plugins use two structs:
//!
//! 1. **`DrumsModel`** - What every thread shares, marked with
//!    `#[beamer::model]` and implementing `Model`: its parameters, how the
//!    host sees it and how it creates the processor.
//! 2. **`DrumsProcessor`** - What the audio thread owns, created by
//!    `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - **Multi-output auxiliary buses**
//! - 4 mono output buses: Kick, Snare, Crash, Hi-Hat
//! - GM MIDI drum note mapping (notes 36, 38, 42, 49)
//! - 16-voice polyphony (4 voices per drum type)
//! - Individual synthesis algorithm per drum type
//! - A level parameter per drum
//! - Sample-accurate MIDI note triggering
//! - Sample-accurate automation (`sample_accurate_automation` in
//!   `Config.toml`), since the levels are read once per hit
//! - Velocity-sensitive response
//! - A stream sender for the hits and a value sender for the level of
//!   each output, for the editor
//!
//! # Editor
//!
//! The editor in `webview/` is Svelte, on the packages beamer-api,
//! beamer-controls and beamer-svelte. bun and Vite build it into
//! `webview/dist/`, which the export macro embeds. It shows a channel
//! strip per drum, in the order of the output buses: a lamp, the fader of
//! the level parameter with a meter of the drum's output and the output it
//! plays through. The audio thread sends what only it knows: every hit
//! (a stream sender) and the level of each output (a value sender).
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
//! // Configuration: no input, four mono outputs.
//! const LAYOUTS: &'static [Layout] = &[Layout {
//!     inputs: &[],
//!     outputs: &[BusLayout::MONO; 4],
//! }];
//! fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
//!     match index {
//!         // Main bus.
//!         0 => Some(BusInfo::main("Kick")),
//!         // Aux buses 0 to 2.
//!         1 => Some(BusInfo::aux("Snare")),
//!         2 => Some(BusInfo::aux("Hi-Hat")),
//!         3 => Some(BusInfo::aux("Crash")),
//!         _ => None,
//!     }
//! }
//!
//! // Processing
//! let kick_out = buffer.channel(0); // Main bus
//! let snare_out = aux.output(0).unwrap().output(0); // Aux bus 0
//! let hihat_out = aux.output(1).unwrap().output(0); // Aux bus 1
//! let crash_out = aux.output(2).unwrap().output(0); // Aux bus 2
//! ```

use beamer::prelude::*;
use beamer::serde::Serialize;

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

/// The lowest level the editor's meters show in dB, the level parameters'
/// lowest value too.
const METER_FLOOR_DB: f32 = -60.0;

/// How fast the held peaks of the meters fall in dB per second.
const METER_FALL_DB_PER_SECOND: f64 = 20.0;

/// The meters at rest: no output on any bus.
const METERS_AT_REST: Meters = [METER_FLOOR_DB; 4];

/// Room for the hits of two ticks of the editor (a tick is about 16 ms),
/// more than a drummer or a sequence plays.
const HITS_CAPACITY: usize = 256;

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
// Model
// =============================================================================

/// Plugin model - just output levels for each drum.
///
/// This example focuses on demonstrating multi-output buses, so synthesis
/// parameters are fixed internally. Only the output levels are exposed.
#[beamer::export]
#[beamer::model]
pub struct DrumsModel {
    #[parameter(id = "kick_level", name = "Kick", default = 0.0,
                range = -60.0..=6.0, kind = "db")]
    pub kick_level: FloatParameter,

    #[parameter(id = "snare_level", name = "Snare", default = 0.0,
                range = -60.0..=6.0, kind = "db")]
    pub snare_level: FloatParameter,

    #[parameter(id = "hihat_level", name = "Hi-Hat", default = 0.0,
                range = -60.0..=6.0, kind = "db")]
    pub hihat_level: FloatParameter,

    #[parameter(id = "crash_level", name = "Crash", default = 0.0,
                range = -60.0..=6.0, kind = "db")]
    pub crash_level: FloatParameter,

    /// Events for the editor. The processor's senders are made from it.
    pub events: WebViewEventQueue,
}

impl Model for DrumsModel {
    type Setup = SampleRate;
    type Processor = DrumsProcessor;
    // No audio input, a mono output each for kick, snare, hi-hat and crash.
    const LAYOUTS: &'static [Layout] = &[Layout {
        inputs: &[],
        outputs: &[BusLayout::MONO; 4],
    }];

    fn prepare(self: Arc<Self>, setup: SampleRate) -> DrumsProcessor {
        // Initialize voice pools (4 voices per drum type)
        let voices: [[DrumVoice; VOICES_PER_DRUM]; 4] = std::array::from_fn(|drum_idx| {
            std::array::from_fn(|voice_idx| DrumVoice::new(DRUM_TYPES[drum_idx], voice_idx))
        });

        DrumsProcessor {
            voices,
            sample_rate: setup.hz(),
            time_counter: 0,
            render_buffers: std::array::from_fn(|_| vec![0.0; MAX_BUFFER_SIZE]),
            hits_sender: self.events.stream_sender("hits", HITS_CAPACITY),
            meters_sender: self.events.value_sender("meters", METERS_AT_REST),
            meters: METERS_AT_REST,
            model: self,
        }
    }

    fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
        match index {
            // Main bus.
            0 => Some(BusInfo::main("Kick")),
            // Aux buses 0 to 2.
            1 => Some(BusInfo::aux("Snare")),
            2 => Some(BusInfo::aux("Hi-Hat")),
            3 => Some(BusInfo::aux("Crash")),
            _ => None,
        }
    }
}

// =============================================================================
// Editor
// =============================================================================

/// A drum that was hit, sent to the editor, which lights the drum's lamp.
/// As JSON an object with the drum (0 kick, 1 snare, 2 hi-hat, 3 crash, the
/// order of the output buses) and the velocity from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(crate = "beamer::serde")]
pub struct Hit {
    pub drum: u8,
    pub velocity: f32,
}

/// The held peak of each output bus in dB, at least the floor of the
/// meters, in the order of the buses. As JSON an array of four numbers.
pub type Meters = [f32; 4];

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
// Processor
// =============================================================================

/// Maximum expected buffer size for pre-allocation (covers all common DAW configs).
const MAX_BUFFER_SIZE: usize = 8192;

/// Runtime processor implementing the Processor trait.
pub struct DrumsProcessor {
    model: Arc<DrumsModel>,
    voices: [[DrumVoice; VOICES_PER_DRUM]; 4], // [drum_type][voice_index]
    sample_rate: f64,
    time_counter: u64,
    render_buffers: [Vec<f64>; 4], // [kick, snare, hihat, crash]

    /// Sends every hit to the editor. Sends nothing while no editor is
    /// open.
    hits_sender: WebViewStreamSender<Hit>,

    /// Sends the meters with every block while they show something. A pause
    /// tells the editor that the host stopped calling the plugin (Logic's
    /// power button, Reaper's FX switch): it lets them fall on its own then.
    meters_sender: WebViewValueSender<Meters>,

    /// The meters `meters_sender` sent last.
    meters: Meters,
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
        // Tell the editor. Pushing never waits or allocates.
        self.hits_sender.push(Hit { drum: drum_idx as u8, velocity });
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
                DrumType::Kick => synthesize_kick(voice, &self.model, sample_rate),
                DrumType::Snare => synthesize_snare(voice, &self.model, sample_rate),
                DrumType::Crash => synthesize_crash(voice, &self.model, sample_rate),
                DrumType::HiHat => synthesize_hihat(voice, &self.model, sample_rate),
            };

            sum += sample * voice.velocity as f64;
        }

        // The level parameter as a gain: -60 dB at its lowest, as its text
        // says, not silence.
        let level = match drum_type {
            DrumType::Kick => &self.model.kick_level,
            DrumType::Snare => &self.model.snare_level,
            DrumType::Crash => &self.model.crash_level,
            DrumType::HiHat => &self.model.hihat_level,
        };

        sum * level.as_linear()
    }

    /// Hold the peak of each output and send the meters. Each peak falls for
    /// the length of the block, then rises to the block's own. The editor
    /// shows the newest value it gets, so a hit between two of its frames
    /// stays visible.
    fn update_meters(&mut self, num_samples: usize) {
        let fall = (METER_FALL_DB_PER_SECOND * num_samples as f64 / self.sample_rate) as f32;
        let meters: Meters = std::array::from_fn(|bus| {
            let peak = self.render_buffers[bus][..num_samples].iter().fold(0.0_f64, |peak, sample| peak.max(sample.abs()));
            let block_db = if peak > 0.0 { (20.0 * peak.log10()) as f32 } else { METER_FLOOR_DB };
            (self.meters[bus] - fall).max(block_db).max(METER_FLOOR_DB)
        });
        // While the meters show something they go out with every block, also
        // when they hold still. Meters at rest send nothing, so the editor
        // has nothing to do.
        if meters != self.meters || meters != METERS_AT_REST {
            self.meters = meters;
            self.meters_sender.send(&meters);
        }
    }

    /// Generic processing for any sample type.
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        aux: &mut AuxiliaryBuffers<S>,
        context: &ProcessContext,
    ) {
        let num_samples = buffer.num_samples();
        // The MIDI of this call, sorted by sample offset.
        let events = context.midi_input();

        // Clear pre-allocated render buffers (no heap allocation)
        for buf in &mut self.render_buffers {
            buf.resize(num_samples, 0.0);
            buf[..num_samples].fill(0.0);
        }

        let mut event_idx = 0;

        // Sample-accurate processing loop
        for sample_idx in 0..num_samples {
            // Process MIDI events at this sample offset
            while event_idx < events.len() {
                let event = &events[event_idx];
                if event.sample_offset as usize <= sample_idx {
                    if let MidiEventKind::NoteOn(note_on) = &event.event {
                        self.handle_note_on(note_on.note_id, note_on.pitch, note_on.velocity);
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

        self.update_meters(num_samples);

        // Write to output buses
        // Bus 0 (main) = Kick
        let kick_out = buffer.channel(0);
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
    }
}

impl Processor for DrumsProcessor {
    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
        self.process_generic(buffer, aux, context);
    }

    fn process_f64(&mut self, buffer: &mut Buffer<f64>, aux: &mut AuxiliaryBuffers<f64>, context: &ProcessContext) {
        self.process_generic(buffer, aux, context);
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
fn synthesize_kick(voice: &mut DrumVoice, _model: &DrumsModel, sample_rate: f64) -> f64 {
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
fn synthesize_snare(voice: &mut DrumVoice, _model: &DrumsModel, sample_rate: f64) -> f64 {
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
fn synthesize_crash(voice: &mut DrumVoice, _model: &DrumsModel, sample_rate: f64) -> f64 {
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
fn synthesize_hihat(voice: &mut DrumVoice, _model: &DrumsModel, sample_rate: f64) -> f64 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_note_in_the_context_starts_at_its_offset() {
        let mut processor = prepared(DrumsModel::default());
        // The kick, on the main bus.
        let events = [MidiEvent::note_on(32, 9, 36, 1.0, 36, 0.0, 0)];
        let mut kick = vec![0.0f32; 64];
        let mut buffer = Buffer::new([&mut kick[..]], std::iter::empty(), 0, 64);
        let mut aux = AuxiliaryBuffers::empty();
        let context = ProcessContext::new(44100.0, 64, Transport::default()).with_midi_input(&events);
        processor.process(&mut buffer, &mut aux, &context);
        assert!(kick[..32].iter().all(|sample| *sample == 0.0), "silent before the note");
        assert!(kick[32..].iter().any(|sample| *sample != 0.0), "the kick sounds");
    }

    // The processor of `model` at 44.1 kHz. The queue is claimed, as a
    // plugin instance does, so that its senders work.
    fn prepared(model: DrumsModel) -> DrumsProcessor {
        model.events.claim();
        Arc::new(model).prepare(SampleRate(44100.0))
    }

    // Render one block of `samples` with `events`, the kick on the main bus
    // and the others on three aux buses.
    fn render(processor: &mut DrumsProcessor, events: &[MidiEvent], samples: usize) -> [Vec<f32>; 4] {
        let mut outputs: [Vec<f32>; 4] = std::array::from_fn(|_| vec![0.0; samples]);
        let [kick, snare, hihat, crash] = &mut outputs;
        let mut buffer = Buffer::new([&mut kick[..]], std::iter::empty(), 0, samples);
        let mut aux = AuxiliaryBuffers::new(
            Vec::<Vec<&[f32]>>::new(),
            vec![vec![&mut snare[..]], vec![&mut hihat[..]], vec![&mut crash[..]]],
            samples,
        );
        let context = ProcessContext::new(44100.0, samples, Transport::default()).with_midi_input(events);
        processor.process(&mut buffer, &mut aux, &context);
        outputs
    }

    #[test]
    fn test_hit_and_meters_json_is_what_the_editor_reads() {
        let hit = Hit { drum: 2, velocity: 0.5 };
        assert_eq!(serde_json::to_string(&hit).unwrap(), r#"{"drum":2,"velocity":0.5}"#);
        assert_eq!(serde_json::to_string(&METERS_AT_REST).unwrap(), "[-60.0,-60.0,-60.0,-60.0]");
    }

    #[test]
    fn test_meters_hold_the_peak_of_each_bus_and_fall_in_silence() {
        let mut processor = prepared(DrumsModel::default());
        // The snare, on aux bus 0.
        let events = [MidiEvent::note_on(0, 9, 38, 1.0, 38, 0.0, 0)];
        render(&mut processor, &events, 512);
        let hit = processor.meters;
        assert!(hit[1] > -20.0, "the snare's bus shows its level: {hit:?}");
        assert_eq!(hit[0], METER_FLOOR_DB, "the kick's bus stays at the floor");
        // Ten blocks of 4410 samples: a second, long after the snare. The
        // peak it held falls 20 dB.
        for _ in 0..10 {
            render(&mut processor, &[], 4410);
        }
        assert!((processor.meters[1] - (hit[1] - 20.0)).abs() < 0.5, "the peak falls 20 dB a second: {:?}", processor.meters);
        // Three more seconds and every meter rests at the floor.
        for _ in 0..30 {
            render(&mut processor, &[], 4410);
        }
        assert_eq!(processor.meters, METERS_AT_REST);
    }

    #[test]
    fn test_the_editor_gets_every_hit_and_the_meters_until_they_rest() {
        let model = Arc::new(DrumsModel::default());
        model.events.claim();
        model.events.editor_opened();
        let mut processor = Arc::clone(&model).prepare(SampleRate(44100.0));
        // The snare softly, a note no drum plays and the crash.
        let events = [
            MidiEvent::note_on(0, 9, 38, 0.5, 38, 0.0, 0),
            MidiEvent::note_on(0, 9, 60, 1.0, 60, 0.0, 0),
            MidiEvent::note_on(0, 9, 49, 1.0, 49, 0.0, 0),
        ];
        render(&mut processor, &events, 512);
        let script = model.events.take_script().expect("the hits and the meters go out");
        assert!(
            script.contains(r#"_onEvent("hits",[{"drum":1,"velocity":0.5},{"drum":3,"velocity":1.0}]);"#),
            "{script}"
        );
        assert!(script.contains(r#"_onValue("meters",["#), "{script}");
        // The meters go out once more when they come to rest, then nothing.
        for _ in 0..50 {
            render(&mut processor, &[], 4410);
        }
        let script = model.events.take_script().expect("the meters at rest go out");
        assert!(script.contains(r#"_onValue("meters",[-60.0,-60.0,-60.0,-60.0]);"#), "{script}");
        render(&mut processor, &[], 4410);
        assert_eq!(model.events.take_script(), None);
    }

    #[test]
    fn test_the_lowest_level_plays_at_minus_60_db() {
        let events = [MidiEvent::note_on(0, 9, 36, 1.0, 36, 0.0, 0)];
        let full = render(&mut prepared(DrumsModel::default()), &events, 512);
        let model = DrumsModel::default();
        model.kick_level.set(-60.0);
        let low = render(&mut prepared(model), &events, 512);
        let peak = |samples: &[f32]| samples.iter().fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        let ratio = peak(&low[0]) / peak(&full[0]);
        assert!((ratio - 0.001).abs() < 0.0001, "-60 dB is a thousandth, not silence: {ratio}");
    }
}
