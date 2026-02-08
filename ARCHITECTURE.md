# Beamer Architecture

This document describes the high-level architecture of Beamer, a Rust framework for building audio plugins with WebView-based GUIs.

For detailed API documentation, see [docs/REFERENCE.md](docs/REFERENCE.md).
For example coverage and testing roadmap, see [docs/EXAMPLE_COVERAGE.md](docs/EXAMPLE_COVERAGE.md).

---

## Overview

### What Is Beamer?

A Rust framework for building audio plugins (Audio Unit and VST3) with WebView-based GUIs. Named after the beams that connect notes in sheet music, Beamer links your DSP logic and WebView interface together. Inspired by Tauri's architecture but focused specifically on the audio plugin context.

### Why?

- **Rust for audio**: Memory safety, performance, no GC pauses
- **WebView for UI**: Leverage modern web technologies (React, Svelte, Vue, etc.)
- **Multi-format**: Audio Unit and VST3 support from a single codebase
- **Lightweight**: Use OS-native WebViews, no bundled browser engine
- **Cross-platform**: macOS (Intel and Apple Silicon) and Windows

### Goals

- Audio Unit support (macOS, AUv2 and AUv3) ✅
- VST3 plugin support (VST3 3.8, MIT licensed) ✅
- WebView GUI using OS-native engines
- Cross-platform: macOS and Windows
- Tauri-inspired IPC (invoke/emit pattern)
- Optional parameter binding helpers
- Developer-friendly: hot reload in dev mode
- Framework-agnostic frontend (React, Svelte, Vue, vanilla JS)
- MIDI event processing (instruments and MIDI effects)

---

## Architecture Diagrams

### Audio Unit Architecture (Hybrid ObjC/Rust)

The AU wrapper uses a **hybrid architecture**: native Objective-C for Apple runtime compatibility, with all DSP in Rust via C-ABI bridge. Both AUv2 (`.component`) and AUv3 (`.appex`) formats are supported through the same C-ABI bridge layer.

```
┌──────────────────────────────────────────────────────────────────┐
│                     DAW Host (macOS)                             │
├──────────────────────────────────────────────────────────────────┤
│   AUv2 API (.component)         │     AUv3 API (.appex)          │
│   AudioComponentPlugInInterface │     AUAudioUnit subclass       │
├─────────────────────────────────┼────────────────────────────────┤
│                                 │                                │
│   Audio Thread                  │         Main Thread            │
│   ┌──────────────┐              │         ┌──────────────────┐   │
│   │              │              │         │  Native ObjC     │   │
│   │ Render Call  │◄─────────────┼────────►│  Wrapper Layer   │   │
│   │  (AUv2/v3)   │   C-ABI      │         │                  │   │
│   │              │   calls      │         └────────┬─────────┘   │
│   └──────┬───────┘              │                  │             │
│          │                      │                  │ NSView      │
│          │ beamer_au_render()   │         ┌────────▼─────────┐   │
│   ┌──────▼───────┐              │         │                  │   │
│   │ bridge.rs    │              │         │  WebView Window  │   │
│   │ RenderBlock  │              │         │   (WKWebView)    │   │
│   │ AuProcessor  │              │         │                  │   │
│   └──────────────┘              │         └──────────────────┘   │
│                                 │                                │
└─────────────────────────────────┴────────────────────────────────┘
```

**Why Hybrid?** Native Objective-C integrates naturally with Apple's frameworks, provides better debuggability with Apple's tools, and avoids the complexity of Rust FFI bindings for `AUAudioUnit` subclassing. The hybrid approach guarantees Apple compatibility while keeping all audio processing in Rust.

### VST3 Architecture

The VST3 wrapper uses COM (Component Object Model) interfaces implemented directly in Rust. A single `Vst3Processor<P>` class implements all required interfaces, with the processor handling audio on the audio thread and the edit controller managing parameters on the UI thread.

