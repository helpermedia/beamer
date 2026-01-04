# Beamer API Reference

**Version:** 0.1.0

This document provides detailed API documentation for Beamer. For high-level architecture and design decisions, see [ARCHITECTURE.md](../ARCHITECTURE.md).

---

## Table of Contents

1. [Core API](#1-core-api)
2. [MIDI Reference](#2-midi-reference)
3. [VST3 Integration](#3-vst3-integration)
4. [Future Phases](#4-future-phases)
5. [Appendices](#appendices)

---

## 1. Core API

> **Note**: Many API behaviors depend on VST3 host requirements. See [Section 3: VST3 Integration](#3-vst3-integration) for critical constraints like entry point naming, bundle structure, and parameter flag mapping.

### 1.1 Plugin Trait

```rust
pub trait Plugin: Send + Sync + 'static {
    /// Plugin configuration (name, UID, buses, etc.)
    fn config() -> &'static PluginConfig;

    /// Parameter definitions
    fn params(&self) -> &dyn Parameters;

    /// Create the audio processor
    fn create_processor(&self) -> Box<dyn AudioProcessor>;
}
```

### 1.2 AudioProcessor Trait

```rust
pub trait AudioProcessor: Send {
    /// Called when sample rate or max buffer size changes.
    fn setup(&mut self, sample_rate: f64, max_buffer_size: usize);

    /// Process audio. Called on the audio thread.
    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &ProcessContext,
    );

    /// Process MIDI events. Called before process() each block.
    fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
        // Default: pass through
        for event in input {
            output.push(*event);
        }
    }

    /// Whether this plugin wants MIDI input.
    fn wants_midi(&self) -> bool { false }

    /// Tail length in samples (for reverbs, delays).
    fn tail_samples(&self) -> u32 { 0 }

    /// Bypass crossfade duration in samples.
    fn bypass_ramp_samples(&self) -> u32 { 64 }

    /// Whether this plugin supports f64 processing natively.
    fn supports_double_precision(&self) -> bool { false }

    /// Process audio in f64. Only called if supports_double_precision() is true.
    fn process_f64(
        &mut self,
        buffer: &mut Buffer<f64>,
        aux: &mut AuxiliaryBuffers<f64>,
        context: &ProcessContext,
    ) {
        // Default: no-op (framework converts via f32 path)
    }
}
```

### 1.3 Parameters

Beamer provides two parameter APIs:
- **`Parameters` trait**: Low-level VST3 integration (manual implementation)
- **`Params` trait + derive macro**: High-level ergonomic API (recommended)
- **Parameter smoothing**: Opt-in smoothing to avoid zipper noise during automation

#### Derive Macro (Recommended)

**Declarative Style** — Macro generates everything including `Default`:

```rust
use beamer::prelude::*;
use beamer::Params;

#[derive(Params)]
pub struct GainParams {
    #[param(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParam,

    #[param(id = "bypass", bypass)]
    pub bypass: BoolParam,
}

// No manual new() or Default impl needed - macro generates everything!
```

The `#[derive(Params)]` macro generates:
- `Params` trait implementation (count, iter, by_id, save_state, load_state)
- `Parameters` trait implementation (VST3 integration)
- `Units` trait implementation (parameter groups)
- `Default` implementation (when all required attributes are present)
- Compile-time FNV-1a hash constants: `PARAM_GAIN_VST3_ID`, `PARAM_BYPASS_VST3_ID`
- Compile-time collision detection for duplicate IDs

#### Declarative Attributes

| Attribute | Description | Required |
|-----------|-------------|----------|
| `id = "..."` | String ID (hashed to u32 for VST3) | Yes |
| `name = "..."` | Display name in DAW | For Default |
| `default = <value>` | Default value (float, int, or bool) | For Default |
| `range = start..=end` | Value range | For FloatParam/IntParam |
| `kind = "..."` | Unit type (see below) | Optional |
| `group = "..."` | Visual grouping without nested struct | Optional |
| `short_name = "..."` | Short name for constrained UIs | Optional |
| `smoothing = "exp:5.0"` | Parameter smoothing (`exp` or `linear`) | Optional |
| `bypass` | Mark as bypass parameter (BoolParam only) | Optional |

**Kind Values:** `db`, `hz`, `ms`, `seconds`, `percent`, `pan`, `ratio`, `linear`, `semitones`

Supported field types: `FloatParam`, `IntParam`, `BoolParam`, `EnumParam<E>`

#### Parameter Types

**FloatParam** — Continuous floating-point parameter:

```rust
// Linear range
let freq = FloatParam::new("Frequency", 1000.0, 20.0..=20000.0);

// Decibel range (stores dB, use as_linear() for DSP)
let gain = FloatParam::db("Gain", 0.0, -60.0..=12.0);

// In DSP code:
let amplitude = gain.as_linear();  // 0 dB → 1.0, -6 dB → ~0.5
let db_value = gain.get();         // Returns dB for display
```

**IntParam** — Integer parameter:

```rust
let voices = IntParam::new("Voices", 8, 1..=64);
```

**BoolParam** — Toggle parameter:

```rust
let bypass = BoolParam::new("Bypass", false);
```

**EnumParam** — Discrete choice parameter:

```rust
use beamer::EnumParam as DeriveEnumParam;

#[derive(Copy, Clone, PartialEq, DeriveEnumParam)]
pub enum FilterType {
    #[name = "Low Pass"]
    LowPass,
    #[default]
    #[name = "High Pass"]
    HighPass,
    #[name = "Band Pass"]
    BandPass,
    Notch,  // Uses "Notch" as display name
}

#[derive(Params)]
pub struct FilterParams {
    #[param(id = "filter_type")]
    pub filter_type: EnumParam<FilterType>,
}

impl Default for FilterParams {
    fn default() -> Self {
        Self {
            // Uses HighPass (from #[default]) as the default value
            filter_type: EnumParam::new("Filter Type"),
        }
    }
}

// In DSP code:
match self.params.filter_type.get() {
    FilterType::LowPass => { /* ... */ }
    FilterType::HighPass => { /* ... */ }
    FilterType::BandPass => { /* ... */ }
    FilterType::Notch => { /* ... */ }
}
```

The `#[derive(EnumParam)]` macro generates the `EnumParamValue` trait implementation.

| Attribute | Purpose |
|-----------|---------|
| `#[name = "..."]` | Display name for variant (defaults to identifier) |
| `#[default]` | Mark as default variant (defaults to first) |

EnumParam constructors:

| Constructor | Purpose |
|-------------|---------|
| `EnumParam::new(name)` | Uses `#[default]` variant or first |
| `EnumParam::with_value(name, variant)` | Explicit default override |

#### Parameter Smoothing

Avoid zipper noise during automation by adding smoothing to parameters:

```rust
// Add smoother during parameter creation
let gain = FloatParam::db("Gain", 0.0, -60.0..=12.0)
    .with_smoother(SmoothingStyle::Exponential(5.0));  // 5ms time constant
```

**Smoothing Styles:**

| Style | Behavior | Use Case |
|-------|----------|----------|
| `SmoothingStyle::None` | Instant (default) | Non-audio parameters |
| `SmoothingStyle::Linear(ms)` | Linear ramp | Predictable timing |
| `SmoothingStyle::Exponential(ms)` | One-pole IIR, can cross zero | dB gain, most musical parameters |
| `SmoothingStyle::Logarithmic(ms)` | Log-domain, positive values only | Frequencies (Hz), other positive-only params |

**Sample Rate Initialization:**

Call `set_sample_rate()` in `setup()` to initialize smoothers:

```rust
impl AudioProcessor for MyPlugin {
    fn setup(&mut self, sample_rate: f64, _max_buffer_size: usize) {
        self.params.set_sample_rate(sample_rate);
    }
}
```

> **Oversampling:** If your plugin uses oversampling, pass the actual processing rate:
> `self.params.set_sample_rate(sample_rate * oversampling_factor as f64);`

**Per-Sample Processing:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &ProcessContext) {
    for (input, output) in buffer.zip_channels() {
        for (i, o) in input.iter().zip(output.iter_mut()) {
            let gain = self.params.gain.next_smoothed();  // Advances smoother
            *o = *i * gain as f32;
        }
    }
}
```

**Block Processing:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &ProcessContext) {
    let gain = self.params.gain.smoothed();  // Current value, no advance
    self.params.gain.skip_smoothing(buffer.len());

    for (input, output) in buffer.zip_channels() {
        for (i, o) in input.iter().zip(output.iter_mut()) {
            *o = *i * gain as f32;
        }
    }
}
```

**Buffer Fill:**

```rust
let mut gain_buffer = [0.0f32; 512];
let len = buffer.len().min(512);
self.params.gain.fill_smoothed_f32(&mut gain_buffer[..len]);
// Use gain_buffer[i] per sample
```

**Smoothing API:**

| Method | Description |
|--------|-------------|
| `.with_smoother(style)` | Builder: add smoothing to parameter |
| `.set_sample_rate(sr)` | Initialize with sample rate (call in setup) |
| `.next_smoothed()` | Get next value, advance smoother (per-sample) |
| `.smoothed()` | Get current value without advancing |
| `.skip_smoothing(n)` | Skip n samples (block processing) |
| `.fill_smoothed(buf)` | Fill buffer with smoothed values |
| `.is_smoothing()` | Check if currently ramping |
| `.reset_smoothing()` | Reset to current value (no ramp) |

**Thread Safety Note:**

Smoothing methods require `&mut self` and run on the audio thread only. The underlying parameter value uses atomic storage for thread-safe access from UI/host threads.

**Automatic Reset on State Load:**

The framework automatically calls `reset_smoothing()` after loading state to prevent unwanted ramps to loaded parameter values.

#### Flat Visual Grouping

Use `group = "..."` for visual grouping in the DAW without nested structs:

```rust
#[derive(Params)]
pub struct SynthParams {
    #[param(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
    pub cutoff: FloatParam,

    #[param(id = "reso", name = "Resonance", default = 0.5, range = 0.0..=1.0, group = "Filter")]
    pub resonance: FloatParam,

    #[param(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db", group = "Output")]
    pub gain: FloatParam,
}

// Access is flat: params.cutoff, params.resonance, params.gain
// DAW shows collapsible "Filter" and "Output" groups
```

**Flat vs Nested Grouping:**

| Feature | Flat (`group = "..."`) | Nested (`#[nested(...)]`) |
|---------|------------------------|---------------------------|
| Struct layout | Single struct | Separate struct per group |
| Access pattern | `params.cutoff` | `params.filter.cutoff` |
| Reusability | N/A | Same struct reusable |
| Complexity | Simple | More structure |

Choose flat grouping for simple organization; nested for reusable parameter collections.

#### Nested Parameter Groups

Use `#[nested]` to organize parameters into separate structs with VST3 units:

```rust
#[derive(Params)]
pub struct SynthParams {
    #[param(id = "master", name = "Master", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub master: FloatParam,

    #[nested(group = "Filter")]
    pub filter: FilterParams,

    #[nested(group = "Amp Envelope")]
    pub amp_env: EnvelopeParams,
}

#[derive(Params)]
pub struct FilterParams {
    #[param(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz")]
    pub cutoff: FloatParam,

    #[param(id = "resonance", name = "Resonance", default = 0.5, range = 0.0..=1.0)]
    pub resonance: FloatParam,
}

#[derive(Params)]
pub struct EnvelopeParams {
    #[param(id = "attack", name = "Attack", default = 10.0, range = 0.1..=1000.0, kind = "ms")]
    pub attack: FloatParam,

    #[param(id = "release", name = "Release", default = 100.0, range = 0.1..=5000.0, kind = "ms")]
    pub release: FloatParam,
}
```

With declarative attributes, `set_unit_ids()` is called automatically in the generated `Default` implementation.

#### State Serialization Format

Parameters are serialized using path-based IDs to support nested groups without collisions:

```
Format: [path_len: u8][path: utf8][value: f64]*

Path examples:
- "gain"              — top-level parameter
- "filter/cutoff"     — parameter in "Filter" group
- "osc1/filter/res"   — deeply nested parameter
```

The same nested struct can be reused in multiple groups without ID collision:

```rust
#[nested(group = "Osc 1")]
pub osc1: OscParams,

#[nested(group = "Osc 2")]
pub osc2: OscParams,  // Same struct, different paths: "osc1/attack" vs "osc2/attack"
```

#### Low-Level Parameters Trait

For manual control, implement `Parameters` directly:

```rust
pub trait Parameters: Send + Sync {
    fn count(&self) -> usize;
    fn info(&self, index: usize) -> Option<&ParamInfo>;
    fn get_normalized(&self, id: ParamId) -> ParamValue;
    fn set_normalized(&self, id: ParamId, value: ParamValue);
    fn normalized_to_string(&self, id: ParamId, normalized: ParamValue) -> String;
    fn string_to_normalized(&self, id: ParamId, string: &str) -> Option<ParamValue>;
    fn normalized_to_plain(&self, id: ParamId, normalized: ParamValue) -> ParamValue;
    fn plain_to_normalized(&self, id: ParamId, plain: ParamValue) -> ParamValue;
}

pub struct ParamInfo {
    pub id: ParamId,
    pub name: &'static str,
    pub short_name: &'static str,
    pub units: &'static str,
    pub default_normalized: f64,
    pub step_count: i32,
    pub flags: ParamFlags,
    pub unit_id: UnitId,  // VST3 parameter group (0 = root)
}

pub struct ParamFlags {
    pub can_automate: bool,
    pub is_readonly: bool,
    pub is_bypass: bool,  // Maps to VST3 kIsBypass (see §3.2)
}

impl ParamInfo {
    /// Convenience constructor for bypass parameters.
    pub const fn bypass(id: ParamId) -> Self;
}
```

### 1.4 Buffer Types

Beamer uses a two-buffer architecture for multi-bus support with stack allocation for real-time safety.

#### Main Buffer

```rust
/// Main audio buffer (main bus only).
/// Generic over sample type S (f32 or f64).
pub struct Buffer<'a, S: Sample = f32> {
    inputs: [Option<&'a [S]>; MAX_CHANNELS],
    outputs: [Option<&'a mut [S]>; MAX_CHANNELS],
    num_inputs: usize,
    num_outputs: usize,
    num_samples: usize,
}

impl<'a, S: Sample> Buffer<'a, S> {
    pub fn num_samples(&self) -> usize;
    pub fn num_input_channels(&self) -> usize;
    pub fn num_output_channels(&self) -> usize;
    pub fn input(&self, channel: usize) -> &[S];
    pub fn output(&mut self, channel: usize) -> &mut [S];
    pub fn copy_to_output(&mut self);
    pub fn zip_channels(&mut self) -> impl Iterator<Item = (&[S], &mut [S])>;
    pub fn apply_output_gain(&mut self, gain: S);
}
```

#### Auxiliary Buffers

```rust
/// Auxiliary buffers for sidechain and multi-bus.
pub struct AuxiliaryBuffers<'a, S: Sample = f32> { /* ... */ }

impl<'a, S: Sample> AuxiliaryBuffers<'a, S> {
    /// Get the first auxiliary input (typically sidechain).
    pub fn sidechain(&self) -> Option<AuxInput<'_, S>>;

    /// Get auxiliary input by index.
    pub fn input(&self, bus: usize) -> Option<AuxInput<'_, S>>;

    /// Get auxiliary output by index.
    pub fn output(&mut self, bus: usize) -> Option<AuxOutput<'_, 'a, S>>;
}

/// Immutable view of an auxiliary input bus.
pub struct AuxInput<'a, S: Sample> { /* ... */ }

impl<'a, S: Sample> AuxInput<'a, S> {
    pub fn num_channels(&self) -> usize;
    pub fn channel(&self, index: usize) -> &[S];
    pub fn rms(&self, channel: usize) -> S;
}

/// Mutable view of an auxiliary output bus.
/// Two lifetimes resolve variance issues with nested mutable references.
pub struct AuxOutput<'borrow, 'data, S: Sample> { /* ... */ }

impl<'borrow, 'data, S: Sample> AuxOutput<'borrow, 'data, S> {
    pub fn num_channels(&self) -> usize;
    pub fn channel(&mut self, index: usize) -> &mut [S];
    pub fn iter_channels(&mut self) -> impl Iterator<Item = &mut [S]>;
    pub fn clear(&mut self);
}
```

**Why Two Lifetimes for AuxOutput?**

The type `&'a mut [&'a mut T]` is **invariant** because mutable references don't allow lifetime shortening. The solution uses `'borrow` for the outer reference and `'data` for the inner data, allowing the borrow to be shorter while preserving safety.

### 1.5 ProcessContext and Transport

```rust
#[derive(Copy, Clone, Debug)]
pub struct ProcessContext {
    pub sample_rate: f64,
    pub num_samples: usize,
    pub transport: Transport,
}

impl ProcessContext {
    pub fn samples_per_beat(&self) -> Option<f64>;
    pub fn buffer_duration(&self) -> f64;
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Transport {
    // Tempo and time signature
    pub tempo: Option<f64>,
    pub time_sig_numerator: Option<i32>,
    pub time_sig_denominator: Option<i32>,

    // Position
    pub project_time_samples: Option<i64>,
    pub project_time_beats: Option<f64>,
    pub bar_position_beats: Option<f64>,

    // Loop/Cycle
    pub cycle_start_beats: Option<f64>,
    pub cycle_end_beats: Option<f64>,

    // Transport state (always valid)
    pub is_playing: bool,
    pub is_recording: bool,
    pub is_cycle_active: bool,

    // Advanced timing
    pub system_time_ns: Option<i64>,
    pub continuous_time_samples: Option<i64>,
    pub samples_to_next_clock: Option<i32>,

    // SMPTE/Timecode
    pub smpte_offset_subframes: Option<i32>,
    pub frame_rate: Option<FrameRate>,
}

impl Transport {
    pub fn time_signature(&self) -> Option<(i32, i32)>;
    pub fn cycle_range(&self) -> Option<(f64, f64)>;
    pub fn is_looping(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameRate {
    #[default] Fps24,
    Fps25, Fps2997, Fps30,
    Fps2997Drop, Fps30Drop,
    Fps50, Fps5994, Fps60,
    Fps5994Drop, Fps60Drop,
}
```

### 1.6 Sample Trait (f32/f64)

```rust
pub trait Sample:
    Copy + Default + Send + Sync + 'static
    + Add<Output = Self> + Sub<Output = Self>
    + Mul<Output = Self> + Div<Output = Self>
    + PartialOrd
{
    const ZERO: Self;
    const ONE: Self;
    fn from_f32(value: f32) -> Self;
    fn to_f32(self) -> f32;
    fn from_f64(value: f64) -> Self;
    fn to_f64(self) -> f64;
    fn abs(self) -> Self;
    fn sqrt(self) -> Self;
    fn sin(self) -> Self;
    fn cos(self) -> Self;
    fn min(self, other: Self) -> Self;
    fn max(self, other: Self) -> Self;
    fn clamp(self, min: Self, max: Self) -> Self;
}
```

**Write DSP once:**

```rust
impl MyPlugin {
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        _aux: &mut AuxiliaryBuffers<S>,
        _ctx: &ProcessContext,
    ) {
        let gain = S::from_f32(self.params.gain_linear());
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}
```

### 1.7 Soft Bypass

```rust
pub enum BypassState {
    Active,
    RampingToBypassed,
    Bypassed,
    RampingToActive,
}

pub enum CrossfadeCurve {
    Linear,      // Slight loudness dip at center
    EqualPower,  // Constant loudness (recommended)
    SCurve,      // Faster start/end, smoother middle
}

pub struct BypassHandler { /* ... */ }

impl BypassHandler {
    pub fn new(ramp_samples: u32, curve: CrossfadeCurve) -> Self;

    /// Automatic crossfade handling.
    pub fn process<S: Sample, F>(
        &mut self,
        buffer: &mut Buffer<S>,
        is_bypassed: bool,
        process_fn: F,
    ) where F: FnMut(&mut Buffer<S>);

    pub fn state(&self) -> BypassState;
    pub fn ramp_samples(&self) -> u32;
}
```

**Usage:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &ProcessContext) {
    let is_bypassed = self.params.bypass_normalized() > 0.5;

    self.bypass_handler.process(buffer, is_bypassed, |buf| {
        self.process_reverb(buf);  // Only runs when not fully bypassed
    });
}
```

---

## 2. MIDI Reference

### 2.1 Event Types

All MIDI types are `Copy` for real-time safety.

```rust
#[derive(Debug, Clone, Copy)]
pub struct MidiEvent {
    pub sample_offset: u32,
    pub event: MidiEventKind,
}

#[derive(Debug, Clone, Copy)]
pub enum MidiEventKind {
    // Note events
    NoteOn(NoteOn),
    NoteOff(NoteOff),
    PolyPressure(PolyPressure),

    // Channel events
    ControlChange(ControlChange),
    PitchBend(PitchBend),
    ChannelPressure(ChannelPressure),
    ProgramChange(ProgramChange),

    // Advanced VST3 events
    SysEx(SysEx),
    NoteExpressionValue(NoteExpressionValue),
    NoteExpressionInt(NoteExpressionInt),
    NoteExpressionText(NoteExpressionText),
    ChordInfo(ChordInfo),
    ScaleInfo(ScaleInfo),
}
```

#### Note Events

```rust
pub struct NoteOn {
    pub channel: MidiChannel,  // 0-15
    pub pitch: MidiNote,       // 0-127
    pub velocity: f32,         // 0.0-1.0
    pub note_id: NoteId,       // For tracking
    pub tuning: f32,           // Cents (±120.0) for MPE/microtonal
    pub length: i32,           // Samples (0 = unknown)
}

pub struct NoteOff {
    pub channel: MidiChannel,
    pub pitch: MidiNote,
    pub velocity: f32,
    pub note_id: NoteId,
    pub tuning: f32,
}

pub struct PolyPressure {
    pub channel: MidiChannel,
    pub pitch: MidiNote,
    pub pressure: f32,
    pub note_id: NoteId,
}
```

#### Channel Events

```rust
pub struct ControlChange {
    pub channel: MidiChannel,
    pub controller: u8,   // 0-127
    pub value: f32,       // 0.0-1.0
}

pub struct PitchBend {
    pub channel: MidiChannel,
    pub value: f32,       // -1.0 to 1.0
}

pub struct ChannelPressure {
    pub channel: MidiChannel,
    pub pressure: f32,
}

pub struct ProgramChange {
    pub channel: MidiChannel,
    pub program: u8,
}
```

#### Constructors

```rust
MidiEvent::note_on(offset, channel, pitch, velocity, note_id, tuning, length)
MidiEvent::note_off(offset, channel, pitch, velocity, note_id, tuning)
MidiEvent::poly_pressure(offset, channel, pitch, pressure, note_id)
MidiEvent::control_change(offset, channel, controller, value)
MidiEvent::pitch_bend(offset, channel, value)
MidiEvent::channel_pressure(offset, channel, pressure)
MidiEvent::program_change(offset, channel, program)
MidiEvent::sysex(offset, &data)
MidiEvent::note_expression_value(offset, note_id, type_id, value)
MidiEvent::chord_info(offset, root, bass_note, mask, name)
MidiEvent::scale_info(offset, root, mask, name)
```

### 2.2 MidiBuffer

```rust
pub struct MidiBuffer { /* Fixed capacity: 1024 events */ }

impl MidiBuffer {
    pub fn new() -> Self;
    pub fn push(&mut self, event: MidiEvent);
    pub fn iter(&self) -> impl Iterator<Item = &MidiEvent>;
    pub fn len(&self) -> usize;
    pub fn clear(&mut self);
    pub fn has_overflowed(&self) -> bool;
}
```

### 2.3 SysEx Handling

**Buffer Size (Cargo features):**

| Feature | Size |
|---------|------|
| (default) | 512 bytes |
| `sysex-256` | 256 bytes |
| `sysex-1024` | 1024 bytes |
| `sysex-2048` | 2048 bytes |

**Plugin-Declared Capacity:**

```rust
pub static CONFIG: PluginConfig = PluginConfig::new("My Sampler", UID)
    .with_sysex_slots(64)         // Default: 16
    .with_sysex_buffer_size(4096); // Default: 512
```

**Heap Fallback (optional feature: `sysex-heap-fallback`):**
Overflow messages stored in heap, emitted next block. Breaks real-time guarantee.

### 2.4 Note Expression (MPE)

```rust
pub mod note_expression {
    pub const VOLUME: u32 = 0;
    pub const PAN: u32 = 1;
    pub const TUNING: u32 = 2;      // MPE pitch
    pub const VIBRATO: u32 = 3;
    pub const EXPRESSION: u32 = 4;
    pub const BRIGHTNESS: u32 = 5;
    pub const CUSTOM_START: u32 = 100000;
}

pub struct NoteExpressionValue {
    pub note_id: NoteId,
    pub expression_type: u32,
    pub value: f64,
}
```

**INoteExpressionController** — Advertise supported expressions:

```rust
impl Plugin for MyMPESynth {
    fn note_expression_count(&self, _bus: i32, _channel: i16) -> usize { 3 }

    fn note_expression_info(&self, _bus: i32, _channel: i16, index: usize)
        -> Option<NoteExpressionTypeInfo>
    {
        match index {
            0 => Some(NoteExpressionTypeInfo::new(note_expression::VOLUME, "Volume", "Vol")),
            1 => Some(NoteExpressionTypeInfo::new(note_expression::PAN, "Pan", "Pan")
                .with_flags(NoteExpressionTypeFlags::IS_BIPOLAR)),
            2 => Some(NoteExpressionTypeInfo::new(note_expression::TUNING, "Tuning", "Tune")
                .with_units("semitones")),
            _ => None,
        }
    }
}
```

**Physical UI Mapping** — Map MPE controllers to expressions:

```rust
fn physical_ui_mappings(&self, _bus: i32, _channel: i16) -> &[PhysicalUIMap] {
    &[
        PhysicalUIMap::y_axis(note_expression::BRIGHTNESS),
        PhysicalUIMap::pressure(note_expression::EXPRESSION),
    ]
}
```

**MPE Zone Configuration:**

```rust
fn enable_mpe_input_processing(&mut self, enabled: bool) -> bool { true }
fn set_mpe_input_device_settings(&mut self, settings: MpeInputDeviceSettings) -> bool { true }

// Presets
MpeInputDeviceSettings::lower_zone()  // Master=0, Members=1-14
MpeInputDeviceSettings::upper_zone()  // Master=15, Members=14-1
```

### 2.5 MIDI Mapping and Learn

**IMidiMapping** — CC to parameter:

```rust
fn midi_cc_to_param(&self, _bus: i32, _channel: i16, cc: u8) -> Option<u32> {
    match cc {
        cc::MOD_WHEEL => Some(PARAM_VIBRATO),
        cc::EXPRESSION => Some(PARAM_VOLUME),
        _ => None,
    }
}
```

**IMidiLearn:**

```rust
fn on_midi_learn(&mut self, _bus: i32, _channel: i16, cc: u8) -> bool {
    if let Some(param_id) = self.learning_param.take() {
        self.midi_map.insert(cc, param_id);
        true
    } else {
        false
    }
}
```

**MIDI 2.0:** `midi1_assignments()`, `midi2_assignments()`, `on_midi2_learn()`

### 2.6 Keyswitch Controller

```rust
fn keyswitch_count(&self, _bus: i32, _channel: i16) -> usize { 4 }

fn keyswitch_info(&self, _bus: i32, _channel: i16, index: usize) -> Option<KeyswitchInfo> {
    match index {
        0 => Some(KeyswitchInfo::new(keyswitch_type::NOTE_ON_KEY, "Sustain", 24)),
        1 => Some(KeyswitchInfo::new(keyswitch_type::NOTE_ON_KEY, "Staccato", 25)),
        _ => None,
    }
}
```

### 2.7 RPN/NRPN Helpers

**Constants:**

```rust
pub mod rpn {
    pub const PITCH_BEND_SENSITIVITY: u16 = 0x0000;
    pub const FINE_TUNING: u16 = 0x0001;
    pub const COARSE_TUNING: u16 = 0x0002;
    pub const MPE_CONFIGURATION: u16 = 0x0006;
    pub const NULL: u16 = 0x7F7F;
}
```

**RpnTracker** — Real-time safe decoder:

```rust
struct MyPlugin {
    rpn_tracker: RpnTracker,
}

fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
    for event in input {
        if let MidiEventKind::ControlChange(cc) = &event.event {
            if let Some(msg) = self.rpn_tracker.process_cc(cc) {
                if msg.is_pitch_bend_sensitivity() {
                    let (semitones, cents) = msg.pitch_bend_sensitivity();
                    // ...
                }
            }
        }
    }
}
```

**ParameterNumberMessage:**

```rust
pub struct ParameterNumberMessage {
    pub channel: MidiChannel,
    pub kind: ParameterNumberKind,  // Rpn or Nrpn
    pub parameter: u16,
    pub value: f32,
    pub is_increment: bool,
    pub is_decrement: bool,
}
```

### 2.8 CC Utilities

**Constants:**

```rust
pub mod cc {
    pub const BANK_SELECT_MSB: u8 = 0;
    pub const MOD_WHEEL: u8 = 1;
    pub const VOLUME: u8 = 7;
    pub const PAN: u8 = 10;
    pub const EXPRESSION: u8 = 11;
    pub const BANK_SELECT_LSB: u8 = 32;
    pub const SUSTAIN: u8 = 64;
    pub const DATA_ENTRY_MSB: u8 = 6;
    pub const DATA_ENTRY_LSB: u8 = 38;
    pub const NRPN_LSB: u8 = 98;
    pub const NRPN_MSB: u8 = 99;
    pub const RPN_LSB: u8 = 100;
    pub const RPN_MSB: u8 = 101;
}
```

**ControlChange Methods:**

```rust
impl ControlChange {
    pub fn is_bank_select(&self) -> bool;
    pub fn is_14bit_msb(&self) -> bool;
    pub fn is_14bit_lsb(&self) -> bool;
    pub fn lsb_pair(&self) -> Option<u8>;
    pub fn msb_pair(&self) -> Option<u8>;
    pub fn is_rpn_nrpn_related(&self) -> bool;
    pub fn is_sustain_pedal(&self) -> bool;
}
```

**14-bit CC Helpers:**

```rust
let combined = combine_14bit_cc(msb_value, lsb_value);  // → 0.0-1.0
let (msb, lsb) = split_14bit_cc(combined);

let combined = combine_14bit_raw(msb, lsb);  // → 0-16383
let (msb, lsb) = split_14bit_raw(combined);
```

### 2.9 VST3 Event Mapping

| Beamer Type | VST3 Event ID | Direction |
|------------|---------------|-----------|
| NoteOn | 0 | In/Out |
| NoteOff | 1 | In/Out |
| SysEx | 2 | In/Out |
| PolyPressure | 3 | In/Out |
| NoteExpressionValue | 4 | In/Out |
| NoteExpressionText | 5 | In only |
| ChordInfo | 6 | In only |
| ScaleInfo | 7 | In only |
| NoteExpressionInt | 8 | In/Out |
| ControlChange | 65535 (CC 0-127) | In/Out |
| ChannelPressure | 65535 (CC 128) | In/Out |
| PitchBend | 65535 (CC 129) | In/Out |
| ProgramChange | 65535 (CC 130) | In/Out |

---

## 3. VST3 Integration

### 3.1 Bundle Structure

**macOS:**
```
MyPlugin.vst3/
├── Contents/
│   ├── Info.plist
│   ├── MacOS/
│   │   └── MyPlugin
│   └── PkgInfo
```

**Windows:**
```
MyPlugin.vst3/
├── Contents/
│   └── x86_64-win/
│       └── MyPlugin.vst3
```

**Linux:**
```
MyPlugin.vst3/
├── Contents/
│   └── x86_64-linux/
│       └── MyPlugin.so
```

### 3.2 Critical Requirements

| Requirement | Details |
|-------------|---------|
| **PFactoryInfo.flags** | Must be `16` (kUnicode). Other values cause plugin to not appear in DAW. |
| **Entry points** | Must be lowercase: `bundleEntry`, `bundleExit` |
| **Mach-O patching** | Not needed. DAWs load cdylib fine. |
| **Code signing** | Not needed for local development. |

### 3.3 Build System

```bash
cargo xtask bundle gain --release
```

**Cargo.toml:**

```toml
[lib]
crate-type = ["cdylib"]

[profile.release]
lto = true
```

### 3.4 Install Locations

| Platform | Location |
|----------|----------|
| macOS | `~/Library/Audio/Plug-Ins/VST3/` |
| Windows | `C:\Program Files\Common Files\VST3\` |
| Linux | `~/.vst3/` |

### 3.5 Plugin Categories

```rust
// Audio effect
PluginConfig::new("My Effect", UID).with_category("Fx")

// Instrument
PluginConfig::new("My Synth", UID).with_category("Instrument")
```

---

## 4. Future Phases

### 4.1 Phase 2: WebView Integration

Add platform-native WebView embedding to plugin windows.

#### Platform Backends

| Platform | Backend | Rust Approach |
|----------|---------|---------------|
| Windows | WebView2 (Edge/Chromium) | `webview2` crate or direct COM |
| macOS | WKWebView | `objc2` + `icrate` |
| Linux | WebKitGTK | `webkit2gtk` crate |

#### IPlugView Implementation

```rust
pub struct WebViewPlugView {
    webview: Option<PlatformWebView>,
    frame: Option<*mut IPlugFrame>,
    size: Size,
}

impl IPlugViewTrait for WebViewPlugView {
    fn is_platform_type_supported(&self, platform_type: FIDString) -> tresult;
    fn attached(&mut self, parent: *mut c_void, platform_type: FIDString) -> tresult;
    fn removed(&mut self) -> tresult;
    fn on_size(&mut self, new_size: *mut ViewRect) -> tresult;
    fn get_size(&self, size: *mut ViewRect) -> tresult;
    fn can_resize(&self) -> tresult;
    fn set_frame(&mut self, frame: *mut IPlugFrame) -> tresult;
}
```

#### Resource Loading

```rust
pub enum ResourceSource {
    /// Embedded in binary (release builds)
    Embedded { index_html: &'static str, assets: &'static [(&'static str, &'static [u8])] },
    /// Directory on disk (dev builds)
    Directory(PathBuf),
    /// Development server URL (hot reload)
    DevServer(String),
}
```

### 4.2 Phase 3: IPC & Parameter Binding

Tauri-style bidirectional communication between Rust and JavaScript.

#### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      JavaScript                              │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  window.__PLUGIN__ = {                              │    │
│  │    invoke(cmd, args) → Promise                      │    │
│  │    on(event, callback)                              │    │
│  │    emit(event, data)                                │    │
│  │    getParam(id) → ParamState                        │    │
│  │  }                                                  │    │
│  └─────────────────────────────────────────────────────┘    │
│              │                         ▲                    │
│              ▼                         │                    │
│     plugin://invoke/...        evaluateJavascript()         │
│     (custom URL scheme)        (push events to JS)          │
└──────────────┼─────────────────────────┼────────────────────┘
               │                         │
┌──────────────▼─────────────────────────┼────────────────────┐
│                       Rust                                   │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  IpcHandler {                                       │    │
│  │    fn handle_invoke(cmd, args) → Result<Value>      │    │
│  │    fn emit(event, data)                             │    │
│  │  }                                                  │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

#### IPC Protocol

**Request (JS → Rust):**
```json
{ "id": 1, "cmd": "setParameter", "args": { "paramId": 0, "value": 0.75 } }
```

**Response (Rust → JS):**
```json
{ "id": 1, "result": { "ok": true } }
```

**Event (Rust → JS):**
```json
{ "event": "parameterChanged", "data": { "paramId": 0, "value": 0.75 } }
```

#### JavaScript API

```javascript
// Invoke a Rust command
const result = await window.__PLUGIN__.invoke('getParameterValue', { paramId: 0 });

// Listen for events
window.__PLUGIN__.on('parameterChanged', (data) => {
    console.log(`Param ${data.paramId} = ${data.value}`);
});

// Parameter state helper with automation support
const gain = window.__PLUGIN__.getParam(0);
gain.onValueChanged((value, display) => updateKnob(value));

// Proper automation gesture
gain.beginEdit();
gain.setValue(0.75);
gain.endEdit();
```

#### Built-in Commands

| Command | Purpose |
|---------|---------|
| `getParameterInfo` | Get all parameter definitions |
| `getParameterValue` | Get current normalized value + display string |
| `beginEdit` | Start automation gesture |
| `performEdit` | Set value during gesture |
| `endEdit` | End automation gesture |

### 4.3 Phase 4: Developer Experience

- Hot reload: Detect dev server, auto-refresh on file changes
- CLI tooling: `cargo beamer new`, `cargo beamer dev`
- Documentation generation from plugin metadata

### 4.4 Phase 5: Examples & Polish

- Real-world examples (EQ, compressor, synth)
- Performance profiling and optimization
- Cross-DAW validation (Cubase, Ableton, Logic, REAPER, Bitwig)

---

## Appendices

### A. Quick Reference

**Commands:**

```bash
cargo build --release
cargo xtask bundle gain --release
cargo test
cargo clippy
```

**Constants:**

| Constant | Value |
|----------|-------|
| `MAX_CHANNELS` | 32 |
| `MAX_BUSES` | 16 |
| `MAX_AUX_BUSES` | 15 |
| `MIDI_BUFFER_CAPACITY` | 1024 |
| `MAX_SYSEX_SIZE` | 512 (default) |

### B. Example: Simple Gain

```rust
use beamer::prelude::*;
use beamer::Params;

#[derive(Params)]
struct GainParams {
    #[param(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParam,
}

impl GainParams {
    fn gain_linear(&self) -> f32 {
        self.gain.as_linear() as f32
    }
}

fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &ProcessContext) {
    let gain = self.params.gain_linear();
    for (input, output) in buffer.zip_channels() {
        for (i, o) in input.iter().zip(output.iter_mut()) {
            *o = *i * gain;
        }
    }
}
```

### C. Example: Sidechain Compressor

```rust
fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, _ctx: &ProcessContext) {
    let key_level = aux.sidechain().map(|sc| sc.rms(0)).unwrap_or(0.0);
    let reduction = self.compute_gain_reduction(key_level);
    buffer.copy_to_output();
    buffer.apply_output_gain(reduction);
}
```

### D. Example: MIDI Transpose

```rust
fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
    for event in input {
        match &event.event {
            MidiEventKind::NoteOn(note) => {
                output.push(MidiEvent::note_on(
                    event.sample_offset,
                    note.channel,
                    note.pitch.saturating_add(2).min(127),
                    note.velocity,
                    note.note_id,
                    note.tuning,
                    note.length,
                ));
            }
            MidiEventKind::NoteOff(note) => {
                output.push(MidiEvent::note_off(
                    event.sample_offset,
                    note.channel,
                    note.pitch.saturating_add(2).min(127),
                    note.velocity,
                    note.note_id,
                    note.tuning,
                ));
            }
            _ => output.push(*event),
        }
    }
}

fn wants_midi(&self) -> bool { true }
```
