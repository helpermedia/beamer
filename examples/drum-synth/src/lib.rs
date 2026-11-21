//! Beamer Drum Synth - Example drum synthesizer: a stereo instrument whose
//! drums can each go to an output of their own.
//!
//! # Two-Struct Pattern
//!
//! Beamer plugins use two structs:
//!
//! 1. **`DrumSynthModel`** - What every thread shares, marked with
//!    `#[beamer::model]` and implementing `Model`: its parameters, how the
//!    host sees it and how it creates the processor.
//! 2. **`DrumSynthProcessor`** - What the audio thread owns, created by
//!    `prepare()`, implements `Processor`
//!
//! # Features Demonstrated
//!
//! - **Multi-output auxiliary buses**: four stereo outputs, Output 1 the
//!   main one with every drum mixed, Outputs 2 to 4 off until the host
//!   turns them on
//! - Routing per drum: an output parameter sends a drum to any of the four
//!   outputs, as drum instruments do, so each drum can have one of its
//!   own. A drum sent to an output the host does not use is not heard,
//!   until the host turns that output on
//! - A pan parameter per drum (`kind = "pan"`), which places it in the
//!   stereo picture with an equal power pan law: hi-hat and crash start a
//!   little to the sides
//! - GM MIDI drum note mapping (notes 36, 38, 42, 49), also played from the
//!   editor's pads
//! - 16-voice polyphony (4 voices per drum type)
//! - Individual synthesis algorithm per drum type
//! - A level parameter per drum
//! - A setting of the editor in a `State` field: the tab it shows, saved
//!   with the project
//! - Sample-accurate MIDI note triggering
//! - Sample-accurate automation (`sample_accurate_automation` in
//!   `Config.toml`), since the routes and the pans are read once per call
//! - Velocity-sensitive response
//! - A stream sender for the hits and a value sender for the level of
//!   each drum, for the editor
//!
//! # Editor
//!
//! The editor in `webview/` is Svelte, on the packages beamer-api,
//! beamer-controls and beamer-svelte. bun and Vite build it into
//! `webview/dist/`, which the export macro embeds. It has two tabs. Pads:
//! a pad per drum that flashes on each hit and plays the drum when pressed
//! (notes from the editor). Mixer: a channel strip per drum with a light
//! that flashes on each hit, the balance (the pan parameter), the fader of
//! the level parameter with a meter of the drum and the output it plays
//! through. The editor keeps the tab it shows in the `State` field `tab`.
//! The audio thread sends what only it knows: every hit (a stream sender)
//! and the level of each drum (a value sender).
//!
//! # MIDI Note Mapping (GM Standard)
//!
//! | MIDI Note | Name         | Drum Type |
//! |-----------|--------------|-----------|
//! | 36 (C1)   | Kick Drum    | Kick      |
//! | 38 (D1)   | Snare        | Snare     |
//! | 42 (F#1)  | Closed Hat   | Hi-Hat    |
//! | 49 (C#2)  | Crash Cymbal | Crash     |
//!
//! # Multi-Output Bus Pattern
//!
//! Bus 0 is the main bus (accessed via `Buffer`), buses 1 and up are
//! auxiliary (accessed via `AuxiliaryBuffers`). An aux output is off until
//! the host turns it on; while it is off, `aux.output()` gives `None`.
//!
//! ```ignore
//! // No input, four stereo outputs.
//! const LAYOUTS: &'static [Layout] = &[Layout {
//!     inputs: &[],
//!     outputs: &[BusLayout::STEREO; 4],
//! }];
//! fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
//!     match index {
//!         0 => Some(BusInfo::main("Output 1")),
//!         // Aux buses start off.
//!         1 => Some(BusInfo::aux("Output 2")),
//!         // ... and so on up to "Output 4".
//!         _ => None,
//!     }
//! }
//!
//! // Processing: the main bus, then aux bus 0 (the second output).
//! let left = buffer.channel(0);
//! if let Some(mut out2) = aux.output(0) { let left = out2.output(0); }
//! ```

use beamer::prelude::*;
use beamer::serde::{Deserialize, Serialize};

/// Number of voices per drum type
const VOICES_PER_DRUM: usize = 4;