```
┌─────────────────────────────────────────────────────────────────┐
│                         DAW Host                                │
├─────────────────────────────────────────────────────────────────┤
│                      VST3 Interface                             │
│              (IComponent, IAudioProcessor, IEditController)     │
├────────────────────────────────┬────────────────────────────────┤
│                                │                                │
│    Audio Thread                │         UI Thread              │
│    ┌──────────────┐            │         ┌──────────────────┐   │
│    │              │            │         │                  │   │
│    │  Processor   │◄───────────┼────────►│  EditController  │   │
│    │  (DSP code)  │  lock-free │         │                  │   │
│    │              │  queue     │         └────────┬─────────┘   │
│    └──────────────┘            │                  │             │
│                                │                  │ IPlugView   │
│                                │         ┌────────▼─────────┐   │
│                                │         │                  │   │
│                                │         │  WebView Window  │   │
│                                │         │  (WKWebView /    │   │
│                                │         │   WebView2)      │   │
│                                │         │                  │   │
│                                │         └──────────────────┘   │
└────────────────────────────────┴────────────────────────────────┘
```

**Why COM in Rust?** The VST3 SDK is C++ based, but Rust can implement COM interfaces directly using vtable pointers. This avoids C++ interop complexity while maintaining full compatibility with VST3 hosts.

### Unified Core

Both formats share the same core traits and processing logic:

```
┌─────────────────────────────────────────────────────────────────┐
│                       beamer-core                               │
│  • Descriptor trait (unprepared state)                          │
│  • Processor trait (prepared state)                             │
│  • Buffer, AuxiliaryBuffers, MidiBuffer                         │
│  • Parameters trait, ParameterStore                             │
│  • ProcessContext, Transport                                    │
└──────────────────────┬──────────────────┬───────────────────────┘
                       │                  │
         ┌─────────────▼──────┐  ┌────────▼─────────────┐
         │    beamer-au       │  │   beamer-vst3        │
         │                    │  │                      │
         │ • AuProcessor<P>   │  │ • Vst3Processor<P>   │
         │ • C-ABI bridge     │  │ • COM interfaces     │
         │ • Native ObjC wrap │  │ • VST3 MIDI          │
         │ • UMP MIDI         │  │ • Factory            │
         └────────────────────┘  └──────────────────────┘
```

---

## Threading Model

| Thread | Responsibilities | Constraints |
|--------|------------------|-------------|
| **Audio Thread** | DSP processing, buffer handling | Real-time safe: no allocations, no locks, no syscalls |
| **UI Thread** | Parameter changes, WebView, IPC | Can allocate, can block (briefly) |
| **Host Thread** | Plugin lifecycle, state save/load | Varies by host |

---

## Crate Structure

```
beamer/
├── crates/
│   ├── beamer/              # Main crate (re-exports)
│   ├── beamer-core/         # Plugin traits, MIDI types, buffers
│   ├── beamer-macros/       # Proc macros (#[derive(Parameters)], #[derive(EnumParameter)], #[derive(HasParameters)], #[derive(Presets)])
│   ├── beamer-utils/        # Shared utilities (zero deps)
│   ├── beamer-au/           # Audio Unit wrapper implementation (macOS)
│   ├── beamer-vst3/         # VST3 wrapper implementation
│   └── beamer-webview/      # WebView per platform (planned)
├── examples/
│   ├── gain/                # Audio effect example
│   ├── compressor/          # Dynamics compressor
│   ├── equalizer/           # 3-band parametric EQ
│   ├── delay/               # Delay effect with tempo sync
│   ├── synthesizer/         # Polyphonic synthesizer with MIDI CC emulation
│   ├── drums/               # Drum synthesizer with multi-output buses
│   └── midi-transform/      # MIDI effect example
└── xtask/                   # Build tooling (bundle, install)
```

### Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `beamer` | Facade crate, re-exports public API via `prelude` |
| `beamer-core` | Platform-agnostic traits (`Descriptor`, `Processor`, `HasParameters`), buffer types, MIDI types, shared `Config` |
| `beamer-macros` | `#[derive(Parameters)]`, `#[derive(EnumParameter)]`, `#[derive(HasParameters)]`, `#[derive(Presets)]` proc macros |
| `beamer-utils` | Internal utilities shared between crates (zero external deps) |
| `beamer-au` | Audio Unit (AUv2 and AUv3) integration via hybrid ObjC/Rust architecture, C-ABI bridge, `AuConfig` (macOS only) |
| `beamer-vst3` | VST3 SDK integration, COM interfaces, host communication, `Vst3Config` |
| `beamer-webview` | Platform-native WebView embedding (planned) |

---

## Plugin Lifecycle

Beamer uses type-safe initialization via `prepare()` that eliminates placeholder values:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Descriptor (Unprepared)                     │
│  • Created via Default::default()                               │
│  • Holds parameters and bus configuration                       │
│  • No sample rate or audio state                                │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
                                  │ prepare(config)
                                  │ [setupProcessing]
                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Processor (Prepared)                          │
│  • Created with real sample rate and buffer size                │
│  • Allocates DSP state (delay buffers, filter coefficients)     │
│  • Ready for process() calls                                    │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
                                  │ unprepare()
                                  │ [sample rate change]
                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Descriptor (Unprepared)                     │
│  • Parameters preserved                                         │
│  • DSP state discarded                                          │
│  • Ready for prepare() with new config                          │
└─────────────────────────────────────────────────────────────────┘
```

### Why This Design?

Audio plugins need sample rate for buffer allocation, filter coefficients, and envelope timing, but the sample rate isn't known until the host calls `setupProcessing()`. The `prepare()` design ensures DSP state is only created with valid configuration.

### Design Rationale

Beamer's design follows the Rust principle of **making invalid states unrepresentable**. This is the **typestate pattern** - different types represent different states, and the compiler enforces valid transitions.

The `Processor` type is always fully initialized, so `process()` code is clean:

```rust
impl Processor for DelayProcessor {
    fn process(&mut self, buffer: &mut Buffer, ...) {
        // self.sample_rate is guaranteed valid
        // self.buffer is guaranteed allocated
        // No Option<T>, no .expect(), no placeholder checks
    }
}
```

### Three-Struct Pattern

Beamer plugins use three structs for clear separation of concerns:

1. **`*Parameters`** - Pure parameter definitions with `#[derive(Parameters)]`
2. **`*Descriptor`** - Plugin descriptor that holds parameters and implements `Descriptor`
3. **`*Processor`** - Runtime processor created by `prepare()`, implements `Processor`

```rust
// 1. Parameters - pure data
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
    fn process(&mut self, buffer: &mut Buffer, ...) { /* ... */ }
}
```

**Plugins with DSP state** add fields to the Processor:

```rust
#[derive(HasParameters)]
struct DelayProcessor {
    #[parameters]
    parameters: DelayParameters,
    sample_rate: f64,    // Always valid
    buffer: Vec<f64>,    // Always allocated
}
```

### Setup Types

| Type | Use Case | Value |
|------|----------|-------|
| `()` | Stateless plugins (gain, pan) | - |
| `SampleRate` | Most plugins (delay, filter, envelope) | `f64` via `.hz()` |
| `MaxBufferSize` | FFT, lookahead | `usize` |
| `MainOutputChannels` | Per-channel state | `u32` |
| `(A, B, ...)` | Combine multiple types | Tuples up to 5 elements |

For IDE autocomplete, use `beamer::setup::*` to import all available types.

### Trait Responsibilities

