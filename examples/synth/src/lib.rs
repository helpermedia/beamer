//! Beamer Synth - Example polyphonic synthesizer demonstrating the Beamer framework.
//!
//! # Three-Struct Pattern
//!
//! Beamer plugins use three structs for clear separation of concerns:
//!
//! 1. **`SynthParameters`** - Pure parameter definitions with `#[derive(Parameters)]`
//! 2. **`SynthDescriptor`** - Plugin descriptor that holds parameters and implements `Descriptor`
//! 3. **`SynthProcessor`** - Runtime processor created by `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - MIDI note events with sample-accurate timing
//! - 8-voice polyphony with voice stealing
//! - ADSR envelope generator
//! - Naive waveform oscillators (sine, saw, square, triangle)
//! - Simple one-pole lowpass filter with resonance
//! - `EnumParameter` for waveform selection
//! - `IntParameter` for transpose (+-2 octaves)
//! - Flat parameter groups (`group = "..."`)
//! - Parameter smoothing for filter cutoff/resonance
//! - Pitch bend messages (+-2 semitones)
//! - Mod wheel (CC 1) to control vibrato depth and filter cutoff
//! - Polyphonic aftertouch (per-note vibrato control)
//! - Channel aftertouch (global vibrato control)
//! - `SampleRate` setup for sample-rate-dependent initialization

use beamer::prelude::*;

// =============================================================================
// Plugin Configuration
// =============================================================================

/// Shared plugin configuration (format-agnostic metadata)
pub static CONFIG: Config = Config::new("Beamer Synth")
    .with_vendor("Beamer Framework")
    .with_url("https://github.com/helpermedia/beamer")
    .with_email("support@example.com")
    .with_version(env!("CARGO_PKG_VERSION"));

/// VST3-specific configuration
#[cfg(feature = "vst3")]
pub static VST3_CONFIG: Vst3Config = Vst3Config::new("B3A2C1D0-E4F5-A6B7-C8D9-E0F112233445")
    .with_categories("Instrument|Synth");

/// AU-specific configuration
/// Uses manufacturer code "Bmer" and subtype "synt" for identification
/// MusicDevice type indicates this is an instrument/synthesizer
#[cfg(feature = "au")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::MusicDevice,
    "Bmer",
    "synt",
);

/// Number of polyphonic voices
const NUM_VOICES: usize = 8;

/// Pi constant for oscillator calculations
const PI: f64 = std::f64::consts::PI;

/// Pitch bend range in semitones (±2 semitones is standard)
const PITCH_BEND_RANGE: f64 = 2.0;

/// Vibrato depth in semitones at max mod wheel
const VIBRATO_DEPTH_SEMITONES: f64 = 1.0;

/// Vibrato LFO rate in Hz
const VIBRATO_RATE_HZ: f64 = 5.0;

/// Filter cutoff modulation range in Hz (added to base cutoff when mod wheel is at max)
const CUTOFF_MOD_RANGE: f64 = 8000.0;

// =============================================================================
// Enum Types
// =============================================================================

/// Oscillator waveform selection.
#[derive(Copy, Clone, PartialEq, EnumParameter)]
pub enum Waveform {
    #[name = "Sine"]
    Sine,
    #[default]
    #[name = "Saw"]
    Saw,
    #[name = "Square"]
    Square,
    #[name = "Triangle"]
    Triangle,
}

/// ADSR envelope stage.
#[derive(Copy, Clone, PartialEq)]
enum EnvelopeStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

// =============================================================================
// Parameters
// =============================================================================

/// Synthesizer plugin parameters.
///
/// The `#[derive(Parameters)]` macro generates:
/// - `Parameters` trait (count, iter, by_id, save_state, load_state)
/// - `ParameterStore` trait (host integration)
/// - `Default` trait (from attribute values)
///
/// Parameters are organized into flat groups: Oscillator, Envelope, Filter, Global.
/// Filter parameters use exponential smoothing to prevent zipper noise.
#[derive(Parameters)]
pub struct SynthParameters {
    // =========================================================================
    // Oscillator
    // =========================================================================

    /// Oscillator waveform selection
    #[parameter(id = "waveform", name = "Waveform", group = "Oscillator")]
    pub waveform: EnumParameter<Waveform>,

