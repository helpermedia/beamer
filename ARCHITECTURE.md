# Beamer Architecture

This document describes the high-level architecture of Beamer, a Rust framework for building audio plugins with WebView-based GUIs.

For detailed API documentation, see [docs/REFERENCE.md](docs/REFERENCE.md).
For building, installing and testing plugins, see [docs/BUILDING.md](docs/BUILDING.md).
For the WebView GUI system, see [docs/WEBVIEW.md](docs/WEBVIEW.md).
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
- **Cross-platform**: macOS today (Apple Silicon and Intel, tested on Apple Silicon); Windows planned (the architecture is platform-agnostic, but the WebView backend is macOS-only so far)

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

**Why Hybrid?** Native Objective-C integrates naturally with Apple's frameworks, provides better debuggability with Apple's tools and avoids the complexity of Rust FFI bindings for `AUAudioUnit` subclassing. The hybrid approach guarantees Apple compatibility while keeping all audio processing in Rust.

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
│    │  (DSP code)  │  shared    │         │                  │   │
│    │              │  params    │         └────────┬─────────┘   │
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
│  • Model trait (what every thread shares)                       │
│  • Processor trait (created for an audio configuration)         │
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

A host saves and loads the state, asks for latency and tail and resets a plugin from its own threads while the audio thread processes. While audio runs, the processor belongs to the audio thread, so the wrappers never hand it to another thread:

- The state is the model's. Parameter values are atomic and `State` fields are replaced as a whole, so the wrappers save and load it through a shared reference.
- The audio thread publishes latency and tail for every block. Host threads read the published values.
- A request that belongs to the audio thread (a processor reset, the smoother reset after a state load) is a flag the audio thread acts on at the start of its next block.

---

## Crate Structure

```
beamer/
├── crates/
│   ├── beamer/              # Main crate (re-exports)
│   ├── beamer-core/         # Plugin traits, MIDI types, buffers
│   ├── beamer-macros/       # Proc macros (#[beamer::export], #[beamer::model], #[beamer::group], #[derive(EnumParameter)])
│   ├── beamer-utils/        # Shared utilities (zero deps)
│   ├── beamer-au/           # Audio Unit wrapper implementation (macOS)
│   ├── beamer-vst3/         # VST3 wrapper implementation
│   └── beamer-webview/      # WebView per platform
├── examples/
│   ├── gain/                # Audio effect example
│   ├── compressor/          # Dynamics compressor
│   ├── equalizer/           # 3-band parametric EQ
│   ├── delay/               # Delay effect with tempo sync
│   ├── synthesizer/         # Polyphonic synthesizer with MIDI CC emulation
│   ├── drums/               # Drum synthesizer with multi-output buses
│   ├── midi-transform/      # MIDI effect example
│   └── webview-demo/        # WebView GUI with React
└── xtask/                   # Build tooling (bundle, install)
```

### Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `beamer` | Facade crate, re-exports public API via `prelude` |
| `beamer-core` | Platform-agnostic traits (`Model`, `Processor`, `Saved`, `WebViewEvents`), buffer types, MIDI types, `Config` |
| `beamer-macros` | Attribute macros: `#[beamer::model]`, `#[beamer::group]` and `#[beamer::export]` (reads Config.toml). Derive macro: `#[derive(EnumParameter)]` |
| `beamer-utils` | Internal utilities shared between crates (zero external deps) |
| `beamer-au` | Audio Unit (AUv2 and AUv3) integration via hybrid ObjC/Rust architecture, C-ABI bridge (macOS only) |
| `beamer-vst3` | VST3 SDK integration, COM interfaces, host communication |
| `beamer-webview` | Platform-native WebView embedding |

---

## Plugin Lifecycle

Beamer uses type-safe initialization via `prepare()` that eliminates placeholder values:

```
┌─────────────────────────────────────────────────────────────────┐
│          Model (lives as long as the plugin instance)           │
│  • Created via Default::default(), kept in one Arc              │
│  • Holds the parameters, State fields and the web UI events     │
│  • Answers the host about buses and MIDI                        │
│  • No audio state                                               │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
                                  │ model.prepare(setup), with a
                                  │ clone of the Arc [setupProcessing]
                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Processor (Prepared)                          │
│  • Created with real sample rate and buffer size                │
│  • Shares the model through its own Arc                         │
│  • Allocates DSP state (delay buffers, filter coefficients)     │
│  • Ready for process() calls                                    │
└─────────────────────────────────┬───────────────────────────────┘
                                  │
                                  │ dropped
                                  │ [audio setup change, resources released]
                                  ▼
┌─────────────────────────────────────────────────────────────────┐
│          Model (still the same one)                             │
│  • Parameters and State fields untouched                        │
│  • DSP state gone with the Processor                            │
│  • Ready for prepare() with new setup                           │
└─────────────────────────────────────────────────────────────────┘
```

### Why This Design?

Audio plugins need sample rate for buffer allocation, filter coefficients and envelope timing, but the sample rate isn't known until the host calls `setupProcessing()`. The `prepare()` design ensures DSP state is only created with valid configuration.

### Design Rationale

Beamer's design follows the Rust principle of **making invalid states unrepresentable**. A `Processor` can only come from `prepare()`, which receives the real audio configuration, so there is no processor that has not been set up.

The `Processor` type is always fully initialized, so `process()` code is clean:

```rust
impl Processor for DelayProcessor {
    fn process(&mut self, buffer: &mut Buffer, ...) {
        // self.sample_rate is guaranteed valid.
        // self.buffer is guaranteed allocated.
        // No Option<T>, no .expect(), no placeholder checks.
    }
}
```

### Two-Struct Pattern

A Beamer plugin is two structs: the **Model**, which every thread shares, and the **Processor**, which the audio thread owns.

1. **`*Model`** - Marked with `#[beamer::model]` and implementing `Model`: the parameters, `State` fields and everything else the threads share, how the host sees the plugin (buses, MIDI) and how it creates the processor
2. **`*Processor`** - Created by `prepare()`, implements `Processor` and shares the model

