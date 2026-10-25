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
│                                │         │   (WKWebView)    │   │
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
| **Audio Thread** | DSP processing, buffer handling, factory presets that arrive with the audio (VST3 program change) | Real-time safe: no allocations, no blocking locks (the AU render only tries its locks and skips or refuses the call when one is held), no syscalls |
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
│  • Answers the host about buses and MIDI mapping                │
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
                                  │ [audio setup or accepted buses change, resources released]
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

A Beamer plugin is two structs: the **Model** that every thread shares and the **Processor** that the audio thread owns.

1. **`*Model`** - Marked with `#[beamer::model]` and implementing `Model`: the parameters, `State` fields and everything else the threads share, how the host sees the plugin (buses, MIDI mapping) and how it creates the processor. MIDI input and output are declared in `Config.toml`
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

A host reads and sets parameters, saves and loads state and opens the editor from its own threads while the audio thread processes. The model is therefore used by several threads at the same time whatever the plugin does. The type says so:

1. **One place**: The model never moves. What the host sets is what the audio thread reads, before and after `prepare()`.
2. **No locks on the audio thread**: Values and smoother state are atomics, so every thread works through a shared reference. Access stays a direct field access: `self.model.gain.get()`.
3. **A lasting model**: `prepare()` gets a clone of the `Arc`, so the model keeps answering the host (buses, MIDI mapping) while audio runs. Host threads may use it at the same time, so every method of `Model` other than `prepare()` takes `&self` and a model is `Sync`.
4. **Clear roles**: What other threads need to see lives in the model. Everything else in the Processor belongs to the audio thread alone.
5. **Room for more than numbers**: Data that is not a parameter (a sample path, a table, a setting) is a `State` or `Handover` field of the model. Host and UI threads replace such a value as a whole; the audio thread reads it through a reader without waiting.

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

`beamer-webview` uses platform APIs directly (`objc2` + `objc2-web-kit` for WKWebView) rather than a wrapper like `wry`. Audio plugin hosts provide their own parent views (`NSView`, `HWND`) that the WebView must attach to. The AU wrapper needs a C-ABI bridge for its generated ObjC code. A high-level wrapper would add indirection without solving either problem.

---

## Format-Specific Implementation Details

While both formats share the same `beamer-core` abstractions, they differ significantly in their platform APIs.

### Audio Unit Implementation

**Architecture**: Hybrid Objective-C/Rust
- AUv2: `AudioComponentPlugInInterface` with selector-based dispatch
- AUv3: `Beamer<Name>AuWrapper_<codes>`, a native ObjC subclass of `AUAudioUnit` generated per plugin
- Shared C-ABI bridge layer (`BeamerAuBridge.h` and `bridge.rs`) with 40+ functions
- Uses type erasure (`AuPluginInstance` trait) for generic plugin support
- Render blocks call into Rust via `beamer_au_render()`
- Full feature parity with VST3 wrapper

**Key Files**:

*Objective-C Layer:*
- [objc/BeamerAuBridge.h](crates/beamer-au/objc/BeamerAuBridge.h) - C-ABI declarations
- [xtask/src/au_codegen/auv3_wrapper.m](xtask/src/au_codegen/auv3_wrapper.m) - AUv3 wrapper template (generated per-plugin by xtask)
- [xtask/src/au_codegen/auv2_wrapper.c](xtask/src/au_codegen/auv2_wrapper.c) - AUv2 wrapper template (generated per-plugin by xtask)
- [xtask/src/au_codegen/auv3_extension.m](xtask/src/au_codegen/auv3_extension.m) and [auv3_extension_gui.m](xtask/src/au_codegen/auv3_extension_gui.m) - AUv3 extension entry (without and with a GUI)
- [build.rs](crates/beamer-au/build.rs) - Framework linking (ObjC compilation is handled by xtask)

