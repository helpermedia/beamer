# Design: Combined Plugin Macro

> **Status:** Not planned. This document exists for future reference.
>
> **Decision:** We chose explicit Plugin and Processor structs. See [ARCHITECTURE.md](../ARCHITECTURE.md#parameter-ownership) for the rationale.
>
> **Update:** The `save_state()`/`load_state()` boilerplate mentioned in this document has been eliminated via default implementations in `AudioProcessor`. These methods now automatically delegate to `Parameters::save_state()`/`load_state()`.

---

## Overview

This document describes an alternative approach where a **single macro generates both Plugin and Processor** structs from one definition. The user would declare parameters and processor fields once, and the macro would generate all boilerplate.

---

## Current Architecture (Explicit Structs)

```rust
// User defines Parameters
#[derive(Parameters)]
pub struct MyParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

// User defines Plugin (unprepared state)
#[derive(Default, HasParameters)]
pub struct MyPlugin {
    #[parameters]
    parameters: MyParameters,
}

impl Plugin for MyPlugin {
    type Setup = SampleRate;
    type Processor = MyProcessor;

    fn prepare(self, setup: SampleRate) -> MyProcessor {
        MyProcessor {
            parameters: self.parameters,
            sample_rate: setup.hz(),
        }
    }
}

// User defines Processor (prepared state)
#[derive(HasParameters)]
pub struct MyProcessor {
    #[parameters]
    parameters: MyParameters,
    sample_rate: f64,
}

impl AudioProcessor for MyProcessor {
    type Plugin = MyPlugin;

    fn unprepare(self) -> MyPlugin {
        MyPlugin { parameters: self.parameters }
    }

    fn process(&mut self, buffer: &mut Buffer, ...) { ... }

    // save_state/load_state: automatically handled by default implementations
}
```

---

## Proposed Architecture (Combined Macro)

```rust
// User defines Parameters (unchanged)
#[derive(Parameters)]
pub struct MyParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

// Single macro generates both Plugin and Processor
beamer::define_plugin! {
    plugin: MyPlugin,
    processor: MyProcessor,
    parameters: MyParameters,
    setup: SampleRate,

    // Fields only present in Processor (DSP state)
    processor_fields: {
        sample_rate: f64,
    },

    // Initialization for processor fields
    prepare: |setup| {
        sample_rate: setup.hz(),
    },
}

// User implements only the DSP logic
impl MyProcessor {
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        let gain = self.parameters.gain.as_linear() as f32;
        buffer.apply_output_gain(gain);
    }
}
```

---

## What the Macro Would Generate

```rust
// Generated Plugin struct
#[derive(Default, HasParameters)]
pub struct MyPlugin {
    #[parameters]
    parameters: MyParameters,
}

// Generated Plugin impl
impl Plugin for MyPlugin {
    type Setup = SampleRate;
    type Processor = MyProcessor;

    fn prepare(self, setup: SampleRate) -> MyProcessor {
        MyProcessor {
            parameters: self.parameters,
            // From prepare block:
            sample_rate: setup.hz(),
        }
    }
}

// Generated Processor struct
#[derive(HasParameters)]
pub struct MyProcessor {
    #[parameters]
    parameters: MyParameters,
    // From processor_fields:
    sample_rate: f64,
}

// Generated AudioProcessor impl (partial)
impl AudioProcessor for MyProcessor {
    type Plugin = MyPlugin;

    fn unprepare(self) -> MyPlugin {
        MyPlugin { parameters: self.parameters }
    }

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(self.parameters.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        self.parameters.load_state(data).map_err(PluginError::StateError)
    }

    // process() NOT generated - user must implement
}
```

---

## What Would Need to Change

### 1. New Procedural Macro

```rust
// In beamer-macros/src/lib.rs
#[proc_macro]
pub fn define_plugin(input: TokenStream) -> TokenStream {
    // Parse:
    // - plugin name
    // - processor name
    // - parameters type
    // - setup type
    // - processor_fields block
    // - prepare block
    //
    // Generate:
    // - Plugin struct with #[derive(Default, HasParameters)]
    // - Plugin impl with prepare()
    // - Processor struct with #[derive(HasParameters)]
    // - Partial AudioProcessor impl (unprepare, save_state, load_state)
}
```

### 2. Syntax Design Decisions

**Option A: Declarative (shown above)**
```rust
beamer::define_plugin! {
    plugin: MyPlugin,
    processor: MyProcessor,
    parameters: MyParameters,
    setup: SampleRate,
    processor_fields: { sample_rate: f64 },
    prepare: |setup| { sample_rate: setup.hz() },
}
```

**Option B: Attribute-based**
```rust
#[beamer::plugin(MyPlugin -> MyProcessor)]
struct PluginDef {
    #[parameters]
    parameters: MyParameters,

    #[processor_only]
    sample_rate: f64,
}
```

**Option C: Derive-based**
```rust
#[derive(BeamerPlugin)]
#[beamer(plugin = "MyPlugin", processor = "MyProcessor")]
struct MyParameters {
    #[parameter(...)]
    pub gain: FloatParameter,
}
```

### 3. Handling Custom prepare() Logic

The challenge: most plugins need custom initialization in `prepare()`.

**Approach 1: Closure in macro**
```rust
prepare: |setup| {
    sample_rate: setup.hz(),
    delay_buffer: vec![0.0; (setup.hz() * 2.0) as usize],
}
```

**Approach 2: Trait with default**
```rust
trait ProcessorInit {
    fn init(setup: &SampleRate) -> Self::ProcessorFields;
}
```

**Approach 3: Builder pattern**
```rust
processor_fields: {
    sample_rate: f64 = setup.hz(),
    delay_buffer: Vec<f64> = vec![0.0; (setup.hz() * 2.0) as usize],
}
```

---

## Code Comparison

### Delay Plugin - Current (~80 lines for boilerplate)

```rust
#[derive(Parameters)]
pub struct DelayParameters {
    #[parameter(id = "time", name = "Time", default = 500.0, range = 1.0..=2000.0, kind = "ms")]
    pub time: FloatParameter,
    #[parameter(id = "feedback", name = "Feedback", default = 0.5, range = 0.0..=0.95)]
    pub feedback: FloatParameter,
    #[parameter(id = "mix", name = "Mix", default = 0.5, range = 0.0..=1.0)]
    pub mix: FloatParameter,
}

#[derive(Default, HasParameters)]
pub struct DelayPlugin {
    #[parameters]
    parameters: DelayParameters,
}

impl Plugin for DelayPlugin {
    type Setup = SampleRate;
    type Processor = DelayProcessor;

    fn prepare(self, setup: SampleRate) -> DelayProcessor {
        let max_samples = (setup.hz() * 2.0) as usize;
        DelayProcessor {
            parameters: self.parameters,
            buffer_l: vec![0.0; max_samples],
            buffer_r: vec![0.0; max_samples],
            write_pos: 0,
            sample_rate: setup.hz(),
        }
    }
}

#[derive(HasParameters)]
pub struct DelayProcessor {
    #[parameters]
    parameters: DelayParameters,
    buffer_l: Vec<f64>,
    buffer_r: Vec<f64>,
    write_pos: usize,
    sample_rate: f64,
}

impl AudioProcessor for DelayProcessor {
    type Plugin = DelayPlugin;

    fn unprepare(self) -> DelayPlugin {
        DelayPlugin { parameters: self.parameters }
    }

    fn process(&mut self, buffer: &mut Buffer, ...) {
        // DSP code
    }

    fn save_state(&self) -> PluginResult<Vec<u8>> {
        Ok(self.parameters.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        self.parameters.load_state(data).map_err(PluginError::StateError)
    }
}
```

### Delay Plugin - Combined Macro (~50 lines)

```rust
#[derive(Parameters)]
pub struct DelayParameters {
    #[parameter(id = "time", name = "Time", default = 500.0, range = 1.0..=2000.0, kind = "ms")]
    pub time: FloatParameter,
    #[parameter(id = "feedback", name = "Feedback", default = 0.5, range = 0.0..=0.95)]
    pub feedback: FloatParameter,
    #[parameter(id = "mix", name = "Mix", default = 0.5, range = 0.0..=1.0)]
    pub mix: FloatParameter,
}

beamer::define_plugin! {
    plugin: DelayPlugin,
    processor: DelayProcessor,
    parameters: DelayParameters,
    setup: SampleRate,

    processor_fields: {
        buffer_l: Vec<f64>,
        buffer_r: Vec<f64>,
        write_pos: usize,
        sample_rate: f64,
    },

    prepare: |setup| {
        let max_samples = (setup.hz() * 2.0) as usize;
        buffer_l: vec![0.0; max_samples],
        buffer_r: vec![0.0; max_samples],
        write_pos: 0,
        sample_rate: setup.hz(),
    },
}

impl DelayProcessor {
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        // DSP code - same as before
    }
}
```

**Savings: ~30 lines (38% reduction)**

---

## Advantages

### 1. Less Boilerplate

- No duplicate struct definitions
- No manual `unprepare()` implementation
- ~~No manual `save_state()`/`load_state()` implementations~~ (Now solved via default implementations)
- Single source of truth for field names

### 2. Reduced Error Surface

- Can't forget to move parameters in `prepare()`/`unprepare()`
- Can't mismatch field names between Plugin and Processor
- Macro enforces correct structure

### 3. Declarative Feel

- Plugin structure is declared, not imperatively constructed
- Closer to how you'd describe a plugin conceptually

---

## Disadvantages

### 1. Magic / Opacity

- Two structs generated from one definition
- Harder to understand what code actually exists
- IDE support may struggle with generated types
- Error messages point to macro, not user code

### 2. Limited Flexibility

- Custom `prepare()` logic requires escape hatches
- Can't easily add custom derives to generated structs
- Can't add custom methods to Plugin struct
- Bus configuration (`input_bus_count`, etc.) needs special handling

### 3. Macro Complexity

- Proc macro is significantly more complex to write and maintain
- Edge cases multiply (generics, lifetimes, visibility, attributes)
- Debugging macro issues is harder than debugging regular code

### 4. Learning Curve

- Users must learn macro syntax in addition to traits
- Documentation must explain both the macro and what it generates
- Harder to understand by reading examples

### 5. Doesn't Eliminate Much

Current boilerplate per plugin:
- `#[derive(Default, HasParameters)]` — 1 line
- `#[parameters]` attribute — 1 line
- `#[derive(HasParameters)]` — 1 line
- `#[parameters]` attribute — 1 line

**Total: 4 lines** of actual boilerplate. The rest (struct definitions, field declarations, trait impls) would still exist in some form in the macro.

---

## Analysis: Where It Helps vs Doesn't

| Plugin Type | Current Boilerplate | Macro Savings | Worth It? |
|-------------|--------------------:|---------------:|-----------|
| **Simple (gain)** | ~15 lines | ~8 lines | Maybe |
| **Medium (delay)** | ~25 lines | ~12 lines | Maybe |
| **Complex (synth)** | ~35 lines | ~15 lines | Probably not |

For complex plugins, the custom `prepare()` logic dominates. The macro's `prepare:` block would be nearly as verbose as the current explicit implementation.

### Why the "Savings" Are Misleading

The ~30 lines "saved" is not actual elimination of work:

| Current Explicit | Combined Macro |
|------------------|----------------|
| `struct MyProcessor { ... }` | `processor_fields: { ... }` |
| `sample_rate: f64,` | `sample_rate: f64,` |
| `buffer: Vec<f64>,` | `buffer: Vec<f64>,` |
| `prepare()` body | `prepare:` block body |

The struct definitions and field declarations still exist—just in a different syntax. You're trading Rust syntax users already know for custom macro syntax they must learn.

### The Real Trade-off

| What You Save | What It Costs |
|---------------|---------------|
| ~4 lines of ceremony | Complex proc macro to maintain |
| | Worse compiler error messages |
| | IDE struggles with generated types |
| | New syntax for users to learn |
| | Debugging macro internals |

The boilerplate being eliminated (`#[derive(HasParameters)]`, `#[parameters]`) is trivial. The macro doesn't help where help is actually needed—custom `prepare()` logic remains verbose either way.

---

## Alternative: Keep Current + Add Helpers

Instead of a combined macro, we could add smaller helpers:

### Helper 1: Default AudioProcessor impl

```rust
// Macro that implements unprepare + save/load
#[derive(HasParameters, DefaultAudioProcessor)]
#[audio_processor(plugin = MyPlugin)]
pub struct MyProcessor {
    #[parameters]
    parameters: MyParameters,
    sample_rate: f64,
}

// User only implements process()
impl AudioProcessor for MyProcessor {
    fn process(&mut self, ...) { ... }
}
```

### Helper 2: Simple plugin shorthand

```rust
// For plugins with no processor fields
beamer::simple_plugin!(GainPlugin, GainProcessor, GainParameters);

// Generates both structs with just parameters field
// User implements process() on GainProcessor
```

These targeted helpers provide most of the benefit without the complexity of a full combined macro.

---

## Conclusion

The combined macro approach:

1. **Saves ~30 lines** per plugin, but most are just syntax transformation, not elimination
2. **Adds complexity** to the macro crate
3. **Reduces flexibility** for custom initialization
4. **Obscures** what code is generated
5. **Doesn't scale** well to complex plugins

The current explicit approach costs **4 lines of boilerplate** per plugin. The cognitive overhead of a complex macro exceeds this cost.

**Recommendation:** Keep explicit structs. The targeted helpers are the right approach:

| Helper | Use Case | Value |
|--------|----------|-------|
| `simple_plugin!` | Stateless plugins (gain, pan) | High - eliminates real repetition |
| `#[derive(DefaultAudioProcessor)]` | Any plugin | ~~Medium - auto-generates `unprepare`, `save_state`, `load_state`~~ (save_state/load_state now have defaults) |

These helpers address actual repetitive code without hiding struct definitions or requiring users to learn new syntax.

> **Note:** As of the current version, `save_state()`/`load_state()` have default implementations that delegate to `Parameters`, so the `#[derive(DefaultAudioProcessor)]` helper would only need to generate `unprepare()`.

---

## See Also

- [ARCHITECTURE.md](../ARCHITECTURE.md) - Current architecture documentation
- [DESIGN_FRAMEWORK_OWNED_PARAMS.md](DESIGN_FRAMEWORK_OWNED_PARAMS.md) - Alternative: framework-owned parameters
- [REFERENCE.md](REFERENCE.md) - API reference
