# CLAP Support Analysis

This document analyzes the feasibility of adding CLAP format support to Beamer.

## Executive Summary

Adding CLAP support requires **creating a new wrapper crate, not refactoring the core**. The core abstractions are already format-agnostic, as proven by the existing `beamer-vst3` and `beamer-au` implementations.

---

## CLAP Ecosystem Overview

### Current Adoption (2025)

| Metric | Count |
|--------|-------|
| DAWs with CLAP support | ~15-17 |
| Plugin producers | 93+ |
| Available plugins | 400-525+ |

### DAWs with CLAP Support

| DAW | Developer | Notes |
|-----|-----------|-------|
| Bitwig Studio | Bitwig | Co-creator, full support since 4.3 |
| REAPER | Cockos | Early adopter since 6.71 |
| FL Studio | Image-Line | Added in FL Studio 2024 (v21.3+) |
| Fender Studio Pro | PreSonus (Fender) | Basic support since v7, continues in v8 |
| MultitrackStudio | Bremmers Audio | Full support |
| Carla | falkTX | Open-source plugin host |
| Qtractor, Zrythm | Various | Linux DAWs |

### DAWs Without CLAP Support

| DAW | Reason |
|-----|--------|
| Ableton Live | Under consideration, historically slow to adopt formats |
| Logic Pro | Apple owns AU format, unlikely to ever support CLAP |
| Cubase/Nuendo | Steinberg owns VST format |
| Pro Tools | AAX-only ecosystem |

### Notable Plugin Developers with CLAP Support

- **u-he**: Diva, Zebra2, Repro (co-creator of CLAP)
- **FabFilter**: Pro-Q, Pro-L, Pro-C (full support since late 2023)
- **TAL Software**: TAL-U-NO-LX, TAL-Reverb
- **Surge XT**: Major open-source synthesizer
- **Vital/Vitalium**: Popular wavetable synth

---

## Technical Advantages Over VST3

### Threading Model

| Aspect | CLAP | VST3 |
|--------|------|------|
| Thread management | Host-managed thread pool | Plugin-managed |
| Multicore efficiency | 20-25% improvement | Baseline |
| Context switches | Minimized by host | Plugins compete |

CLAP's `thread-pool` extension allows the host to coordinate parallel processing across all plugins, reducing CPU spikes.

### Polyphonic Modulation

| Feature | CLAP | VST3 |
|---------|------|------|
| Per-note automation | Full support | Limited |
| Per-note modulation | Full support | Not available |
| MIDI 2.0 | Native support | Partial |

### API Design

| Aspect | CLAP | VST3 |
|--------|------|------|
| Language | Pure C ABI | C++ with COM-like interfaces |
| License | MIT (no fees) | Proprietary (licensing required) |
| FFI complexity | Simple | Complex |

---

## CLAP API Structure

### Entry Point

```c
// Required export symbol
CLAP_EXPORT extern const clap_plugin_entry_t clap_entry;

typedef struct clap_plugin_entry {
    clap_version_t clap_version;
    bool (*init)(const char *plugin_path);
    void (*deinit)(void);
    const void *(*get_factory)(const char *factory_id);
} clap_plugin_entry_t;
```

### Plugin Lifecycle

```c
typedef struct clap_plugin {
    bool (*init)(const clap_plugin_t *plugin);
    void (*destroy)(const clap_plugin_t *plugin);
    bool (*activate)(const clap_plugin_t *plugin, double sample_rate,
                     uint32_t min_frames, uint32_t max_frames);
    void (*deactivate)(const clap_plugin_t *plugin);
    bool (*start_processing)(const clap_plugin_t *plugin);
    void (*stop_processing)(const clap_plugin_t *plugin);
    clap_process_status (*process)(const clap_plugin_t *plugin,
                                   const clap_process_t *process);
    const void *(*get_extension)(const clap_plugin_t *plugin, const char *id);
} clap_plugin_t;
```

### Core Extensions

| Extension | Purpose |
|-----------|---------|
| `params` | Parameter management and automation |
| `state` | Plugin state save/load |
| `audio-ports` | Audio I/O configuration |
| `note-ports` | MIDI/note event routing |
| `gui` | GUI window creation (Win32, Cocoa, X11) |
| `latency` | Latency reporting |
| `tail` | Audio tail length |

### Advanced Extensions

| Extension | Purpose |
|-----------|---------|
| `voice-info` | Polyphony info for per-voice modulation |
| `remote-controls` | Hardware controller mapping |
| `preset-discovery` | Host-side preset indexing |
| `thread-pool` | Host-managed multicore processing |
| `surround` | Surround formats up to 7.1.4 |

---

## Beamer Architecture Mapping

### Lifecycle Mapping

| Beamer | CLAP |
|--------|------|
| `Descriptor::prepare()` | `clap_plugin::activate()` |
| `Processor::process()` | `clap_plugin::process()` |
| `Processor::unprepare()` | `clap_plugin::deactivate()` |
| `Processor::set_active()` | `start_processing()` / `stop_processing()` |

### Trait Mapping