/// Pi constant for oscillator calculations
const PI: f64 = std::f64::consts::PI;

/// Two pi constant
const TWO_PI: f64 = 2.0 * PI;

/// Kick pitch envelope time constant (seconds)
const KICK_PITCH_ENV_TAU: f64 = 0.05;

/// The number of output buses: the main output and three more, one for
/// each drum.
const OUTPUTS: usize = 4;

/// Crash cymbal metallic oscillator frequencies (Hz).
const CRASH_METALLIC_FREQS: [f64; 6] = [4200.0, 5850.0, 7400.0, 9800.0, 12500.0, 15800.0];

/// The lowest level the editor's meters show in dB, the level parameters'
/// lowest value too.
const METER_FLOOR_DB: f32 = -60.0;

/// How fast the held peaks of the meters fall in dB per second.
const METER_FALL_DB_PER_SECOND: f64 = 20.0;

/// The meters at rest: no drum sounds.
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

/// The output a drum plays through: Output 1 (the main output) to Output
/// 4, named as the buses are.
#[derive(Copy, Clone, PartialEq, Eq, Debug, EnumParameter)]
pub enum Output {
    #[default]
    #[name = "Output 1"]
    Output1,
    #[name = "Output 2"]
    Output2,
    #[name = "Output 3"]
    Output3,
    #[name = "Output 4"]
    Output4,
}

impl Output {
    /// The index of the output bus, 0 for the main output.
    fn bus(self) -> usize {
        match self {
            Output::Output1 => 0,
            Output::Output2 => 1,
            Output::Output3 => 2,
            Output::Output4 => 3,
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

/// The tab the editor shows: the pads or the mixer. A setting of the editor,
/// not a parameter: a host does not automate it. It is saved with the
/// project, so the editor opens on the tab it was left on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TypeScript)]
#[serde(crate = "beamer::serde", rename_all = "snake_case")]
pub enum Tab {
    #[default]
    Pads,
    Mixer,
}

/// Plugin model: a level, a pan and an output for each drum. The levels
/// start at -6 dB, which leaves the main mix of all four drums under full
/// scale. The pans start where the drums sit in the stereo picture: kick
/// and snare in the middle, the hi-hat a little to the right and the crash
/// a little to the left, as the audience sees a drummer at a right-handed
/// kit.
///
/// This example focuses on demonstrating multi-output buses, so synthesis
/// parameters are fixed internally. Only the levels, the pans and the
/// routing are exposed.
#[beamer::export]
#[beamer::model]
pub struct DrumSynthModel {
    #[parameter(id = "kick_level", name = "Kick", default = -6.0,
                range = -60.0..=6.0, kind = "db")]
    pub kick_level: FloatParameter,

    #[parameter(id = "snare_level", name = "Snare", default = -6.0,
                range = -60.0..=6.0, kind = "db")]
    pub snare_level: FloatParameter,

    #[parameter(id = "hihat_level", name = "Hi-Hat", default = -6.0,
                range = -60.0..=6.0, kind = "db")]
    pub hihat_level: FloatParameter,

    #[parameter(id = "crash_level", name = "Crash", default = -6.0,
                range = -60.0..=6.0, kind = "db")]
    pub crash_level: FloatParameter,

    #[parameter(id = "kick_pan", name = "Kick Pan", default = 0.0, kind = "pan")]
    pub kick_pan: FloatParameter,

    #[parameter(id = "snare_pan", name = "Snare Pan", default = 0.0, kind = "pan")]
    pub snare_pan: FloatParameter,

    #[parameter(id = "hihat_pan", name = "Hi-Hat Pan", default = 0.3, kind = "pan")]
    pub hihat_pan: FloatParameter,

    #[parameter(id = "crash_pan", name = "Crash Pan", default = -0.3, kind = "pan")]
    pub crash_pan: FloatParameter,

    #[parameter(id = "kick_output", name = "Kick Output")]
    pub kick_output: EnumParameter<Output>,

    #[parameter(id = "snare_output", name = "Snare Output")]
    pub snare_output: EnumParameter<Output>,

    #[parameter(id = "hihat_output", name = "Hi-Hat Output")]
    pub hihat_output: EnumParameter<Output>,

    #[parameter(id = "crash_output", name = "Crash Output")]
    pub crash_output: EnumParameter<Output>,

    /// The tab the editor shows, saved with the project under the id
    /// "tab". The editor reads and sets it; the processor never does.
    #[state(id = "tab")]
    pub tab: State<Tab>,

    /// Every drum that was hit, sent from the audio thread: the editor
    /// flashes its pad and the light of its strip.
    #[stream(id = "hits")]
    pub hits: WebViewStream<Hit>,

    /// The held peak of each drum, sent from the audio thread.
    #[value(id = "meters")]
    pub meters: WebViewValue<Meters>,

    /// The queue the senders above deliver through.
    pub events: WebViewEventQueue,
}