*Rust Layer:*
- [bridge.rs](crates/beamer-au/src/bridge.rs) - C-ABI implementations
- [instance.rs](crates/beamer-au/src/instance.rs) - The type-erased `AuPluginInstance` trait
- [processor.rs](crates/beamer-au/src/processor.rs) - Plugin wrapper + f64 conversion
- [render.rs](crates/beamer-au/src/render.rs) - RenderBlock + MIDI + parameter events
- [lifecycle.rs](crates/beamer-au/src/lifecycle.rs) - Model and Processor of an instance, prepare
- [transport.rs](crates/beamer-au/src/transport.rs) - Host transport conversion
- [shared_state.rs](crates/beamer-au/src/shared_state.rs) - What host threads and the render call exchange without a lock
- [buffer_storage.rs](crates/beamer-au/src/buffer_storage.rs) - Collecting the pointers of an `AudioBufferList` (`ProcessBufferStorageAuExt`)

`processor.rs` and `render.rs` are separate because the AU render callback crosses the FFI boundary from Objective-C: `render.rs` holds the `RenderBlock` that `beamer_au_render()` calls on the audio thread and that splits the buffer, while `processor.rs` holds the type-erased `AuProcessor<P>` wrapper (lifecycle, parameters and the per-sub-block `process()` call) that both the render block and the bridge functions use.

**MIDI**: AU MIDI events to `beamer-core::MidiEvent`
- Input: MIDI 1.0 channel messages, from legacy MIDI events and from UMP event lists (MIDI 1.0 channel voice packets). No SysEx or MIDI 2.0 input yet
- 1024 event buffer (`MAX_MIDI_EVENTS`, as in VST3)
- `MidiCcState` tracking for CC/pitch bend queries
- MIDI output (channel messages and SysEx) is handed to the host during the render call

**Parameters**: Push model
- AUv3: `AUParameterTree` built from `ParameterStore`, with implementor blocks
  - `implementorValueObserver` - Changes from the host to the plugin
  - `implementorValueProvider` - Reads of the plugin by the host
- AUv2: `GetParameter`, `SetParameter` and `ScheduleParameters`. Changes go through a lock-free event queue into the next render call
- Automation via `AURenderEventParameter`/`ParameterRamp` (a ramp as a change to its end value; sample-accurate with `sample_accurate_automation`, see below)

**State**: NSDictionary with NSData
- The state is the model with its parameters and `State` fields (`Saved::save_state`/`load_state`), prepared or not
- Saved and loaded without the plugin mutex, also while audio renders
- The same state format as VST3

**Real-time Safety**:
- Pre-allocated f64/f32 conversion buffers (main + aux buses)
- Pre-allocated MIDI/SysEx buffers
- No heap allocation in render path

**Render**:
- In place: the AUv2 reports `kAudioUnitProperty_InPlaceProcessing` (global scope) and the AUv3 `canProcessInPlace` only for a main input with as many channels as the main output. With matching channel counts the main input is pulled straight into the output buffers
- Several output buses: the host calls the render once per output bus. Bus 0 renders everything and caches the aux outputs; a call for another bus copies from the cache of the same sample time
- Aux inputs are pulled through the pull block into memory of the wrapper's own, one buffer per host channel
- Host threads and the render: the render holds a read lock on the render block, so allocating and deallocating wait for a running render. The AUv2 shim takes MIDI and parameter events through lock-free rings, gives the render thread its own copy of the host callbacks and refuses element count and maximum frame changes while initialized. The AUv3 hands the host's blocks to the render and keeps replaced ones alive until deallocation

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

**MIDI**: VST3 `Event` union to `beamer-core::MidiEvent`
- 16+ event types (NoteOn, NoteOff, MIDI CC, PolyPressure, etc.)
- Supports VST3-specific events (NoteExpression, Chord, Scale)
- Legacy MIDI CC output for host compatibility

**Parameters**: Pull model via COM methods
- `getParameterInfo()` - Host queries parameter metadata
- `setParamNormalized()` - Host sets parameter value
- `getParamNormalized()` - Host reads parameter value

