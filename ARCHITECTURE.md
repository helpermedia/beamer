# Beamer Architecture

This document describes the high-level architecture of Beamer, a Rust framework for building audio plugins with WebView-based GUIs.

For detailed API documentation, see [docs/REFERENCE.md](docs/REFERENCE.md).
For example coverage and testing roadmap, see [docs/EXAMPLE_COVERAGE.md](docs/EXAMPLE_COVERAGE.md).

---

## Overview

### What Is Beamer?

A Rust framework for building audio plugins (VST3 and Audio Unit) with WebView-based GUIs. Named after the beams that connect notes in sheet music, Beamer links your DSP logic and WebView interface together. Inspired by Tauri's architecture but focused specifically on the audio plugin context.

### Why?

- **Rust for audio**: Memory safety, performance, no GC pauses
- **WebView for UI**: Leverage modern web technologies (React, Svelte, Vue, etc.)
- **Multi-format**: VST3 and Audio Unit support from a single codebase
- **Lightweight**: Use OS-native WebViews, no bundled browser engine
- **Cross-platform**: Windows, macOS (both Intel and Apple Silicon), Linux

### Goals

- VST3 plugin support (VST3 3.8, MIT licensed) ✅
- Audio Unit support (macOS, AUv2 and AUv3) ✅
- WebView GUI using OS-native engines
- Cross-platform: Windows, macOS, Linux
- Tauri-inspired IPC (invoke/emit pattern)
- Optional parameter binding helpers
- Developer-friendly: hot reload in dev mode
- Framework-agnostic frontend (React, Svelte, Vue, vanilla JS)
- MIDI event processing (instruments and MIDI effects)

### Non-Goals

- CLAP/AAX support (can be added later)
- Bundled browser engine (no Electron/CEF)
- Built-in UI components/widgets

---

## Architecture Diagrams

### VST3 Architecture

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
│                                │         │   WebView2 /     │   │
│                                │         │   WebKitGTK)     │   │
│                                │         │                  │   │
│                                │         └──────────────────┘   │
└────────────────────────────────┴────────────────────────────────┘
```

### Audio Unit Architecture (Hybrid ObjC/Rust)

The AU wrapper uses a **hybrid architecture**: native Objective-C for Apple runtime compatibility, with all DSP in Rust via C-ABI bridge. Both AUv2 (`.component`) and AUv3 (`.appex`) formats are supported through the same C-ABI bridge layer.

```
┌─────────────────────────────────────────────────────────────────┐
│                    DAW Host (macOS)                             │
├─────────────────────────────────────────────────────────────────┤
│     AUv2 API (.component)       │      AUv3 API (.appex)        │
│     AudioComponentPlugInInterface│      AUAudioUnit subclass     │
├────────────────────────────────┬┴───────────────────────────────┤
│                                │                                │
│    Audio Thread                │         Main Thread            │
│    ┌──────────────┐            │         ┌──────────────────┐   │
│    │              │            │         │  Native ObjC     │   │
│    │ Render Call  │◄───────────┼────────►│  Wrapper Layer   │   │
│    │  (AUv2/v3)   │   C-ABI    │         │                  │   │
│    │              │   calls    │         └────────┬─────────┘   │
│    └──────┬───────┘            │                  │             │
│           │                    │                  │ NSView      │
│           │ beamer_au_render() │         ┌────────▼─────────┐   │
│    ┌──────▼───────┐            │         │                  │   │
│    │ bridge.rs    │            │         │  WebView Window  │   │
│    │ RenderBlock  │            │         │   (WKWebView)    │   │
│    │ AuProcessor  │            │         │                  │   │
│    └──────────────┘            │         └──────────────────┘   │
│                                │                                │
└────────────────────────────────┴────────────────────────────────┘
```

**Why Hybrid?** Native Objective-C integrates naturally with Apple's frameworks, provides better debuggability with Apple's tools, and avoids the complexity of Rust FFI bindings for `AUAudioUnit` subclassing. The hybrid approach guarantees Apple compatibility while keeping all audio processing in Rust.

### Unified Core

Both formats share the same core traits and processing logic:

```
┌─────────────────────────────────────────────────────────────────┐
│                       beamer-core                               │
│  • Plugin trait (unprepared state)                              │
│  • AudioProcessor trait (prepared state)                        │
│  • Buffer, AuxiliaryBuffers, MidiBuffer                         │
│  • Parameters trait, ParameterStore                             │
│  • ProcessContext, Transport                                    │
└──────────────────────┬──────────────────┬───────────────────────┘
                       │                  │
         ┌─────────────▼──────┐  ┌────────▼─────────────┐
         │   beamer-vst3      │  │    beamer-au         │
         │                    │  │                      │
         │ • Vst3Processor<P> │  │ • AuProcessor<P>     │
         │ • COM interfaces   │  │ • C-ABI bridge       │
         │ • VST3 MIDI        │  │ • Native ObjC wrapper│
         │ • Factory          │  │ • UMP MIDI           │
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
│   ├── beamer-vst3/         # VST3 wrapper implementation
│   ├── beamer-au/           # Audio Unit wrapper implementation (macOS)
│   ├── beamer-macros/       # Proc macros (#[derive(Parameters)], #[derive(HasParameters)])
│   ├── beamer-utils/        # Shared utilities (zero deps)
│   └── beamer-webview/      # WebView per platform (Phase 2)
├── examples/
│   ├── gain/                # Audio effect example
│   ├── delay/               # Delay effect with tempo sync
│   ├── compressor/          # Dynamics compressor
│   ├── synth/               # Polyphonic synthesizer with MIDI CC emulation
│   └── midi-transform/      # MIDI effect example
└── xtask/                   # Build tooling (bundle, install)
```

### Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `beamer` | Facade crate, re-exports public API via `prelude` |
| `beamer-core` | Platform-agnostic traits (`HasParameters`, `Plugin`, `AudioProcessor`), buffer types, MIDI types, shared `PluginConfig` |
| `beamer-vst3` | VST3 SDK integration, COM interfaces, host communication, `Vst3Config` |
| `beamer-au` | Audio Unit (AUv2 and AUv3) integration via hybrid ObjC/Rust architecture, C-ABI bridge, `AuConfig` (macOS only) |
| `beamer-macros` | `#[derive(Parameters)]`, `#[derive(HasParameters)]`, `#[derive(EnumParameter)]` proc macros |
| `beamer-utils` | Internal utilities shared between crates (zero external deps) |
| `beamer-webview` | Platform-native WebView embedding (Phase 2) |