impl Model for DrumSynthModel {
    type Setup = (SampleRate, MaxBufferSize);
    type Processor = DrumSynthProcessor;
    // No audio input, four stereo outputs: the main output and three more
    // that start off.
    const LAYOUTS: &'static [Layout] = &[Layout {
        inputs: &[],
        outputs: &[BusLayout::STEREO; OUTPUTS],
    }];

    fn prepare(self: Arc<Self>, (sample_rate, max_buffer_size): (SampleRate, MaxBufferSize)) -> DrumSynthProcessor {
        // Initialize voice pools (4 voices per drum type)
        let voices: [[DrumVoice; VOICES_PER_DRUM]; 4] = std::array::from_fn(|drum_idx| {
            std::array::from_fn(|voice_idx| DrumVoice::new(DRUM_TYPES[drum_idx], voice_idx))
        });

        DrumSynthProcessor {
            voices,
            sample_rate: sample_rate.hz(),
            time_counter: 0,
            render_buffers: std::array::from_fn(|_| vec![0.0; max_buffer_size.0]),
            hits_sender: self.hits.sender(HITS_CAPACITY),
            meters_sender: self.meters.sender(METERS_AT_REST),
            meters: METERS_AT_REST,
            model: self,
        }
    }

    fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
        match index {
            // The main bus and the aux buses, named as the choices of the
            // output parameters. The aux buses start off.
            0 => Some(BusInfo::main("Output 1")),
            1 => Some(BusInfo::aux("Output 2")),
            2 => Some(BusInfo::aux("Output 3")),
            3 => Some(BusInfo::aux("Output 4")),
            _ => None,
        }
    }
}

// =============================================================================
// Editor
// =============================================================================

/// A drum that was hit, sent to the editor, which flashes the drum's pad
/// and the light of its strip.
/// As JSON an object with the drum (0 kick, 1 snare, 2 hi-hat, 3 crash) and
/// the velocity from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TypeScript)]
#[serde(crate = "beamer::serde")]
pub struct Hit {
    pub drum: u8,
    pub velocity: f32,
}

/// The held peak of each drum in dB, at least the floor of the meters, in
/// the order of the drums. As JSON an array of four numbers.
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

/// Runtime processor implementing the Processor trait.
pub struct DrumSynthProcessor {
    model: Arc<DrumSynthModel>,
    voices: [[DrumVoice; VOICES_PER_DRUM]; 4], // [drum_type][voice_index]
    sample_rate: f64,
    time_counter: u64,
    /// The render of each drum in the block, in the order of `DRUM_TYPES`.
    render_buffers: [Vec<f64>; 4],

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

impl DrumSynthProcessor {
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

    /// The output bus each drum plays through, as its output parameter
    /// chooses. A drum sent to a bus the host has off is not heard.
    fn routes(&self) -> [usize; 4] {
        [
            self.model.kick_output.get(),
            self.model.snare_output.get(),
            self.model.hihat_output.get(),
            self.model.crash_output.get(),
        ]
        .map(Output::bus)
    }

    /// The gain of each drum on the left and the right channel, as its pan
    /// parameter places it.
    fn drum_gains(&self) -> [[f64; 2]; 4] {
        [
            self.model.kick_pan.get(),
            self.model.snare_pan.get(),
            self.model.hihat_pan.get(),
            self.model.crash_pan.get(),
        ]
        .map(pan_gains)
    }