**State**: Binary blob via `IBStream` in the same state format as the AU: the model with its parameters and `State` fields, plus the selected program when the plugin has factory presets

**Factory presets**: A program change parameter and an `IUnitInfo` program list. A program change that arrives with the audio is applied in `process()`, on the audio thread

### Comparison Table

| Feature | Audio Unit | VST3 |
|---------|------------|------|
| **Platform** | macOS only | macOS; Windows planned (UI backend macOS-only so far) |
| **API Style** | Hybrid ObjC/Rust via C-ABI | COM (C++ style) |
| **Language** | ObjC + Rust | Rust + vst3 crate |
| **MIDI Format** | MIDI 1.0 (legacy events and UMP) | VST3 Event union |
| **MIDI Buffer** | 1024 events | 1024 events |
| **MidiCcState** | ✓ | ✓ |
| **MIDI Output** | ✓ (with `midi_output`, the default for MIDI effects) | ✓ (with `midi_output`) |
| **SysEx Output** | ✓ | ✓ (pool) |
| **Parameter Sync** | Push (AUv3 implementor blocks, AUv2 event queue) | Pull (COM methods) |
| **Param Automation** | Per block, or sample-accurate (sub-blocks) with `sample_accurate_automation` | Per block, or sample-accurate (sub-blocks) with `sample_accurate_automation` |
| **Bus Switching** | AUv2: element count or 0-channel format, at Initialize; AUv3: buses stay on | `activateBus()`, at `setupProcessing()` or `setActive(true)` |
| **Audio Buffers** | `AudioBufferList` | `float**` arrays |
| **f64 Conversion** | Pre-allocated | Pre-allocated |
| **State Format** | NSDictionary | Binary blob |
| **State Save/Load** | ✓ | ✓ |
| **Bundle Type** | `.component` (AUv2) / `.appex` (AUv3) | `.vst3` |
| **Registration** | ObjC factory + module init | `GetPluginFactory()` |
| **Feature Parity** | ✓ Full parity | Reference |

The format wrappers are translation layers: the shared logic (buffers and their in-place preparation, sub-blocks, bus negotiation, state, MIDI and transport types) lives in `beamer-core`. The wrappers translate host events and buffers to it.

**Parameter automation.** By default both wrappers apply the parameter changes of a host block before one `process()` call for the whole block, so a change lands up to a block early and the parameter's `Smoother` hides the step. A plugin that sets `sample_accurate_automation = true` in `Config.toml` gets each change at its sample: the wrapper runs the block as sub-blocks that start at the changes, with the loop in `beamer_core::sub_blocks` that both formats share. Each sub-block gets its own `process_midi()` and `process()` call, with its MIDI relative to its start and the transport moved to its first sample. A ramp's end value applies at the start of the ramp, for the smoother to move towards: an AU ramp event, or a VST3 queue segment between two points. A VST3 segment of one sample is a jump (sent as the old value and the new value one sample later), whose new value applies at its point. A point that leaves the parameter at the value it holds is left out, so a jump splits a block once and a ramp across a block not at all. A discrete parameter has no smoother, so a VST3 ramp changes it at each sample where the line reaches the next step (`ParameterRef::is_discrete`). MIDI at or after the end of a block reaches the last sub-block at its last sample. It matters for parameters read at events (such as a setting applied per note) and costs extra `process()` calls while automation moves.