    // =========================================================================
    // Envelope
    // =========================================================================

    /// Envelope attack time in milliseconds
    #[parameter(id = "attack", name = "Attack", default = 5.0, range = 1.0..=2000.0, kind = "ms", group = "Envelope")]
    pub attack: FloatParameter,

    /// Envelope decay time in milliseconds
    #[parameter(id = "decay", name = "Decay", default = 50.0, range = 1.0..=2000.0, kind = "ms", group = "Envelope")]
    pub decay: FloatParameter,

    /// Envelope sustain level (0-100%)
    #[parameter(id = "sustain", name = "Sustain", default = 0.6, range = 0.0..=1.0, kind = "percent", group = "Envelope")]
    pub sustain: FloatParameter,

    /// Envelope release time in milliseconds
    #[parameter(id = "release", name = "Release", default = 30.0, range = 1.0..=5000.0, kind = "ms", group = "Envelope")]
    pub release: FloatParameter,

    // =========================================================================
    // Filter
    // =========================================================================

    /// Lowpass filter cutoff frequency (smoothed)
    #[parameter(id = "cutoff", name = "Cutoff", default = 800.0, range = 20.0..=20000.0, kind = "hz", smoothing = "exp:5.0", group = "Filter")]
    pub cutoff: FloatParameter,

    /// Filter resonance amount (smoothed)
    #[parameter(id = "resonance", name = "Resonance", default = 0.0, range = 0.0..=0.95, kind = "percent", smoothing = "exp:5.0", group = "Filter")]
    pub resonance: FloatParameter,

    // =========================================================================
    // Global Parameters
    // =========================================================================

    /// Transpose in semitones (±2 octaves)
    #[parameter(id = "transpose", name = "Transpose", default = 0, range = -24..=24, kind = "semitones", group = "Global")]
    pub transpose: IntParameter,

    /// Master output gain in dB
    #[parameter(id = "gain", name = "Gain", default = -6.0, range = -60.0..=6.0, kind = "db", group = "Global")]
    pub gain: FloatParameter,
}

// =============================================================================
// Descriptor
// =============================================================================

/// Synthesizer plugin descriptor (unprepared state).
///
/// Holds parameters and describes the plugin to the host before audio
/// configuration is known. Transforms into `SynthProcessor` via `prepare()`.
#[derive(Default, HasParameters)]
pub struct SynthDescriptor {
    #[parameters]
    pub parameters: SynthParameters,
}

impl Descriptor for SynthDescriptor {
    // Synth needs sample rate for filter calculations.
    // See `beamer::setup` for all available types.
    type Setup = SampleRate;
    type Processor = SynthProcessor;

    fn prepare(mut self, setup: SampleRate) -> SynthProcessor {
        // Set sample rate on parameters for smoothing calculations
        self.parameters.set_sample_rate(setup.hz());

        SynthProcessor {
            parameters: self.parameters,
            voices: [Voice::new(); NUM_VOICES],
            sample_rate: setup.hz(),
            time_counter: 0,
            pending_events: Vec::with_capacity(64),
            pitch_bend: 0.0,
            mod_wheel: 0.0,
            vibrato_phase: 0.0,
            channel_pressure: 0.0,
        }
    }

    fn midi_cc_config(&self) -> Option<MidiCcConfig> {
        // Use the SYNTH_FULL preset which includes pitch bend, aftertouch,
        // mod wheel, breath, volume, pan, expression, and sustain.
        // This solves the VST3 MIDI input problem where most DAWs don't send
        // raw MIDI CC events - instead they use IMidiMapping (hidden parameters).
        Some(MidiCcConfig::SYNTH_FULL)
    }

    // =========================================================================
    // Bus Configuration
    // =========================================================================

    fn input_bus_count(&self) -> usize {
        0 // Synth has no audio input
    }

    fn input_bus_info(&self, _index: usize) -> Option<BusInfo> {
        None // No inputs
    }

    fn wants_midi(&self) -> bool {
        true // Synth needs MIDI input
    }
}

// =============================================================================
// Voice
// =============================================================================

