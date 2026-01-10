# beamer-au

Audio Unit implementation layer for the Beamer framework (macOS only).

This crate uses a **hybrid v2/v3 architecture**: AUv2-style `.component` bundles for simple distribution, with a modern v3 `AUAudioUnit` implementation internally. A native Objective-C `AUAudioUnit` subclass handles Apple runtime compatibility, while all DSP and plugin logic remains in Rust via a C-ABI bridge.

- **Hybrid architecture**: Native ObjC `AUAudioUnit` subclass + Rust DSP via C-ABI bridge
- **Full AU lifecycle**: allocate/deallocate render resources, parameter tree, state persistence
- **Parameter automation**: Complete `AUParameterTree` integration with KVO callbacks
- **MIDI support**: MIDI 1.0 and MIDI 2.0 UMP event processing
- **Real-time safe**: Zero-allocation render path with config-based pre-allocation
- **Auxiliary buses**: Sidechain and multi-bus support with pull-based input
- **Transport information**: Tempo, beat position, and playback state

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports everything you need.

Use `beamer-au` directly only if you're:
- Implementing a custom Audio Unit wrapper
- Building macOS-specific tooling
- Contributing to the AU implementation

## Platform Requirements

- **macOS 10.11+** (AUAudioUnit API minimum)
- **Apple Silicon and Intel** supported

Audio Units are macOS-exclusive. This crate will not compile on other platforms.

## Features

Audio Unit plugins share the same `Plugin` and `AudioProcessor` traits as VST3, allowing multi-format builds from a single codebase.

### Production Ready

- ✅ Audio effects (all bus configurations)
- ✅ Instruments/generators (MIDI input)
- ✅ MIDI effects
- ✅ Sidechain/auxiliary buses
- ✅ Parameter automation (full KVO integration)
- ✅ State persistence (cross-compatible with VST3)
- ✅ f32 and f64 processing
- ✅ Transport information (tempo, beat, playback state)

### Limitations

- No custom UI (uses host generic parameter UI)

## Architecture

Beamer AU uses a **hybrid v2/v3 architecture**: AUv2-style `.component` bundles with a modern v3 `AUAudioUnit` implementation internally.

```
┌─────────────────────────────────────────────────────────────────┐
│  AUv2 .component bundle                                         │
│  - Simple distribution (no app extension required)              │
│  - Works with ad-hoc code signing                               │
│  - Compatible with all AU hosts                                 │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  BeamerAudioUnitFactory (minimal v2 shim)                       │
│  - Lookup() returns NULL to defer to v3 API                     │
│  - dispatch_once subclass registration                          │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  BeamerAuWrapper : AUAudioUnit (full v3 implementation)         │
│  - internalRenderBlock for audio processing                     │
│  - parameterTree with KVO callbacks                             │
│  - inputBusses/outputBusses for I/O                             │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Rust Plugin (via C-ABI bridge)                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Why this approach?**

- **Simple distribution**: `.component` bundles just copy to `/Library/Audio/Plug-Ins/Components/`
- **No special signing**: Works without Apple Developer Program membership
- **Modern API**: Uses v3 `AUAudioUnit` for parameters, buses, rendering—not legacy v2 callbacks
- **Wide compatibility**: Loads in all DAWs that support Audio Units

The AUv2 factory is intentionally minimal—it exists only to bootstrap the v3 `AUAudioUnit` subclass. All actual audio processing uses the modern v3 API.

For Mac App Store distribution, AUv3 App Extensions (`.appex` bundles) would be required, which need proper Apple Developer signing and a container app

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)
- [Audio Unit Integration Details](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md#4-audio-unit-integration)

### Implementation Notes

For contributors working on the AU implementation:
- [Buffer Storage Optimization](https://github.com/helpermedia/beamer/blob/main/crates/beamer-au/BUFFER_STORAGE_OPTIMIZATION.md) - Memory optimization strategy
- [Hybrid AU Implementation Plan](https://github.com/helpermedia/beamer/blob/main/docs/HYBRID_AU_IMPLEMENTATION_PLAN.md) - Why hybrid ObjC/Rust was needed
- [AU Crash Analysis](https://github.com/helpermedia/beamer/blob/main/docs/AU_CRASH_ANALYSIS.md) - Investigation of objc2 incompatibility

**Why hybrid?** Apple's `AUAudioUnit` requires specific Objective-C runtime metadata that Rust's `objc2` crate cannot generate correctly. Even a minimal subclass with no method overrides crashes. The hybrid approach uses native ObjC for the AU wrapper while keeping all DSP in Rust.

## Example

```rust
use beamer::prelude::*;
use beamer_au::{export_au, AuConfig, ComponentType, fourcc};

// Shared configuration
pub static CONFIG: PluginConfig = PluginConfig::new("My Plugin")
    .with_vendor("My Company")
    .with_version(env!("CARGO_PKG_VERSION"));

// AU-specific configuration
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    fourcc!(b"Myco"),  // Manufacturer code
    fourcc!(b"mypg"),  // Subtype code
);

// Export Audio Unit
export_au!(CONFIG, AU_CONFIG, MyPlugin);
```

## Building

```bash
# Build AU bundle (creates .component)
cargo xtask bundle my-plugin --au --release

# Build and install to system location
cargo xtask bundle my-plugin --au --release --install
```

## License

MIT