**Bus switching.** A host switches aux buses on and off. The plugin is prepared with the ones it accepts (`ActiveBuses`, `Model::supports_buses`). The negotiation is in `beamer_core::active_buses`, shared by both wrappers: a `BusNegotiation` per instance records what the host asked for (on or off and a `ChannelSet` per bus), keeps the main buses on, checks the request against the model's `LAYOUTS`, asks the plugin and keeps the last combination it accepted when no layout allows the request or the plugin refuses. `channel_layouts::nearest` gives the buses of the listed layout nearest to a request, for a format that suggests one: the VST3 wrapper maps each speaker arrangement of `setBusArrangements()` to a `ChannelSet`, takes the sets when a layout allows them and `supports_buses` accepts them; otherwise it applies the nearest layout the plugin supports (`channel_layouts::nearest_with`) and returns `kResultFalse`, so the host reads it back with `getBusArrangement()`. Host output channels the plugin does not render are silent. The AU bridge serves both AU wrappers: one channel capability per layout, a check of each stream format against the layouts and the layout tags of the listed sets per bus. The AUv3 gives its buses formats with the layout tag of the default set, reports the tags per bus (`supportedChannelLayoutTags`) and checks each bus format on its own in `shouldChangeToFormat` (while render resources are allocated a bus only takes the same channels in the same order again). A format of discrete channels in order, which Apple's bridge for AUv2 hosts makes of a stream format with more than two channels, goes by its channel count. A host's channel layout (tag, bitmap or descriptions) is expanded into channel labels, which name a set in any order; where the host orders the channels otherwise (Apple's 7.x has the side pair before the rear pair), the render block gets a channel order per bus and collects the host's buffers in the set's order. The plugin sees the same channel order in every format. A plugin that would keep other main channel counts than the host's refuses the formats (`kAudioUnitErr_FormatNotSupported`). Per bus, `BusRouting` says how the host's buffers meet the plugin's: connected, off for both or on for one side only. The wrapper bridges the last two with silence, memory that is dropped or nothing. The VST3 wrapper records `activateBus()` and prepares the plugin again at `setupProcessing()` or `setActive(true)` when the accepted buses changed. In `process()` it gives each bus the plugin has on its prepared channel count and leaves out the buses it has off. The AU bridge negotiates for both AU formats when render resources are allocated: the AUv2 shim sends every declared bus with 0 channels for one the host removed (element count) or switched off (0-channel format). The render block pulls only the inputs both sides have on and silences the aux outputs the plugin refused. An AUv3 sends every bus on: the `enabled` flag of its buses follows the host's input connections, which change while audio runs, so it is set from the declaration and never read. An input without a source is `None` for that render call in both AU formats.

---

## Operational Guarantees

This section documents the invariants that Beamer enforces. These are API contracts that plugin authors can rely on.

### Real-Time Safety

**Guarantee**: No heap allocations occur on the audio thread during `process()`.

| Component | Mechanism |
|-----------|-----------|
| `Buffer<S>` | Stack arrays: `[Option<&mut [S]>; MAX_CHANNELS]` for the main channels (in place) and `[Option<&[S]>; MAX_CHANNELS]` for unpaired inputs |
| `AuxiliaryBuffers<S>` | Stack-allocated nested fixed arrays |
| `MidiBuffer` | Fixed capacity of 1024 events (`MAX_MIDI_EVENTS`) and a SysEx byte pool allocated with it (`sysex_bytes_per_block`, 16 KiB by default) |
| `ProcessBufferStorage<S>` | Pre-allocated Vecs with reserved capacity; `clear()` + `push()` never allocate |

**Enforcement**:
- `setupProcessing()` pre-allocates all buffers based on plugin configuration
- `process()` uses only stack storage and pre-allocated pools
- Channel counts are bounded by the accepted bus layout and the reserved capacity, so a misbehaving host cannot cause an allocation
- Calls meant for host threads panic inside `process()` in debug builds: `get()`, `set()` and `reader()` of a `Handover` or `State`, `reset()`, `save()` and `load()` of a `State` and `WebViewEventQueue::emit()`. They lock, allocate or free memory. The audio thread reads through a `HandoverReader`, which does none of that. The wrappers mark the process call with a `ProcessScope`. In release builds the check compiles to nothing

### Deterministic Bus Limits