| Trait | State | Responsibilities |
|-------|-------|------------------|
| `HasParameters` | Both | Parameter access (`parameters()`, `parameters_mut()`, `set_parameters()`) - use `#[derive(HasParameters)]` |
| `Descriptor` | Unprepared | Bus configuration, MIDI mapping, `prepare()` transformation |
| `Processor` | Prepared | DSP processing, state persistence, MIDI processing, `unprepare()` (has default impl) |

### Parameter Ownership

Parameters are **owned** by both `Descriptor` and `Processor`, moving between them during state transitions:

```
Descriptor                       Processor
┌─────────────────────┐        ┌─────────────────────┐
│ parameters ─────────┼──────► │ parameters          │
└─────────────────────┘        └─────────────────────┘
       prepare() moves              unprepare() moves
       parameters →                 ← parameters back
```

This is the **type-state pattern** - a Rust idiom for encoding state machines at the type level. The same pattern appears in `std::fs::File`, builder APIs, and session types.

**Why ownership instead of shared references?**

1. **Zero overhead**: Direct field access: `self.parameters.gain.get()`
2. **No synchronization**: Owned data needs no Arc, Mutex, or atomics for internal access
3. **Clear lifecycle**: Parameters exist exactly where they're used
4. **Smoother mutation**: Smoothers advance state each sample; ownership makes this natural

**The `HasParameters` trait:**

Both `Descriptor` and `Processor` implement `HasParameters` because the host needs parameter access in both states:
- Before `prepare()`: Host queries parameter info, user adjusts values
- After `prepare()`: Host automates parameters during playback

Use `#[derive(HasParameters)]` with a `#[parameters]` field annotation on both Descriptor and Processor:

```rust
// Descriptor with HasParameters
#[derive(Default, HasParameters)]
pub struct GainDescriptor {
    #[parameters]
    parameters: GainParameters,
}

// Processor with HasParameters
#[derive(HasParameters)]
pub struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
    // Additional DSP state...
}
```

The derive macro generates the `parameters()`, `parameters_mut()`, and `set_parameters()` methods automatically.

---

## Plugin Configuration and Export

Beamer uses a **split configuration model** to separate format-agnostic metadata from format-specific identifiers.

### Configuration Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                      beamer-core                               │
│                                                                │
│  Config (shared metadata)                                      │
│  • name, vendor, version                                       │
│  • category, sub_categories                                    │
│  • url, email, has_editor                                      │
└────────────────────┬───────────────────────────────────────────┘
                     │
       ┌─────────────┴──────────────┐
       │                            │
       ▼                            ▼
┌──────────────────┐      ┌──────────────────┐
│    beamer-au     │      │   beamer-vst3    │
│                  │      │                  │
│  AuConfig        │      │  Vst3Config      │
│  • manufacturer  │      │  • component_uid │
│  • subtype       │      │  • controller_uid│
│  • bus_config    │      │  • sysex_slots   │
│                  │      │  • sysex_buf_size│
└──────────────────┘      └──────────────────┘
```

### Example: Multi-Format Plugin

```rust
use beamer::prelude::*;

// Shared configuration (format-agnostic)
// Category is required and determines AU component type and VST3 base category
pub static CONFIG: Config = Config::new("My Gain", Category::Effect)
    .with_vendor("My Company")
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_subcategories(&[Subcategory::Dynamics]);

// AU-specific configuration (macOS only)
#[cfg(feature = "au")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    "Demo",  // Manufacturer code (4 chars)
    "gain",  // Subtype code (4 chars)
);

// VST3-specific configuration (generate UUID with: cargo xtask generate-uuid)
// Subcategories are derived from Config, or override with .with_subcategories()
#[cfg(feature = "vst3")]
pub static VST3_CONFIG: Vst3Config = Vst3Config::new("12345678-9ABC-DEF0-ABCD-EF1234567890");

// Export Audio Unit plugin (macOS only)
#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, MyPlugin);

// Export VST3 plugin
#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, MyPlugin);
```

### Factory Presets

Both export macros support an optional presets parameter for plugins that provide factory presets:

```rust
#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, GainPlugin, GainPresets);

