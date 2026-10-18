# Beamer API Reference

**Version:** 0.2.4

This document provides detailed API documentation for Beamer. For high-level architecture and design decisions, see [ARCHITECTURE.md](../ARCHITECTURE.md). For building, installing and testing plugins, see [BUILDING.md](BUILDING.md). For the WebView GUI system, see [WEBVIEW.md](WEBVIEW.md).

---

## Table of Contents

1. [Core API](#core-api)
2. [MIDI Reference](#midi-reference)
3. [Format Notes](#format-notes)

---

## Core API

### Plugin Configuration

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
log_level = "warn"

# Optional: Advanced SysEx configuration
# sysex_slots = 64
# sysex_buffer_size = 4096
```

**Required Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `name` | String | Plugin name displayed in DAW |
| `category` | String | Plugin type: `"effect"`, `"instrument"`, `"midi_effect"` or `"generator"` |
| `manufacturer_code` | String | 4-character manufacturer code (e.g., `"Myco"`) |
| `plugin_code` | String | 4-character plugin code (e.g., `"mypg"`) |

**Optional Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `vendor` | String | Company/developer name |
| `url` | String | Vendor website |
| `email` | String | Support email |
| `subcategories` | Array | Subcategory strings for DAW browser organization (e.g., `["dynamics", "eq"]`) |
| `has_gui` | Boolean | Whether plugin has a GUI (default: `false`) |
| `gui_size` | Array | Initial GUI size as `[width, height]` in pixels (required when `has_gui` is true) |
| `gui_background_color` | String | Hex color (e.g. `"#1a1a2e"`) painted on the parent view behind the WebView while content loads |
| `vst3_id` | String | Override auto-derived VST3 UUID (format: `"XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"`) |
| `log_level` | String | Log the framework's and the plugin's messages up to this level: `"error"`, `"warn"`, `"info"`, `"debug"` or `"trace"` (see [Logging](#logging)). Without it the plugin installs no logger |

**Advanced Optional Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `vst3_controller_id` | String | VST3 controller UUID for split component/controller architecture (format: `"XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"`) |
| `sysex_slots` | Integer | Number of pre-allocated SysEx output slots for VST3 (default: 16) |
| `sysex_buffer_size` | Integer | Maximum payload of a SysEx output message in bytes for VST3, without F0 and F7 (default: 512) |

**SysEx Configuration:**
- Advanced settings for plugins that send SysEx messages
- Defaults (16 slots, 512 bytes) cover most use cases
- Higher values use more memory but allow more concurrent/larger SysEx messages
- The settings size the VST3 output pool. The AU formats pass SysEx output to the host directly, without a pool

**Notes:**

- Version is automatically read from `CARGO_PKG_VERSION`
- VST3 UUID is auto-derived from `manufacturer_code` and `plugin_code` unless `vst3_id` is specified
- The 4-character codes are used for both AU FourCC identifiers and VST3 UUID derivation
- `category` determines the AU component type (`aufx`, `aumu`, `aumi`, `augn`)
- By convention fields are ordered as in the example above: first identity (`name`, `category`, `subcategories`), then codes (`manufacturer_code`, `plugin_code`) and finally metadata (`vendor`, `url`, `email`)

#### Logging

The framework reports problems through the `log` crate: entries of a saved state it had to skip, MIDI or parameter events the audio thread had to drop, a WebView that could not load. A plugin sees these, and its own `log` messages, once it sets `log_level` in `Config.toml`. The wrappers then install a logger when the plugin is loaded.

On macOS the messages go to the unified system log, under the subsystem `com.beamer.<package>` and a category per format (`vst3`, `au`). This works in every host, also in the sandboxed process of an AUv3. Read them as they come with:

```bash
cargo xtask log <package>
```

or later with `/usr/bin/log show --last 1h --predicate 'subsystem == "com.beamer.<package>"'` (the full path, because `log` is a different command in the zsh shell), or in Console.app. Warnings and errors stay in the system log, debug messages only show up while you watch. Other platforms do not log yet.

Every plugin binary has its own logger, so a plugin cannot get in the way of the host or of other plugins. A plugin that installs a logger of its own does not set `log_level`. Nothing logs on the audio thread: the wrappers note dropped events there and log them later on a host thread, and a plugin's `process()` should not log either.

### Two-Struct Pattern

A Beamer plugin is two structs: the **Model**, which every thread shares, and the **Processor**, which the audio thread owns.

| Struct | Marked with | Purpose |
|--------|-------------|---------|
| `*Model` | `#[beamer::model]`, `impl Model` | The parameters, `State` fields and everything else the threads share. Describes the plugin to the host and creates the processor, for as long as the plugin instance lives |
| `*Processor` | `impl Processor` | DSP logic and state, created by `prepare()`, shares the model |

A nested group of parameters is a struct marked with `#[beamer::group]` (see [Nested Parameter Groups](#nested-parameter-groups)).

**Lifecycle:**

```
Model (in one Arc, lives as long as the instance)
   ▲                 │
   │                 ▼ prepare(self: Arc<Self>, setup)
   │                 │
   └─────shares── Processor (ready for audio)
                     │
                     ▼ dropped when the host releases the audio
                       resources or changes the setup
                 (the model creates the next one)
```

**Minimal Example:**

```rust
use beamer::prelude::*;

// 1. Model - what every thread shares, describes the plugin to the host.
#[beamer::export]
#[beamer::model]
pub struct GainModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

impl Model for GainModel {
    // A plain gain needs no setup. Use SampleRate for delays and
    // MaxBufferSize for FFT.
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
        GainProcessor { model: self }
    }
}

// 2. Processor - owned by the audio thread, shares the model.
pub struct GainProcessor {
    model: Arc<GainModel>,
}

impl Processor for GainProcessor {
    fn process(&mut self, buffer: &mut Buffer, _: &mut AuxiliaryBuffers, _: &ProcessContext) {
        let gain = self.model.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}
```

**Plugin Export:**

The `#[beamer::export]` macro on the model struct reads `Config.toml` from the crate root at compile time and generates:
- `pub static CONFIG: Config` from the TOML configuration
- Optional factory presets from `Presets.toml` (if present)
- Format-specific plugin entry points for AU and VST3

`#[beamer::export]` and `#[beamer::model]` can be written in either order.

See [Plugin Configuration](#plugin-configuration) for the Config.toml format and [Factory Presets](#factory-presets) for Presets.toml.

The following sections cover [Parameters](#parameters), the [Model trait](#model-trait) and the [Processor trait](#processor-trait).

### Parameters

Declare the parameters as fields of the model with declarative attributes. `#[beamer::model]` generates all required trait implementations automatically.

**Features:**
- Declarative parameter definitions with compile-time validation
- Automatic state serialization/deserialization
- Parameter smoothing to avoid zipper noise during automation

#### The Model Attribute

The attribute generates everything, including `Default`:

```rust
use beamer::prelude::*;

#[beamer::model]
pub struct GainModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,

    #[parameter(id = "bypass", bypass)]
    pub bypass: BoolParameter,
}

// No manual new() or Default impl needed - the attribute generates everything!
```

`#[beamer::model]` (and `#[beamer::group]` for a nested group) generates:
- `Parameters` trait implementation (count, iter, by_id, groups, smoothing), which also makes the struct a `ParameterStore` for the host
- `Saved` trait implementation (parameters and `State` fields, saving and loading)
- `WebViewEvents` trait implementation (where the event queue of the web UI is)
- `ParameterGroups` trait implementation (parameter groups)
- `Default` implementation (`State` fields and fields without an attribute get their own default)
- The ids of every parameter: its path as text and the FNV-1a hash of the path as the number hosts see (see [Parameter Ids](#parameter-ids))
- Compile-time detection of duplicate ids in a struct

The generated `Default` takes every parameter from its attribute, so each one needs what its type needs:

- `FloatParameter`: `name`, `default` and `range`. `kind = "percent"` and `kind = "pan"` default to 0.0..=1.0 and -1.0..=1.0, and a `range` replaces that.
- `IntParameter`: `name`, `default` and `range`.
- `BoolParameter`: `name` and `default`, or only `bypass`.
- `EnumParameter`: `name`. Its default is the variant marked `#[default]`, or else the first.

A field cannot have `#[cfg]`: the attribute runs before `#[cfg]` is applied to the fields, so put the whole struct behind the `#[cfg]` instead.

**Writing `Default` yourself:** a model with a plain field whose type has no `Default`, or with a parameter that is set up by hand, uses `#[beamer::model(no_default)]` (or `#[beamer::group(no_default)]`). Your `Default` then has to do what the generated one does after building the value: call `assign_ids("")` to give the parameters their ids and `set_group_ids()` to give them their groups.

```rust
#[beamer::model(no_default)]
pub struct SamplerModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = 0.0..=1.0)]
    pub gain: FloatParameter,

    #[nested(group = "Layer")]
    pub layer: LayerGroup,

    // A handle without a `Default` of its own.
    pub library: Mutex<SampleLibrary>,
}

impl Default for SamplerModel {
    fn default() -> Self {
        let mut model = Self {
            gain: FloatParameter::new("Gain", 0.0, 0.0..=1.0),
            layer: LayerGroup::default(),
            library: Mutex::new(SampleLibrary::open_default()),
        };
        model.assign_ids("");
        model.set_group_ids();
        model
    }
}
```

#### Declarative Attributes

| Attribute | Description | Required |
|-----------|-------------|----------|
| `id = "..."` | String ID (its path is hashed to u32, see [Parameter Ids](#parameter-ids)) | Yes |
| `name = "..."` | Display name in DAW | For Default (see above) |
| `default = <value>` | Default value (float, int or bool) | For Default (see above) |
| `range = start..=end` | Value range | For Default (see above) |
| `kind = "..."` | Unit type (see below) | Optional |
| `group = "..."` | Visual grouping without a nested group | Optional |
| `short_name = "..."` | Short name for constrained UIs | Optional |
| `step = <value>` | Discrete step size (FloatParameter only) | Optional |
| `smoothing = "exp:5.0"` | Parameter smoothing (`exp` or `linear`, FloatParameter only) | Optional |
| `bypass` | Mark as bypass parameter (BoolParameter only) | Optional |

**Kind Values:** `db`, `db_log`, `db_log_offset`, `hz`, `ms`, `seconds`, `percent`, `pan`, `ratio`, `linear`, `semitones`

- `db_log` - Power curve (exponent 2.0) for more resolution near 0 dB (use for thresholds)
- `db_log_offset` - True logarithmic mapping for dB ranges (geometric mean at midpoint)

Supported field types: `FloatParameter`, `IntParameter`, `BoolParameter` and `EnumParameter<E>` with `#[parameter]`, a `#[beamer::group]` struct with `#[nested]` and `State<T>` with `#[state]` (see [State Beyond Parameters](#state-beyond-parameters)). A field without an attribute is not a parameter and is not saved.

#### Parameter Types

**FloatParameter**: Continuous floating-point parameter:

```rust
// Linear range.
let freq = FloatParameter::new("Frequency", 1000.0, 20.0..=20000.0);

// Decibel range (stores dB, use as_linear() for DSP).
let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0);

// In DSP code:
// 0 dB → 1.0, -6 dB → ~0.5.
let amplitude = gain.as_linear();
// Returns dB for display.
let db_value = gain.get();
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
    // Uses "Notch" as display name.
    Notch,
}

#[beamer::model]
pub struct FilterModel {
    #[parameter(id = "filter_type", name = "Filter Type")]
    pub filter_type: EnumParameter<FilterType>,
}

// In DSP code:
match self.model.filter_type.get() {
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
| `.with_id(id)` | Set parameter ID (the macro sets it from the path) |
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
| `.with_id(id)` | Set parameter ID (the macro sets it from the path) |
| `.with_short_name(name)` | Short name for constrained UIs |
| `.with_group(group_id)` | Assign to parameter group |
| `.with_precision(n)` | Display precision (for Float formatter) |
| `.with_formatter(fmt)` | Replace formatter entirely |
| `.readonly()` | Make parameter read-only |
| `.non_automatable()` | Disable automation |

**Precision and Formatter Customization:**

```rust
// High-precision gain for mastering plugins.
let gain = FloatParameter::db("Output", 0.0, -12.0..=12.0)
    // Shows "-0.50 dB" instead of "-0.5 dB".
    .with_precision(2);

// Milliseconds with integer display.
let attack = FloatParameter::ms("Attack", 10.0, 0.1..=100.0)
    // Shows "10" instead of "10.0".
    .with_precision(0);

// Completely custom formatter.
let ratio = FloatParameter::new("Ratio", 4.0, 1.0..=20.0)
    // Shows "4.0:1".
    .with_formatter(Formatter::Ratio { precision: 1 });

// Chain multiple builder methods.
let volume = FloatParameter::db("Volume", 0.0, -60.0..=12.0)
    .with_step_size(0.5)
    .with_precision(2)
    .with_smoother(SmoothingStyle::Exponential(5.0));
```

**Note:** Formatters without precision fields (`Pan`, `Boolean`, `Semitones`, `Frequency`) ignore `.with_precision()` calls.

#### Parameter Smoothing

Avoid zipper noise during automation by adding smoothing to parameters:

```rust
// Add smoother during parameter creation.
let gain = FloatParameter::db("Gain", 0.0, -60.0..=12.0)
    // 5ms time constant.
    .with_smoother(SmoothingStyle::Exponential(5.0));
```

**Smoothing Styles:**

| Style | Behavior | Use Case |
|-------|----------|----------|
| `SmoothingStyle::None` | Instant (default) | Non-audio parameters |
| `SmoothingStyle::Linear(ms)` | Linear ramp | Predictable timing |
| `SmoothingStyle::Exponential(ms)` | One-pole IIR, can cross zero | dB gain, most musical parameters |
| `SmoothingStyle::Logarithmic(ms)` | Log-domain, positive values only | Frequencies (Hz), other positive-only parameters |

**Sample Rate:**

The framework sets the host's sample rate on the smoothers before it calls `prepare()`, so a plugin that processes at the host's rate has nothing to do.

If your plugin uses oversampling, set the rate it actually processes at in `prepare()`:

```rust
impl Model for MyModel {
    type Setup = SampleRate;
    type Processor = MyProcessor;

    fn prepare(self: Arc<Self>, setup: SampleRate) -> MyProcessor {
        self.set_sample_rate(setup.hz() * OVERSAMPLING_FACTOR as f64);
        MyProcessor {
            sample_rate: setup.hz(),
            model: self,
        }
    }
}
```

**Per-Sample Processing:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
    for (input, output) in buffer.zip_channels() {
        for (i, o) in input.iter().zip(output.iter_mut()) {
            // Advances smoother.
            let gain = self.model.gain.tick_smoothed();
            *o = *i * gain as f32;
        }
    }
}
```

**Block Processing:**

```rust
fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
    // Current value, no advance.
    let gain = self.model.gain.smoothed();
    self.model.gain.skip_smoothing(buffer.len());

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
self.model.gain.fill_smoothed_f32(&mut gain_buffer[..len]);
// Use gain_buffer[i] per sample.
```

**Smoothing API:**

| Method | Description |
|--------|-------------|
| `.with_smoother(style)` | Builder: add smoothing to parameter |
| `.set_sample_rate(sr)` | Set the processing rate (only needed when it differs from the host's) |
| `.tick_smoothed()` | Advance smoother, return value (per-sample) |
| `.smoothed()` | Get current value without advancing |
| `.skip_smoothing(n)` | Skip n samples (block processing) |
| `.fill_smoothed(buf)` | Fill buffer with smoothed values |
| `.is_smoothing()` | Check if currently ramping |
| `.reset_smoothing()` | Reset to current value (no ramp) |

**Thread Safety Note:**

Smoothing methods take `&self`, like the methods that read and write the value: a smoother keeps its state in atomics too. They are still meant for the audio thread only. Calling them from a second thread at the same time is memory-safe, but the two disturb each other's ramp.

**Automatic Reset on State Load:**

When a host loads a state, the framework calls `reset_smoothing()` on the audio thread at the start of the next block, so that the loaded values apply at once instead of ramping in.

#### Flat Parameter Grouping

Use `group = "..."` to organize parameters into logical groups without nested group structs:

```rust
#[beamer::model]
pub struct SynthesizerModel {
    #[parameter(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
    pub cutoff: FloatParameter,

    #[parameter(id = "reso", name = "Resonance", default = 0.5, range = 0.0..=1.0, group = "Filter")]
    pub resonance: FloatParameter,

    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db", group = "Output")]
    pub gain: FloatParameter,
}

// Access is flat: model.cutoff, model.resonance, model.gain.
// Groups appear in DAW automation lanes and parameter lists.
// Note: Most DAWs don't display groups in the main plugin UI.
```

**Flat vs Nested Grouping:**

| Feature | Flat (`group = "..."`) | Nested (`#[nested(...)]`) |
|---------|------------------------|---------------------------|
| Struct layout | Single struct | Separate struct per group |
| Access pattern | `model.cutoff` | `model.filter.cutoff` |
| Reusability | N/A | Same struct reusable |
| Complexity | Simple | More structure |

Choose flat grouping for simple organization; nested groups for reusable parameter collections.

#### Nested Parameter Groups

Use `#[nested]` to organize parameters into separate `#[beamer::group]` structs, which hosts show as groups (VST3 units):

```rust
#[beamer::model]
pub struct SynthesizerModel {
    #[parameter(id = "master", name = "Master", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub master: FloatParameter,

    #[nested(group = "Filter")]
    pub filter: FilterGroup,

    #[nested(group = "Amp Envelope")]
    pub amp_env: EnvelopeGroup,
}

#[beamer::group]
pub struct FilterGroup {
    #[parameter(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz")]
    pub cutoff: FloatParameter,

    #[parameter(id = "resonance", name = "Resonance", default = 0.5, range = 0.0..=1.0)]
    pub resonance: FloatParameter,
}

#[beamer::group]
pub struct EnvelopeGroup {
    #[parameter(id = "attack", name = "Attack", default = 10.0, range = 0.1..=1000.0, kind = "ms")]
    pub attack: FloatParameter,

    #[parameter(id = "release", name = "Release", default = 100.0, range = 0.1..=5000.0, kind = "ms")]
    pub release: FloatParameter,
}
```

With declarative attributes, `set_group_ids()` is called automatically in the generated `Default` implementation.

#### Parameter Ids

A parameter is known by its path: the `id` of its attribute, behind the names of the nested fields it sits in. The `/` separates the segments of the path, so an `id` cannot contain one (a compile error). The same holds for the `id` of a `State` field, which also cannot be empty.

```
"master"              - parameter of the root struct
"filter/cutoff"       - parameter in the nested field `filter`
"osc1/filter/res"     - deeply nested parameter
```

The path is used in every place that names a parameter:

| Where | What is used |
|-------|--------------|
| Host (automation, parameter list) | The FNV-1a hash of the path, a `u32`. Read it with `model.filter.cutoff.id()`. |
| Saved state | The path |
| `Presets.toml` | The path as key |
| WebView | The path as `stringId` |

The same nested group can therefore be used in several fields. Each use has its own paths and its own host ids:

```rust
#[nested(group = "Osc 1")]
pub osc1: OscGroup,

// The same struct with other paths: "osc1/attack" and "osc2/attack".
#[nested(group = "Osc 2")]
pub osc2: OscGroup,
```

Paths use the name of the field, not the display name of its group, so renaming a group changes nothing for hosts and saved states. Renaming a field or a parameter id does: a host no longer finds its automation and a saved state no longer finds its value.

The generated `Default` gives all parameters their ids with `assign_ids("")`. A hand-written `Default` (with `no_default`) calls `assign_ids("")` and `set_group_ids()` itself.

Two different paths can have the same hash, although it is unlikely. `#[beamer::model]` reports it at compile time for two ids in one struct. Across nested groups it shows when a plugin instance is created: the plugin does not load. The error names both paths. It is written to standard error and passed to the `log` crate, which a host does not always show, so the dependable way to see it is a test of the plugin that calls the same check:

```rust
#[test]
fn test_parameter_ids_are_unique() {
    let model = SynthesizerModel::default();
    beamer::core::parameter_types::check_unique_ids(&model).unwrap();
}
```

Give one of the two parameters another id.

The VST3 wrapper also refuses a parameter whose host id is one it uses for its own parameters: the MIDI CC parameters (`0x10000000` to `0x10000081`) and the program change parameter (`0x20000000`). The error names the parameter.

#### State Serialization Format

The state of a plugin is the state of its model. It is saved as a list of entries, one per field, each under a path. All numbers are little-endian:

```
Header:  42 4D 52 53 | version: u32 = 1 | payload length: u32
Entry:   kind: u8 | path length: u16 | path: utf8 | value length: u32 | value

kind 1   a parameter: the normalized value as f64
kind 2   a State field: its value as JSON

Path examples:
- "gain"              - top-level parameter or state, by its id
- "filter/cutoff"     - parameter in the nested field `filter`
- "osc1/filter/res"   - deeply nested parameter
```

The paths are the ones described under [Parameter Ids](#parameter-ids). A `State` field has a path of the same form, built from its `#[state(id = "...")]`.

Loading follows these rules:

- Empty data changes nothing. Data with another header or version, or data that is cut off, is refused as a whole and changes nothing.
- Every field takes the value of its entry. A field without a usable entry goes back to its default (a `State` to the value it was created with), so the result of a load depends on the saved data alone.
- An entry that no field asks for is skipped. Parameter values are clamped to 0.0-1.0.

The format is defined in one place, `beamer::core::state_format`.

#### State Beyond Parameters

Not everything a plugin has to remember is a number a host can automate: the path of a loaded sample, a table the user drew, a setting of the editor. Such values go into `State` fields of the model:

```rust
#[beamer::model]
pub struct SamplerModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,

    #[state(id = "sample_path")]
    pub sample_path: State<String>,
}
```

A `State<T>` is saved and loaded with the parameters, under its id. Ids of states and parameters share one namespace, and like a parameter id a state id has to stay the same from one version of a plugin to the next.

The value is set as a whole by host and UI threads and read by the audio thread through a reader:

```rust
// On a host or UI thread, for example in on_invoke().
model.sample_path.set(String::from("kick.wav"));
let current: Arc<String> = model.sample_path.get();

// In prepare(): the audio thread's end, taken before `self` moves into the
// processor.
fn prepare(self: Arc<Self>, _: SampleRate) -> SamplerProcessor {
    SamplerProcessor {
        sample_path: self.sample_path.reader(),
        model: self,
        // ...
    }
}

// In process(): takes no lock, does not allocate and does not free.
if let Some(path) = self.sample_path.changed() {
    // The value was set since the last call, by the plugin or by a loaded state.
}
let path = self.sample_path.current();
```

| Method | Thread | Description |
|--------|--------|-------------|
| `state.set(value)` | Host, UI | Replace the value |
| `state.get()` | Host, UI | The current value as an `Arc<T>` |
| `state.generation()` | Any | How often the value was set |
| `state.reader()` | Host (in `prepare()`) | Create the audio thread's reader |
| `reader.changed()` | Audio | A value that was set since the last call, once |
| `reader.latest()` | Audio | Take over a new value if there is one and return the current |
| `reader.current()` | Audio | The value the reader took over last |

A web UI reaches every `State` field by its path without any code in the plugin: `__BEAMER__.state` reads and sets it and tells the UI when it changes, also behind its back (see [State Methods](WEBVIEW.md#state-methods)).

**What `T` can be:** any type that derives serde's `Serialize` and `Deserialize`. The value is saved as JSON. A plugin without a serde dependency of its own derives through the re-export:

```rust
use beamer::serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
#[serde(crate = "beamer::serde")]
pub struct Sample {
    pub path: String,
    pub root_note: u8,
}
```

JSON cannot hold floats that are not finite (NaN, infinity): a value that contains one does not come back from a saved state. A `HashMap` is saved in a different order each time, so use a `BTreeMap` where the saved bytes should be stable.

**Defaults:** the generated `Default` creates a state with `T::default()`. That is also the value it goes back to when a saved state has no entry for it. A model whose `Default` is written by hand (with `no_default`) creates its states itself, with `State::new(value)`: that value is then the default. It also initializes its fields without an attribute.

**Data that is not saved:** A `Handover<T>` is the same hand-over without the saving, for data the plugin derives from its state, such as the decoded audio of a sample. A field without an attribute is not part of the parameters or the saved state, so a `Handover` can sit next to the parameters:

```rust
#[beamer::model]
pub struct SamplerModel {
    // ...
    #[state(id = "sample_path")]
    pub sample_path: State<String>,

    // Set by a background thread that decoded the file.
    pub decoded: Handover<Vec<f32>>,
}
```

**Threads:** `set()` may wait for another thread that sets the same value and may free memory, so it is not for the audio thread. The audio thread only reads. In debug builds, a call of `get()`, `set()`, `reader()`, `reset()`, `save()` or `load()` from `process()` panics and names the call; release builds do not check. A value the reader no longer uses is released by a later `set()` or when the reader goes away with the Processor, on a host thread. The `webview-demo` example keeps its pan law in a `State` and shows it in its web UI.

**Telling the host:** A host learns that the user changed a parameter through its edits. A change of a `State` field is not an edit, so while the editor is open the VST3 wrapper checks for one at each sync tick (60 times a second) and tells the host that the plugin state changed (`IComponentHandler2::setDirty`), whichever thread set the field. The host can then offer to save the project. A state the host loads does not count as a change.

Audio Units have no such call, so their wrappers do not tell the host. The nearest thing, a change notification for `kAudioUnitProperty_ClassInfo` in an AUv2 or for the `fullState` property in an AUv3, does not make Reaper mark the project as modified. In practice both hosts that were tried still ask: Logic asks to save on closing even when nothing changed, and Reaper marks the project as modified whenever a plugin window closes. The fields are always saved with the plugin state when the user saves the project. A plugin that needs every Audio Unit host to ask keeps such a value in a parameter instead.

#### Low-Level Traits

For manual control, implement `Parameters` (the parameters: iteration, ids, groups, smoothing), `ParameterGroups` (the group tree; an empty impl is enough without groups), `Saved`, `WebViewEvents` and `Default` directly. An empty `impl Saved` saves the parameters under their numeric ids. If the struct has `State` fields, override in `Saved`: `save_entries()` and `load_entries()` to save and load them as well (`writer.state(path, &field)` and `entries.load_state(path, &field)`), `state_generation()` to return the sum of their `generation()`s and `for_each_state()` to visit each with its path. A web UI reaches them through that method. An empty `impl WebViewEvents` is for a struct without a queue; a struct with a `WebViewEventQueue` returns it from `webview_events()`.

Every `Parameters` is also a `ParameterStore`: the parameters as the host sees them, looked up through `Parameters`. A store that keeps values without parameter objects implements `ParameterStore` directly. For a `Parameters`, `info(index)` walks the parameters and allocates. It is for listing them on host and editor threads, never per frame or on the audio thread. `info_by_id()` looks one up through `by_id()` and does not allocate. The trait and the metadata it returns:

```rust
pub trait ParameterStore: Send + Sync {
    fn count(&self) -> usize;
    fn info(&self, index: usize) -> Option<&ParameterInfo>;
    fn get_normalized(&self, id: ParameterId) -> ParameterValue;
    fn set_normalized(&self, id: ParameterId, value: ParameterValue);
    fn normalized_to_string(&self, id: ParameterId, normalized: ParameterValue) -> String;
    fn string_to_normalized(&self, id: ParameterId, string: &str) -> Option<ParameterValue>;
    fn normalized_to_plain(&self, id: ParameterId, normalized: ParameterValue) -> ParameterValue;
    fn plain_to_normalized(&self, id: ParameterId, plain: ParameterValue) -> ParameterValue;

    // Defaults provided.
    fn formatter_kind(&self, id: ParameterId) -> &'static str;
    fn info_by_id(&self, id: ParameterId) -> Option<&ParameterInfo>;
}

pub struct ParameterInfo {
    pub id: ParameterId,
    // The path of the parameter (see Parameter Ids).
    pub string_id: Cow<'static, str>,
    pub name: &'static str,
    pub short_name: &'static str,
    pub units: &'static str,
    // Unit type hint for AU hosts (checkbox, dropdown and so on).
    pub unit: ParameterUnit,
    pub default_normalized: ParameterValue,
    pub step_count: i32,
    pub flags: ParameterFlags,
    // Parameter group (0 = root).
    pub group_id: GroupId,
}

pub struct ParameterFlags {
    pub can_automate: bool,
    pub is_readonly: bool,
    // Maps to VST3 kIsBypass (see Soft Bypass).
    pub is_bypass: bool,
    // Display as dropdown list (for enums).
    pub is_list: bool,
    // Hide from DAW parameter list (used by MIDI CC emulation).
    pub is_hidden: bool,
}

impl ParameterInfo {
    /// Convenience constructor for bypass parameters.
    pub const fn bypass(id: ParameterId) -> Self;
}
```

### Model Trait

The `Model` trait is implemented by the model struct: what every thread shares, and the plugin as the host sees it (buses and MIDI behavior). The framework creates the model with the plugin instance and keeps it in one `Arc` for as long as the instance lives. When the host provides audio setup information, the framework calls `prepare()` with a clone of that `Arc` to create a `Processor`.

```rust
pub trait Model: Saved + WebViewEvents + Default + 'static {
    /// Setup type for prepare() - determines what info is needed
    type Setup: PluginSetup;

    /// The prepared processor type
    type Processor: Processor;

    /// Create a processor for the audio setup. Runs on a host thread and may
    /// allocate. The smoothers already have the host's sample rate. Called
    /// again when the setup changes: the previous processor is dropped first.
    fn prepare(self: Arc<Self>, setup: Self::Setup) -> Self::Processor;

    // Bus configuration (defaults provided). A plugin without audio input,
    // such as most instruments and generators, returns 0 from input_bus_count.
    // An Audio Unit always has an output: one without any bus gets a stereo
    // output in the AU formats. The answers must stay the same for the life
    // of an instance.
    fn input_bus_count(&self) -> usize { 1 }
    fn output_bus_count(&self) -> usize { 1 }
    fn input_bus_info(&self, index: usize) -> Option<BusInfo>;
    fn output_bus_info(&self, index: usize) -> Option<BusInfo>;

    /// Whether this plugin processes MIDI events.
    fn wants_midi(&self) -> bool { false }

    /// MIDI CC configuration for CC emulation (see MIDI CC Emulation).
    fn midi_cc_config(&self) -> Option<MidiCcConfig> { None }

    // MIDI mapping and learn, note expression, keyswitch and MPE methods,
    // all with defaults (see MIDI Reference).

    /// Answers from the web UI (see WEBVIEW.md).
    fn on_invoke(&self, method: &str, args: &[serde_json::Value]) -> Result<serde_json::Value, String> {
        Err(format!("unknown method: {method}"))
    }
    fn on_event(&self, name: &str, data: &serde_json::Value) {}
}
```

`#[beamer::model]` implements the supertraits (all but `Default` with `no_default`); the plugin writes `impl Model` (see [Two-Struct Pattern](#two-struct-pattern) for a complete example).

`prepare()` gets the model's `Arc`. The processor keeps it, or only the readers and values it needs. Take readers before `self` moves into the processor: `Processor { pan_law: self.pan_law.reader(), model: self }`, in that order; the other order does not compile. The model keeps answering the host while audio runs. Its methods are called from host threads, never from the audio thread, and several host threads may use it at the same time: a model is `Sync`. Every method other than `prepare()` takes `&self`, also the ones that change something, such as `on_midi_learn()`: they keep what they change in a `State` field of the model, an atomic or a `Mutex`.

**Plugins with DSP state** add fields to the Processor:

```rust
pub struct DelayProcessor {
    model: Arc<DelayModel>,
    // Always valid.
    sample_rate: f64,
    // Always allocated.
    buffer: Vec<f64>,
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

Compose up to five types using a tuple:

```rust
// Simple plugin - no setup needed.
impl Model for GainModel {
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
        GainProcessor { model: self }
    }
}

// Plugin needing sample rate (delays, filters, envelopes).
impl Model for DelayModel {
    type Setup = SampleRate;
    type Processor = DelayProcessor;
    fn prepare(self: Arc<Self>, sample_rate: SampleRate) -> DelayProcessor {
        DelayProcessor {
            sample_rate: sample_rate.hz(),
            /* DSP state... */
            model: self,
        }
    }
}

// Plugin needing multiple setup values.
impl Model for FftModel {
    type Setup = (SampleRate, MaxBufferSize);
    type Processor = FftProcessor;
    fn prepare(self: Arc<Self>, (sr, mbs): (SampleRate, MaxBufferSize)) -> FftProcessor {
        FftProcessor {
            sample_rate: sr.hz(),
            fft_buffer: vec![0.0; mbs.0],
            model: self,
        }
    }
}
```

### Processor Trait

The `Processor` trait is what the audio thread owns: ready for real-time audio processing. It is created by `Model::prepare()` and dropped when the host releases the audio resources or changes the setup. What has to outlive a Processor, or be seen by other threads, belongs in the model.

```rust
pub trait Processor: Send + 'static {
    /// Process audio. Called on the audio thread.
    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &ProcessContext,
    );

    /// Process MIDI events. Called right before every process() call,
    /// with the events of that call, timed from its first sample.
    fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
        // Default: pass through.
        for event in input {
            output.push(event.clone());
        }
    }

    /// Tail length in samples (for reverbs, delays).
    /// Called on the audio thread for every block.
    fn tail_samples(&self) -> u32 { 0 }

    /// Latency in samples (for lookahead).
    /// Called on the audio thread for every block.
    fn latency_samples(&self) -> u32 { 0 }

    /// Called when plugin is activated/deactivated.
    /// Reset DSP state when active == true. Must be realtime-safe.
    fn set_active(&mut self, active: bool) { }

    /// Bypass crossfade duration in samples.
    /// Called on the audio thread for every block.
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
        // Default: converts to f32, calls process() and converts back.
        // The format wrappers do not use it for a plugin without f64
        // support: they convert themselves, with buffers they allocated
        // in advance, and call process().
    }
}
```

**State:** The state a host saves and restores is the state of the model: its parameters and its `State` fields (see [State Beyond Parameters](#state-beyond-parameters)). `#[beamer::model]` generates saving and loading, and the wrappers call it directly: a `Processor` has no state methods.

**When to implement `set_active()`:** Plugins with internal DSP state (delay lines, filter histories, envelopes, oscillator phases) should override `set_active()` and reset that state when `active == true`. Hosts call `setActive(false)` followed by `setActive(true)` to request a full state reset. Plugins without internal state (simple gain, pan) can use the default empty implementation.

**What runs where:** While audio runs, the processor belongs to the audio thread. A host saves and loads the state, asks for latency and tail and requests a reset from its own threads at the same time, and none of that touches the processor:

- The state is read from and written to the model, whose parameter values are atomic and whose `State` fields are shared.
- `latency_samples()`, `tail_samples()` and `bypass_ramp_samples()` are called on the audio thread for every block. The host gets the values of the most recent block, so they must be realtime-safe.
- An Audio Unit host's reset runs as `set_active(false)` then `set_active(true)` on the audio thread, at the start of the next block. `set_active()` must therefore be realtime-safe: no allocation, no locks.

#### Plugin Lifecycle

The model lives as long as the plugin instance. A Processor exists while the host has audio set up:

```
    ┌───────────────────────────────────────────────┐
    │  Model (created with the instance)            │
    │  held by the framework in one Arc             │
    └───────┬───────────────────────────────▲───────┘
            │ audio setup arrives           │
            │ prepare(clone of the Arc)     │ still there
            ▼                               │
    ┌─────────────────┐                     │
    │  Processor      │◄── process()        │
    │  (prepared)     │    calls            │
    └───────┬─────────┘                     │
            │ setup changes or audio        │
            │ resources are released        │
            ▼                               │
         dropped ───────────────────────────┘
         (the next prepare() creates a new one)
```

A `Processor` can only come from `prepare()`, so it is always fully initialized and `process()` never needs `Option<T>` unwrapping or placeholder checks. A Processor is dropped on a host thread, never on the audio thread. See [ARCHITECTURE.md](../ARCHITECTURE.md#design-rationale) for detailed rationale.

### Factory Presets

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
- Other fields are parameter IDs with their plain values as numbers (e.g., `-6.0` for dB). A bool is `0` or `1`, an enum the index of its variant. Other values, such as `true` or a string, are ignored
- Parameter IDs match the `id` attribute from `#[parameter(id = "gain", ...)]`
- A parameter in a nested group is named by its path, as a quoted key: `"filter/cutoff" = 800.0` (see [Parameter Ids](#parameter-ids))

#### Sparse Presets

Presets can specify only a subset of parameters. Unspecified parameters retain their current values:

```toml
# Full preset: sets all parameters
[[preset]]
name = "Initialize"
gain = 0.0
mix = 1.0
bypass = 0

# Sparse preset: only changes gain, mix and bypass unchanged
[[preset]]
name = "Quiet"
gain = -12.0
```

#### FactoryPresets Trait

The `#[beamer::export]` macro generates an implementation of the `FactoryPresets` trait from `Presets.toml`, so a plugin writes no preset code:

```rust
pub trait FactoryPresets: Send + Sync + 'static {
    /// The type whose parameters the presets set: the plugin's model.
    type Parameters: Parameters;

    /// Number of available presets.
    fn count() -> usize;

    /// Get preset metadata by index.
    fn info(index: usize) -> Option<PresetInfo>;

    /// Get parameter values for a preset.
    fn values(index: usize) -> &'static [PresetValue];

    /// Apply a preset to parameters. Returns true if successful. The default
    /// sets the parameters from values().
    fn apply(index: usize, parameters: &Self::Parameters) -> bool { ... }
}
```

`#[beamer::export]` always uses the generated implementation, or none when there is no `Presets.toml`. It cannot select a hand-written one.

#### MIDI Program Change Mapping

When a plugin has factory presets, the AU wrappers map MIDI Program Change (PC) events to presets:

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

This mirrors VST3's `kIsProgramChange` behavior where the host handles PC→preset mapping automatically. In VST3 a MIDI program change reaches `process_midi()` unchanged. A VST3 host sends the program change as a change of the wrapper's program change parameter, from the edit controller or with a process call. Either way the preset is applied. In a process call it is applied before the changes of single parameters in the same block, and the wrapper then reports the values of the preset's parameters back to the host. No plugin code changes are required - the framework handles this based on whether factory presets are defined.

### Buffer Types

Beamer provides safe, ergonomic access to audio buffers using a two-buffer architecture. The main `Buffer` handles your primary input/output channels, while `AuxiliaryBuffers` provides access to sidechains and multi-bus routing.

**Design Goals:**
- Stack-allocated for real-time safety (no heap allocations in `process()`)
- Clear separation between input (read-only) and output (mutable) channels
- Support for mono, stereo and surround processing
- Generic over sample type (`f32` or `f64`)

#### Main Buffer

```rust
/// Main audio buffer (main bus only).
/// Generic over sample type S (f32 or f64).
pub struct Buffer<'a, S: Sample = f32> {
    inputs: [Option<&'a [S]>; MAX_CHANNELS],
    outputs: [Option<&'a mut [S]>; MAX_CHANNELS],
    num_input_channels: usize,
    num_output_channels: usize,
    num_samples: usize,
}

impl<'a, S: Sample> Buffer<'a, S> {
    pub fn num_samples(&self) -> usize;
    pub fn num_input_channels(&self) -> usize;
    pub fn num_output_channels(&self) -> usize;
    pub fn is_stereo(&self) -> bool;
    pub fn is_mono(&self) -> bool;
    pub fn input(&self, channel: usize) -> &[S];
    pub fn output(&mut self, channel: usize) -> &mut [S];
    pub fn output_checked(&mut self, channel: usize) -> Option<&mut [S]>;
    pub fn inputs(&self) -> impl Iterator<Item = &[S]>;
    pub fn outputs_mut(&mut self) -> impl Iterator<Item = &mut [S]>;
    pub fn zip_channels(&mut self) -> impl Iterator<Item = (&[S], &mut [S])>;
    pub fn copy_to_output(&mut self);
    pub fn clear_outputs(&mut self);
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

    pub fn num_input_buses(&self) -> usize;
    pub fn num_output_buses(&self) -> usize;
    pub fn iter_inputs(&self) -> impl Iterator<Item = AuxInput<'_, S>>;
    pub fn iter_outputs(&mut self) -> impl Iterator<Item = AuxOutput<'_, 'a, S>>;
}

/// Immutable view of an auxiliary input bus.
pub struct AuxInput<'a, S: Sample = f32> { /* ... */ }

impl<'a, S: Sample> AuxInput<'a, S> {
    pub fn num_channels(&self) -> usize;
    pub fn input(&self, index: usize) -> &[S];
    pub fn iter_inputs(&self) -> impl Iterator<Item = &[S]>;
    pub fn sample(&self, channel: usize, index: usize) -> S;
    pub fn rms(&self, channel: usize) -> S;
    pub fn peak(&self, channel: usize) -> S;
    pub fn average(&self, channel: usize) -> S;
}

/// Mutable view of an auxiliary output bus.
/// Two lifetimes resolve variance issues with nested mutable references.
pub struct AuxOutput<'borrow, 'data, S: Sample = f32> { /* ... */ }

impl<'borrow, 'data, S: Sample> AuxOutput<'borrow, 'data, S> {
    pub fn num_channels(&self) -> usize;
    pub fn output(&mut self, index: usize) -> &mut [S];
    pub fn output_checked(&mut self, index: usize) -> Option<&mut [S]>;
    pub fn iter_outputs(&mut self) -> impl Iterator<Item = &mut [S]>;
    pub fn clear(&mut self);
    pub fn fill(&mut self, value: S);
}
```

**Why Two Lifetimes for AuxOutput?**

The type `&'a mut [&'a mut T]` is **invariant** because mutable references don't allow lifetime shortening. The solution uses `'borrow` for the outer reference and `'data` for the inner data, allowing the borrow to be shorter while preserving safety.

### ProcessContext and Transport

The `ProcessContext` provides essential timing and transport information for each audio processing call. This includes sample rate, buffer size and detailed DAW transport state for tempo-synced effects, sequencers and time-based processing.

**What you can do:**
- Sync delays/LFOs to host tempo
- Implement bar/beat-synced effects
- Display timecode in your UI
- Detect loop regions for seamless looping
- Handle SMPTE for post-production

```rust
#[derive(Clone, Debug)]
pub struct ProcessContext<'a> {
    pub sample_rate: f64,
    pub num_samples: usize,
    pub transport: Transport,
    // The MIDI CC state, when the plugin asks for it (see MIDI CC Emulation).
    midi_cc_state: Option<&'a MidiCcState>,
}

impl<'a> ProcessContext<'a> {
    pub fn midi_cc(&self) -> Option<&MidiCcState>;
    pub fn samples_per_beat(&self) -> Option<f64>;
    pub fn buffer_duration(&self) -> f64;
}

#[derive(Copy, Clone, Debug, Default)]
pub struct Transport {
    // Tempo and time signature.
    pub tempo: Option<f64>,
    pub time_sig_numerator: Option<i32>,
    pub time_sig_denominator: Option<i32>,

    // Position.
    pub project_time_samples: Option<i64>,
    pub project_time_beats: Option<f64>,
    pub bar_position_beats: Option<f64>,

    // Loop/Cycle.
    pub cycle_start_beats: Option<f64>,
    pub cycle_end_beats: Option<f64>,

    // Transport state (always valid).
    pub is_playing: bool,
    pub is_recording: bool,
    pub is_cycle_active: bool,

    // Advanced timing.
    pub system_time_ns: Option<i64>,
    pub continuous_time_samples: Option<i64>,
    pub samples_to_next_clock: Option<i32>,

    // SMPTE/Timecode.
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

### Sample Trait (f32/f64)

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
// Model.
#[beamer::model]
pub struct GainModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

impl Model for GainModel {
    // A plain gain needs no setup. Use SampleRate for delays and
    // MaxBufferSize for FFT.
    type Setup = ();
    type Processor = GainProcessor;
    fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
        GainProcessor { model: self }
    }
}

// Processor with generic processing method.
pub struct GainProcessor {
    model: Arc<GainModel>,
}

impl GainProcessor {
    // Generic processing - works for both f32 and f64.
    fn process_generic<S: Sample>(
        &mut self,
        buffer: &mut Buffer<S>,
        _aux: &mut AuxiliaryBuffers<S>,
        _context: &ProcessContext,
    ) {
        let gain = S::from_f64(self.model.gain.as_linear());
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

impl Processor for GainProcessor {
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
- Reverbs, delays or effects with long feedback paths (reduces error accumulation)
- Precision EQs or filters
- Scientific/mastering tools
- Any plugin where rounding errors matter over long processing chains

**When f32 is fine:**
- Simple gain/pan/saturation
- Most dynamics processors
- Synthesizers (often limited by oscillator precision anyway)

### Soft Bypass

```rust
pub enum BypassState {
    Active,
    RampingToBypassed,
    Bypassed,
    RampingToActive,
}

/// What action the plugin should take for this buffer.
pub enum BypassAction {
    /// Fully bypassed: copy input to output.
    Passthrough,
    /// Fully active: run DSP normally.
    Process,
    /// Transitioning: run DSP, then call finish().
    ProcessAndCrossfade,
}

pub enum CrossfadeCurve {
    /// Slight loudness dip at center.
    Linear,
    /// Constant loudness (recommended).
    EqualPower,
    /// Faster start and end, smoother middle.
    SCurve,
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
#[beamer::model]
pub struct ReverbModel {
    // The bypass flag tells hosts this is the plugin's bypass parameter.
    #[parameter(id = "bypass", bypass)]
    pub bypass: BoolParameter,
    // ... other parameters
}

// The processor holds the model and the DSP state.
struct ReverbProcessor {
    model: Arc<ReverbModel>,
    bypass_handler: BypassHandler,
    // ... other reverb state
}

impl Processor for ReverbProcessor {
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        let is_bypassed = self.model.bypass.get();

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

    // Tells the host how long the crossfade takes.
    fn bypass_ramp_samples(&self) -> u32 {
        self.bypass_handler.ramp_samples()
    }
}
```

**Why Split API?** The split pattern (begin/finish) avoids Rust borrow checker conflicts that occur with closure-based APIs when your DSP code needs to access `&mut self`.

---

> **See Also:** For format-specific details, see [Format Notes](#format-notes). Bundle layouts are described in [BUILDING.md](BUILDING.md).

---

## MIDI Reference

The event types, `MidiBuffer` and `MidiCcConfig` are in the prelude. The helpers for note expression, keyswitches, physical UI mapping, MPE, RPN/NRPN and CCs (`NoteExpressionTypeInfo`, `KeyswitchInfo`, `PhysicalUIMap`, `MpeInputDeviceSettings`, `RpnTracker`, the `cc` and `note_expression` modules and the 14-bit functions) are not: import them from `beamer::core`.

### Event Types

The structs of the single messages (`NoteOn`, `ControlChange` and so on) are `Copy`. `MidiEvent` and `MidiEventKind` are `Clone` only, because a SysEx message is boxed to keep the enum small. Cloning an event copies it without allocating, except for a SysEx event, whose clone allocates.

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct MidiEvent {
    pub sample_offset: u32,
    pub event: MidiEventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MidiEventKind {
    // Note events.
    NoteOn(NoteOn),
    NoteOff(NoteOff),
    PolyPressure(PolyPressure),

    // Channel events.
    ControlChange(ControlChange),
    PitchBend(PitchBend),
    ChannelPressure(ChannelPressure),
    ProgramChange(ProgramChange),

    // Advanced VST3 events.
    SysEx(Box<SysEx>),
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
    // 0-15.
    pub channel: MidiChannel,
    // 0-127.
    pub pitch: MidiNote,
    // 0.0-1.0.
    pub velocity: f32,
    // For tracking.
    pub note_id: NoteId,
    // Cents (±120.0) for MPE/microtonal.
    pub tuning: f32,
    // Samples (0 = unknown).
    pub length: i32,
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
    // 0-127.
    pub controller: u8,
    // 0.0-1.0.
    pub value: f32,
}

pub struct PitchBend {
    pub channel: MidiChannel,
    // -1.0 to 1.0.
    pub value: f32,
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
// Semantic constructors (used by VST3).
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

// Raw MIDI 1.0 byte parsing (used by AU).
MidiEvent::from_midi1_bytes(offset, status, channel, data1, data2) -> Option<MidiEvent>
```

**Construction Paths:**
- **Semantic constructors** - Used by VST3 which provides already-parsed event structures
- **Raw byte parsing** (`from_midi1_bytes()`) - Used by AU which provides raw MIDI 1.0 bytes; handles velocity normalization, pitch bend 14-bit decoding and Note On velocity 0 → Note Off conversion

Both formats deliver a note-on with velocity 0 as a `NoteOff`, so a plugin never receives a `NoteOn` with velocity 0.

### MidiBuffer

```rust
pub struct MidiBuffer { /* Fixed capacity: 1024 events */ }

impl MidiBuffer {
    pub fn new() -> Self;
    // Returns false when the buffer is full and the event was dropped.
    pub fn push(&mut self, event: MidiEvent) -> bool;
    pub fn iter(&self) -> impl Iterator<Item = &MidiEvent>;
    pub fn len(&self) -> usize;
    pub fn clear(&mut self);
    pub fn has_overflowed(&self) -> bool;
}
```

### Event Modification

Use `event.clone().with()` to create modified events while preserving the sample offset. Combine with Rust's struct update syntax (`..`) to copy unchanged fields:

```rust
fn process_midi(&mut self, input: &[MidiEvent], output: &mut MidiBuffer) {
    for event in input {
        match &event.event {
            MidiEventKind::NoteOn(note) => {
                output.push(event.clone().with(MidiEventKind::NoteOn(NoteOn {
                    pitch: note.pitch.saturating_add(2).min(127),
                    // Copies channel, velocity, note_id, tuning and length.
                    ..*note
                })));
            }
            MidiEventKind::NoteOff(note) => {
                output.push(event.clone().with(MidiEventKind::NoteOff(NoteOff {
                    pitch: note.pitch.saturating_add(2).min(127),
                    // Copies channel, velocity, note_id and tuning.
                    ..*note
                })));
            }
            _ => {
                output.push(event.clone());
            }
        }
    }
}
```

The plugin says that it processes MIDI on its model:

```rust
impl Model for MyModel {
    // ...
    fn wants_midi(&self) -> bool { true }
}
```

### SysEx Handling

A `SysEx` holds the payload of a message, without the F0 and F7 that frame
it on the wire. The wrappers add them to outgoing messages in AU and VST3,
and the VST3 wrapper takes them off incoming ones. The AU formats do not pass
SysEx input to the plugin yet.

**Buffer Size (Cargo features of `beamer-core`, which a plugin adds as a dependency to enable one):**

| Feature | Size |
|---------|------|
| (default) | 512 bytes |
| `sysex-256` | 256 bytes |
| `sysex-1024` | 1024 bytes |
| `sysex-2048` | 2048 bytes |

**Plugin-Declared Capacity:**

SysEx configuration can be set via Config.toml:

```toml
sysex_slots = 64
sysex_buffer_size = 4096
```

Defaults (16 slots, 512 bytes) work for most plugins. Increase only if:
- Your plugin sends many SysEx messages per buffer (increase slots)
- Your plugin sends large SysEx messages like sample dumps (increase buffer size, together with the buffer size feature above: a message holds at most that many bytes)

These settings apply to VST3, which needs a pool to keep the data alive after
processing. The AU formats hand each message to the host during the render
call, limited only by the buffer size of the Cargo feature above.

**Heap Fallback (optional feature: `sysex-heap-fallback`):**
Overflow messages stored in heap, emitted next block. Breaks real-time guarantee.

### Note Expression (MPE)

```rust
pub mod note_expression {
    pub const VOLUME: u32 = 0;
    pub const PAN: u32 = 1;
    // MPE pitch.
    pub const TUNING: u32 = 2;
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
impl Model for MyMpeSynthesizerModel {
    // ...

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
    const MAPPINGS: &[PhysicalUIMap] = &[
        PhysicalUIMap::y_axis(note_expression::BRIGHTNESS),
        PhysicalUIMap::pressure(note_expression::EXPRESSION),
    ];
    MAPPINGS
}
```

**MPE Zone Configuration:**

```rust
fn enable_mpe_input_processing(&self, enabled: bool) -> bool { true }
fn set_mpe_input_device_settings(&self, settings: MpeInputDeviceSettings) -> bool { true }

// Presets.
// Master channel 0, member channels 1 to 14.
MpeInputDeviceSettings::lower_zone()
// Master channel 15, member channels 14 to 1.
MpeInputDeviceSettings::upper_zone()
```

### MIDI CC Emulation (MidiCcConfig)

VST3 doesn't send MIDI CC, pitch bend or aftertouch directly to plugins. Most DAWs convert these to parameter changes via the `IMidiMapping` interface. `MidiCcConfig` tells the framework which controllers you want - it handles all the state management automatically. The AU wrappers fill the same state from incoming MIDI, so `context.midi_cc()` works in every format:

```rust
use beamer::prelude::*;

// 1. Model.
#[beamer::model]
struct SynthesizerModel {
    #[parameter(id = "volume", name = "Volume", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub volume: FloatParameter,
}

impl Model for SynthesizerModel {
    type Setup = SampleRate;
    type Processor = SynthesizerProcessor;

    fn prepare(self: Arc<Self>, sample_rate: SampleRate) -> SynthesizerProcessor {
        SynthesizerProcessor {
            sample_rate: sample_rate.hz(),
            model: self,
        }
    }

    // Return configuration - framework handles state.
    fn midi_cc_config(&self) -> Option<MidiCcConfig> {
        // Use a preset for common configurations.
        Some(MidiCcConfig::SYNTH_BASIC)

        // Or build a custom configuration:
        // Some(MidiCcConfig::new()
        //     .with_pitch_bend()
        //     .with_mod_wheel()
        //     .with_ccs(&[7, 10, 11, 64]))
    }
}

// 2. Processor.
struct SynthesizerProcessor {
    model: Arc<SynthesizerModel>,
    sample_rate: f64,
}

impl Processor for SynthesizerProcessor {
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
        // Access CC values directly via ProcessContext.
        if let Some(cc) = context.midi_cc() {
            // -1.0 to 1.0.
            let pitch_bend = cc.pitch_bend();
            // 0.0 to 1.0.
            let mod_wheel = cc.mod_wheel();
            // 0.0 to 1.0.
            let volume = cc.cc(7);
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
        // -1.0 to 1.0.
        let pitch = cc.pitch_bend();
        // 0.0 to 1.0.
        let mod_whl = cc.mod_wheel();
        // 0.0 to 1.0.
        let volume = cc.cc(7);
    }
}
```

### Manual MIDI Mapping

For custom CC-to-parameter mapping (instead of receiving as MIDI events):

**IMidiMapping**: CC to parameter:

```rust
fn midi_cc_to_parameter(&self, _bus: i32, _channel: i16, cc: u8) -> Option<u32> {
    match cc {
        cc::MOD_WHEEL => Some(self.vibrato.id()),
        cc::EXPRESSION => Some(self.volume.id()),
        _ => None,
    }
}
```

**IMidiLearn:**

```rust
// Host threads share the model, so what MIDI learn changes sits
// behind a Mutex (or in a `State` field, which is also saved).
fn on_midi_learn(&self, _bus: i32, _channel: i16, cc: u8) -> bool {
    let mut learn = self.learn.lock().unwrap();
    if let Some(parameter_id) = learn.parameter.take() {
        learn.map.insert(cc, parameter_id);
        true
    } else {
        false
    }
}
```

**MIDI 2.0:** `midi1_assignments()`, `midi2_assignments()`, `on_midi1_learn()` and `on_midi2_learn()`

### Keyswitch Controller

```rust
fn keyswitch_count(&self, _bus: i32, _channel: i16) -> usize { 2 }

fn keyswitch_info(&self, _bus: i32, _channel: i16, index: usize) -> Option<KeyswitchInfo> {
    match index {
        0 => Some(KeyswitchInfo::new(keyswitch_type::NOTE_ON_KEY, "Sustain", 24)),
        1 => Some(KeyswitchInfo::new(keyswitch_type::NOTE_ON_KEY, "Staccato", 25)),
        _ => None,
    }
}
```

### RPN/NRPN Helpers

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
struct MyProcessor {
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
    // Rpn or Nrpn.
    pub kind: ParameterNumberKind,
    pub parameter: u16,
    pub value: f32,
    pub is_increment: bool,
    pub is_decrement: bool,
}
```

### CC Utilities

**Constants:**

```rust
pub mod cc {
    pub const BANK_SELECT_MSB: u8 = 0;
    pub const MOD_WHEEL: u8 = 1;
    pub const VOLUME: u8 = 7;
    pub const PAN: u8 = 10;
    pub const EXPRESSION: u8 = 11;
    pub const BANK_SELECT_LSB: u8 = 32;
    pub const SUSTAIN_PEDAL: u8 = 64;
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
// 0.0 to 1.0.
let combined = combine_14bit_cc(msb_value, lsb_value);
let (msb, lsb) = split_14bit_cc(combined);

// 0 to 16383.
let combined = combine_14bit_raw(msb, lsb);
let (msb, lsb) = split_14bit_raw(combined);
```

### VST3 Event Mapping

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

## Format Notes

Both formats share the same core traits (`Model`, `Processor`, `Parameters`), so one codebase targets both. The `#[beamer::export]` macro generates the entry points for every format enabled through Cargo features (`au`, `vst3`). Building, bundle layouts, installing and testing are covered in [BUILDING.md](BUILDING.md), and the design of both wrappers in [ARCHITECTURE.md](../ARCHITECTURE.md#format-specific-implementation-details).

### Audio Unit

Audio Units are supported on macOS through the `beamer-au` crate, as AUv2 (`.component`) and AUv3 (`.appex`).

#### AU Component Types

The `category` field in `Config.toml` determines the AU component type:

| Category | AU Component Type | Use For |
|----------|-------------------|---------|
| `"effect"` | `aufx` | EQ, compressor, reverb |
| `"midi_effect"` | `aumi` | Arpeggiator, harmonizer, MIDI effects |
| `"instrument"` | `aumu` | Synths, samplers, drum machines |
| `"generator"` | `augn` | Test tones, noise generators |

#### FourCC Codes

The `manufacturer_code` and `plugin_code` fields in `Config.toml` are the 4-character FourCC identifiers of the Audio Unit:

- `manufacturer_code`: Use your company/product abbreviation (4 ASCII chars)
- `plugin_code`: Unique identifier for this specific plugin (4 ASCII chars)
- Avoid conflicts: Check [Apple's registry](https://developer.apple.com/library/archive/documentation/General/Conceptual/ExtensibilityPG/AudioUnit.html)
- Use lowercase for effects, MixedCase for instruments (convention)

### VST3

VST3 plugins are built through the `beamer-vst3` crate. With the `vst3` feature enabled, the macro generates the platform entry points: `GetPluginFactory`, plus `bundleEntry`/`bundleExit` on macOS and `InitDll`/`ExitDll` on Windows.

#### Component ID

The VST3 component UUID is derived from `manufacturer_code` and `plugin_code`:

```
FNV-1a-128("beamer-vst3-uid" + manufacturer_code + plugin_code)
```

The same codes always produce the same UUID, so no manual UUID is needed. To keep the ID of an already shipped plugin, set it in `Config.toml`:

```toml
vst3_id = "12345678-9ABC-DEF0-1234-567890ABCDEF"
```

The format is `XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX` (36 characters with dashes). `cargo xtask generate-uuid` prints a new one.

#### VST3 Subcategories

VST3 subcategories are derived from `Config.toml`:

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

Common subcategories: `"dynamics"`, `"eq"`, `"filter"`, `"delay"`, `"reverb"`, `"modulation"`, `"distortion"`, `"synth"` and `"sampler"`.