Nested groups of parameters are structs marked with `#[beamer::group]`. See [Two-Struct Pattern](docs/REFERENCE.md#two-struct-pattern) for a complete example and [Parameters](docs/REFERENCE.md#parameters) for the parameter attributes.

**Plugins with DSP state** add fields to the Processor:

```rust
struct DelayProcessor {
    model: Arc<DelayModel>,
    // Always valid.
    sample_rate: f64,
    // Always allocated.
    buffer: Vec<f64>,
}
```

### Setup Types

`prepare()` receives only what the plugin asks for through its `Setup` type: `()` for stateless plugins, `SampleRate` for most others or a tuple such as `(SampleRate, MaxBufferSize)`. See [Model Trait](docs/REFERENCE.md#model-trait) for all setup types.

### Trait Responsibilities

| Trait | Lives | Responsibilities |
|-------|-------|------------------|
| `Model` | As long as the plugin instance | Bus configuration, MIDI mapping, answers to the web UI, creating a Processor with `prepare()` |
| `Parameters`, `ParameterGroups`, `Saved`, `WebViewEvents`, `Default` | With the model | Generated by `#[beamer::model]`: parameter access, groups, `State` fields, saving and loading, the event queue of the web UI plus `Default` (unless `no_default`). Every `Parameters` is a `ParameterStore` for the host as well |
| `Processor` | While audio resources are allocated | DSP processing, MIDI processing |

### The Shared Model

The model is one struct in an `Arc`. The framework holds it for the life of the plugin instance and hands a clone of the `Arc` to every Processor that `prepare()` creates:

```
Framework                        Processor
┌─────────────────────┐        ┌─────────────────────┐
│ Arc<Model> ─────────┼───┬────┼─ Arc<Model>         │
└─────────────────────┘   │    └─────────────────────┘
                          ▼
                 ┌─────────────────┐
                 │      Model      │  also used by the host
                 │   (atomics,     │  and the UI
                 │  State fields)  │
                 └─────────────────┘
```

**Why shared?**

A host reads and sets parameters, saves and loads state and opens the editor from its own threads while the audio thread processes. The model is therefore used by several threads at the same time whatever the plugin does, and the type says so:

1. **One place**: The model never moves. What the host sets is what the audio thread reads, before and after `prepare()`.
2. **No locks on the audio thread**: Values and smoother state are atomics, so every thread works through a shared reference. Access stays a direct field access: `self.model.gain.get()`.
3. **A lasting model**: `prepare()` gets a clone of the `Arc`, so the model keeps answering the host (buses, MIDI mapping) while audio runs. Host threads may use it at the same time, so every method of `Model` other than `prepare()` takes `&self` and a model is `Sync`.
4. **Clear roles**: What other threads need to see lives in the model. Everything else in the Processor belongs to the audio thread alone.
5. **Room for more than numbers**: Data that is not a parameter (a sample path, a table, a setting) is a `State` or `Handover` field of the model. Host and UI threads replace such a value as a whole, and the audio thread reads it through a reader without waiting.

Smoothing methods take `&self` like the value accessors. They are meant for the audio thread: a second thread advancing a smoother at the same time is memory-safe but disturbs the ramp.

---

## Plugin Configuration and Export

Beamer uses **TOML-based configuration** with compile-time code generation to eliminate boilerplate.

### Configuration Architecture

Plugin metadata is declared in `Config.toml` in the plugin crate root. Factory presets are Rust code: `Model::PRESETS`, a `const` slice of `Preset` values. The `#[beamer::export]` attribute macro reads `Config.toml` at compile time and generates:
- The `CONFIG` static with all metadata
- Format entry points via `export_plugin!` macro

```
Plugin Crate Root
├── Cargo.toml
├── Config.toml              # Required: plugin metadata
└── src/
    └── lib.rs
        ├── Model struct with #[beamer::export] and #[beamer::model]
        ├── Factory presets (optional, Model::PRESETS)
        └── Processor struct
```

See [Plugin Configuration](docs/REFERENCE.md#plugin-configuration) for all `Config.toml` fields and [Factory Presets](docs/REFERENCE.md#factory-presets) for presets.

### Why TOML Configuration?

1. **Cleaner code** - No manual CONFIG statics or export macros
2. **Easier to modify** - Change metadata without touching Rust code
3. **Consistent format** - Same structure across all plugins

### Generated Code

The `#[beamer::export]` macro generates:

1. **CONFIG static**: `pub static CONFIG: Config = Config::new(...).with_vendor(...).with_version(...)`
2. **Plugin exports**: Calls `export_plugin!(CONFIG, ModelStruct)`

The macro uses `include_str!()` to track file dependencies, so Cargo automatically rebuilds when Config.toml changes.

### VST3 ID Generation

VST3 requires a unique 128-bit identifier. By default, Beamer derives it from your `manufacturer_code` and `plugin_code` using FNV-1a hashing. This ensures consistent IDs across builds.

To use an explicit UUID (e.g., for backward compatibility):
```toml
vst3_id = "12345678-9ABC-DEF0-ABCD-EF1234567890"
```

### Building

Plugins are built, bundled and installed with `cargo xtask bundle`. See [BUILDING.md](docs/BUILDING.md).

### WebView Assets

Plugins with a `webview/` directory embed their web UI at compile time: the `#[beamer::export]` macro scans `webview/dist/` (or `webview/` itself for plain HTML projects), embeds every file via `include_bytes!()` and enables the GUI. See [Asset Embedding](docs/WEBVIEW.md#asset-embedding) for the detection rules.

The web assets (HTML, CSS, JS) are served at runtime through a custom URL scheme (`beamer://`) rather than a local HTTP server.

`beamer-webview` uses platform APIs directly (`objc2` + `objc2-web-kit` for WKWebView) rather than a wrapper like `wry`. Audio plugin hosts provide their own parent views (`NSView`, `HWND`) that the WebView must attach to, and the AU wrapper needs a C-ABI bridge for its generated ObjC code. A high-level wrapper would add indirection without solving either problem.

---

## Format-Specific Implementation Details

While both formats share the same `beamer-core` abstractions, they differ significantly in their platform APIs.

### Audio Unit Implementation

**Architecture**: Hybrid Objective-C/Rust
- AUv2: `AudioComponentPlugInInterface` with selector-based dispatch
- AUv3: `Beamer<Name>AuWrapper_<codes>`, a native ObjC subclass of `AUAudioUnit` generated per plugin
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
- [lifecycle.rs](crates/beamer-au/src/lifecycle.rs) - Model and Processor of an instance, prepare
- [transport.rs](crates/beamer-au/src/transport.rs) - Host transport conversion

`processor.rs` and `render.rs` are separate because the AU render callback crosses the FFI boundary from Objective-C: `render.rs` holds the `RenderBlock` that `beamer_au_render()` calls on the audio thread and that splits the buffer, while `processor.rs` holds the type-erased `AuProcessor<P>` wrapper (lifecycle, parameters and the per-sub-block `process()` call) that both the render block and the bridge functions use.

**MIDI**: UMP MIDI 1.0/2.0 → `beamer-core::MidiEvent`
- Universal MIDI Packet format (32-bit packets)
- Supports Note On/Off, CC, Pitch Bend, Channel Pressure, SysEx
- 1024 event buffer (matches VST3)
- `MidiCcState` tracking for CC/pitch bend queries
- MIDI output (channel messages and SysEx) is handed to the host during the render call

**Parameters**: Push model via KVO (Key-Value Observing)
- `AUParameterTree` built from `ParameterStore`
- `implementorValueObserver` - Host → plugin changes
- `implementorValueProvider` - Plugin → host reads
- Automation via `AURenderEventParameter`/`ParameterRamp` (a ramp as a change to its end value; sample-accurate with `sample_accurate_automation`, see below)

**State**: NSDictionary with NSData
- The state is the model with its parameters and `State` fields (`Saved::save_state`/`load_state`), prepared or not
- Saved and loaded without the plugin mutex, also while audio renders
- Compatible with VST3 format

**Real-time Safety**:
- Pre-allocated f64↔f32 conversion buffers (main + aux buses)
- Pre-allocated MIDI/SysEx buffers
- No heap allocation in render path

### VST3 Implementation

**Architecture**: COM-based (Component Object Model)
- Single `Vst3Processor<P>` class implements over a dozen COM interfaces
- Uses combined component pattern (processor + controller in one class)
- Direct function pointer vtables for interface calls

**Key Files**:
- [processor.rs](crates/beamer-vst3/src/processor.rs) - Main wrapper
- [factory.rs](crates/beamer-vst3/src/factory.rs) - COM factory registration
- [export.rs](crates/beamer-vst3/src/export.rs) - Platform entry points
- [webview.rs](crates/beamer-vst3/src/webview.rs) - WebView editor (`IPlugView`)
- [component_handler.rs](crates/beamer-vst3/src/component_handler.rs) - The host's component handler, shared by the controller and its editors

**MIDI**: VST3 `Event` union → `beamer-core::MidiEvent`
- 16+ event types (NoteOn, NoteOff, MIDI CC, PolyPressure, etc.)
- Supports VST3-specific events (NoteExpression, Chord, Scale)
- Legacy MIDI CC output for host compatibility

**Parameters**: Pull model via COM methods
- `getParameterInfo()` - Host queries parameter metadata
- `setParamNormalized()` - Host sets parameter value
- `getParamNormalized()` - Host reads parameter value

**State**: Binary blob via `IBStream`, the same bytes as in the AU: the model with its parameters and `State` fields

### Comparison Table

| Feature | Audio Unit | VST3 |
|---------|------------|------|
| **Platform** | macOS only | macOS; Windows planned (UI backend macOS-only so far) |
| **API Style** | Hybrid ObjC/Rust via C-ABI | COM (C++ style) |
| **Language** | ObjC + Rust | Rust + vst3 crate |
| **MIDI Format** | UMP MIDI 1.0/2.0 | VST3 Event union |
| **MIDI Buffer** | 1024 events | 1024 events |
| **MidiCcState** | ✓ | ✓ |
| **MIDI Output** | ✓ (instruments/MIDI effects only) | ✓ |
| **SysEx Output** | ✓ | ✓ (pool) |
| **Parameter Sync** | Push (KVO callbacks) | Pull (COM methods) |
| **Param Automation** | Per block, or sample-accurate (sub-blocks) with `sample_accurate_automation` | Per block, or sample-accurate (sub-blocks) with `sample_accurate_automation` |
| **Audio Buffers** | `AudioBufferList` | `float**` arrays |
| **f64 Conversion** | Pre-allocated | Pre-allocated |
| **State Format** | NSDictionary | Binary blob |
| **State Save/Load** | ✓ | ✓ |
| **Bundle Type** | `.component` (AUv2) / `.appex` (AUv3) | `.vst3` |
| **Registration** | ObjC factory + module init | `GetPluginFactory()` |
| **Feature Parity** | ✓ Full parity | Reference |

The format wrappers are thin translation layers: all DSP, parameter, MIDI event, buffer and transport logic lives in `beamer-core`, and only the host-facing API code differs per format.

**Parameter automation.** By default both wrappers apply the parameter changes of a host block before one `process()` call for the whole block, so a change lands up to a block early and the parameter's `Smoother` hides the step. A plugin that sets `sample_accurate_automation = true` in `Config.toml` gets each change at its sample: the wrapper runs the block as sub-blocks that start at the changes, with the loop in `beamer_core::sub_blocks` that both formats share. Each sub-block gets its own `process_midi()` and `process()` call, with its MIDI relative to its start and the transport moved to its first sample. A ramp's end value applies at the start of the ramp, for the smoother to move towards: an AU ramp event, or a VST3 queue segment between two points. A VST3 segment of one sample is a jump (sent as the old value and the new value one sample later), whose new value applies at its point, and a point that leaves the parameter at the value it holds is left out, so a jump splits a block once and a ramp across a block not at all. A discrete parameter has no smoother, so a VST3 ramp changes it at each sample where the line reaches the next step (`ParameterRef::is_discrete`). MIDI at or after the end of a block reaches the last sub-block at its last sample. It matters for parameters read at events, such as a setting applied per note, and costs extra `process()` calls while automation moves.

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
- Calls meant for host threads panic inside `process()` in debug builds: `get()`, `set()` and `reader()` of a `Handover` or `State`, `reset()`, `save()` and `load()` of a `State` and `WebViewEventQueue::emit()`. They lock, allocate or free memory. The audio thread reads through a `HandoverReader`, which does none of that. The wrappers mark the process call with a `ProcessScope`, and in release builds the check compiles to nothing

### Deterministic Bus Limits

**Guarantee**: Channel and bus counts are bounded at compile time.

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_CHANNELS` | 32 | Supports up to 22.2 surround and Dolby Atmos 9.1.6 |
| `MAX_BUSES` | 16 | Main + sidechain + 14 aux buses |
| `MAX_AUX_BUSES` | 15 | Auxiliary buses (total minus main) |

**Enforcement**:
- `CachedBusConfig::validate()` checks the model's bus configuration against these constants before the Processor is created: VST3 `setupProcessing()` returns `kResultFalse` and logs an error, AU `allocateRenderResources` returns `kAudioUnitErr_FormatNotSupported`
- `validate_speaker_arrangement()` rejects invalid host arrangements in `setBusArrangements()`

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
- Config-based allocation: pointer capacity, scratch and internal buffers follow the actual channel counts, not the worst case
- Lazy aux allocation: no heap allocation for plugins without aux buses
- Internal output buffers for every plugin with main outputs (hosts may provide null pointers)
- Scratch memory per input and output channel, for hosts that give overlapping buffers
- `clear()` resets length to 0 without deallocating
- `push()` into reserved capacity never allocates
- `prepare_in_place()` runs once per process call, before the sub-blocks: each main output channel gets its input (or silence), and a channel that would overlap another gets scratch, so the plugin processes the main bus in place and its slices never overlap

**Plugin-Declared Capacity** (configurable via Config.toml):
```toml
# Config.toml
sysex_slots = 64 # Default: 16
sysex_buffer_size = 4096 # Default: 512 bytes
```

### Allocation Lifecycle

The buffer allocation flow ensures all memory is reserved before audio processing begins:

```
Plugin Load (creates the Model)
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
│   • CachedBusConfig::validate()                             │
│     - Buses ≤ MAX_BUSES, channels per bus ≤ MAX_CHANNELS    │
│     - Return kResultFalse + log if exceeded                 │
│   • model.prepare(setup) → Processor                        │
│     - Model stays, Processor created                        │
│     - DSP state allocated with real sample rate             │
│   • ProcessBufferStorage::allocate_from_config()            │
│     - main_inputs / main_outputs.reserve(main_channels)     │
│     - aux_inputs/aux_outputs[i].reserve(aux_channels[i])    │
│   • All Vecs now have capacity, length = 0                  │
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
    ▼ (on audio setup change)
┌─────────────────────────────────────────────────────────────┐
│ setupProcessing() with new setup                            │
│   • The Processor is dropped                                │
│     - Parameters untouched, DSP state discarded             │
│   • model.prepare(new_setup) → Processor                    │
│     - DSP state reallocated for the new setup               │
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