#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, GainPlugin, GainPresets);
```

If no presets type is specified, `NoPresets` is used automatically.

### Configuration Fields

**Config** (shared):
- `name` - Display name in DAW (required, constructor param)
- `category` - Plugin type: `Category::Effect`, `Instrument`, `MidiEffect`, or `Generator` (required, constructor param)
- `vendor` - Company/developer name
- `version` - Semantic version string
- `subcategories` - Array of `Subcategory` values (e.g., `&[Subcategory::Dynamics]`)
- `url`, `email` - Contact information
- `has_editor` - GUI enabled flag

**AuConfig** (AU-specific):
- `manufacturer` - 4-character manufacturer code (FourCC)
- `subtype` - 4-character plugin subtype code (FourCC)
- `tags` - Optional AU tags (derived from `Config.subcategories` if not set)
- `bus_config` - Optional custom bus configuration

**Vst3Config** (VST3-specific):
- `component_uid` - 128-bit unique identifier (TUID)
- `controller_uid` - Optional separate controller UID (for split architecture)
- `subcategories` - Optional override (derived from `Config` if not set)
- `sysex_slots` - Number of SysEx output buffers
- `sysex_buffer_size` - Size of each SysEx buffer

### Why Split Configuration?

1. **Shared metadata** - Write plugin name, vendor, version once
2. **Format requirements** - AU needs FourCC codes, VST3 needs UIDs
3. **Conditional compilation** - AU export only compiles on macOS
4. **Future extensibility** - Possible to add CLAP, AAX, LV2 without affecting core

### Building Multi-Format Plugins

Use `xtask` to build both formats:

```bash
# AUv2 only (macOS, native architecture)
cargo xtask bundle my-plugin --auv2 --release

# AUv3 only (macOS, native architecture)
cargo xtask bundle my-plugin --auv3 --release

# VST3 only (native architecture)
cargo xtask bundle my-plugin --vst3 --release

# All formats (macOS)
cargo xtask bundle my-plugin --auv2 --auv3 --vst3 --release

# Install to system plugin directories
cargo xtask bundle my-plugin --auv2 --auv3 --vst3 --release --install