/// A single synthesizer voice.
///
/// Each voice contains oscillator, envelope, and filter state, plus
/// per-note polyphonic pressure tracking for vibrato modulation.
/// Uses soft retrigger: when stealing a voice, the envelope level
/// is not reset (preventing clicks), but poly pressure is reset to
/// ensure new notes start without inherited vibrato.
///
/// # Voice Architecture
///
/// ```text
/// ┌─────────────┐     ┌──────────┐     ┌────────────┐
/// │ Oscillator  │────→│ Envelope │────→│   Filter   │────→ output
/// │ (waveform)  │     │  (ADSR)  │     │ (lowpass)  │
/// └─────────────┘     └──────────┘     └────────────┘
///       ↑                   ↑                ↑
///  pitch + bend         velocity        cutoff + res
/// ```
#[derive(Copy, Clone)]
struct Voice {
    // Voice state
    active: bool,
    note_id: i32,
    pitch: u8,
    velocity: f32,
    note_on_time: u64,

    // Oscillator state (phase accumulator, 0.0-1.0)
    phase: f64,

    // Envelope state (current level and stage)
    envelope_level: f64,
    envelope_stage: EnvelopeStage,

    // Filter state (one-pole lowpass with resonance)
    filter_state: f64,

    // Polyphonic pressure (per-note aftertouch, 0.0 to 1.0)
    poly_pressure: f64,
}

impl Voice {
    fn new() -> Self {
        Self {
            active: false,
            note_id: -1,
            pitch: 60,
            velocity: 1.0,
            note_on_time: 0,
            phase: 0.0,
            envelope_level: 0.0,
            envelope_stage: EnvelopeStage::Idle,
            filter_state: 0.0,
            poly_pressure: 0.0,
        }
    }

    /// Trigger a new note on this voice.
    ///
    /// Uses soft retrigger: envelope_level is NOT reset, so the attack
    /// stage ramps from the current level to 1.0, preventing clicks
    /// when stealing voices.
    fn trigger(&mut self, note_id: i32, pitch: u8, velocity: f32, time: u64) {
        self.active = true;
        self.note_id = note_id;
        self.pitch = pitch;
        self.velocity = velocity;
        self.note_on_time = time;
        self.phase = 0.0;
        // Soft retrigger: don't reset envelope_level
        self.envelope_stage = EnvelopeStage::Attack;
        // Reset polyphonic pressure (new note shouldn't inherit old pressure)
        self.poly_pressure = 0.0;
    }

    /// Release the note (enter release stage).
    fn release(&mut self) {
        if self.active && self.envelope_stage != EnvelopeStage::Release {
            self.envelope_stage = EnvelopeStage::Release;
        }
    }