---

## Two-Phase Plugin Lifecycle

Beamer uses a type-safe two-phase initialization that eliminates placeholder values:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Plugin (Unprepared)                         │
│  • Created via Default::default()                               │
│  • Holds parameters and bus configuration                       │
│  • No sample rate or audio state                                │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
                                  │ prepare(config)
                                  │ [setupProcessing]
                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                   AudioProcessor (Prepared)                     │
│  • Created with real sample rate and buffer size                │
│  • Allocates DSP state (delay buffers, filter coefficients)     │
│  • Ready for process() calls                                    │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
                                  │ unprepare()
                                  │ [sample rate change]
                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Plugin (Unprepared)                         │
│  • Parameters preserved                                         │
│  • DSP state discarded                                          │
│  • Ready for prepare() with new config                          │
└─────────────────────────────────────────────────────────────────┘
```

### Why Two Phases?

Audio plugins need sample rate for buffer allocation, filter coefficients, and envelope timing—but the sample rate isn't known until the host calls `setupProcessing()`. The traditional pattern used placeholder values:

```rust
// ❌ Old pattern - placeholder values cause bugs
struct MyPlugin {
    sample_rate: f64,  // 44100.0 placeholder, overwritten later
    buffer: Vec<f64>,  // Allocated with wrong size!
}
```

Beamer's solution makes it impossible to process audio without valid configuration:

```rust
// ✅ New pattern - type system enforces correctness
impl Plugin for MyPlugin {
    type Setup = SampleRate;

    fn prepare(self, setup: SampleRate) -> MyProcessor {
        MyProcessor {
            sample_rate: setup.hz(),  // Real value from start!
            buffer: vec![0.0; (setup.hz() * 2.0) as usize],  // Correct size!
        }
    }
}
```

### Setup Types

| Type | Use Case | Value |
|------|----------|-------|
| `()` | Stateless plugins (gain, pan) | — |
| `SampleRate` | Most plugins (delay, filter, envelope) | `f64` via `.hz()` |
| `MaxBufferSize` | FFT, lookahead | `usize` |
| `MainOutputChannels` | Per-channel state | `u32` |
| `(A, B, ...)` | Combine multiple types | Tuples up to 5 elements |

### Trait Responsibilities

| Trait | State | Responsibilities |
|-------|-------|------------------|
| `HasParameters` | Both | Parameter access (`parameters()`, `parameters_mut()`) - supertrait of Plugin and AudioProcessor |
| `Plugin` | Unprepared | Bus configuration, MIDI mapping, `prepare()` transformation |
| `AudioProcessor` | Prepared | DSP processing, state persistence, MIDI processing, `unprepare()` |

### Parameter Ownership

Parameters are **owned** by both `Plugin` and `AudioProcessor`, moving between them during state transitions:

```
Plugin                          AudioProcessor
┌─────────────────────┐        ┌─────────────────────┐
│ parameters ─────────┼──────► │ parameters          │
└─────────────────────┘        └─────────────────────┘
       prepare() moves              unprepare() moves
       parameters →                 ← parameters back