# Universal binary for distribution (x86_64 + arm64)
cargo xtask bundle my-plugin --auv2 --auv3 --vst3 --arch universal --release
```

**Architecture options**: `--arch native` (default), `--arch universal`, `--arch arm64`, `--arch x86_64`

---

## Format-Specific Implementation Details

While both formats share the same `beamer-core` abstractions, they differ significantly in their platform APIs.

### Audio Unit Implementation

**Architecture**: Hybrid Objective-C/Rust
- AUv2: `AudioComponentPlugInInterface` with selector-based dispatch
- AUv3: `BeamerAuWrapper` native ObjC class (subclass of `AUAudioUnit`)
- Shared C-ABI bridge layer (`BeamerAuBridge.h` ↔ `bridge.rs`) with 40+ functions
- Uses type erasure (`AuPluginInstance` trait) for generic plugin support
- Render blocks call into Rust via `beamer_au_render()`
- Full feature parity with VST3 wrapper

**Key Files**:

*Objective-C Layer:*
- [objc/BeamerAuBridge.h](crates/beamer-au/objc/BeamerAuBridge.h) - C-ABI declarations
- [xtask/src/au_codegen/auv3_wrapper.m](xtask/src/au_codegen/auv3_wrapper.m) - AUv3 wrapper template (generated per-plugin by xtask)
- [xtask/src/au_codegen/auv2_wrapper.c](xtask/src/au_codegen/auv2_wrapper.c) - AUv2 wrapper template (generated per-plugin by xtask)
- [build.rs](crates/beamer-au/build.rs) - Framework linking (ObjC compilation is handled by xtask)

*Rust Layer:*
- [bridge.rs](crates/beamer-au/src/bridge.rs) - C-ABI implementations
- [processor.rs](crates/beamer-au/src/processor.rs) - Plugin wrapper + f64 conversion
- [render.rs](crates/beamer-au/src/render.rs) - RenderBlock + MIDI + parameter events
- [lifecycle.rs](crates/beamer-au/src/lifecycle.rs) - State machine + prepare
- [sysex_pool.rs](crates/beamer-core/src/sysex_pool.rs) - SysEx output pool (in beamer-core, shared with VST3)

**MIDI**: UMP MIDI 1.0/2.0 → `beamer-core::MidiEvent`
- Universal MIDI Packet format (32-bit packets)
- Supports Note On/Off, CC, Pitch Bend, Channel Pressure, SysEx
- 1024 event buffer (matches VST3)
- `MidiCcState` tracking for CC/pitch bend queries
- `SysExOutputPool` for real-time safe SysEx output

**Parameters**: Push model via KVO (Key-Value Observing)
- `AUParameterTree` built from `ParameterStore`
- `implementorValueObserver` - Host → plugin changes
- `implementorValueProvider` - Plugin → host reads
- Automation via `AURenderEventParameter`/`ParameterRamp` (buffer-quantized with smoother interpolation)

**State**: NSDictionary with NSData
- Full processor state persistence (`save_state`/`load_state`)
- Deferred state loading via `pending_state` (matches VST3)
- Compatible with VST3 format

**Real-time Safety**:
- Pre-allocated f64↔f32 conversion buffers (main + aux buses)
- Pre-allocated MIDI/SysEx buffers
- No heap allocation in render path

### VST3 Implementation

**Architecture**: COM-based (Component Object Model)
- Single `Vst3Processor<P>` class implements 15+ COM interfaces
- Uses combined component pattern (processor + controller in one class)
- Direct function pointer vtables for interface calls

**Key Files**:
- [processor.rs](crates/beamer-vst3/src/processor.rs) - Main wrapper
- [factory.rs](crates/beamer-vst3/src/factory.rs) - COM factory registration
- [export.rs](crates/beamer-vst3/src/export.rs) - Platform entry points

**MIDI**: VST3 `Event` union → `beamer-core::MidiEvent`
- 16+ event types (NoteOn, NoteOff, MIDI CC, PolyPressure, etc.)
- Supports VST3-specific events (NoteExpression, Chord, Scale)
- Legacy MIDI CC output for host compatibility

**Parameters**: Pull model via COM methods
- `getParameterInfo()` - Host queries parameter metadata
- `setParamNormalized()` - Host sets parameter value
- `getParamNormalized()` - Host reads parameter value

**State**: Binary blob via `IBStream`

### Comparison Table

| Feature | Audio Unit | VST3 |
|---------|------------|------|
| **Platform** | macOS only | macOS and Windows |
| **API Style** | Hybrid ObjC/Rust via C-ABI | COM (C++ style) |
| **Language** | ObjC + Rust + cc crate | Rust + vst3-sys |
| **Code Size** | Multiple files (ObjC + Rust) | Single file |
| **MIDI Format** | UMP MIDI 1.0/2.0 | VST3 Event union |
| **MIDI Buffer** | 1024 events | 1024 events |
| **MidiCcState** | ✓ | ✓ |
| **MIDI Output** | ✓ (instruments/MIDI effects only) | ✓ |
| **SysEx Output** | ✓ (pool) | ✓ (pool) |
| **Parameter Sync** | Push (KVO callbacks) | Pull (COM methods) |
| **Param Automation** | Buffer-quantized + smoothing | Buffer-quantized + smoothing |
| **Audio Buffers** | `AudioBufferList` | `float**` arrays |
| **f64 Conversion** | Pre-allocated | Pre-allocated |
| **State Format** | NSDictionary | Binary blob |
| **Processor State** | ✓ | ✓ |
| **Bundle Type** | `.component` (AUv2) / `.appex` (AUv3) | `.vst3` |
| **Registration** | ObjC factory + module init | `GetPluginFactory()` |
| **Feature Parity** | ✓ Full parity | Reference |

### Code Reuse Statistics

**Shared** (beamer-core): ~100%
- All DSP processing logic
- Parameter management
- MIDI event representation
- Buffer abstractions
- Transport and context

**Format-specific**: ~0% overlap
- Different C APIs (COM vs ObjC)
- Different MIDI formats
- Different parameter models
- Different state serialization

The format wrappers are **thin translation layers** that adapt the platform API to `beamer-core` abstractions.

---

## Operational Guarantees

This section documents the invariants that Beamer enforces. These are API contracts that plugin authors can rely on.

### Real-Time Safety

**Guarantee**: No heap allocations occur on the audio thread during `process()`.

| Component | Mechanism |
|-----------|-----------|
| `Buffer<S>` | Stack-allocated `[Option<&[S]>; MAX_CHANNELS]` arrays |
| `AuxiliaryBuffers<S>` | Stack-allocated nested fixed arrays |
| `MidiBuffer` | Pre-allocated fixed capacity (1024 events default) |
| `SysExOutputPool` | Pre-allocated slots (16 × 512 bytes default) |
| `ProcessBufferStorage<S>` | Pre-allocated Vecs with reserved capacity; `clear()` + `push()` never allocate |

**Enforcement**:
- `setupProcessing()` pre-allocates all buffers based on plugin configuration
- `process()` uses only stack storage and pre-allocated pools
- Bounds checking via `.take(max)` prevents allocation even if host misbehaves

### Deterministic Bus Limits

**Guarantee**: Channel and bus counts are bounded at compile time.

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_CHANNELS` | 32 | Supports up to 22.2 surround and Dolby Atmos 9.1.6 |
| `MAX_BUSES` | 16 | Main + sidechain + 14 aux buses |
| `MAX_AUX_BUSES` | 15 | Auxiliary buses (total minus main) |