    /// Mix the drums that play through `bus` into `out`, the channel
    /// `channel` (0 left, 1 right) of it, each at its gain on that channel.
    /// Every bus is stereo.
    fn mix_into<S: Sample>(&self, out: &mut [S], routes: &[usize; 4], gains: &[[f64; 2]; 4], bus: usize, channel: usize, num_samples: usize) {
        for (i, sample) in out[..num_samples].iter_mut().enumerate() {
            let mut sum = 0.0;
            for (drum, route) in routes.iter().enumerate() {
                if *route == bus {
                    sum += self.render_buffers[drum][i] * gains[drum][channel];
                }
            }
            *sample = S::from_f64(sum);
        }
    }

    /// Hold the peak of each drum and send the meters. Each peak falls for
    /// the length of the block, then rises to the block's own. The editor
    /// shows the newest value it gets, so a hit between two of its frames
    /// stays visible.
    fn update_meters(&mut self, num_samples: usize) {
        let fall = (METER_FALL_DB_PER_SECOND * num_samples as f64 / self.sample_rate) as f32;
        let meters: Meters = std::array::from_fn(|drum| {
            let peak = self.render_buffers[drum][..num_samples].iter().fold(0.0_f64, |peak, sample| peak.max(sample.abs()));
            let block_db = if peak > 0.0 { (20.0 * peak.log10()) as f32 } else { METER_FLOOR_DB };
            (self.meters[drum] - fall).max(block_db).max(METER_FLOOR_DB)
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

        // Clear the render buffers, made for the host's largest block in
        // `prepare()`, so nothing is allocated here.
        for buf in &mut self.render_buffers {
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

        // Mix each drum into the output it plays through: the main bus,
        // then the aux buses the host has on. The routes and the pans are
        // read once per call; with sample-accurate automation a change in
        // the block starts a call of its own.
        // Every output is written, also one no drum plays through.
        let routes = self.routes();
        let gains = self.drum_gains();
        for channel in 0..2 {
            self.mix_into(buffer.channel(channel), &routes, &gains, 0, channel, num_samples);
        }
        for bus in 1..OUTPUTS {
            if let Some(mut output) = aux.output(bus - 1) {
                for channel in 0..2 {
                    self.mix_into(output.output(channel), &routes, &gains, bus, channel, num_samples);
                }
            }
        }
    }
}

impl Processor for DrumSynthProcessor {
    // The f64 path is native: a host that processes in f64 gets
    // `process_f64()` without a conversion.
    fn supports_double_precision(&self) -> bool {
        true
    }

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
fn synthesize_kick(voice: &mut DrumVoice, _model: &DrumSynthModel, sample_rate: f64) -> f64 {
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
fn synthesize_snare(voice: &mut DrumVoice, _model: &DrumSynthModel, sample_rate: f64) -> f64 {
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
fn synthesize_crash(voice: &mut DrumVoice, _model: &DrumSynthModel, sample_rate: f64) -> f64 {
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
fn synthesize_hihat(voice: &mut DrumVoice, _model: &DrumSynthModel, sample_rate: f64) -> f64 {
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

/// The gains of a mono source at `pan` (-1 left to 1 right) on the left
/// and the right channel, equal in power across the picture and 1.0 each in
/// the middle, so a drum in the middle keeps its level on both channels.
/// Both sides take the sine of mirrored angles, so the middle is the same
/// on both and each end leaves the other side exactly silent (a cosine of
/// pi/2 would leave a rounding error).
fn pan_gains(pan: f64) -> [f64; 2] {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    [std::f64::consts::SQRT_2 * (std::f64::consts::FRAC_PI_2 - angle).sin(), std::f64::consts::SQRT_2 * angle.sin()]
}

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
        let mut processor = prepared(DrumSynthModel::default());
        // The kick, on the main bus.
        let events = [MidiEvent::note_on(32, 9, 36, 1.0, 36, 0.0, 0)];
        let [[left, _], ..] = render(&mut processor, &events, 64);
        assert!(left[..32].iter().all(|sample| *sample == 0.0), "silent before the note");
        assert!(left[32..].iter().any(|sample| *sample != 0.0), "the kick sounds");
    }

    // Which outputs a test's host has on: the main output only, as a host
    // starts, or all four.
    const MAIN_ONLY: [bool; OUTPUTS] = [true, false, false, false];
    const ALL_ON: [bool; OUTPUTS] = [true; OUTPUTS];

    // The processor of `model` at 44.1 kHz. The queue is claimed, as a
    // plugin instance does, so that its senders work.
    fn prepared(model: DrumSynthModel) -> DrumSynthProcessor {
        model.events.claim();
        Arc::new(model).prepare((SampleRate(44100.0), MaxBufferSize(4410)))
    }

    // Render one block of `samples` with `events` into the stereo outputs
    // `on`, by bus and channel, as a wrapper does: an output that is off has
    // no channels. The buffers start full of 1.0, so a channel the
    // processor leaves unwritten shows.
    fn render_into(processor: &mut DrumSynthProcessor, events: &[MidiEvent], samples: usize, on: [bool; OUTPUTS]) -> [[Vec<f32>; 2]; OUTPUTS] {
        let mut outputs: [[Vec<f32>; 2]; OUTPUTS] = std::array::from_fn(|_| [vec![1.0; samples], vec![1.0; samples]]);
        let [main, aux_outputs @ ..] = &mut outputs;
        let [main_left, main_right] = main;
        let mut buffer = Buffer::new([&mut main_left[..], &mut main_right[..]], std::iter::empty(), 0, samples);
        let aux_channels: Vec<Vec<&mut [f32]>> = aux_outputs
            .iter_mut()
            .zip(&on[1..])
            .map(|([left, right], on)| if *on { vec![&mut left[..], &mut right[..]] } else { Vec::new() })
            .collect();
        let mut aux = AuxiliaryBuffers::new(Vec::<Vec<&[f32]>>::new(), aux_channels, samples);
        let context = ProcessContext::new(44100.0, samples, Transport::default()).with_midi_input(events);
        processor.process(&mut buffer, &mut aux, &context);
        outputs
    }

    fn render(processor: &mut DrumSynthProcessor, events: &[MidiEvent], samples: usize) -> [[Vec<f32>; 2]; OUTPUTS] {
        render_into(processor, events, samples, MAIN_ONLY)
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    }

    // The peak of both channels of an output.
    fn bus_peak([left, right]: &[Vec<f32>; 2]) -> f32 {
        peak(left).max(peak(right))
    }

    // The note-ons of the four drums at the start of a block.
    const ALL_DRUMS: [MidiEvent<'static>; 4] = [
        MidiEvent::note_on(0, 9, 36, 1.0, 36, 0.0, 0),
        MidiEvent::note_on(0, 9, 38, 1.0, 38, 0.0, 0),
        MidiEvent::note_on(0, 9, 42, 1.0, 42, 0.0, 0),
        MidiEvent::note_on(0, 9, 49, 1.0, 49, 0.0, 0),
    ];

    #[test]
    fn test_every_drum_plays_on_the_main_output_by_default() {
        let outputs = render_into(&mut prepared(DrumSynthModel::default()), &ALL_DRUMS, 512, ALL_ON);
        assert!(peak(&outputs[0][0]) > 0.1 && peak(&outputs[0][1]) > 0.1, "the main output sounds");
        for output in &outputs[1..] {
            assert_eq!(bus_peak(output), 0.0, "the other outputs are written silent");
        }
    }

    #[test]
    fn test_a_drum_goes_to_the_output_it_is_routed_to() {
        let model = DrumSynthModel::default();
        model.snare_output.set(Output::Output3);
        model.crash_output.set(Output::Output3);
        let outputs = render_into(&mut prepared(model), &[ALL_DRUMS[1], ALL_DRUMS[3]], 512, ALL_ON);
        assert!(peak(&outputs[2][0]) > 0.01 && peak(&outputs[2][1]) > 0.01, "Output 3 has the snare and the crash on both sides");
        for bus in [0, 1, 3] {
            assert_eq!(bus_peak(&outputs[bus]), 0.0, "output {bus} is silent");
        }
    }

    #[test]
    fn test_a_drum_routed_to_an_output_that_is_off_is_not_heard() {
        let model = DrumSynthModel::default();
        // Output 2 is off, Output 3 is on.
        model.kick_output.set(Output::Output2);
        model.snare_output.set(Output::Output3);
        let on = [true, false, true, false];
        let outputs = render_into(&mut prepared(model), &ALL_DRUMS[..2], 512, on);
        assert_eq!(bus_peak(&outputs[0]), 0.0, "the kick does not play on the main output");
        assert!(bus_peak(&outputs[2]) > 0.1, "the snare plays on Output 3, which is on");
    }

    #[test]
    fn test_the_hi_hat_sits_right_the_crash_left_and_the_kick_in_the_middle() {
        let [[left, right], ..] = render(&mut prepared(DrumSynthModel::default()), &ALL_DRUMS[2..3], 512);
        assert!(peak(&right) > peak(&left) * 1.3, "the hi-hat is louder on the right");
        let [[left, right], ..] = render(&mut prepared(DrumSynthModel::default()), &ALL_DRUMS[3..], 512);
        assert!(peak(&left) > peak(&right) * 1.3, "the crash is louder on the left");
        let [[left, right], ..] = render(&mut prepared(DrumSynthModel::default()), &ALL_DRUMS[..1], 512);
        assert!(peak(&left) > 0.1, "the kick sounds");
        assert_eq!(left, right, "the kick is the same on both sides");
    }

    #[test]
    fn test_the_pan_parameters_start_where_the_drums_sit() {
        let model = DrumSynthModel::default();
        let pans = [&model.kick_pan, &model.snare_pan, &model.hihat_pan, &model.crash_pan];
        let texts = pans.map(|pan| pan.display_normalized(pan.get_normalized()));
        assert_eq!(texts, ["C", "C", "R 30", "L 30"]);
        for pan in pans {
            assert_eq!((pan.normalized_to_plain(0.0), pan.normalized_to_plain(1.0)), (-1.0, 1.0), "{}", pan.info().name);
        }
    }

    #[test]
    fn test_a_pan_parameter_moves_its_drum_on_every_output() {
        // The kick hard right on the main output.
        let model = DrumSynthModel::default();
        model.kick_pan.set(1.0);
        let [[left, right], ..] = render(&mut prepared(model), &ALL_DRUMS[..1], 512);
        assert_eq!(peak(&left), 0.0, "nothing of the kick on the left");
        assert!(peak(&right) > 0.1, "the kick on the right");
        // The snare hard left on Output 2, the hi-hat moved to the middle.
        let model = DrumSynthModel::default();
        model.snare_pan.set(-1.0);
        model.snare_output.set(Output::Output2);
        model.hihat_pan.set(0.0);
        let outputs = render_into(&mut prepared(model), &ALL_DRUMS[1..3], 512, ALL_ON);
        assert!(peak(&outputs[1][0]) > 0.1, "the snare on the left of Output 2");
        assert_eq!(peak(&outputs[1][1]), 0.0, "nothing of the snare on the right of Output 2");
        assert!(peak(&outputs[0][0]) > 0.01, "the hi-hat sounds on the main output");
        assert_eq!(outputs[0][0], outputs[0][1], "the hi-hat is the same on both sides");
    }

    #[test]
    fn test_the_tab_is_saved_as_the_editor_reads_it() {
        let model = DrumSynthModel::default();
        assert_eq!(model.tab.save().unwrap(), br#""pads""#, "the editor opens on the pads");
        model.tab.load(br#""mixer""#).unwrap();
        assert_eq!(*model.tab.get(), Tab::Mixer);
        assert!(model.tab.load(br#""drums""#).is_err(), "a tab the editor does not have");
        assert_eq!(*model.tab.get(), Tab::Mixer, "is ignored");
    }

    #[test]
    fn test_the_pan_law_keeps_the_power_and_the_middle_at_full_level() {
        // Both sides take the sine of the same angle in the middle, so they
        // are equal exactly; the value is 1.0 up to the platform's sine.
        let [left, right] = pan_gains(0.0);
        assert_eq!(left, right);
        assert!((left - 1.0).abs() < 1e-12, "{left}");
        assert_eq!(pan_gains(-1.0), [std::f64::consts::SQRT_2, 0.0]);
        assert_eq!(pan_gains(1.0), [0.0, std::f64::consts::SQRT_2]);
        for pan in [-1.0, -0.3, 0.3, 0.7, 1.0] {
            let [left, right] = pan_gains(pan);
            assert!((left * left + right * right - 2.0).abs() < 1e-9, "equal power at {pan}");
            let [mirror_left, mirror_right] = pan_gains(-pan);
            assert!((left - mirror_right).abs() < 1e-9 && (right - mirror_left).abs() < 1e-9, "-{pan} mirrors {pan}");
        }
    }

    #[test]
    fn test_the_f64_path_renders_as_the_f32_path() {
        let mut processor = prepared(DrumSynthModel::default());
        assert!(processor.supports_double_precision());
        let samples = 512;
        let mut main = [vec![1.0_f64; samples], vec![1.0_f64; samples]];
        let [left, right] = &mut main;
        let mut buffer = Buffer::new([&mut left[..], &mut right[..]], std::iter::empty(), 0, samples);
        let aux_channels: Vec<Vec<&mut [f64]>> = (1..OUTPUTS).map(|_| Vec::new()).collect();
        let mut aux = AuxiliaryBuffers::new(Vec::<Vec<&[f64]>>::new(), aux_channels, samples);
        let context = ProcessContext::new(44100.0, samples, Transport::default()).with_midi_input(&ALL_DRUMS);
        processor.process_f64(&mut buffer, &mut aux, &context);
        let [[f32_left, f32_right], ..] = render(&mut prepared(DrumSynthModel::default()), &ALL_DRUMS, samples);
        for (f64_channel, f32_channel) in [(&main[0], &f32_left), (&main[1], &f32_right)] {
            assert!(f32_channel.iter().any(|sample| *sample != 0.0), "the drums sound");
            for (a, b) in f64_channel.iter().zip(f32_channel) {
                assert!((*a as f32 - b).abs() < 1e-6, "{a} against {b}");
            }
        }
    }

    #[test]
    fn test_the_main_mix_of_all_drums_stays_under_full_scale_at_the_defaults() {
        let outputs = render(&mut prepared(DrumSynthModel::default()), &ALL_DRUMS, 4096);
        assert!(bus_peak(&outputs[0]) < 1.0, "peak {}", bus_peak(&outputs[0]));
    }

    #[test]
    fn test_hit_and_meters_json_is_what_the_editor_reads() {
        let hit = Hit { drum: 2, velocity: 0.5 };
        assert_eq!(serde_json::to_string(&hit).unwrap(), r#"{"drum":2,"velocity":0.5}"#);
        assert_eq!(serde_json::to_string(&METERS_AT_REST).unwrap(), "[-60.0,-60.0,-60.0,-60.0]");
    }

    #[test]
    fn test_meters_hold_the_peak_of_each_drum_and_fall_in_silence() {
        let mut processor = prepared(DrumSynthModel::default());
        // The snare.
        let events = [MidiEvent::note_on(0, 9, 38, 1.0, 38, 0.0, 0)];
        render(&mut processor, &events, 512);
        let hit = processor.meters;
        assert!(hit[1] > -20.0, "the snare's meter shows its level: {hit:?}");
        assert_eq!(hit[0], METER_FLOOR_DB, "the kick's meter stays at the floor");
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
        let model = Arc::new(DrumSynthModel::default());
        model.events.claim();
        model.events.editor_opened();
        let mut processor = Arc::clone(&model).prepare((SampleRate(44100.0), MaxBufferSize(4410)));
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
        let model = DrumSynthModel::default();
        model.kick_level.set(0.0);
        let full = render(&mut prepared(model), &events, 512);
        let model = DrumSynthModel::default();
        model.kick_level.set(-60.0);
        let low = render(&mut prepared(model), &events, 512);
        let ratio = peak(&low[0][0]) / peak(&full[0][0]);
        assert!((ratio - 0.001).abs() < 0.0001, "-60 dB is a thousandth, not silence: {ratio}");
    }
}
