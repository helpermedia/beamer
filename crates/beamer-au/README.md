# beamer-au

Audio Unit v3 implementation layer for the Beamer framework (macOS only).

This crate provides native AUv3 support using a **hybrid Objective-C/Rust architecture**. A native `AUAudioUnit` subclass handles Apple runtime compatibility, while all DSP and plugin logic remains in Rust via a C-ABI bridge.

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

- **macOS 10.11+** (AUv3 minimum requirement)
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
- No AUv2 legacy support (v3 only)

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