**Guarantee**: Channel and bus counts are bounded at compile time.

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_CHANNELS` | 32 | Supports up to 22.2 surround and Dolby Atmos 9.1.6 |
| `MAX_BUSES` | 16 | Main + sidechain + 14 aux buses |
| `MAX_AUX_BUSES` | 15 | Auxiliary buses (total minus main) |

**Enforcement**:
- `validate_layouts()` checks the model's layouts against these constants when the plugin is compiled (`#[beamer::export]`) and again before the Processor is created: VST3 `setupProcessing()` returns `kResultFalse`, AU `allocateRenderResources` returns `kAudioUnitErr_FormatNotSupported`, both log an error. The AU also checks the configuration the host set with `CachedBusConfig::validate()`
- `setBusArrangements()` refuses an arrangement without speakers or with more than `MAX_CHANNELS` and suggests the nearest listed layout

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
- `MidiBuffer::has_sysex_overflowed()` flag set when a SysEx message did not fit into the pool: it is dropped whole, never cut
- The audio thread does not log: it notes an overflow in `ProcessWarnings` with one atomic operation and a host thread logs it when the plugin is prepared again or released

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
    // Private: scratch per input and output channel and silence.
}
```

- Pre-allocated in `setupProcessing()` based on plugin's **actual** bus configuration (not worst-case)
- Config-based allocation: pointer capacity, scratch and internal buffers follow the actual channel counts, not the worst case
- Lazy aux allocation: no heap allocation for plugins without aux buses
- Internal output buffers that the AU wrapper points null host output buffers at
- Scratch memory per input and output channel, for hosts that give overlapping buffers and for channels without memory: an output writes scratch whose samples are dropped, an input reads silence
- `clear()` resets length to 0 without deallocating
- `push()` into reserved capacity never allocates
- `prepare_in_place()` runs once per process call, before the sub-blocks: each main output channel gets its input (or silence). A channel that would overlap another gets scratch, so the plugin processes the main bus in place and its slices never overlap

**Plugin-Declared Capacity** (configurable via Config.toml):
```toml
# Config.toml
# SysEx bytes one block holds in each direction, counting the F0 and F7
# of each message. Default: 16384.
sysex_bytes_per_block = 65536
```

### Allocation Lifecycle

The buffer allocation flow ensures all memory is reserved before audio processing begins:

```
Plugin Load (creates the Model)
    │
    ▼
┌─────────────────────────────────────────────────────────────┐
│ setBusArrangements(inputs, outputs)  [VST3 host call]       │
│   • Each arrangement to a ChannelSet (none above            │
│     MAX_CHANNELS)                                           │
│   • A layout allows the sets, supports_buses accepts them:  │
│     kResultTrue                                             │
│   • Otherwise the nearest supported layout, kResultFalse    │
│     (the host reads it back with getBusArrangement())       │
└─────────────────────────────────────────────────────────────┘
    │
    ▼
┌─────────────────────────────────────────────────────────────┐
│ setupProcessing(sample_rate, max_block_size)                │
│   • Negotiate the buses (supports_buses)                    │
│   • validate_layouts()                                      │
│     - Buses ≤ MAX_BUSES, channels per bus ≤ MAX_CHANNELS    │
│     - Return kResultFalse + log if exceeded                 │
│   • model.prepare(setup) creates the Processor, when there  │
│     is none or the setup or the accepted buses changed      │
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
│   • Channels bounded by the accepted layout and capacity    │
│   • prepare_in_place() - each output gets its input         │
│   • Per sub-block: apply parameter changes, build           │
│     Buffer/AuxiliaryBuffers, process_midi(), process()      │
└─────────────────────────────────────────────────────────────┘
    │
    ▼ (on audio setup or accepted buses change)
┌─────────────────────────────────────────────────────────────┐
│ setupProcessing() or setActive(true) with new setup         │
│   • The Processor is dropped                                │
│     - Parameters untouched, DSP state discarded             │
│   • model.prepare(new_setup) creates a new Processor        │
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
