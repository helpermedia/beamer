# Beamer API Reference

**Version:** 0.2.2

This document provides detailed API documentation for Beamer. For high-level architecture and design decisions, see [ARCHITECTURE.md](../ARCHITECTURE.md).

---

## Table of Contents

1. [Core API](#1-core-api)
2. [MIDI Reference](#2-midi-reference)
3. [Audio Unit Integration](#3-audio-unit-integration)
4. [VST3 Integration](#4-vst3-integration)
5. [Future Phases](#5-future-phases)

---

## 1. Core API

### 1.1 Plugin Configuration

Every plugin requires a `Config.toml` file in the crate root (next to `Cargo.toml`) containing format-agnostic metadata. This configuration is read at compile time by the `#[beamer::export]` macro and applies to all plugin formats (AU and VST3).

**Config.toml:**

```toml
name = "My Plugin"
category = "effect"
subcategories = ["dynamics"]
manufacturer_code = "Myco"
plugin_code = "mypg"
vendor = "My Company"
url = "https://example.com"
email = "support@example.com"
```

**Required Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `name` | String | Plugin name displayed in DAW |
| `category` | String | Plugin type: `"effect"`, `"instrument"`, `"midi_effect"`, or `"generator"` |
| `manufacturer_code` | String | 4-character manufacturer code (e.g., `"Myco"`) |
| `plugin_code` | String | 4-character plugin code (e.g., `"mypg"`) |

**Optional Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `vendor` | String | Company/developer name |
| `url` | String | Vendor website |
| `email` | String | Support email |
| `subcategories` | Array | Subcategory strings for DAW browser organization (e.g., `["dynamics", "eq"]`) |
| `has_editor` | Boolean | Whether plugin has a GUI (default: `false`) |
| `vst3_id` | String | Override auto-derived VST3 UUID (format: `"XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"`) |

**Notes:**

- Version is automatically read from `CARGO_PKG_VERSION`
- VST3 UUID is auto-derived from `manufacturer_code` and `plugin_code` unless `vst3_id` is specified
- The 4-character codes are used for both AU FourCC identifiers and VST3 UUID derivation
- `category` determines the AU component type (`aufx`, `aumu`, `aumi`, `augn`)

### 1.2 Three-Struct Pattern

Beamer plugins use three structs for clear separation of concerns:

| Struct | Derive | Purpose |
|--------|--------|---------|
| `*Parameters` | `#[derive(Parameters)]` | Pure parameter definitions with declarative attributes |
| `*Descriptor` | `#[derive(Default, HasParameters)]` | Holds parameters, describes plugin to host before audio config |
| `*Processor` | `#[derive(HasParameters)]` | Prepared state with DSP logic, created by `prepare()` |

**Lifecycle:**

```
Parameters (data) ──owns──▶ Descriptor (unprepared)
                                │
                                ▼ prepare(setup)
                                │
                            Processor (prepared, ready for audio)
                                │
                                ▼ unprepare()
                                │
                            Descriptor (parameters preserved)
```

**Minimal Example:**

```rust
use beamer::prelude::*;

// 1. Parameters - pure data with derive macros
#[derive(Parameters)]
pub struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

// 2. Descriptor - holds parameters, describes plugin to host
#[beamer::export]
#[derive(Default, HasParameters)]
pub struct GainDescriptor {
    #[parameters]
    parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    // No setup needed for simple effects; use SampleRate for delays, MaxBufferSize for FFT
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

// 3. Processor - prepared state, ready for audio
#[derive(HasParameters)]
pub struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
}

impl Processor for GainProcessor {
    type Descriptor = GainDescriptor;
    fn process(&mut self, buffer: &mut Buffer, _: &mut AuxiliaryBuffers, _: &ProcessContext) {
        let gain = self.parameters.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}
```

**Plugin Export:**

The `#[beamer::export]` macro on the descriptor struct reads `Config.toml` from the crate root at compile time and generates:
- `pub static CONFIG: Config` from the TOML configuration
- Optional factory presets from `Presets.toml` (if present)
- Format-specific plugin entry points for AU and VST3

See [Section 1.1](#11-plugin-configuration) for Config.toml format and [Section 1.6](#16-factory-presets) for Presets.toml.

The following sections cover: Parameters (§1.3), Descriptor trait (§1.4), Processor trait (§1.5).

### 1.3 Parameters

Use `#[derive(Parameters)]` to define plugin parameters with declarative attributes. The macro generates all required trait implementations automatically.

**Features:**
- Declarative parameter definitions with compile-time validation
- Automatic state serialization/deserialization
- Parameter smoothing to avoid zipper noise during automation

#### Derive Macro

**Declarative Style**: Macro generates everything including `Default`:

```rust
use beamer::prelude::*;

#[derive(Parameters)]
pub struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,

    #[parameter(id = "bypass", bypass)]
    pub bypass: BoolParameter,
}

// No manual new() or Default impl needed - macro generates everything!
```

The `#[derive(Parameters)]` macro generates:
- `Parameters` trait implementation (count, iter, by_id, save_state, load_state)
- `ParameterStore` trait implementation (host integration)
- `ParameterGroups` trait implementation (parameter groups)
- `Default` implementation (when all required attributes are present)
- Compile-time FNV-1a hash constants: `PARAM_GAIN_ID`, `PARAM_BYPASS_ID`
- Compile-time collision detection for duplicate IDs

Use `#[derive(HasParameters)]` separately on Descriptor and Processor structs to generate parameter access methods.

#### Declarative Attributes

| Attribute | Description | Required |
|-----------|-------------|----------|
| `id = "..."` | String ID (hashed to u32) | Yes |
| `name = "..."` | Display name in DAW | For Default |
| `default = <value>` | Default value (float, int, or bool) | For Default |
| `range = start..=end` | Value range | For FloatParameter/IntParameter |
| `kind = "..."` | Unit type (see below) | Optional |
| `group = "..."` | Visual grouping without nested struct | Optional |
| `short_name = "..."` | Short name for constrained UIs | Optional |
| `smoothing = "exp:5.0"` | Parameter smoothing (`exp` or `linear`) | Optional |
| `bypass` | Mark as bypass parameter (BoolParameter only) | Optional |

**Kind Values:** `db`, `db_log`, `db_log_offset`, `hz`, `ms`, `seconds`, `percent`, `pan`, `ratio`, `linear`, `semitones`

- `db_log` - Power curve (exponent 2.0) for more resolution near 0 dB (use for thresholds)
- `db_log_offset` - True logarithmic mapping for dB ranges (geometric mean at midpoint)

Supported field types: `FloatParameter`, `IntParameter`, `BoolParameter`, `EnumParameter<E>`

#### Parameter Types

**FloatParameter**: Continuous floating-point parameter:

```rust
// Linear range
let freq = FloatParameter::new("Frequency", 1000.0, 20.0..=20000.0);

// Decibel range (stores dB, use as_linear() for DSP)
let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0);

// In DSP code:
let amplitude = gain.as_linear(); // 0 dB → 1.0, -6 dB → ~0.5
let db_value = gain.get(); // Returns dB for display
```

**IntParameter**: Integer parameter:

```rust
let voices = IntParameter::new("Voices", 8, 1..=64);
```

**BoolParameter**: Toggle parameter:

```rust
let bypass = BoolParameter::new("Bypass", false);
```

**EnumParameter**: Discrete choice parameter:

```rust
use beamer::prelude::*;

#[derive(Copy, Clone, PartialEq, EnumParameter)]
pub enum FilterType {
    #[name = "Low Pass"]
    LowPass,
    #[default]
    #[name = "High Pass"]
    HighPass,
    #[name = "Band Pass"]
    BandPass,
    Notch, // Uses "Notch" as display name
}

#[derive(Parameters)]
pub struct FilterParameters {
    #[parameter(id = "filter_type", name = "Filter Type")]
    pub filter_type: EnumParameter<FilterType>,
}

// In DSP code:
match self.parameters.filter_type.get() {
    FilterType::LowPass => { /* ... */ }
    FilterType::HighPass => { /* ... */ }
    FilterType::BandPass => { /* ... */ }
    FilterType::Notch => { /* ... */ }
}
```

The `#[derive(EnumParameter)]` macro generates the `EnumParameterValue` trait implementation.

| Attribute | Purpose |
|-----------|---------|
| `#[name = "..."]` | Display name for variant (defaults to identifier) |
| `#[default]` | Mark as default variant (defaults to first) |

EnumParameter constructors:

| Constructor | Purpose |
|-------------|---------|
| `EnumParameter::new(name)` | Uses `#[default]` variant or first |
| `EnumParameter::with_value(name, variant)` | Explicit default override |

#### Builder Methods

All parameter types support builder methods for customization. Chain these after constructors:

**FloatParameter Builder Methods:**

| Method | Description |
|--------|-------------|
| `.with_id(id)` | Set parameter ID (usually via macro) |
| `.with_short_name(name)` | Short name for constrained UIs |
| `.with_group(group_id)` | Assign to parameter group |
| `.with_step_size(size)` | Enable discrete stepping (e.g., 0.5 dB increments) |
| `.with_precision(n)` | Display precision (decimal places) |
| `.with_formatter(fmt)` | Replace formatter entirely |
| `.with_smoother(style)` | Add parameter smoothing |
| `.readonly()` | Make parameter read-only |
| `.non_automatable()` | Disable automation |

**IntParameter Builder Methods:**

| Method | Description |
|--------|-------------|
| `.with_id(id)` | Set parameter ID (usually via macro) |
| `.with_short_name(name)` | Short name for constrained UIs |
| `.with_group(group_id)` | Assign to parameter group |
| `.with_precision(n)` | Display precision (for Float formatter) |
| `.with_formatter(fmt)` | Replace formatter entirely |
| `.readonly()` | Make parameter read-only |
| `.non_automatable()` | Disable automation |

**Precision and Formatter Customization:**

```rust
// High-precision gain for mastering plugins
let gain = FloatParameter::db("Output", 0.0, -12.0..=12.0)
    .with_precision(2); // Shows "-0.50 dB" instead of "-0.5 dB"

// Milliseconds with integer display
let attack = FloatParameter::ms("Attack", 10.0, 0.1..=100.0)
    .with_precision(0); // Shows "10" instead of "10.0"

// Completely custom formatter
let ratio = FloatParameter::new("Ratio", 4.0, 1.0..=20.0)
    .with_formatter(Formatter::Ratio { precision: 1 }); // Shows "4.0:1"

// Chain multiple builder methods
let volume = FloatParameter::db("Volume", 0.0, -60.0..=12.0)
    .with_step_size(0.5)
    .with_precision(2)
    .with_smoother(SmoothingStyle::Exponential(5.0));
```

**Note:** Formatters without precision fields (`Pan`, `Boolean`, `Semitones`, `Frequency`) ignore `.with_precision()` calls.

#### Parameter Smoothing

Avoid zipper noise during automation by adding smoothing to parameters:

```rust
// Add smoother during parameter creation
let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
    .with_smoother(SmoothingStyle::Exponential(5.0));  // 5ms time constant
```

**Smoothing Styles:**

| Style | Behavior | Use Case |
|-------|----------|----------|
| `SmoothingStyle::None` | Instant (default) | Non-audio parameters |
| `SmoothingStyle::Linear(ms)` | Linear ramp | Predictable timing |
| `SmoothingStyle::Exponential(ms)` | One-pole IIR, can cross zero | dB gain, most musical parameters |
| `SmoothingStyle::Logarithmic(ms)` | Log-domain, positive values only | Frequencies (Hz), other positive-only parameters |

**Sample Rate Initialization:**

Call `set_sample_rate()` in `prepare()` to initialize smoothers:

```rust
// Initialize smoothers in prepare()
impl Descriptor for MyDescriptor {
    type Setup = SampleRate;
    type Processor = MyProcessor;

    fn prepare(mut self, setup: SampleRate) -> MyProcessor {
        self.parameters.set_sample_rate(setup.hz());  // Initialize smoothers
        MyProcessor {
            parameters: self.parameters,
            sample_rate: setup.hz(),
        }
    }
}
```

> **Oversampling:** If your plugin uses oversampling, pass the actual processing rate:
> `self.set_sample_rate(setup.hz() * oversampling_factor as f64);`

**Per-Sample Processing:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
    for (input, output) in buffer.zip_channels() {
        for (i, o) in input.iter().zip(output.iter_mut()) {
            let gain = self.gain.tick_smoothed();  // Advances smoother
            *o = *i * gain as f32;
        }
    }
}
```

**Block Processing:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
    let gain = self.gain.smoothed();  // Current value, no advance
    self.gain.skip_smoothing(buffer.len());

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
self.gain.fill_smoothed_f32(&mut gain_buffer[..len]);
// Use gain_buffer[i] per sample
```

**Smoothing API:**

| Method | Description |
|--------|-------------|
| `.with_smoother(style)` | Builder: add smoothing to parameter |
| `.set_sample_rate(sr)` | Initialize with sample rate (call in prepare) |
| `.tick_smoothed()` | Advance smoother, return value (per-sample) |
| `.smoothed()` | Get current value without advancing |
| `.skip_smoothing(n)` | Skip n samples (block processing) |
| `.fill_smoothed(buf)` | Fill buffer with smoothed values |
| `.is_smoothing()` | Check if currently ramping |
| `.reset_smoothing()` | Reset to current value (no ramp) |

**Thread Safety Note:**

Smoothing methods require `&mut self` and run on the audio thread only. The underlying parameter value uses atomic storage for thread-safe access from UI/host threads.

**Automatic Reset on State Load:**

The framework automatically calls `reset_smoothing()` after loading state to prevent unwanted ramps to loaded parameter values.

#### Flat Parameter Grouping

Use `group = "..."` to organize parameters into logical groups without nested structs:

```rust
#[derive(Parameters)]
pub struct SynthesizerParameters {
    #[parameter(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
    pub cutoff: FloatParameter,

    #[parameter(id = "reso", name = "Resonance", default = 0.5, range = 0.0..=1.0, group = "Filter")]
    pub resonance: FloatParameter,

    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db", group = "Output")]
    pub gain: FloatParameter,
}

// Access is flat: parameters.cutoff, parameters.resonance, parameters.gain
// Groups appear in DAW automation lanes and parameter lists (e.g., Cubase Quick Controls)
// Note: Most DAWs don't display groups in the main plugin UI
```

**Flat vs Nested Grouping:**

| Feature | Flat (`group = "..."`) | Nested (`#[nested(...)]`) |
|---------|------------------------|---------------------------|
| Struct layout | Single struct | Separate struct per group |
| Access pattern | `parameters.cutoff` | `parameters.filter.cutoff` |
| Reusability | N/A | Same struct reusable |
| Complexity | Simple | More structure |

Choose flat grouping for simple organization; nested for reusable parameter collections.

#### Nested Parameter Groups

Use `#[nested]` to organize parameters into separate structs with VST3 units:

```rust
#[derive(Parameters)]
pub struct SynthesizerParameters {
    #[parameter(id = "master", name = "Master", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub master: FloatParameter,

    #[nested(group = "Filter")]
    pub filter: FilterParameters,

    #[nested(group = "Amp Envelope")]
    pub amp_env: EnvelopeParameters,
}

#[derive(Parameters)]
pub struct FilterParameters {
    #[parameter(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz")]
    pub cutoff: FloatParameter,

    #[parameter(id = "resonance", name = "Resonance", default = 0.5, range = 0.0..=1.0)]
    pub resonance: FloatParameter,
}

#[derive(Parameters)]
pub struct EnvelopeParameters {
    #[parameter(id = "attack", name = "Attack", default = 10.0, range = 0.1..=1000.0, kind = "ms")]
    pub attack: FloatParameter,

    #[parameter(id = "release", name = "Release", default = 100.0, range = 0.1..=5000.0, kind = "ms")]
    pub release: FloatParameter,
}
```

With declarative attributes, `set_group_ids()` is called automatically in the generated `Default` implementation.

#### State Serialization Format

Parameters are serialized using path-based IDs to support nested groups without collisions:

```
Format: [path_len: u8][path: utf8][value: f64]*

Path examples:
- "gain"              - top-level parameter
- "filter/cutoff"     - parameter in "Filter" group
- "osc1/filter/res"   - deeply nested parameter
```

The same nested struct can be reused in multiple groups without ID collision:

```rust
#[nested(group = "Osc 1")]
pub osc1: OscParameters,

#[nested(group = "Osc 2")]
pub osc2: OscParameters, // Same struct, different paths: "osc1/attack" vs "osc2/attack"
```

#### Low-Level Parameters Trait

For manual control, implement `Parameters` directly:

```rust
pub trait Parameters: Send + Sync {
    fn count(&self) -> usize;
    fn info(&self, index: usize) -> Option<&ParameterInfo>;
    fn get_normalized(&self, id: ParameterId) -> ParameterValue;
    fn set_normalized(&self, id: ParameterId, value: ParameterValue);
    fn normalized_to_string(&self, id: ParameterId, normalized: ParameterValue) -> String;
    fn string_to_normalized(&self, id: ParameterId, string: &str) -> Option<ParameterValue>;
    fn normalized_to_plain(&self, id: ParameterId, normalized: ParameterValue) -> ParameterValue;
    fn plain_to_normalized(&self, id: ParameterId, plain: ParameterValue) -> ParameterValue;
}

pub struct ParameterInfo {
    pub id: ParameterId,
    pub name: &'static str,
    pub short_name: &'static str,
    pub units: &'static str,
    pub default_normalized: f64,
    pub step_count: i32,
    pub flags: ParameterFlags,
    pub group_id: GroupId, // Parameter group (0 = root)
}

pub struct ParameterFlags {
    pub can_automate: bool,
    pub is_readonly: bool,
    pub is_bypass: bool, // Maps to VST3 kIsBypass (see §3.2)
    pub is_list: bool, // Display as dropdown list (for enums)
    pub is_hidden: bool, // Hide from DAW parameter list (used by MIDI CC emulation)
}

impl ParameterInfo {
    /// Convenience constructor for bypass parameters.
    pub const fn bypass(id: ParameterId) -> Self;
}
```

### 1.4 Descriptor Trait

The `Descriptor` trait represents a plugin in its **unprepared state** - before the host provides audio configuration. When the host calls `setupProcessing()`, the plugin transforms into a `Processor` via the `prepare()` method.

```rust
pub trait Descriptor: HasParameters + Default {
    /// Setup type for prepare() - determines what info is needed
    type Setup: PluginSetup;

    /// The prepared processor type
    type Processor: Processor<Descriptor = Self, Parameters = Self::Parameters>;

    /// Transform into a prepared processor with audio configuration.
    /// Consumes self - the plugin moves into the prepared state.
    fn prepare(self, setup: Self::Setup) -> Self::Processor;

    // Bus configuration (defaults provided)
    fn input_bus_count(&self) -> usize { 1 }
    fn output_bus_count(&self) -> usize { 1 }
    fn input_bus_info(&self, index: usize) -> Option<BusInfo>;
    fn output_bus_info(&self, index: usize) -> Option<BusInfo>;

    /// Whether this plugin processes MIDI events (queried before prepare).
    fn wants_midi(&self) -> bool { false }
}

// HasParameters supertrait provides parameter access
pub trait HasParameters: Send + 'static {
    type Parameters: Parameters + ParameterGroups + Default;
    fn parameters(&self) -> &Self::Parameters;
    fn parameters_mut(&mut self) -> &mut Self::Parameters;
    fn set_parameters(&mut self, params: Self::Parameters);
}
```

**HasParameters via Derive Macro:** Use `#[derive(HasParameters)]` on both Descriptor and Processor structs with a `#[parameters]` field annotation:

```rust
// 1. Parameters - pure data with derive macros
#[derive(Parameters)]
pub struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

// 2. Descriptor - holds parameters, describes plugin to host
#[derive(Default, HasParameters)]
pub struct GainDescriptor {
    #[parameters]
    parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    // No setup needed for simple effects; use SampleRate for delays, MaxBufferSize for FFT
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

// 3. Processor - prepared state, ready for audio
#[derive(HasParameters)]
pub struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
}

impl Processor for GainProcessor {
    type Descriptor = GainDescriptor;
    fn process(&mut self, buffer: &mut Buffer, _: &mut AuxiliaryBuffers, _: &ProcessContext) {
        let gain = self.parameters.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}
```

**Plugins with DSP state** add fields to the Processor:

```rust
#[derive(HasParameters)]
pub struct DelayProcessor {
    #[parameters]
    parameters: DelayParameters,
    sample_rate: f64, // Always valid
    buffer: Vec<f64>, // Always allocated
    write_pos: usize,
}
```

#### PluginSetup Types

Request exactly what your plugin needs using composable types:

| Type | When to Use | Value |
|------|-------------|-------|
| `()` | Stateless plugins (gain, pan) | - |
| `SampleRate` | Most plugins (delay, filter, envelope) | `f64` via `.hz()` |
| `MaxBufferSize` | FFT, lookahead | `usize` |
| `MainInputChannels` | Per-channel input processing | `u32` |
| `MainOutputChannels` | Per-channel output state | `u32` |
| `AuxInputCount` | Sidechain-aware processing | `usize` |
| `AuxOutputCount` | Multi-bus output | `usize` |
| `ProcessMode` | Offline quality settings | enum |

For IDE autocomplete, use `beamer::setup::*` to import all available types.

Compose multiple types using tuples:

```rust
// Simple plugin - no setup needed
impl Descriptor for GainDescriptor {
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

// Plugin needing sample rate (delays, filters, smoothing)
impl Descriptor for DelayDescriptor {
    type Setup = SampleRate;
    type Processor = DelayProcessor;
    fn prepare(self, sample_rate: SampleRate) -> DelayProcessor {
        DelayProcessor {
            parameters: self.parameters,
            sample_rate: sample_rate.hz(),
            /* DSP state... */
        }
    }
}

// Plugin needing multiple setup values
impl Descriptor for FftDescriptor {
    type Setup = (SampleRate, MaxBufferSize);
    type Processor = FftProcessor;
    fn prepare(self, (sr, mbs): (SampleRate, MaxBufferSize)) -> FftProcessor {
        FftProcessor {
            parameters: self.parameters,
            sample_rate: sr.hz(),
            fft_buffer: vec![0.0; mbs.0],
        }
    }
}
```

### 1.5 Processor Trait

The `Processor` trait represents a plugin in its **prepared state** - ready for real-time audio processing. Created by `Descriptor::prepare()`, it can transform back to unprepared state via `unprepare()`.

```rust
pub trait Processor: HasParameters {
    /// The unprepared definition type this processor came from
    type Descriptor: Descriptor<Processor = Self, Parameters = Self::Parameters>;

    /// Transform back to unprepared state.
    /// Default implementation transfers parameters to a new Descriptor::default().
    /// Override only if Descriptor has additional state beyond parameters.
    fn unprepare(self) -> Self::Descriptor { /* default impl */ }

    // Note: parameters() and parameters_mut() are provided by HasParameters supertrait

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

    /// Called when plugin is activated/deactivated.
    /// Reset DSP state when active == true.
    fn set_active(&mut self, active: bool) { }

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

    /// MIDI CC configuration for CC emulation (see §2.5).
    fn midi_cc_config(&self) -> Option<MidiCcConfig> { None }

    /// State persistence (default: delegates to Parameters trait)
    fn save_state(&self) -> PluginResult<Vec<u8>> { Ok(self.parameters().save_state()) }
    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> { ... }
}
```

**When to implement `set_active()`:** Plugins with internal DSP state (delay lines, filter histories, envelopes, oscillator phases) should override `set_active()` and reset that state when `active == true`. Hosts call `setActive(false)` followed by `setActive(true)` to request a full state reset. Plugins without internal state (simple gain, pan) can use the default empty implementation.

#### Plugin Lifecycle

The plugin transitions between states based on host actions:

```
                    ┌─────────────────┐
                    │  Descriptor     │
                    │  (unprepared)   │
                    └────────┬────────┘
                             │ setupProcessing(true)
                             │ + prepare(config)
                             ▼
                    ┌─────────────────┐
                    │  Processor      │
                    │  (prepared)     │◄───── process() calls
                    └────────┬────────┘
                             │ setProcessing(false)
                             │ + unprepare()
                             ▼
                    ┌─────────────────┐
                    │  Descriptor     │
                    │  (unprepared)   │
                    └─────────────────┘
```

This is the **typestate pattern**, a Rust idiom for encoding state machines at the type level. The `Processor` type is always fully initialized, so `process()` never needs `Option<T>` unwrapping or placeholder checks. See [ARCHITECTURE.md](../ARCHITECTURE.md#design-rationale) for detailed rationale.

### 1.6 Factory Presets

Factory presets let plugins provide built-in presets that appear in host preset menus (e.g., Logic's preset browser, VST3 program changes). Users can browse and load these presets without needing separate preset files.

#### Presets.toml Format

Factory presets are defined in a `Presets.toml` file in the crate root (next to `Config.toml`). The `#[beamer::export]` macro automatically detects and loads this file.

**Presets.toml:**

```toml
[[preset]]
name = "Default"
gain = 0.0

[[preset]]
name = "Subtle"
gain = -6.0

[[preset]]
name = "Boost"
gain = 6.0
```

**Format:**
- Each preset is defined with `[[preset]]` (TOML array of tables)
- `name` field specifies the display name in the DAW
- Other fields are parameter IDs with their plain values (e.g., `-6.0` for dB)
- Parameter IDs match the `id` attribute from `#[parameter(id = "gain", ...)]`

#### Sparse Presets

Presets can specify only a subset of parameters. Unspecified parameters retain their current values:

```toml
# Full preset: sets all parameters
[[preset]]
name = "Initialize"
gain = 0.0
mix = 1.0
bypass = false

# Sparse preset: only changes gain, mix and bypass unchanged
[[preset]]
name = "Quiet"
gain = -12.0
```

#### Integration with Export Macro

The `#[beamer::export]` macro automatically detects and loads `Presets.toml` from the crate root:

**Presets.toml:**

```toml
[[preset]]
name = "Init"
gain = 0.0
tone = 0.5

[[preset]]
name = "Warm"
gain = 3.0
tone = 0.7
```

The macro generates the preset implementation automatically. No manual preset struct or trait implementation needed.

#### FactoryPresets Trait

The derive macro generates an implementation of the `FactoryPresets` trait. For advanced use cases, you can implement it manually:

```rust
pub trait FactoryPresets: Send + Sync + 'static {
    /// The parameter struct this preset collection applies to.
    type Parameters: Parameters;

    /// Number of available presets.
    fn count() -> usize;

    /// Get preset metadata by index.
    fn info(index: usize) -> Option<PresetInfo>;

    /// Get parameter values for a preset.
    fn values(index: usize) -> &'static [PresetValue];

    /// Apply a preset to parameters. Returns true if successful.
    fn apply(index: usize, parameters: &Self::Parameters) -> bool;
}
```

#### MIDI Program Change Mapping

When a plugin has factory presets, MIDI Program Change (PC) events are automatically mapped to presets at the framework level:

| PC Number | Action |
|-----------|--------|
| 0 | Apply Preset 0 |
| 1 | Apply Preset 1 |
| ... | ... |
| N-1 | Apply Preset N-1 (last preset) |
| ≥N | Pass through to plugin (out of range) |

**Behavior:**
- PC events within the preset range are applied and **filtered out** (not passed to `process_midi()`)
- PC events outside the preset range pass through unchanged
- When multiple PC events arrive in the same buffer, they are processed in order (last one wins)
- Plugins without factory presets (`NoPresets`) pass all PC events through unchanged

This mirrors VST3's `kIsProgramChange` behavior where the host handles PC→preset mapping automatically. No plugin code changes are required - the framework handles this based on whether factory presets are defined.

### 1.7 Buffer Types

Beamer provides safe, ergonomic access to audio buffers using a two-buffer architecture. The main `Buffer` handles your primary input/output channels, while `AuxiliaryBuffers` provides access to sidechains and multi-bus routing.

**Design Goals:**
- Stack-allocated for real-time safety (no heap allocations in `process()`)
- Clear separation between input (read-only) and output (mutable) channels
- Support for both mono, stereo, and surround processing
- Generic over sample type (`f32` or `f64`)

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

### 1.8 ProcessContext and Transport

The `ProcessContext` provides essential timing and transport information for each audio processing call. This includes sample rate, buffer size, and detailed DAW transport state for tempo-synced effects, sequencers, and time-based processing.

**What you can do:**
- Sync delays/LFOs to host tempo
- Implement bar/beat-synced effects
- Display timecode in your UI
- Detect loop regions for seamless looping
- Handle SMPTE for post-production

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

### 1.9 Sample Trait (f32/f64)

The `Sample` trait lets you write DSP code once and support both `f32` and `f64` processing. This is the recommended pattern for plugins that want to offer native double-precision support.

**Why?** Some DAWs can process audio at 64-bit precision to reduce accumulation of rounding errors in complex processing chains. Plugins that support this can provide better quality in those hosts.

**The Sample Trait:**

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

**Pattern: Write generic DSP code once**

```rust
// Parameters
#[derive(Parameters)]
pub struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

// Descriptor
#[derive(Default, HasParameters)]
pub struct GainDescriptor {
    #[parameters]
    parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    // No setup needed for simple effects; use SampleRate for delays, MaxBufferSize for FFT
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

// Processor with generic processing method
#[derive(HasParameters)]
pub struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
}

impl GainProcessor {
    // Generic processing - works for both f32 and f64
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        _aux: &mut AuxiliaryBuffers<S>,
        _context: &ProcessContext,
    ) {
        let gain = S::from_f32(self.parameters.gain.as_linear() as f32);
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

impl Processor for GainProcessor {
    type Descriptor = GainDescriptor;

    fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
        self.process_generic(buffer, aux, context);
    }

    fn supports_double_precision(&self) -> bool {
        true
    }

    fn process_f64(&mut self, buffer: &mut Buffer<f64>, aux: &mut AuxiliaryBuffers<f64>, context: &ProcessContext) {
        // Same code - just different sample type!
        self.process_generic(buffer, aux, context);
    }
}
```

**When to use f64:**
- Reverbs, delays, or effects with long feedback paths (reduces error accumulation)
- Precision EQs or filters
- Scientific/mastering tools
- Any plugin where rounding errors matter over long processing chains

**When f32 is fine:**
- Simple gain/pan/saturation
- Most dynamics processors
- Synthesizers (often limited by oscillator precision anyway)

### 1.10 Soft Bypass

```rust
pub enum BypassState {
    Active,
    RampingToBypassed,
    Bypassed,
    RampingToActive,
}

/// What action the plugin should take for this buffer.
pub enum BypassAction {
    Passthrough, // Fully bypassed - copy input to output
    Process, // Fully active - run DSP normally
    ProcessAndCrossfade, // Transitioning - run DSP, then call finish()
}

pub enum CrossfadeCurve {
    Linear, // Slight loudness dip at center
    EqualPower, // Constant loudness (recommended)
    SCurve, // Faster start/end, smoother middle
}

pub struct BypassHandler { /* ... */ }

impl BypassHandler {
    pub fn new(ramp_samples: u32, curve: CrossfadeCurve) -> Self;

    /// Begin bypass processing. Returns what action to take.
    pub fn begin(&mut self, bypassed: bool) -> BypassAction;

    /// Finish bypass processing by applying crossfade.
    /// Call after DSP when begin() returned ProcessAndCrossfade.
    pub fn finish<S: Sample>(&mut self, buffer: &mut Buffer<S>);

    pub fn state(&self) -> BypassState;
    pub fn ramp_samples(&self) -> u32;
}
```

**Usage:**

```rust
// Processor struct holds parameters + DSP state
#[derive(HasParameters)]
struct ReverbProcessor {
    #[parameters]
    parameters: ReverbParameters,
    bypass_handler: BypassHandler,
    // ... other reverb state
}

impl Processor for ReverbProcessor {
    type Descriptor = ReverbDescriptor;

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        let is_bypassed = self.parameters.bypass.get();

        match self.bypass_handler.begin(is_bypassed) {
            BypassAction::Passthrough => {
                buffer.copy_to_output();
            }
            BypassAction::Process => {
                self.process_reverb(buffer);
            }
            BypassAction::ProcessAndCrossfade => {
                self.process_reverb(buffer);
                self.bypass_handler.finish(buffer);
            }
        }
    }
}
```

**Why Split API?** The split pattern (begin/finish) avoids Rust borrow checker conflicts that occur with closure-based APIs when your DSP code needs to access `&mut self`.

---

> **See Also:** For format-specific details on plugin export, bundle structure, and host requirements, see [Section 3: Audio Unit Integration](#3-audio-unit-integration) and [Section 4: VST3 Integration](#4-vst3-integration).

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
    pub channel: MidiChannel, // 0-15
    pub pitch: MidiNote, // 0-127
    pub velocity: f32, // 0.0-1.0
    pub note_id: NoteId, // For tracking
    pub tuning: f32, // Cents (±120.0) for MPE/microtonal
    pub length: i32, // Samples (0 = unknown)
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
    pub controller: u8, // 0-127
    pub value: f32, // 0.0-1.0
}

pub struct PitchBend {
    pub channel: MidiChannel,
    pub value: f32, // -1.0 to 1.0
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
// Semantic constructors (used by VST3)
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

// Raw MIDI 1.0 byte parsing (used by AU)
MidiEvent::from_midi1_bytes(offset, status, channel, data1, data2) -> Option<MidiEvent>
```

**Construction Paths:**
- **Semantic constructors** - Used by VST3 which provides already-parsed event structures
- **Raw byte parsing** (`from_midi1_bytes()`) - Used by AU which provides raw MIDI 1.0 bytes; handles velocity normalization, pitch bend 14-bit decoding, and Note On velocity 0 → Note Off conversion

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

### 2.3 Event Modification

Use `event.with()` to create modified events while preserving the sample offset. Combine with Rust's struct update syntax (`..`) to copy unchanged fields:

```rust
fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
    for event in input {
        match &event.event {
            MidiEventKind::NoteOn(note) => {
                output.push(event.with(MidiEventKind::NoteOn(NoteOn {
                    pitch: note.pitch.saturating_add(2).min(127),
                    ..*note // copies channel, velocity, note_id, tuning, length
                })));
            }
            MidiEventKind::NoteOff(note) => {
                output.push(event.with(MidiEventKind::NoteOff(NoteOff {
                    pitch: note.pitch.saturating_add(2).min(127),
                    ..*note // copies channel, velocity, note_id, tuning
                })));
            }
            _ => output.push(*event),
        }
    }
}

fn wants_midi(&self) -> bool { true }
```

### 2.4 SysEx Handling

**Buffer Size (Cargo features):**

| Feature | Size |
|---------|------|
| (default) | 512 bytes |
| `sysex-256` | 256 bytes |
| `sysex-1024` | 1024 bytes |
| `sysex-2048` | 2048 bytes |

**Plugin-Declared Capacity:**

SysEx configuration is currently an advanced feature not exposed via `Config.toml`. The defaults (16 slots, 512 bytes per message) work for most plugins. For custom SysEx configuration, you would need to manually modify the generated `CONFIG` static (advanced usage).

**Heap Fallback (optional feature: `sysex-heap-fallback`):**
Overflow messages stored in heap, emitted next block. Breaks real-time guarantee.

### 2.5 Note Expression (MPE)

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

**INoteExpressionController**: Advertise supported expressions:

```rust
impl Descriptor for MyMPESynthesizer{
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

**Physical UI Mapping**: Map MPE controllers to expressions:

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

### 2.6 MIDI CC Emulation (MidiCcConfig)

VST3 doesn't send MIDI CC, pitch bend, or aftertouch directly to plugins. Most DAWs convert these to parameter changes via the `IMidiMapping` interface. `MidiCcConfig` tells the framework which controllers you want - it handles all the state management automatically:

```rust
use beamer::prelude::*;

// 1. Parameters
#[derive(Parameters)]
struct SynthesizerParameters {
    #[parameter(id = "volume", name = "Volume", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub volume: FloatParameter,
}

// 2. Descriptor
#[derive(Default, HasParameters)]
struct SynthesizerDescriptor {
    #[parameters]
    parameters: SynthesizerParameters,
}

impl Descriptor for SynthesizerDescriptor {
    type Setup = SampleRate;
    type Processor = SynthesizerProcessor;

    fn prepare(self, sample_rate: SampleRate) -> SynthesizerProcessor {
        SynthesizerProcessor {
            parameters: self.parameters,
            sample_rate: sample_rate.hz(),
        }
    }

    // Return configuration - framework handles state
    fn midi_cc_config(&self) -> Option<MidiCcConfig> {
        // Use a preset for common configurations
        Some(MidiCcConfig::SYNTH_BASIC)

        // Or build a custom configuration
        // Some(MidiCcConfig::new()
        //     .with_pitch_bend()
        //     .with_mod_wheel()
        //     .with_ccs(&[7, 10, 11, 64]))
    }
}

// 3. Processor
#[derive(HasParameters)]
struct SynthesizerProcessor {
    #[parameters]
    parameters: SynthesizerParameters,
    sample_rate: f64,
}

impl Processor for SynthesizerProcessor {
    type Descriptor = SynthesizerDescriptor;

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
        // Access CC values directly via ProcessContext
        if let Some(cc) = context.midi_cc() {
            let pitch_bend = cc.pitch_bend();  // -1.0 to 1.0
            let mod_wheel = cc.mod_wheel();    // 0.0 to 1.0
            let volume = cc.cc(7);             // 0.0 to 1.0
        }
    }
    // ...
}
```

**How it works:**
1. Plugin returns `MidiCcConfig` from `midi_cc_config()` - pure configuration
2. Framework creates and owns `MidiCcState` internally
3. Framework exposes hidden parameters for each enabled controller
4. DAW queries `IMidiMapping` and maps MIDI controllers to these parameters
5. Framework converts parameter changes to `MidiEvent` before `process_midi()`
6. Plugin can also read current values directly via `context.midi_cc()`

**Presets (const, ready to use):**

| Preset | Controllers Included |
|--------|---------------------|
| `MidiCcConfig::SYNTH_BASIC` | Pitch bend, mod wheel, volume (7), expression (11), sustain (64) |
| `MidiCcConfig::SYNTH_FULL` | Basic + aftertouch, breath (2), pan (10) |
| `MidiCcConfig::EFFECT_BASIC` | Mod wheel, expression (11) |

**Builder Methods (all const fn):**

| Method | Description |
|--------|-------------|
| `.with_pitch_bend()` | Enable pitch bend (±1.0, centered at 0) |
| `.with_aftertouch()` | Enable channel aftertouch (0.0-1.0) |
| `.with_mod_wheel()` | Enable CC 1 (0.0-1.0) |
| `.with_cc(n)` | Enable single CC (0-127). **Panics if n ≥ 128.** |
| `.with_ccs(&[...])` | Enable multiple CCs (not const fn). **Panics if any CC ≥ 128.** |
| `.with_all_ccs()` | Enable all 128 CCs (creates many parameters) |

> **Note:** `with_cc()` and `with_ccs()` panic on invalid CC numbers (≥128) to catch typos like `.with_cc(130)` at runtime. In const context, this becomes a compile-time error.

**Reading Values via ProcessContext:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
    if let Some(cc) = context.midi_cc() {
        let pitch = cc.pitch_bend();   // -1.0 to 1.0
        let mod_whl = cc.mod_wheel();  // 0.0 to 1.0
        let volume = cc.cc(7);         // 0.0 to 1.0
    }
}
```

### 2.7 Manual MIDI Mapping

For custom CC-to-parameter mapping (instead of receiving as MIDI events):

**IMidiMapping**: CC to parameter:

```rust
fn midi_cc_to_parameter(&self, _bus: i32, _channel: i16, cc: u8) -> Option<u32> {
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
    if let Some(parameter_id) = self.learning_parameter.take() {
        self.midi_map.insert(cc, parameter_id);
        true
    } else {
        false
    }
}
```

**MIDI 2.0:** `midi1_assignments()`, `midi2_assignments()`, `on_midi2_learn()`

### 2.8 Keyswitch Controller

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

### 2.9 RPN/NRPN Helpers

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

**RpnTracker**: Real-time safe decoder:

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
    pub kind: ParameterNumberKind, // Rpn or Nrpn
    pub parameter: u16,
    pub value: f32,
    pub is_increment: bool,
    pub is_decrement: bool,
}
```

### 2.10 CC Utilities

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

### 2.11 VST3 Event Mapping

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

## 3. Audio Unit Integration

Beamer supports Audio Unit plugins on macOS through the `beamer-au` crate, with both **AUv2** (`.component`) and **AUv3** (`.appex`) formats fully implemented. Audio Units share the same core traits (`Descriptor`, `Processor`, `Parameters`) as VST3, allowing you to target all formats from a single codebase.

### 3.1 Architecture Overview

The `beamer-au` crate uses a **hybrid Objective-C/Rust architecture**:
- **Objective-C**: Native `AUAudioUnit` subclass (`BeamerAuWrapper`) for Apple runtime compatibility
- **Rust**: All DSP, parameters, and plugin logic via C-ABI bridge functions

This approach was chosen for several reasons:
- **Runtime compatibility**: `AUAudioUnit` subclassing requires Objective-C runtime metadata that Rust FFI bindings struggle to generate correctly
- **Simplicity**: Native ObjC integrates naturally with Apple frameworks without abstraction layers
- **Debuggability**: Apple's tools (Instruments, lldb, auval) work better with native ObjC code
- **Fewer dependencies**: No need for objc2, block2, or related crates

The hybrid architecture guarantees Apple compatibility while keeping all audio processing in Rust.

```
┌─────────────────────────────────────────────┐
│   AU Host (Logic, GarageBand, Reaper)       │
├─────────────────────────────────────────────┤
│      BeamerAuWrapper (Native Objective-C)   │
│      objc/BeamerAuWrapper.m                 │
├─────────────────────────────────────────────┤
│           C-ABI Bridge Layer                │
│   objc/BeamerAuBridge.h ↔ src/bridge.rs     │
├─────────────────────────────────────────────┤
│           beamer-au (Rust)                  │
│   AuProcessor, RenderBlock, factory         │
├─────────────────────────────────────────────┤
│              beamer-core traits             │
│   Descriptor, Processor, Parameters         │
└─────────────────────────────────────────────┘
```

#### File Structure

```
crates/beamer-au/
├── build.rs                    # Compiles Objective-C via cc crate
├── objc/
│   ├── BeamerAuBridge.h        # C-ABI function declarations
│   ├── BeamerAuWrapper.h       # ObjC class interface
│   └── BeamerAuWrapper.m       # Native AUAudioUnit subclass
└── src/
    ├── bridge.rs               # C-ABI implementations
    ├── factory.rs              # Plugin factory registration
    ├── processor.rs            # AuProcessor<P> wrapper (lifecycle, state)
    ├── render.rs               # RenderBlock (audio, MIDI, parameters)
    ├── instance.rs             # AuPluginInstance trait
    └── ...
```

**Why two files vs VST3's single `processor.rs`?** Unlike VST3 (which implements a single COM interface in Rust), AU's render callback crosses an FFI boundary from Objective-C. `processor.rs` handles plugin lifecycle on the main thread, while `render.rs` contains the `RenderBlock` for real-time audio processing on the audio thread. This separation reflects the different threading contexts and the ObjC/Rust boundary.

**Key Features:**
- Both AUv2 and AUv3 formats supported (macOS 10.11+)
- Full parameter automation via `AUParameterTree` (AUv3) and properties (AUv2)
- Parameter automation with smoother interpolation (buffer-quantized)
- MIDI input (legacy MIDI 1.0 and MIDI 2.0 UMP, 1024 event buffer)
- MIDI output via `scheduleMIDIEventBlock` (instruments/MIDI effects only)
- MIDI CC state tracking (`MidiCcState` for mod wheel, pitch bend, etc.)
- SysEx output via pre-allocated `SysExOutputPool`
- Sidechain/auxiliary buses with real bus layout forwarding
- Full state persistence (processor `save_state`/`load_state` + deferred loading)
- f32/f64 processing with pre-allocated conversion buffers
- Transport information (tempo, beat position, playback state)
- Real-time safe: no heap allocation in render path

### 3.2 Configuration

Audio Unit plugins are configured via `Config.toml` in the crate root. The configuration is shared across all plugin formats.

**Config.toml:**

```toml
name = "My Plugin"
category = "effect"
subcategories = ["dynamics"]
manufacturer_code = "Myco"
plugin_code = "mypg"
vendor = "My Company"
```

#### Plugin Categories

The `category` field in `Config.toml` determines the AU component type:

| Category | AU Component Type | Use For |
|----------|-------------------|---------|
| `"effect"` | `aufx` | EQ, compressor, reverb |
| `"midi_effect"` | `aumi` | Arpeggiator, harmonizer, MIDI effects |
| `"instrument"` | `aumu` | Synths, samplers, drum machines |
| `"generator"` | `augn` | Test tones, noise generators |

#### FourCC Codes

The `manufacturer_code` and `plugin_code` fields in `Config.toml` specify the 4-character FourCC identifiers used by Audio Units:

**Best Practices:**
- `manufacturer_code`: Use your company/product abbreviation (4 ASCII chars)
- `plugin_code`: Unique identifier for this specific plugin (4 ASCII chars)
- Avoid conflicts: Check [Apple's registry](https://developer.apple.com/library/archive/documentation/General/Conceptual/ExtensibilityPG/AudioUnit.html)
- Use lowercase for effects, MixedCase for instruments (convention)

### 3.3 Export Macro

The `#[beamer::export]` attribute macro on your descriptor struct automatically generates AU (and VST3) plugin entry points at compile time.

**Config.toml** (place in crate root next to Cargo.toml):

```toml
name = "Beamer Gain"
category = "effect"
subcategories = ["dynamics"]
manufacturer_code = "Bmer"
plugin_code = "gain"
vendor = "Beamer Framework"
```

**Rust code:**

```rust
use beamer::prelude::*;

#[beamer::export]
#[derive(Default, HasParameters)]
pub struct GainDescriptor {
    #[parameters]
    pub parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    // implementation...
}
```

The macro reads `Config.toml` and generates:
- `pub static CONFIG: Config` from the TOML fields
- AU entry points (macOS only, when `au` feature is enabled)
- VST3 entry points (when `vst3` feature is enabled)
- Optional factory presets from `Presets.toml` if present

No manual `export_au!` or `export_vst3!` calls needed.

### 3.4 Bundle Structure

#### AUv2 Bundle (`.component`)

```
MyPlugin.component/
├── Contents/
│   ├── Info.plist              # Metadata and AudioComponents
│   ├── MacOS/
│   │   └── MyPlugin            # Rust binary (universal or arch-specific)
│   ├── PkgInfo
│   └── Resources/
│       └── (assets, if any)
```

#### AUv3 Bundle (`.appex` in `.app`)

AUv3 plugins are App Extensions that must be embedded in a host application:

```
MyPlugin.app/
├── Contents/
│   ├── Info.plist
│   ├── MacOS/
│   │   └── MyPlugin            # Minimal host app
│   └── PlugIns/
│       └── MyPlugin.appex/
│           ├── Contents/
│           │   ├── Info.plist  # Extension metadata
│           │   ├── MacOS/
│           │   │   └── MyPlugin  # Rust binary
│           │   └── Resources/
│           └── ...
```

**Info.plist AudioComponents:**

```xml
<key>AudioComponents</key>
<array>
    <dict>
        <key>type</key>
        <string>aufx</string>              <!-- Effect -->
        <key>subtype</key>
        <string>gain</string>              <!-- Your subtype code -->
        <key>manufacturer</key>
        <string>Demo</string>              <!-- Your manufacturer code -->
        <key>name</key>
        <string>Beamer Gain</string>
        <key>version</key>
        <integer>65536</integer>           <!-- 1.0.0 = 0x00010000 -->
        <key>factoryFunction</key>
        <string>BeamerAudioUnitFactory</string>
    </dict>
</array>
```

**Category to AU Type Codes:**

| Category | Type Code | Description |
|----------|-----------|-------------|
| `Category::Effect` | `aufx` | Audio effect |
| `Category::MidiEffect` | `aumi` | MIDI effect (receives/produces MIDI) |
| `Category::Instrument` | `aumu` | Instrument/synthesizer |
| `Category::Generator` | `augn` | Audio generator |

### 3.5 Build System

Use `cargo xtask` to build and bundle Audio Unit plugins:

```bash
# Build AUv2 bundle (native architecture, fastest for development)
cargo xtask bundle my-plugin --auv2 --release

# Build AUv3 bundle
cargo xtask bundle my-plugin --auv3 --release

# Build and install to system location
cargo xtask bundle my-plugin --auv2 --release --install

# Build AUv2 and VST3
cargo xtask bundle my-plugin --auv2 --vst3 --release --install

# Build universal binary for distribution (x86_64 + arm64)
cargo xtask bundle my-plugin --auv2 --arch universal --release
```

**Architecture Options:**
- `--arch native` (default) - Build for current machine's architecture
- `--arch universal` - Build fat binary for distribution (x86_64 + arm64)
- `--arch arm64` - Build for Apple Silicon only
- `--arch x86_64` - Build for Intel only

**Build Options:**
- `--release` - Build with optimizations (required for real-time audio performance)
- `--install` - Install to user directory (see Install Locations below)
- `--clean` - Clean build caches before building (use when ObjC changes aren't picked up)
- `--verbose` / `-v` - Show detailed build output

**Install Locations:**

```
AUv2: ~/Library/Audio/Plug-Ins/Components/
AUv3: ~/Applications/
VST3: ~/Library/Audio/Plug-Ins/VST3/
```

**Code Signing:**

macOS requires code signing for plugins to load:

```bash
# Ad-hoc signing (development)
codesign --force --deep --sign - MyPlugin.component

# Developer ID signing (distribution)
codesign --force --deep --sign "Developer ID Application: Your Name" MyPlugin.component
```

The `xtask` tool automatically performs ad-hoc signing during bundling.

### 3.6 C-ABI Bridge Interface

The bridge layer (`objc/BeamerAuBridge.h` ↔ `src/bridge.rs`) defines the contract between the native wrappers (both AUv2 and AUv3) and Rust. Both formats share the same 40+ bridge functions:

#### Instance Lifecycle

```c
// Create/destroy plugin instances
BeamerAuInstanceHandle beamer_au_create_instance(void);
void beamer_au_destroy_instance(BeamerAuInstanceHandle instance);
```

#### Render Resources

```c
// Allocate/deallocate for audio processing
int32_t beamer_au_allocate_render_resources(
    BeamerAuInstanceHandle instance,
    double sample_rate,
    uint32_t max_frames,
    BeamerAuSampleFormat sample_format,
    const BeamerAuBusConfig* bus_config
);
void beamer_au_deallocate_render_resources(BeamerAuInstanceHandle instance);
```

#### Audio Rendering

```c
// Main render callback (real-time thread)
int32_t beamer_au_render(
    BeamerAuInstanceHandle instance,
    uint32_t* action_flags,
    const AudioTimeStamp* timestamp,
    uint32_t frame_count,
    int32_t output_bus_number,
    AudioBufferList* output_data,
    const AURenderEvent* events,
    void* pull_input_block,
    void* musical_context_block,
    void* transport_state_block,
    void* schedule_midi_block
);
```

#### Parameters

```c
// Query and modify parameters
uint32_t beamer_au_get_parameter_count(BeamerAuInstanceHandle instance);
bool beamer_au_get_parameter_info(BeamerAuInstanceHandle instance, uint32_t index, BeamerAuParameterInfo* out_info);
float beamer_au_get_parameter_value(BeamerAuInstanceHandle instance, uint32_t param_id);
void beamer_au_set_parameter_value(BeamerAuInstanceHandle instance, uint32_t param_id, float value);
```

#### State Persistence

```c
// Save/load plugin state
uint32_t beamer_au_get_state_size(BeamerAuInstanceHandle instance);
uint32_t beamer_au_get_state(BeamerAuInstanceHandle instance, uint8_t* buffer, uint32_t size);
int32_t beamer_au_set_state(BeamerAuInstanceHandle instance, const uint8_t* buffer, uint32_t size);
```

#### Bus Configuration

```c
// Query bus layout
uint32_t beamer_au_get_input_bus_count(BeamerAuInstanceHandle instance);
uint32_t beamer_au_get_output_bus_count(BeamerAuInstanceHandle instance);
uint32_t beamer_au_get_input_bus_channel_count(BeamerAuInstanceHandle instance, uint32_t bus_index);
uint32_t beamer_au_get_output_bus_channel_count(BeamerAuInstanceHandle instance, uint32_t bus_index);
```

#### MIDI Support

```c
// Check MIDI capabilities
bool beamer_au_accepts_midi(BeamerAuInstanceHandle instance);
bool beamer_au_produces_midi(BeamerAuInstanceHandle instance);
```

### 3.7 Example: Multi-Format Plugin

**Config.toml** (place in crate root next to Cargo.toml):

```toml
name = "Universal Gain"
category = "effect"
subcategories = ["dynamics"]
manufacturer_code = "Myco"
plugin_code = "gain"
vendor = "My Company"
```

**Rust code** (src/lib.rs):

```rust
use beamer::prelude::*;

// 1. Parameters
#[derive(Parameters)]
pub struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

// 2. Descriptor
#[beamer::export]
#[derive(Default, HasParameters)]
pub struct GainDescriptor {
    #[parameters]
    parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

// 3. Processor
#[derive(HasParameters)]
pub struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
}

impl Processor for GainProcessor {
    type Descriptor = GainDescriptor;

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        let gain = self.parameters.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}
```

The `#[beamer::export]` macro generates entry points for both AU (macOS) and VST3 when their respective features are enabled. Use Cargo features in your build commands:

```bash
# Build AU only (macOS)
cargo build --features au

# Build VST3 only
cargo build --features vst3

# Build both formats
cargo build --features au,vst3
```

### 3.8 Development: Testing Both AUv2 and AUv3

When you build the same plugin as both AUv2 and AUv3, macOS bridges them together if they share the same **type/subtype/manufacturer** codes. Hosts like Logic Pro and Reaper will show only one version (typically preferring AUv3).

This is by design - Apple intended AUv3 to be a drop-in replacement for AUv2 with the same identifiers allowing seamless project compatibility.

**To test both formats side by side during development:**

Create a separate test project with a different `plugin_code`:

```toml
# Main plugin Config.toml (builds as AUv3)
name = "Beamer Gain"
manufacturer_code = "Bmer"
plugin_code = "gain"

# Test plugin Config.toml (builds as AUv2 with different code)
name = "Beamer Gain (Test)"
manufacturer_code = "Bmer"
plugin_code = "gai2"  # Different plugin code
```

With different plugin codes, hosts treat them as separate plugins and both will appear in the plugin list.

**Alternative:** Uninstall one format before testing the other, then run `killall -9 AudioComponentRegistrar` to refresh the host's plugin cache.

---

## 4. VST3 Integration

Beamer supports VST3 plugins on macOS and Windows through the `beamer-vst3` crate. VST3 plugins share the same core traits (`Descriptor`, `Processor`, `Parameters`) as other formats, allowing you to target all formats from a single codebase.

### 4.1 Configuration

VST3 plugins are configured via the same `Config.toml` file used for AU plugins. The VST3 component UUID is automatically derived from your `manufacturer_code` and `plugin_code` via FNV-1a-128 hash.

**Config.toml:**

```toml
name = "My Plugin"
category = "effect"
subcategories = ["dynamics"]
manufacturer_code = "Myco"
plugin_code = "mypg"
vendor = "My Company"
```

**UUID Auto-Derivation:**

The VST3 UUID is automatically derived as:
```
FNV-1a-128("beamer-vst3-uid" + manufacturer_code + plugin_code)
```

This ensures:
- Deterministic UUIDs (same codes always produce the same UUID)
- No UUID collisions between different plugins
- No manual UUID generation needed

**UUID Override:**

If you need a specific UUID (e.g., to maintain compatibility with an existing shipped plugin), add it to `Config.toml`:

```toml
vst3_id = "12345678-9ABC-DEF0-1234-567890ABCDEF"
```

The format is `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX` (36 characters with dashes).

### 4.2 Export Macro

The `#[beamer::export]` attribute macro automatically generates VST3 entry points when the `vst3` feature is enabled.

**Config.toml:**

```toml
name = "My Synth"
category = "instrument"
manufacturer_code = "Myco"
plugin_code = "synt"
vendor = "My Company"
```

**Rust code:**

```rust
use beamer::prelude::*;

#[beamer::export]
#[derive(Default, HasParameters)]
pub struct SynthDescriptor {
    #[parameters]
    pub parameters: SynthParameters,
}

impl Descriptor for SynthDescriptor {
    // implementation...
}
```

The macro generates:
- `pub static CONFIG: Config` from Config.toml
- VST3 component UUID (auto-derived from manufacturer_code + plugin_code)
- Platform-specific entry points (`GetPluginFactory`, `bundleEntry`/`bundleExit` on macOS, `InitDll`/`ExitDll` on Windows)
- Optional factory presets if `Presets.toml` is present

The same code compiles for both AU and VST3 formats. Use Cargo features to control which formats to build.

### 4.3 Bundle Structure

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

### 4.4 Build System

```bash
cargo xtask bundle gain --vst3 --release
```

**Cargo.toml:**

```toml
[lib]
crate-type = ["cdylib"]

[profile.release]
lto = true
```

### 4.5 Install Locations

| Platform | Location |
|----------|----------|
| macOS | `~/Library/Audio/Plug-Ins/VST3/` |
| Windows | `C:\Program Files\Common Files\VST3\` |

### 4.6 Plugin Categories

VST3 subcategories are derived automatically from `Config.toml`:

```toml
# category = "effect" with subcategories = ["dynamics"] becomes "Fx|Dynamics"
name = "My Compressor"
category = "effect"
subcategories = ["dynamics"]
manufacturer_code = "Myco"
plugin_code = "comp"

# category = "instrument" with subcategories = ["synth"] becomes "Instrument|Synth"
name = "My Synth"
category = "instrument"
subcategories = ["synth"]
manufacturer_code = "Myco"
plugin_code = "synt"
```

Common subcategories: `Subcategory::Dynamics`, `Eq`, `Filter`, `Delay`, `Reverb`, `Modulation`, `Distortion`, `Synth`, `Sampler`, etc.

---

## 5. Future Phases

### 5.1 Phase 2: WebView Integration

Add platform-native WebView embedding to plugin windows.

#### Platform Backends

| Platform | Backend | Rust Approach |
|----------|---------|---------------|
| macOS | WKWebView | `objc2` + `icrate` |
| Windows | WebView2 (Edge/Chromium) | `webview2` crate or direct COM |

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

### 5.2 Phase 3: IPC & Parameter Binding

Tauri-style bidirectional communication between Rust and JavaScript.

#### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      JavaScript                             │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  window.__PLUGIN__ = {                              │    │
│  │    invoke(cmd, args) → Promise                      │    │
│  │    on(event, callback)                              │    │
│  │    emit(event, data)                                │    │
│  │    getParameter(id) → ParameterState                │    │
│  │  }                                                  │    │
│  └─────────────────────────────────────────────────────┘    │
│              │                         ▲                    │
│              ▼                         │                    │
│     plugin://invoke/...        evaluateJavascript()         │
│     (custom URL scheme)        (push events to JS)          │
└──────────────┼─────────────────────────┼────────────────────┘
               │                         │
┌──────────────▼─────────────────────────┼────────────────────┐
│                       Rust                                  │
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
{ "id": 1, "cmd": "setParameter", "args": { "parameterId": 0, "value": 0.75 } }
```

**Response (Rust → JS):**
```json
{ "id": 1, "result": { "ok": true } }
```

**Event (Rust → JS):**
```json
{ "event": "parameterChanged", "data": { "parameterId": 0, "value": 0.75 } }
```

#### JavaScript API

```javascript
// Invoke a Rust command
const result = await window.__PLUGIN__.invoke('getParameterValue', { parameterId: 0 });

// Listen for events
window.__PLUGIN__.on('parameterChanged', (data) => {
    console.log(`Parameter ${data.parameterId} = ${data.value}`);
});

// Parameter state helper with automation support
const gain = window.__PLUGIN__.getParameter(0);
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

### 5.3 Phase 4: Developer Experience

- Hot reload: Detect dev server, auto-refresh on file changes
- CLI tooling: `cargo beamer new`, `cargo beamer dev`
- Documentation generation from plugin metadata

### 5.4 Phase 5: Examples & Polish

- Real-world examples (equalizer, compressor, synthesizer)
- Performance profiling and optimization
- Cross-DAW validation (Cubase, Ableton, Logic, REAPER, Bitwig)

### 5.5 Core API Enhancements

#### Sample-Accurate Parameter Automation

**Current Behavior:** Both VST3 and AU wrappers apply parameter changes at the start of each audio buffer, using the last value in the automation queue. The existing `Smoother` infrastructure then interpolates to avoid zipper noise.

**Limitation:** This approach is buffer-quantized rather than sample-accurate. For most plugins this is imperceptible, but edge cases exist:
- Ultra-fast LFO modulation of parameters
- Sample-accurate gate/trigger parameters
- Precision timing for transient designers

**Planned Enhancement:** Add dynamic ramp support to `beamer_core::Smoother`:

```rust
// New API (proposed)
impl Smoother {
    /// Set target with explicit ramp duration in samples.
    /// Overrides the default smoothing time for this transition only.
    pub fn set_target_with_samples(&mut self, target: f64, ramp_samples: u32);
}

// Usage in parameter handling
for event in &events.ramps {
    if let Some(param) = parameters.by_id(event.param_id) {
        param.set_normalized_with_ramp(event.end_value, event.ramp_duration_samples);
    }
}
```

**Alternative:** Sub-block processing that splits the buffer at parameter event boundaries. Higher overhead but provides true sample-accuracy.

**Priority:** Low - current behavior matches industry standard (VST3 SDK reference implementation uses same approach) and covers 99%+ of use cases.