    /// Process one sample of audio for this voice.
    ///
    /// # Processing Pipeline
    ///
    /// 1. **Oscillator** - Generate raw waveform at the note's frequency
    /// 2. **Envelope** - Apply ADSR amplitude shaping
    /// 3. **Filter** - Apply resonant lowpass filter
    ///
    /// # Arguments
    /// * `params` - Plugin parameters (envelope times, etc.)
    /// * `waveform` - Selected oscillator waveform
    /// * `cutoff` - Filter cutoff frequency in Hz (smoothed)
    /// * `resonance` - Filter resonance 0.0-0.95 (smoothed)
    /// * `pitch_bend` - Pitch bend amount -1.0 to +1.0
    /// * `transpose_semitones` - Transpose offset in semitones
    /// * `sample_rate` - Current sample rate in Hz
    ///
    /// # Returns
    /// The processed audio sample for this voice
    #[allow(clippy::too_many_arguments)]
    fn process_sample<S: Sample>(
        &mut self,
        params: &SynthParameters,
        waveform: Waveform,
        cutoff: f64,
        resonance: f64,
        pitch_bend: f64,
        transpose_semitones: i32,
        sample_rate: f64,
    ) -> S {
        if !self.active {
            return S::ZERO;
        }

        // =================================================================
        // 1. Oscillator - Generate waveform at note frequency
        // =================================================================
        // MIDI pitch to frequency formula:
        //   freq = 440 * 2^((pitch - 69 + bend + transpose) / 12)
        //
        // Where: pitch 69 = A4 = 440 Hz
        let bend_semitones = pitch_bend * PITCH_BEND_RANGE;
        let freq = 440.0 * 2.0_f64.powf((self.pitch as f64 - 69.0 + bend_semitones + transpose_semitones as f64) / 12.0);
        let phase_inc = freq / sample_rate;

        // Waveform generation (naive, non-bandlimited):
        //   Sine:     sin(2π * phase)
        //   Saw:      2 * phase - 1 (ramp from -1 to +1)
        //   Square:   +1 if phase < 0.5, else -1
        //   Triangle: 4 * |phase - 0.5| - 1 (tent shape)
        let osc = match waveform {
            Waveform::Sine => (self.phase * 2.0 * PI).sin(),
            Waveform::Saw => 2.0 * self.phase - 1.0,
            Waveform::Square => {
                if self.phase < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Triangle => 4.0 * (self.phase - 0.5).abs() - 1.0,
        };

        // Advance phase (wrap at 1.0)
        self.phase += phase_inc;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        // =================================================================
        // 2. ADSR Envelope
        // =================================================================
        // Classic 4-stage envelope:
        //
        // Level ^
        //   1.0 |    /\
        //       |   /  \______ Sustain
        //       |  /          \
        //   0.0 |_/____________\___
        //        A  D    S    R
        //
        let attack_samples = (params.attack.get() / 1000.0 * sample_rate).max(1.0);
        let decay_samples = (params.decay.get() / 1000.0 * sample_rate).max(1.0);
        let sustain_level = params.sustain.get();
        let release_samples = (params.release.get() / 1000.0 * sample_rate).max(1.0);

        match self.envelope_stage {
            EnvelopeStage::Idle => {}
            EnvelopeStage::Attack => {
                // Linear ramp from current level to 1.0
                self.envelope_level += 1.0 / attack_samples;
                if self.envelope_level >= 1.0 {
                    self.envelope_level = 1.0;
                    self.envelope_stage = EnvelopeStage::Decay;
                }
            }
            EnvelopeStage::Decay => {
                // Linear ramp from 1.0 to sustain level
                let decrement = (1.0 - sustain_level) / decay_samples;
                self.envelope_level -= decrement;
                if self.envelope_level <= sustain_level {
                    self.envelope_level = sustain_level;
                    self.envelope_stage = EnvelopeStage::Sustain;
                }
            }
            EnvelopeStage::Sustain => {
                // Hold at sustain level until note-off
                self.envelope_level = sustain_level;
            }
            EnvelopeStage::Release => {
                // Exponential decay to zero
                // Faster than linear, sounds more natural
                self.envelope_level -= self.envelope_level / release_samples;
                if self.envelope_level < 0.0001 {
                    self.envelope_level = 0.0;
                    self.active = false;
                    self.envelope_stage = EnvelopeStage::Idle;
                }
            }
        }

        // Apply envelope and velocity scaling
        let mut sample = osc * self.envelope_level * self.velocity as f64;

        // =================================================================
        // 3. One-Pole Lowpass Filter with Resonance
        // =================================================================
        // Simple IIR filter with feedback for resonance:
        //
        //   α = ω / (1 + ω)  where ω = 2π * cutoff / sample_rate
        //   feedback = resonance * (filter_state - input)
        //   filter_state += α * (input + feedback - filter_state)
        //
        // Higher resonance = more "squelchy" character
        let omega = 2.0 * PI * cutoff / sample_rate;
        let alpha = omega / (1.0 + omega);
        let feedback = resonance * (self.filter_state - sample);
        self.filter_state += alpha * (sample + feedback - self.filter_state);

        // Clamp filter state to prevent instability at high resonance
        self.filter_state = self.filter_state.clamp(-4.0, 4.0);
        sample = self.filter_state;

        S::from_f64(sample)
    }
}

// =============================================================================
// Processor
// =============================================================================

/// Synthesizer plugin processor (prepared state).
///
/// Ready for audio processing. Created by `SynthDescriptor::prepare()`.
/// Manages 8 polyphonic voices with sample-accurate MIDI timing and
/// oldest-note voice stealing.
#[derive(HasParameters)]
pub struct SynthProcessor {
    /// Plugin parameters
    #[parameters]
    parameters: SynthParameters,
    /// Polyphonic voices
    voices: [Voice; NUM_VOICES],
    /// Current sample rate (real value from start!)
    sample_rate: f64,
    /// Voice allocation time counter
    time_counter: u64,
    /// Pending MIDI events for sample-accurate processing
    pending_events: Vec<MidiEvent>,
    /// Current pitch bend value (-1.0 to +1.0)
    pitch_bend: f64,
    /// Current mod wheel value (0.0 to 1.0)
    mod_wheel: f64,
    /// Vibrato LFO phase (0.0 to 1.0)
    vibrato_phase: f64,
    /// Channel pressure (global aftertouch, 0.0 to 1.0)
    channel_pressure: f64,
}

impl SynthProcessor {
    /// Handle a note-on event with voice allocation.
    ///
    /// # Voice Allocation Strategy
    ///
    /// 1. **Retrigger** - If the same note_id is already playing, retrigger it
    ///    (soft retrigger: envelope continues from current level)
    /// 2. **Free voice** - Find an inactive voice and use it
    /// 3. **Voice stealing** - If all voices are active, steal the oldest one
    ///
    /// # Arguments
    /// * `note_id` - VST3 note identifier (for tracking note-off)
    /// * `pitch` - MIDI pitch (0-127, 60 = middle C)
    /// * `velocity` - Note velocity (0.0-1.0)
    fn handle_note_on(&mut self, note_id: i32, pitch: u8, velocity: f32) {
        // 1. Check for retrigger (same note_id already playing)
        for voice in &mut self.voices {
            if voice.note_id == note_id && voice.active {
                voice.trigger(note_id, pitch, velocity, self.time_counter);
                self.time_counter += 1;
                return;
            }
        }

        // 2. Find free voice
        for voice in &mut self.voices {
            if !voice.active {
                voice.trigger(note_id, pitch, velocity, self.time_counter);
                self.time_counter += 1;
                return;
            }
        }

        // 3. Steal oldest voice (simple "oldest note" algorithm)
        // More sophisticated synths might use "lowest velocity" or "release phase"
        let oldest_idx = self
            .voices
            .iter()
            .enumerate()
            .min_by_key(|(_, v)| v.note_on_time)
            .map(|(idx, _)| idx)
            .unwrap_or(0);

        self.voices[oldest_idx].trigger(note_id, pitch, velocity, self.time_counter);
        self.time_counter += 1;
    }