| Beamer Trait | CLAP Extension |
|--------------|----------------|
| `ParameterStore` | `CLAP_EXT_PARAMS` |
| `ParameterGroups` | Parameter grouping in `clap_param_info` |
| `BusInfo` / `BusLayout` | `CLAP_EXT_AUDIO_PORTS` |
| `MidiEvent` / `MidiBuffer` | `CLAP_EXT_NOTE_PORTS` |
| `save_state()` / `load_state()` | `CLAP_EXT_STATE` |
| `latency_samples()` | `CLAP_EXT_LATENCY` |
| `tail_samples()` | `CLAP_EXT_TAIL` |

### Parameter Mapping

| Beamer `ParameterStore` | CLAP `clap_plugin_params` |
|-------------------------|---------------------------|
| `count()` | `count()` |
| `info()` | `get_info()` |
| `get_normalized()` | `get_value()` |
| `set_normalized()` | Via `CLAP_EVENT_PARAM_VALUE` events |
| `normalized_to_string()` | `value_to_text()` |
| `string_to_normalized()` | `text_to_value()` |

---

## Rust Implementation Options

### clap-sys (Recommended)

Low-level Rust FFI bindings for the CLAP C API.

- **Crate:** [clap-sys](https://crates.io/crates/clap-sys) v0.5.0
- **License:** MIT/Apache-2.0 (no GPL concerns)
- **Approach:** Hand-written bindings (not bindgen)
- **Dependencies:** Zero runtime dependencies

```toml
[dependencies]
beamer-core = { path = "../beamer-core" }
clap-sys = "0.5"
```

### Reference Implementations

| Project | Notes |
|---------|-------|
| [nih-plug](https://github.com/robbert-vdh/nih-plug) | High-level framework with VST3 + CLAP |
| [coupler](https://github.com/coupler-rs/coupler) | Framework with VST3 + CLAP (early development) |
| [clack](https://github.com/prokopyl/clack) | Safe wrapper library (not yet on crates.io) |

---

## Implementation Plan

### Crate Structure

```
crates/beamer-clap/
├── src/
│   ├── lib.rs           # Public API, ClapConfig
│   ├── config.rs        # Plugin metadata configuration
│   ├── export.rs        # export_clap! macro
│   ├── factory.rs       # clap_plugin_factory implementation
│   ├── instance.rs      # clap_plugin wrapper
│   ├── processor.rs     # Audio processing bridge
│   ├── extensions/
│   │   ├── params.rs    # CLAP_EXT_PARAMS
│   │   ├── audio_ports.rs
│   │   ├── note_ports.rs
│   │   ├── state.rs
│   │   └── latency.rs
│   └── util.rs
└── Cargo.toml
```

### Configuration Pattern

Following `beamer-vst3` conventions:

```rust
pub struct ClapConfig {
    pub id: &'static str,           // "com.vendor.plugin-name"
    pub name: &'static str,
    pub vendor: &'static str,
    pub url: &'static str,
    pub version: &'static str,
    pub description: Option<&'static str>,
    pub features: &'static [ClapFeature],
}

// Export macro
export_clap!(CONFIG, CLAP_CONFIG, MyPlugin);
```

### Key Implementation Tasks

1. **Plugin Entry Point**: `clap_entry` symbol with factory
2. **Audio Processing**: Map `clap_process` to `Processor::process()`
3. **Parameters**: Translate parameter events to/from `set_normalized()`
4. **State**: Map `save_state()`/`load_state()` to `clap_plugin_state`
5. **xtask Bundling**: Install to correct locations per platform

### Plugin Installation Locations

| Platform | Location |
|----------|----------|
| macOS | `~/Library/Audio/Plug-ins/CLAP/` |
| Windows | `C:\Program Files\Common Files\CLAP\` |
| Linux | `~/.clap` or `/usr/lib/clap` |

---

## CLAP-Specific Features

Optional extensions that could be added later:

| Feature | Priority | Notes |
|---------|----------|-------|
| Polyphonic modulation | Low | CLAP's standout feature, complex |
| Voice info | Low | Per-voice parameter modulation |
| Remote controls | Low | Hardware controller mapping |
| Preset discovery | Medium | Host preset browser integration |
| GUI (Cocoa/Win32/X11) | Medium | Different from VST3/AU |

These don't require core changes - they'd be CLAP-specific extensions in `beamer-clap`.

---

## Effort Assessment

| Task | Scope |
|------|-------|
| Create `beamer-clap` crate structure | Small |
| Implement plugin factory/instance | Medium |
| Audio processing bridge | Medium |
| Parameter extension | Small |
| State extension | Small |
| xtask bundling support | Small |
| Testing & validation | Medium |

**Total:** Medium - comparable to the AU implementation effort.

---

## References

### Official Resources
- [CLAP Specification](https://github.com/free-audio/clap) - Official repository
- [CLever Audio Plugin](https://cleveraudio.org/) - Official website
- [CLAP Database](https://clapdb.tech/) - Plugin/DAW tracking

### Rust Implementations
- [nih-plug](https://github.com/robbert-vdh/nih-plug) - Rust plugin framework (VST3 + CLAP)
- [coupler](https://github.com/coupler-rs/coupler) - Rust plugin framework (VST3 + CLAP)
- [clap-sys](https://crates.io/crates/clap-sys) - Rust FFI bindings (MIT/Apache-2.0)
- [clack](https://github.com/prokopyl/clack) - Safe Rust wrapper

### Beamer References
- [beamer-au](../crates/beamer-au/) - Reference for wrapper patterns
- [ROADMAP_MULTI_FORMAT.md](ROADMAP_MULTI_FORMAT.md) - Overall format strategy