**Enforcement**:
- `validate_bus_limits()` checks plugin config against constants at initialization
- `validate_speaker_arrangement()` rejects invalid host arrangements in `setBusArrangements()`
- `setupProcessing()` returns `kResultFalse` and logs error if limits exceeded

### MIDI Data Fidelity

**Guarantee**: MIDI data passes through without loss or corruption under normal conditions.

| Aspect | Mechanism |
|--------|-----------|
| **Tuning preservation** | `NoteOn.tuning` and `NoteOff.tuning` fields (f32 cents, ±120.0) |
| **Length preservation** | `NoteOn.length` field (i32 samples, 0 = unknown) |
| **Sample accuracy** | `MidiEvent.sample_offset` preserved through VST3 round-trip |
| **Note ID tracking** | `NoteId` maintained for proper note-on/note-off pairing |

**Overflow Handling**:
- `MidiBuffer::has_overflowed()` flag set when capacity exceeded
- `SysExOutputPool::has_overflowed()` flag set when pool exhausted
- Automatic `log::warn!()` on first overflow per block
- Optional `sysex-heap-fallback` feature for guaranteed SysEx delivery (breaks real-time guarantee)

### Buffer Management Contracts

**ProcessBufferStorage** (defined in `beamer-core`, with format-specific extensions):
```rust
pub struct ProcessBufferStorage<S: Sample> {
    pub main_inputs: Vec<*const S>,
    pub main_outputs: Vec<*mut S>,
    pub aux_inputs: Vec<Vec<*const S>>,
    pub aux_outputs: Vec<Vec<*mut S>>,
    pub internal_output_buffers: Option<Vec<Vec<S>>>,
    pub max_frames: usize,
}
```