    /// Handle a note-off event.
    ///
    /// Finds all voices with the matching note_id and transitions them
    /// to the release stage of their ADSR envelope.
    fn handle_note_off(&mut self, note_id: i32) {
        for voice in &mut self.voices {
            if voice.note_id == note_id && voice.active {
                voice.release();
            }
        }
    }

    /// Generic processing implementation for both f32 and f64.
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        _aux: &mut AuxiliaryBuffers<S>,
        _context: &ProcessContext,
    ) {
        let num_samples = buffer.num_samples();
        let waveform = self.parameters.waveform.get();
        let gain = S::from_f64(self.parameters.gain.as_linear());

        let mut event_idx = 0;

        for sample_idx in 0..num_samples {
            // Process MIDI events at this sample offset (sample-accurate)
            while event_idx < self.pending_events.len() {
                let event = &self.pending_events[event_idx];
                if event.sample_offset as usize <= sample_idx {
                    match &event.event {
                        MidiEventKind::NoteOn(note_on) => {
                            if note_on.velocity > 0.0 {
                                self.handle_note_on(note_on.note_id, note_on.pitch, note_on.velocity);
                            } else {
                                // Velocity 0 note-on is treated as note-off
                                self.handle_note_off(note_on.note_id);
                            }
                        }
                        MidiEventKind::NoteOff(note_off) => {
                            self.handle_note_off(note_off.note_id);
                        }
                        MidiEventKind::PitchBend(pb) => {
                            // pb.value should be -1.0 to +1.0, with 0.0 as center
                            self.pitch_bend = pb.value as f64;
                        }
                        MidiEventKind::ControlChange(cc) => {
                            // CC 1 = Mod wheel
                            if cc.is_mod_wheel() {
                                self.mod_wheel = cc.value as f64;
                            }
                        }
                        MidiEventKind::PolyPressure(poly) => {
                            // Find voice(s) with matching note_id and update pressure
                            for voice in &mut self.voices {
                                if voice.note_id == poly.note_id && voice.active {
                                    voice.poly_pressure = poly.pressure as f64;
                                }
                            }
                        }
                        MidiEventKind::ChannelPressure(cp) => {
                            // Global aftertouch affects all voices
                            self.channel_pressure = cp.pressure as f64;
                        }
                        _ => {}
                    }
                    event_idx += 1;
                } else {
                    break;
                }
            }

            // Update vibrato LFO
            let vibrato_phase_inc = VIBRATO_RATE_HZ / self.sample_rate;
            self.vibrato_phase += vibrato_phase_inc;
            if self.vibrato_phase >= 1.0 {
                self.vibrato_phase -= 1.0;
            }

            // Calculate base vibrato LFO (sine wave, no scaling yet)
            let vibrato_lfo = (self.vibrato_phase * 2.0 * PI).sin();

            // =================================================================
            // Filter Modulation
            // =================================================================
            // Mod wheel controls filter brightness by adding to base cutoff.
            // - Cutoff parameter = base frequency (your starting point)
            // - Mod wheel adds up to +8000 Hz (opens filter for brightness)
            // - Clamped at 20kHz to stay below Nyquist frequency
            let base_cutoff = self.parameters.cutoff.tick_smoothed();
            let cutoff_modulation = self.mod_wheel * CUTOFF_MOD_RANGE;
            let cutoff = (base_cutoff + cutoff_modulation).min(20000.0);
            let resonance = self.parameters.resonance.tick_smoothed();

            // Render all voices
            let mut out_l = S::ZERO;
            let mut out_r = S::ZERO;

            for voice in &mut self.voices {
                if voice.active {
                    // =============================================================
                    // Per-Voice Vibrato Depth Calculation
                    // =============================================================
                    // We use a global LFO (all voices vibrato in sync) but calculate
                    // per-voice depth to allow pressure-based expression.
                    //
                    // Pressure Priority:
                    //   1. If PolyPressure > 0: use poly pressure (per-note control)
                    //   2. Else: use ChannelPressure (global aftertouch)
                    //
                    // Mod Wheel Combination:
                    //   - Mod wheel and pressure are additive (both can contribute)
                    //   - Range: 0.0 to 2.0 (allows super-expressive 2x depth)
                    //   - Example: mod wheel at 100% + pressure at 100% = 200% depth
                    let pressure_depth = if voice.poly_pressure > 0.0 {
                        voice.poly_pressure  // Use poly pressure if present
                    } else {
                        self.channel_pressure  // Fall back to channel pressure
                    };

                    let total_vibrato_depth = (self.mod_wheel + pressure_depth).min(2.0);

                    // Scale LFO by depth
                    let vibrato = vibrato_lfo * total_vibrato_depth * VIBRATO_DEPTH_SEMITONES;

                    // Per-voice pitch modulation (pitch bend + this voice's vibrato)
                    let total_pitch_mod = self.pitch_bend + vibrato / PITCH_BEND_RANGE;

                    let sample = voice.process_sample::<S>(
                        &self.parameters,
                        waveform,
                        cutoff,
                        resonance,
                        total_pitch_mod,
                        self.parameters.transpose.get() as i32,
                        self.sample_rate,
                    );
                    out_l = out_l + sample;
                    out_r = out_r + sample;
                }
            }

            // Apply master gain and write to output
            buffer.output(0)[sample_idx] = out_l * gain;
            buffer.output(1)[sample_idx] = out_r * gain;
        }

        self.pending_events.clear();
    }
}

impl Processor for SynthProcessor {
    type Descriptor = SynthDescriptor;

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &ProcessContext,
    ) {
        self.process_generic(buffer, aux, context);
    }

    fn supports_double_precision(&self) -> bool {
        true
    }

    fn process_f64(
        &mut self,
        buffer: &mut Buffer<f64>,
        aux: &mut AuxiliaryBuffers<f64>,
        context: &ProcessContext,
    ) {
        self.process_generic(buffer, aux, context);
    }

    fn process_midi(&mut self, input: &[MidiEvent], _output: &mut MidiBuffer) {
        // Store events for sample-accurate processing in process()
        self.pending_events.extend_from_slice(input);
    }

    fn wants_midi(&self) -> bool {
        true
    }

    fn tail_samples(&self) -> u32 {
        // Return max release time in samples (5 seconds)
        (5.0 * self.sample_rate) as u32
    }
}

// =============================================================================
// Plugin Exports
// =============================================================================

#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, SynthDescriptor);

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, SynthDescriptor);