```

This is the **type-state pattern**—a Rust idiom for encoding state machines at the type level. The same pattern appears in `std::fs::File`, builder APIs, and session types.

**Why ownership instead of shared references?**

1. **Zero overhead** — Direct field access: `self.parameters.gain.get()`
2. **No synchronization** — Owned data needs no Arc, Mutex, or atomics for internal access
3. **Clear lifecycle** — Parameters exist exactly where they're used
4. **Smoother mutation** — Smoothers advance state each sample; ownership makes this natural

**The `HasParameters` trait:**

Both `Plugin` and `AudioProcessor` implement `HasParameters` because the host needs parameter access in both states:
- Before `prepare()`: Host queries parameter info, user adjusts values
- After `prepare()`: Host automates parameters during playback

The `#[derive(HasParameters)]` macro generates accessor methods for any struct with a `#[parameters]` field:

```rust
#[derive(Default, HasParameters)]
pub struct MyPlugin {
    #[parameters]
    parameters: MyParameters,
}

#[derive(HasParameters)]
pub struct MyProcessor {
    #[parameters]
    parameters: MyParameters,
    // ... DSP state fields
}
```

---

## Plugin Configuration and Export

Beamer uses a **split configuration model** to separate format-agnostic metadata from format-specific identifiers.

### Configuration Architecture

```
┌────────────────────────────────────────────────────────────────┐
│                      beamer-core                               │
│                                                                │
│  PluginConfig (shared metadata)                               │
│  • name, vendor, version                                       │
│  • category, sub_categories                                    │
│  • url, email, has_editor                                      │
└────────────────────┬───────────────────────────────────────────┘
                     │
       ┌─────────────┴──────────────┐
       │                            │
       ▼                            ▼
┌──────────────────┐      ┌──────────────────┐
│   beamer-vst3    │      │    beamer-au     │
│                  │      │                  │
│  Vst3Config      │      │  AuConfig        │
│  • component_uid │      │  • component_type│
│  • controller_uid│      │  • manufacturer  │
│  • sysex_slots   │      │  • subtype       │
│  • sysex_buffer  │      │  • bus_config    │
└──────────────────┘      └──────────────────┘
```

### Example: Multi-Format Plugin

```rust
use beamer::prelude::*;
use beamer_vst3::{export_vst3, Vst3Config};
use beamer_au::{export_au, AuConfig, ComponentType, fourcc};

// Shared configuration (format-agnostic)
pub static CONFIG: PluginConfig = PluginConfig::new("My Gain")
    .with_vendor("My Company")
    .with_version(env!("CARGO_PKG_VERSION"))
    .with_sub_categories("Fx|Gain");

// VST3-specific configuration
#[cfg(feature = "vst3")]
const COMPONENT_UID: beamer::vst3::Steinberg::TUID =
    beamer::vst3::uid(0x12345678, 0x9ABCDEF0, 0xABCDEF12, 0x34567890);

#[cfg(feature = "vst3")]
pub static VST3_CONFIG: Vst3Config = Vst3Config::new(COMPONENT_UID);

// AU-specific configuration (macOS only)
#[cfg(feature = "au")]
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    fourcc!(b"Demo"),  // Manufacturer code (4 chars)
    fourcc!(b"gain"),  // Subtype code (4 chars)
);

// Export VST3 plugin
export_vst3!(CONFIG, VST3_CONFIG, MyPlugin);

// Export Audio Unit plugin (macOS only)
#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, MyPlugin);
```

### Factory Presets

Both export macros support an optional presets parameter for plugins that provide factory presets:

```rust
#[cfg(feature = "vst3")]
export_vst3!(CONFIG, VST3_CONFIG, GainPlugin, GainPresets);

#[cfg(feature = "au")]
export_au!(CONFIG, AU_CONFIG, GainPlugin, GainPresets);
```

