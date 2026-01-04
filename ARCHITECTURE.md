# Beamer Architecture

This document describes the high-level architecture of Beamer, a Rust framework for building VST3 audio plugins with WebView-based GUIs.

For detailed API documentation, see [docs/REFERENCE.md](docs/REFERENCE.md).

---

## Overview

### What Is Beamer?

A Rust framework for building VST3 audio plugins with WebView-based GUIs. Named after the beams that connect notes in sheet music, Beamer links your DSP logic and WebView interface together. Inspired by Tauri's architecture but focused specifically on the audio plugin context.

### Why?

- **Rust for audio**: Memory safety, performance, no GC pauses
- **WebView for UI**: Leverage modern web technologies (React, Svelte, Vue, etc.)
- **VST3 only**: Focused scope, VST3 3.8 is now MIT licensed
- **Lightweight**: Use OS-native WebViews, no bundled browser engine

### Goals

- VST3 plugin support (VST3 3.8, MIT licensed)
- WebView GUI using OS-native engines
- Cross-platform: Windows, macOS, Linux
- Tauri-inspired IPC (invoke/emit pattern)
- Optional parameter binding helpers
- Developer-friendly: hot reload in dev mode
- Framework-agnostic frontend (React, Svelte, Vue, vanilla JS)
- MIDI event processing (instruments and MIDI effects)

### Non-Goals

- CLAP/AU/AAX support (can be added later)
- Bundled browser engine (no Electron/CEF)
- Built-in UI components/widgets

---

## Architecture Diagram

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
│   ├── beamer-macros/       # Proc macros (#[derive(Params)])
│   └── beamer-webview/      # WebView per platform (Phase 2)
├── examples/
│   ├── gain/                # Audio effect example
│   └── midi-transform/      # MIDI instrument example
└── xtask/                   # Build tooling
```

### Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `beamer` | Facade crate, re-exports public API via `prelude` |
| `beamer-core` | Platform-agnostic traits (`Plugin`, `AudioProcessor`), buffer types, MIDI types |
| `beamer-vst3` | VST3 SDK integration, COM interfaces, host communication |
| `beamer-macros` | `#[derive(Params)]`, `#[derive(EnumParam)]` proc macros |
| `beamer-webview` | Platform-native WebView embedding (Phase 2) |

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

**Plugin-Declared Capacity**:
```rust
pub static CONFIG: PluginConfig = PluginConfig::new("My Plugin", UID)
    .with_sysex_slots(64)         // Default: 16
    .with_sysex_buffer_size(4096); // Default: 512 bytes
```

### Allocation Lifecycle

The buffer allocation flow ensures all memory is reserved before audio processing begins:

```
Plugin Load
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
| [VST3 SDK](https://github.com/steinbergmedia/vst3sdk) | The standard we implement |