- Pre-allocated in `setupProcessing()` based on plugin's **actual** bus configuration (not worst-case)
- Config-based allocation: stereo plugin uses 32 bytes, not 4KB worst-case
- Lazy aux allocation: no heap allocation for plugins without aux buses
- Internal output buffers allocated only for instruments (hosts may provide null pointers)
- `clear()` resets length to 0 without deallocating
- `push()` into reserved capacity never allocates

**Plugin-Declared Capacity** (VST3-specific):
```rust
pub static VST3_CONFIG: Vst3Config = Vst3Config::new("12345678-9ABC-DEF0-ABCD-EF1234567890")
    .with_sysex_slots(64)         // Default: 16
    .with_sysex_buffer_size(4096); // Default: 512 bytes
```

### Allocation Lifecycle

The buffer allocation flow ensures all memory is reserved before audio processing begins:

```
Plugin Load (creates Descriptor in Unprepared state)
    │
    ▼
┌─────────────────────────────────────────────────────────────┐
│ validate_bus_limits(plugin_config)                          │
│   • Check declared buses ≤ MAX_BUSES                        │
│   • Check declared channels per bus ≤ MAX_CHANNELS          │
│   • Return error if exceeded (plugin fails to load)         │
└─────────────────────────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────────────────────┐
│ setBusArrangements(inputs, outputs)  [VST3 host call]       │
│   • validate_speaker_arrangement() for each bus             │
│   • Reject if any arrangement exceeds MAX_CHANNELS          │
│   • Return kResultFalse on rejection (host tries another)   │
└─────────────────────────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────────────────────┐
│ setupProcessing(sample_rate, max_block_size)                │
│   • Descriptor::prepare(config) → Processor                 │
│     - Descriptor consumed, Processor created                │
│     - DSP state allocated with real sample rate             │
│   • ProcessBufferStorage::allocate()                        │
│     - input_ptrs.reserve(main_channels)                     │
│     - output_ptrs.reserve(main_channels)                    │
│     - aux_input_ptrs[i].reserve(aux_channels[i])            │
│     - aux_output_ptrs[i].reserve(aux_channels[i])           │
│   • All Vecs now have capacity, length = 0                  │
│   • Return kResultFalse + log if allocation fails           │
└─────────────────────────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────────────────────┐
│ process() [audio thread, called repeatedly]                 │
│   • storage.clear() - sets len=0, no deallocation           │
│   • storage.push(ptr) - into reserved capacity, no alloc    │
│   • .take(MAX_CHANNELS) - bounds check even if host lies    │
│   • Build Buffer/AuxiliaryBuffers from pointers             │
│   • Call Processor::process()                               │
└─────────────────────────────────────────────────────────────┘
    │
    ▼ (on sample rate change)
┌─────────────────────────────────────────────────────────────┐
│ setupProcessing() with new config                           │
│   • Processor::unprepare() → Descriptor                     │
│     - Parameters preserved, DSP state discarded             │
│   • Descriptor::prepare(new_config) → Processor             │
│     - DSP state reallocated for new sample rate             │
└─────────────────────────────────────────────────────────────┘
```

**Key invariant**: After `setupProcessing()` succeeds, `process()` never allocates.

---

## Inspiration

| Project | |
|---------|---|
| [Tauri](https://tauri.app) | WebView integration, IPC patterns |
| [Apple AUv3](https://developer.apple.com/documentation/audiotoolbox/audio_unit_v3_plug-ins) | Audio Unit v3 specification |
| [VST3 SDK](https://github.com/steinbergmedia/vst3sdk) | VST3 specification and reference |
| [Coupler](https://github.com/coupler-rs/coupler) | VST3 Rust bindings (dependency) |
| [nih-plug](https://github.com/robbert-vdh/nih-plug) | Rust plugin framework reference |
| [iPlug2](https://github.com/iPlug2/iPlug2) | C++ plugin framework reference |
| [JUCE](https://juce.com) | C++ plugin framework reference |