If no presets type is specified, `NoPresets` is used automatically.

### Configuration Fields

**PluginConfig** (shared):
- `name` - Display name in DAW
- `vendor` - Company/developer name
- `version` - Semantic version string
- `category` - Main category ("Fx", "Instrument")
- `sub_categories` - Pipe-separated subcategories ("Dynamics|Compressor")
- `url`, `email` - Contact information
- `has_editor` - GUI enabled flag

**Vst3Config** (VST3-specific):
- `component_uid` - 128-bit unique identifier (TUID)
- `controller_uid` - Optional separate controller UID (for split architecture)
- `sysex_slots` - Number of SysEx output buffers
- `sysex_buffer_size` - Size of each SysEx buffer

**AuConfig** (AU-specific):
- `component_type` - AU type: `Effect`, `MusicDevice`, or `MidiProcessor`
- `manufacturer` - 4-character manufacturer code (FourCC)
- `subtype` - 4-character plugin subtype code (FourCC)
- `bus_config` - Optional custom bus configuration

### Why Split Configuration?

1. **Shared metadata** - Write plugin name, vendor, version once
2. **Format requirements** - VST3 needs UIDs, AU needs FourCC codes
3. **Conditional compilation** - AU export only compiles on macOS
4. **Future extensibility** - Easy to add CLAP, AAX without affecting core

### Building Multi-Format Plugins

Use `xtask` to build both formats:

```bash
# VST3 only (native architecture)
cargo xtask bundle my-plugin --vst3 --release

# AUv2 only (macOS, native architecture)
cargo xtask bundle my-plugin --auv2 --release

# AUv3 only (macOS, native architecture)
cargo xtask bundle my-plugin --auv3 --release

# All formats (macOS)
cargo xtask bundle my-plugin --vst3 --auv2 --auv3 --release

# Install to system plugin directories
cargo xtask bundle my-plugin --vst3 --auv2 --release --install

# Universal binary for distribution (x86_64 + arm64)
cargo xtask bundle my-plugin --vst3 --auv2 --arch universal --release
```

**Architecture options**: `--arch native` (default), `--arch universal`, `--arch arm64`, `--arch x86_64`

---

## Format-Specific Implementation Details

While both formats share the same `beamer-core` abstractions, they differ significantly in their platform APIs.

### VST3 Implementation

**Architecture**: COM-based (Component Object Model)
- Single `Vst3Processor<P>` class implements 15+ COM interfaces
- Uses combined component pattern (processor + controller in one class)
- Direct function pointer vtables for interface calls

**Key Files** (~3,800 lines):
- [processor.rs](crates/beamer-vst3/src/processor.rs) - Main wrapper (3,238 lines)
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

### Audio Unit Implementation

**Architecture**: Hybrid Objective-C/Rust
- AUv2: `AudioComponentPlugInInterface` with selector-based dispatch
- AUv3: `BeamerAuWrapper` native ObjC class (subclass of `AUAudioUnit`)
- Shared C-ABI bridge layer (`BeamerAuBridge.h` ↔ `bridge.rs`) with 40+ functions
- Uses type erasure (`AuPluginInstance` trait) for generic plugin support
- Render blocks call into Rust via `beamer_au_render()`
- Full feature parity with VST3 wrapper

**Key Files** (~6,500 lines total):

*Objective-C Layer:*
- [objc/BeamerAuWrapper.m](crates/beamer-au/objc/BeamerAuWrapper.m) - Native AUAudioUnit subclass (~700 lines)
- [objc/BeamerAuBridge.h](crates/beamer-au/objc/BeamerAuBridge.h) - C-ABI declarations (~400 lines)
- [build.rs](crates/beamer-au/build.rs) - ObjC compilation via `cc` crate

*Rust Layer:*
- [bridge.rs](crates/beamer-au/src/bridge.rs) - C-ABI implementations (~1,100 lines)
- [processor.rs](crates/beamer-au/src/processor.rs) - Plugin wrapper + f64 conversion (~650 lines)
- [render.rs](crates/beamer-au/src/render.rs) - RenderBlock + MIDI + parameter events (~1,400 lines)
- [lifecycle.rs](crates/beamer-au/src/lifecycle.rs) - State machine + prepare (~350 lines)
- [sysex_pool.rs](crates/beamer-au/src/sysex_pool.rs) - SysEx output pool (~120 lines)

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

