# Beamer API Reference

**Version:** 0.1.6

This document provides detailed API documentation for Beamer. For high-level architecture and design decisions, see [ARCHITECTURE.md](../ARCHITECTURE.md).

---

## Table of Contents

1. [Core API](#1-core-api)
2. [MIDI Reference](#2-midi-reference)
3. [VST3 Integration](#3-vst3-integration)
4. [Audio Unit Integration](#4-audio-unit-integration)
5. [Future Phases](#5-future-phases)
6. [Appendices](#appendices)

---

## 1. Core API

> **Note**: Many API behaviors depend on VST3 host requirements. See [Section 3: VST3 Integration](#3-vst3-integration) for critical constraints like entry point naming, bundle structure, and parameter flag mapping.

### 1.1 Plugin Trait

The `Plugin` trait represents a plugin in its **unprepared state** — before the host provides audio configuration. When the host calls `setupProcessing()`, the plugin transforms into an `AudioProcessor` via the `prepare()` method.

```rust
pub trait Plugin: HasParameters + Default {
    /// Setup type for prepare() — determines what info is needed
    type Setup: PluginSetup;

    /// The prepared processor type
    type Processor: AudioProcessor<Plugin = Self, Parameters = Self::Parameters>;

    /// Transform into a prepared processor with audio configuration.
    /// Consumes self — the plugin moves into the prepared state.
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
    type Parameters: Parameters + Units + Parameters;
    fn parameters(&self) -> &Self::Parameters;
    fn parameters_mut(&mut self) -> &mut Self::Parameters;
}
```

**HasParameters Derive Macro:** Use `#[derive(HasParameters)]` to eliminate boilerplate:

```rust
#[derive(Default, HasParameters)]
struct GainPlugin {
    #[parameters]
    parameters: GainParameters,
}
```

#### PluginSetup Types

Request exactly what your plugin needs using composable types:

| Type | When to Use | Value |
|------|-------------|-------|
| `()` | Stateless plugins (gain, pan) | — |
| `SampleRate` | Most plugins (delay, filter, envelope) | `f64` via `.hz()` |
| `MaxBufferSize` | FFT, lookahead | `usize` |
| `MainInputChannels` | Per-channel input processing | `u32` |
| `MainOutputChannels` | Per-channel output state | `u32` |
| `AuxInputCount` | Sidechain-aware processing | `usize` |
| `AuxOutputCount` | Multi-bus output | `usize` |
| `ProcessMode` | Offline quality settings | enum |

Compose multiple types using tuples:

```rust
// Simple plugin — no setup needed
impl Plugin for GainPlugin {
    type Setup = ();
    fn prepare(self, _: ()) -> GainProcessor { /* ... */ }
}

// Plugin needing sample rate (delays, filters, smoothing)
impl Plugin for DelayPlugin {
    type Setup = SampleRate;
    fn prepare(self, sample_rate: SampleRate) -> DelayProcessor {
        // sample_rate.hz() returns the sample rate
    }
}

// Plugin needing multiple setup values
impl Plugin for FftPlugin {
    type Setup = (SampleRate, MaxBufferSize);
    fn prepare(self, (sr, mbs): (SampleRate, MaxBufferSize)) -> FftProcessor {
        // sr.0, mbs.0 contain the values
    }
}
```

### 1.2 AudioProcessor Trait

The `AudioProcessor` trait represents a plugin in its **prepared state** — ready for real-time audio processing. Created by `Plugin::prepare()`, it can transform back to unprepared state via `unprepare()`.

```rust
pub trait AudioProcessor: HasParameters {
    /// The unprepared plugin type this processor came from
    type Plugin: Plugin<Processor = Self, Parameters = Self::Parameters>;

    /// Transform back to unprepared state.
    /// Called when host calls setProcessing(false).
    fn unprepare(self) -> Self::Plugin;

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

    /// State persistence
    fn save_state(&self) -> PluginResult<Vec<u8>>;
    fn load_state(&mut self, data: &[u8]) -> PluginResult<()>;
}
```

**When to implement `set_active()`:** Plugins with internal DSP state (delay lines, filter histories, envelopes, oscillator phases) should override `set_active()` and reset that state when `active == true`. Hosts call `setActive(false)` followed by `setActive(true)` to request a full state reset. Plugins without internal state (simple gain, pan) can use the default empty implementation.

#### Two-Phase Lifecycle

The plugin transitions between states based on host actions:

```
                    ┌─────────────────┐
                    │  Plugin         │
                    │  (unprepared)   │
                    └────────┬────────┘
                             │ setupProcessing(true)
                             │ + prepare(config)
                             ▼
                    ┌─────────────────┐
                    │  AudioProcessor │
                    │  (prepared)     │◄───── process() calls
                    └────────┬────────┘
                             │ setProcessing(false)
                             │ + unprepare()
                             ▼
                    ┌─────────────────┐
                    │  Plugin         │
                    │  (unprepared)   │
                    └─────────────────┘
```

### 1.3 Parameters

Beamer provides two parameter APIs:
- **`Parameters` trait**: Low-level VST3 integration (manual implementation)
- **`Parameters` trait + derive macro**: High-level ergonomic API (recommended)
- **Parameter smoothing**: Opt-in smoothing to avoid zipper noise during automation

#### Derive Macro (Recommended)

**Declarative Style** — Macro generates everything including `Default`:

```rust
use beamer::prelude::*;
use beamer::Parameters;

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

- `db_log` — Power curve (exponent 2.0) for more resolution near 0 dB (use for thresholds)
- `db_log_offset` — True logarithmic mapping for dB ranges (geometric mean at midpoint)

Supported field types: `FloatParameter`, `IntParameter`, `BoolParameter`, `EnumParameter<E>`

#### Parameter Types

**FloatParameter** — Continuous floating-point parameter:

```rust
// Linear range
let freq = FloatParameter::new("Frequency", 1000.0, 20.0..=20000.0);

// Decibel range (stores dB, use as_linear() for DSP)
let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0);

// In DSP code:
let amplitude = gain.as_linear();  // 0 dB → 1.0, -6 dB → ~0.5
let db_value = gain.get();         // Returns dB for display
```

**IntParameter** — Integer parameter:

```rust
let voices = IntParameter::new("Voices", 8, 1..=64);
```

**BoolParameter** — Toggle parameter:

```rust
let bypass = BoolParameter::new("Bypass", false);
```

**EnumParameter** — Discrete choice parameter:

```rust
use beamer::EnumParameter as DeriveEnumParameter;

#[derive(Copy, Clone, PartialEq, DeriveEnumParameter)]
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

#[derive(Parameters)]
pub struct FilterParameters {
    #[parameter(id = "filter_type")]
    pub filter_type: EnumParameter<FilterType>,
}

impl Default for FilterParameters {
    fn default() -> Self {
        Self {
            // Uses HighPass (from #[default]) as the default value
            filter_type: EnumParameter::new("Filter Type",
        }
    }
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
    .with_precision(2);  // Shows "-0.50 dB" instead of "-0.5 dB"

// Milliseconds with integer display
let attack = FloatParameter::ms("Attack", 10.0, 0.1..=100.0)
    .with_precision(0);  // Shows "10" instead of "10.0"

// Completely custom formatter
let ratio = FloatParameter::new("Ratio", 4.0, 1.0..=20.0)
    .with_formatter(Formatter::Ratio { precision: 1 });  // Shows "4.0:1"

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
impl Plugin for MyPlugin {
    type Setup = SampleRate;

    fn prepare(mut self, setup: SampleRate) -> MyProcessor {
        self.parameters.set_sample_rate(setup.hz());
        MyProcessor { parameters: self.parameters, /* ... */ }
    }
}
```

> **Oversampling:** If your plugin uses oversampling, pass the actual processing rate:
> `self.parameters.set_sample_rate(setup.hz() * oversampling_factor as f64);`

**Per-Sample Processing:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
    for (input, output) in buffer.zip_channels() {
        for (i, o) in input.iter().zip(output.iter_mut()) {
            let gain = self.parameters.gain.tick_smoothed();  // Advances smoother
            *o = *i * gain as f32;
        }
    }
}
```

**Block Processing:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
    let gain = self.parameters.gain.smoothed();  // Current value, no advance
    self.parameters.gain.skip_smoothing(buffer.len());

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
self.parameters.gain.fill_smoothed_f32(&mut gain_buffer[..len]);
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

#### Flat Visual Grouping

Use `group = "..."` for visual grouping in the DAW without nested structs:

```rust
#[derive(Parameters)]
pub struct SynthParameters {
    #[parameter(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
    pub cutoff: FloatParameter,

    #[parameter(id = "reso", name = "Resonance", default = 0.5, range = 0.0..=1.0, group = "Filter")]
    pub resonance: FloatParameter,

    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db", group = "Output")]
    pub gain: FloatParameter,
}

// Access is flat: parameters.cutoff, parameters.resonance, parameters.gain
// DAW shows collapsible "Filter" and "Output" groups
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
pub struct SynthParameters {
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
- "gain"              — top-level parameter
- "filter/cutoff"     — parameter in "Filter" group
- "osc1/filter/res"   — deeply nested parameter
```

The same nested struct can be reused in multiple groups without ID collision:

```rust
#[nested(group = "Osc 1")]
pub osc1: OscParameters,

#[nested(group = "Osc 2")]
pub osc2: OscParameters,  // Same struct, different paths: "osc1/attack" vs "osc2/attack"
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
    pub group_id: GroupId,  // Parameter group (0 = root)
}

pub struct ParameterFlags {
    pub can_automate: bool,
    pub is_readonly: bool,
    pub is_bypass: bool,   // Maps to VST3 kIsBypass (see §3.2)
    pub is_list: bool,     // Display as dropdown list (for enums)
    pub is_hidden: bool,   // Hide from DAW parameter list (used by MIDI CC emulation)
}

impl ParameterInfo {
    /// Convenience constructor for bypass parameters.
    pub const fn bypass(id: ParameterId) -> Self;
}
```

### 1.4 Factory Presets

Factory presets let plugins provide built-in presets that appear in host preset menus (e.g., Logic's preset browser, VST3 program changes). Users can browse and load these presets without needing separate preset files.

#### Derive Macro

Use `#[derive(Presets)]` on an enum to define factory presets:

```rust
use beamer::prelude::*;
use beamer::Presets;

#[derive(Presets)]
#[preset(parameters = GainParameters)]
pub enum GainPresets {
    #[preset(name = "Default", values(gain = 0.0))]
    Default,

    #[preset(name = "Subtle", values(gain = -6.0))]
    Subtle,

    #[preset(name = "Boost", values(gain = 6.0))]
    Boost,
}
```

#### Attributes

| Attribute | Location | Description |
|-----------|----------|-------------|
| `#[preset(parameters = Type)]` | Enum | Links to the Parameters type for validation |
| `#[preset(name = "...", values(...))]` | Variant | Defines preset name and parameter values |

**Values syntax:** `values(param_id = value, ...)` where:
- `param_id` matches the parameter's `id` attribute (e.g., `gain` for `#[parameter(id = "gain", ...)]`)
- `value` is the plain/display value (e.g., `-6.0` for dB), not normalized

#### Sparse Presets

Presets can specify only a subset of parameters. Unspecified parameters retain their current values:

```rust
#[derive(Presets)]
#[preset(parameters = MyParameters)]
pub enum MyPresets {
    // Full preset: sets all parameters
    #[preset(name = "Initialize", values(gain = 0.0, mix = 1.0, bypass = false))]
    Init,

    // Sparse preset: only changes gain, mix and bypass unchanged
    #[preset(name = "Quiet", values(gain = -12.0))]
    Quiet,
}
```

#### Integration with Export Macros

Pass the presets type as the final argument to export macros:

```rust
// VST3 with factory presets
#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, MyPlugin, MyPresets);

// AU with factory presets
#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, MyPlugin, MyPresets);

// Without factory presets (uses NoPresets internally)
#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, MyPlugin);

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, MyPlugin);
```

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

This mirrors VST3's `kIsProgramChange` behavior where the host handles PC→preset mapping automatically. No plugin code changes are required—the framework handles this based on whether factory presets are defined.

### 1.5 Buffer Types

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

### 1.6 ProcessContext and Transport

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

### 1.7 Sample Trait (f32/f64)

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
impl MyProcessor {
    // Generic processing - works for both f32 and f64
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        _aux: &mut AuxiliaryBuffers<S>,
        _context: &ProcessContext,
    ) {
        let gain = S::from_f32(self.parameters.gain_linear());
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

// Delegate from both AudioProcessor methods
impl AudioProcessor for MyProcessor {
    type Plugin = MyPlugin;

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

    // ... other methods
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

### 1.8 Soft Bypass

```rust
pub enum BypassState {
    Active,
    RampingToBypassed,
    Bypassed,
    RampingToActive,
}

/// What action the plugin should take for this buffer.
pub enum BypassAction {
    Passthrough,          // Fully bypassed - copy input to output
    Process,              // Fully active - run DSP normally
    ProcessAndCrossfade,  // Transitioning - run DSP, then call finish()
}

pub enum CrossfadeCurve {
    Linear,      // Slight loudness dip at center
    EqualPower,  // Constant loudness (recommended)
    SCurve,      // Faster start/end, smoother middle
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
```

**Why Split API?** The split pattern (begin/finish) avoids Rust borrow checker conflicts that occur with closure-based APIs when your DSP code needs to access `&mut self`.

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

// Raw MIDI 1.0 byte parsing (used by AU, future CLAP/LV2)
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

### 2.5 MIDI CC Emulation (MidiCcConfig)

VST3 doesn't send MIDI CC, pitch bend, or aftertouch directly to plugins. Most DAWs convert these to parameter changes via the `IMidiMapping` interface. `MidiCcConfig` tells the framework which controllers you want—it handles all the state management automatically:

```rust
use beamer::prelude::*;
use beamer::HasParameters;

// Unprepared plugin state - no midi_cc field needed!
#[derive(Default, HasParameters)]
struct MySynthPlugin {
    #[parameters]
    parameters: MyParameters,
}

impl Plugin for MySynthPlugin {
    type Setup = SampleRate;
    type Processor = MySynthProcessor;

    fn prepare(mut self, _setup: SampleRate) -> MySynthProcessor {
        MySynthProcessor {
            parameters: self.parameters,
            // No midi_cc to move - framework manages it!
        }
    }

    // Just return configuration - framework handles state
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

// Prepared processor state - no midi_cc field needed!
#[derive(HasParameters)]
struct MySynthProcessor {
    #[parameters]
    parameters: MyParameters,
}

impl AudioProcessor for MySynthProcessor {
    type Plugin = MySynthPlugin;

    fn unprepare(self) -> MySynthPlugin {
        MySynthPlugin { parameters: self.parameters }
        // No midi_cc to move back!
    }

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

### 2.6 Manual MIDI Mapping

For custom CC-to-parameter mapping (instead of receiving as MIDI events):

**IMidiMapping** — CC to parameter:

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

### 2.7 Keyswitch Controller

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

### 2.8 RPN/NRPN Helpers

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

### 2.9 CC Utilities

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

### 2.10 VST3 Event Mapping

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

### 3.2 Build System

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

### 3.3 Install Locations

| Platform | Location |
|----------|----------|
| macOS | `~/Library/Audio/Plug-Ins/VST3/` |
| Windows | `C:\Program Files\Common Files\VST3\` |
| Linux | `~/.vst3/` |

### 3.4 Plugin Categories

VST3 uses categories for plugin browser organization:

```rust
// Audio effect
Vst3Config::new(uid(...)).with_categories("Fx|Dynamics")

// Instrument
Vst3Config::new(uid(...)).with_categories("Instrument|Synth")
```

Common VST3 categories: `Fx`, `Instrument`, `Dynamics`, `EQ`, `Filter`, `Delay`, `Reverb`, `Mastering`, etc.

---

## 4. Audio Unit Integration

Beamer supports Audio Unit plugins on macOS through the `beamer-au` crate, with both **AUv2** (`.component`) and **AUv3** (`.appex`) formats fully implemented. Audio Units share the same core traits (`Plugin`, `AudioProcessor`, `Parameters`) as VST3, allowing you to target all formats from a single codebase.

### 4.1 Architecture Overview

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
│   AuProcessor, RenderBlock, factory        │
├─────────────────────────────────────────────┤
│              beamer-core traits             │
│   Plugin, AudioProcessor, Parameters        │
└─────────────────────────────────────────────┘
```

#### File Structure

```
crates/beamer-au/
├── build.rs                    # Compiles Objective-C via cc crate
├── objc/
│   ├── BeamerAuBridge.h        # C-ABI function declarations
│   ├── BeamerAuWrapper.h       # ObjC class interface
│   └── BeamerAuWrapper.m       # Native AUAudioUnit subclass (~700 lines)
└── src/
    ├── bridge.rs               # C-ABI implementations (~1100 lines)
    ├── factory.rs              # Plugin factory registration
    ├── processor.rs            # AuProcessor<P> wrapper (lifecycle, state)
    ├── render.rs               # RenderBlock (audio, MIDI, parameters)
    ├── instance.rs             # AuPluginInstance trait
    └── ...
```

**Why two files vs VST3's single `processor.rs`?** Unlike VST3 (which implements a single COM interface in Rust), AU's render callback crosses an FFI boundary from Objective-C. `processor.rs` handles plugin lifecycle on the main thread, while `render.rs` contains the `RenderBlock` for real-time audio processing on the audio thread. This separation reflects the different threading contexts and the ObjC/Rust boundary.

**Key Features (Full VST3 Parity):**
- Both AUv2 and AUv3 formats supported (macOS 10.11+)
- Full parameter automation via `AUParameterTree` (AUv3) and properties (AUv2)
- Parameter automation with smoother interpolation (buffer-quantized, matches VST3)
- MIDI input (legacy MIDI 1.0 and MIDI 2.0 UMP, 1024 event buffer)
- MIDI output via `scheduleMIDIEventBlock` (instruments/MIDI effects only)
- MIDI CC state tracking (`MidiCcState` for mod wheel, pitch bend, etc.)
- SysEx output via pre-allocated `SysExOutputPool`
- Sidechain/auxiliary buses with real bus layout forwarding
- Full state persistence (processor `save_state`/`load_state` + deferred loading)
- f32/f64 processing with pre-allocated conversion buffers
- Transport information (tempo, beat position, playback state)
- Real-time safe: no heap allocation in render path

### 4.2 Configuration

Audio Unit plugins require two configuration objects: the shared `PluginConfig` (from `beamer-core`) and the AU-specific `AuConfig`.

```rust
use beamer_core::PluginConfig;
use beamer_au::{AuConfig, ComponentType};

// Shared configuration (format-agnostic metadata)
pub static CONFIG: PluginConfig = PluginConfig::new("My Plugin")
    .with_vendor("My Company")
    .with_version(env!("CARGO_PKG_VERSION"));

// AU-specific configuration
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,      // Effect, MusicEffect, or Generator
    "Myco",           // Manufacturer code (4 chars)
    "mypg",           // Subtype code (4 chars, unique)
);
```

#### Component Types

| Type | Description | Use For |
|------|-------------|---------|
| `ComponentType::Effect` | Standard audio effect | EQ, compressor, reverb |
| `ComponentType::MusicEffect` | Musical effects receiving MIDI | Arpeggiator, harmonizer |
| `ComponentType::Generator` | Instrument/synthesizer | Synths, samplers, drums |

#### FourCC Codes

Audio Units use FourCharCode identifiers:

```rust
// Using the fourcc! macro
fourcc!(b"Demo")  // Compile-time constant

// Or at runtime
FourCharCode::from_bytes(*b"Demo")
FourCharCode::from_str("Demo")
```

**Best Practices:**
- Manufacturer code: Use your company/product abbreviation
- Subtype code: Unique identifier for this specific plugin
- Avoid conflicts: Check [Apple's registry](https://developer.apple.com/library/archive/documentation/General/Conceptual/ExtensibilityPG/AudioUnit.html)
- Use lowercase for effects, MixedCase for instruments (convention)

### 4.3 Export Macro

The `export_au!` macro creates the necessary entry points for Audio Unit discovery and instantiation:

```rust
use beamer::prelude::*;
use beamer_au::{export_au, AuConfig, ComponentType};

#[cfg(target_os = "macos")]
use beamer_au::{export_au, AuConfig, ComponentType};

// Shared config
pub static CONFIG: PluginConfig = PluginConfig::new("Beamer Gain")
    .with_vendor("Beamer Framework")
    .with_version(env!("CARGO_PKG_VERSION"));

// AU config
#[cfg(target_os = "macos")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    "Demo",
    "gain",
);

// Export for macOS only
#[cfg(target_os = "macos")]
export_au!(CONFIG, AU_CONFIG, MyPlugin);
```

**Multi-Format Export:**

```rust
// Export both VST3 and AU from the same plugin
#[cfg(not(target_os = "macos"))]
export_vst3!(CONFIG, VST3_CONFIG, MyPlugin);

#[cfg(target_os = "macos")]
export_au!(CONFIG, AU_CONFIG, MyPlugin);
```

### 4.4 Bundle Structure

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

**Component Type Codes:**

| ComponentType | Type Code | Description |
|--------------|-----------|-------------|
| `Effect` | `aufx` | Audio effect |
| `MusicEffect` | `aumf` | Musical effect (receives MIDI) |
| `Generator` | `aumu` | Instrument/generator |

### 4.5 Build System

Use `cargo xtask` to build and bundle Audio Unit plugins:

```bash
# Build AUv2 bundle (native architecture, fastest for development)
cargo xtask bundle my-plugin --auv2 --release

# Build AUv3 bundle
cargo xtask bundle my-plugin --auv3 --release

# Build and install to system location
cargo xtask bundle my-plugin --auv2 --release --install

# Build VST3 and AUv2
cargo xtask bundle my-plugin --vst3 --auv2 --release --install

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
VST3: ~/Library/Audio/Plug-Ins/VST3/
AUv2: ~/Library/Audio/Plug-Ins/Components/
AUv3: ~/Applications/
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

### 4.6 C-ABI Bridge Interface

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

### 4.7 Current Status

**Implementation Status: Complete**

Both AUv2 and AUv3 formats are fully implemented with full VST3 feature parity.

**Implemented (VST3 Parity):**
- ✅ Audio effects (all bus configurations)
- ✅ Instruments/generators (MIDI input + output)
- ✅ MIDI effects (MIDI input + output)
- ✅ Sidechain/auxiliary buses
- ✅ Parameter automation (KVO for AUv3, properties for AUv2)
- ✅ State persistence (save/load presets)
- ✅ f32 and f64 processing
- ✅ Transport information (tempo, beat, playback state)
- ✅ Real-time safe render path (no allocations, panic catching)
- ✅ Thread-safe parameter access (RwLock for render block)

**Architecture Features:**
- AUv2: `AudioComponentPlugInInterface` with selector-based dispatch
- AUv3: Native Objective-C `AUAudioUnit` subclass
- Shared C-ABI bridge with 40+ functions for wrapper ↔ Rust communication
- Pre-allocated buffers for audio processing
- Weak/strong self pattern in parameter callbacks (prevents use-after-free)
- Comprehensive null pointer validation

**Limitations:**
- No custom UI (uses host generic parameter UI)
- MIDI output only for instruments/MIDI effects (not audio effects)
- macOS only (Audio Units are Apple-exclusive)

**Validation Commands:**
```bash
# Build and install AUv2
cargo xtask bundle gain --auv2 --release --install

# Validate with Apple's auval tool
auval -v aufx gain Bmer

# Check installed location
ls ~/Library/Audio/Plug-Ins/Components/
```

### 4.8 Example: Multi-Format Plugin

```rust
use beamer::prelude::*;

// Shared configuration
pub static CONFIG: PluginConfig = PluginConfig::new("Universal Gain")
    .with_vendor("My Company")
    .with_version(env!("CARGO_PKG_VERSION"));

// VST3 configuration
#[cfg(feature = "vst3")]
const COMPONENT_UID: beamer::vst3::Steinberg::TUID =
    beamer::vst3::uid(0x12345678, 0x9ABCDEF0, 0xABCDEF12, 0x34567890);
#[cfg(feature = "vst3")]
pub static VST3_CONFIG: beamer_vst3::Vst3Config =
    beamer_vst3::Vst3Config::new(COMPONENT_UID)
        .with_categories("Fx|Dynamics");

// AU configuration (macOS only)
#[cfg(target_os = "macos")]
use beamer_au::{AuConfig, ComponentType};
#[cfg(target_os = "macos")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    "Myco",
    "gain",
);

// Plugin implementation (format-agnostic)
#[derive(Parameters)]
pub struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

#[derive(Default, HasParameters)]
pub struct GainPlugin {
    #[parameters]
    parameters: GainParameters,
}

impl Plugin for GainPlugin {
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

#[derive(HasParameters)]
pub struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
}

impl AudioProcessor for GainProcessor {
    type Plugin = GainPlugin;
    fn unprepare(self) -> GainPlugin {
        GainPlugin { parameters: self.parameters }
    }
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        let gain = self.parameters.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(self.parameters.save_state())
    }
    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        self.parameters.load_state(data).map_err(PluginError::StateError)
    }
}

// Format-specific exports
#[cfg(not(target_os = "macos"))]
export_vst3!(CONFIG, VST3_CONFIG, GainPlugin);

#[cfg(target_os = "macos")]
export_au!(CONFIG, AU_CONFIG, GainPlugin);
```

### 4.9 Development: Testing Both AUv2 and AUv3

When you build the same plugin as both AUv2 and AUv3, macOS bridges them together if they share the same **type/subtype/manufacturer** codes. Hosts like Logic Pro and Reaper will show only one version (typically preferring AUv3).

This is by design—Apple intended AUv3 to be a drop-in replacement for AUv2 with the same identifiers allowing seamless project compatibility.

**To test both formats side by side during development:**

Create a separate test project with a different subtype code:

```rust
// Main plugin (builds as AUv3)
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    "Bmer",
    "gain",  // subtype: "gain"
);

// Test plugin (builds as AUv2 with different subtype)
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    "Bmer",
    "gai2",  // subtype: "gai2" — different!
);
```

With different subtype codes, hosts treat them as separate plugins and both will appear in the plugin list.

**Alternative:** Uninstall one format before testing the other, then run `killall -9 AudioComponentRegistrar` to refresh the host's plugin cache.

---

## 5. Future Phases

### 5.1 Phase 2: WebView Integration

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

### 5.2 Phase 3: IPC & Parameter Binding

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
│  │    getParameter(id) → ParameterState                        │    │
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

- Real-world examples (EQ, compressor, synth)
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

**Priority:** Low — current behavior matches industry standard (VST3 SDK reference implementation uses same approach) and covers 99%+ of use cases.

---

## 6. Appendices

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

// =============================================================================
// Parameters
// =============================================================================

#[derive(Parameters)]
pub struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

impl GainParameters {
    fn gain_linear(&self) -> f32 {
        self.gain.as_linear() as f32
    }
}

// =============================================================================
// Plugin (Unprepared State)
// =============================================================================

#[cfg(feature = "vst3")]
const COMPONENT_UID: beamer::vst3::Steinberg::TUID =
    beamer::vst3::uid(0x12345678, 0x12345678, 0x12345678, 0x12345678);

pub static CONFIG: PluginConfig = PluginConfig::new("My Gain")
    .with_vendor("My Company")
    .with_version("1.0.0");

#[derive(Default, HasParameters)]
pub struct GainPlugin {
    #[parameters]
    parameters: GainParameters,
}

impl Plugin for GainPlugin {
    type Setup = ();  // Simple gain doesn't need sample rate
    type Processor = GainProcessor;

    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

// =============================================================================
// Audio Processor (Prepared State)
// =============================================================================

#[derive(HasParameters)]
pub struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
}

impl AudioProcessor for GainProcessor {
    type Plugin = GainPlugin;

    fn unprepare(self) -> GainPlugin {
        GainPlugin { parameters: self.parameters }
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        let gain = self.parameters.gain_linear();
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(self.parameters.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        self.parameters.load_state(data).map_err(PluginError::StateError)
    }
}

// =============================================================================
// VST3 Export
// =============================================================================

#[cfg(feature = "vst3")]
pub static VST3_CONFIG: beamer_vst3::Vst3Config =
    beamer_vst3::Vst3Config::new(COMPONENT_UID);

#[cfg(feature = "vst3")]
beamer_vst3::export_vst3!(CONFIG, VST3_CONFIG, GainPlugin);
```

### C. Example: Sidechain Compressor

```rust
fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
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