### Comparison Table

| Feature | VST3 | Audio Unit |
|---------|------|------------|
| **Platform** | Windows, macOS, Linux | macOS only |
| **API Style** | COM (C++ style) | Hybrid ObjC/Rust via C-ABI |
| **Language** | Rust + vst3-sys | ObjC + Rust + cc crate |
| **Code Size** | ~3,800 lines (1 file) | ~6,500 lines (ObjC + Rust) |
| **MIDI Format** | VST3 Event union | UMP MIDI 1.0/2.0 |
| **MIDI Buffer** | 1024 events | 1024 events |
| **MidiCcState** | ✓ | ✓ |
| **MIDI Output** | ✓ | ✓ (instruments/MIDI effects only) |
| **SysEx Output** | ✓ (pool) | ✓ (pool) |
| **Parameter Sync** | Pull (COM methods) | Push (KVO callbacks) |
| **Param Automation** | Buffer-quantized + smoothing | Buffer-quantized + smoothing |
| **Audio Buffers** | `float**` arrays | `AudioBufferList` |
| **f64 Conversion** | Pre-allocated | Pre-allocated |
| **State Format** | Binary blob | NSDictionary |
| **Processor State** | ✓ | ✓ |
| **Bundle Type** | `.vst3` | `.component` (AUv2) / `.appex` (AUv3) |
| **Registration** | `GetPluginFactory()` | ObjC factory + module init |
| **Feature Parity** | Reference | ✓ Full parity |

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

The format wrappers are **thin translation layers** (~3,500-3,800 lines each) that adapt the platform API to `beamer-core` abstractions.

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

**ProcessBufferStorage**:
```rust
pub struct ProcessBufferStorage<S: Sample> {
    input_ptrs: Vec<*const S>,
    output_ptrs: Vec<*mut S>,
    aux_input_ptrs: Vec<Vec<*const S>>,
    aux_output_ptrs: Vec<Vec<*mut S>>,
}
```

- Pre-allocated in `setupProcessing()` based on plugin's declared bus configuration
- Capacity reserved for `MAX_CHANNELS` per bus, `MAX_BUSES` total
- `clear()` resets length to 0 without deallocating
- `push()` into reserved capacity never allocates

**Plugin-Declared Capacity** (VST3-specific):
```rust
pub static VST3_CONFIG: Vst3Config = Vst3Config::new(COMPONENT_UID)
    .with_sysex_slots(64)         // Default: 16
    .with_sysex_buffer_size(4096); // Default: 512 bytes
```

### Allocation Lifecycle

The buffer allocation flow ensures all memory is reserved before audio processing begins:

```
Plugin Load (creates Plugin in Unprepared state)
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
│   • Plugin::prepare(config) → AudioProcessor                │
│     - Plugin consumed, AudioProcessor created               │
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
│   • storage.clear() — sets len=0, no deallocation           │
│   • storage.push(ptr) — into reserved capacity, no alloc    │
│   • .take(MAX_CHANNELS) — bounds check even if host lies    │
│   • Build Buffer/AuxiliaryBuffers from pointers             │
│   • Call AudioProcessor::process()                          │
└─────────────────────────────────────────────────────────────┘
    │
    ▼ (on sample rate change)
┌─────────────────────────────────────────────────────────────┐
│ setupProcessing() with new config                           │
│   • AudioProcessor::unprepare() → Plugin                    │
│     - Parameters preserved, DSP state discarded             │
│   • Plugin::prepare(new_config) → AudioProcessor            │
│     - DSP state reallocated for new sample rate             │
└─────────────────────────────────────────────────────────────┘
```

**Key invariant**: After `setupProcessing()` succeeds, `process()` never allocates.

---

## Inspiration

| Project | |
|---------|---|
| [Tauri](https://tauri.app) | WebView integration, IPC patterns |
| [iPlug2](https://github.com/iPlug2/iPlug2) | C++ plugin framework reference |
| [JUCE](https://juce.com) | C++ plugin framework reference |
| [nih-plug](https://github.com/robbert-vdh/nih-plug) | Rust plugin framework reference |
| [Coupler](https://github.com/coupler-rs/coupler) | VST3 Rust bindings (dependency) |
| [VST3 SDK](https://github.com/steinbergmedia/vst3sdk) | VST3 specification and reference |
| [Apple AUv3](https://developer.apple.com/documentation/audiotoolbox/audio_unit_v3_plug-ins) | Audio Unit v3 specification |
