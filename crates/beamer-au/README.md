# beamer-au

Audio Unit implementation for the Beamer framework (macOS only).

This crate provides **both AUv2 and AUv3** implementations that share a common C-ABI bridge to Rust. A native Objective-C layer handles Apple runtime compatibility, while all DSP and plugin logic remains in Rust.

- **Dual format support**: Full AUv2 and AUv3 implementations
- **Shared bridge**: Both formats use the same 40+ C-ABI bridge functions
- **Full AU lifecycle**: allocate/deallocate render resources, parameter tree, state persistence
- **Parameter automation**: Complete parameter integration with host callbacks
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

Audio Unit plugins share the same `Descriptor` and `Processor` traits as VST3, allowing multi-format builds from a single codebase.

### Production Ready

- AUv2 `.component` bundles (auval validated)
- AUv3 via App Extension (`.appex`)
- Audio effects (all bus configurations)
- Instruments/generators (MIDI input)
- MIDI effects
- Sidechain/auxiliary buses
- Parameter automation
- State persistence (cross-compatible with VST3)
- f32 and f64 processing
- Transport information (tempo, beat, playback state)

### Limitations

- No custom UI (uses host generic parameter UI)

## Architecture

Beamer AU provides dual-format support through a shared C-ABI bridge layer:

```
┌─────────────────────────────────────────────────────────────────┐
│                        Host Application                          │
│                    (Logic, GarageBand, etc.)                     │
└─────────────────────────────────────────────────────────────────┘
           │                                    │
           │ AUv2 API                           │ AUv3 API
           ▼                                    ▼
┌─────────────────────────────┐   ┌─────────────────────────────┐
│  AUv2 .component bundle     │   │  AUv3 .appex bundle         │
│  ─────────────────────────  │   │  ─────────────────────────  │
│  Factory → Interface        │   │  AUAudioUnit subclass       │
│  Open/Close/Lookup          │   │  - parameterTree            │
│  Initialize/Render          │   │  - internalRenderBlock      │
│  Get/SetProperty            │   │  - inputBusses/outputBusses │
│  Get/SetParameter           │   │                             │
└─────────────────────────────┘   └─────────────────────────────┘
           │                                    │
           └──────────────┬─────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────────┐
│                    C-ABI Bridge Layer                            │
│  ─────────────────────────────────────────────────────────────  │
│  beamer_au_create_instance()    beamer_au_render()              │
│  beamer_au_allocate_render_resources()                          │
│  beamer_au_get/set_parameter_value()                            │
│  beamer_au_get/set_state()                                      │
│  ... 40+ bridge functions                                       │
└─────────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Rust Plugin Implementation                    │
│  ─────────────────────────────────────────────────────────────  │
│  Descriptor trait + Processor trait                             │
│  Same code as VST3                                              │
└─────────────────────────────────────────────────────────────────┘
```

### AUv2 vs AUv3

| Format | Bundle | Build command |
|--------|--------|---------------|
| AUv2 | `.component` | `--auv2` |
| AUv3 | `.appex` in `.app` | `--auv3` |

Both formats are fully supported. AUv3 is required for iOS/iPadOS.

### Why Objective-C?

Apple's Audio Unit APIs require specific Objective-C runtime metadata that Rust cannot generate correctly. Even using Rust's `objc2` crate, minimal AUAudioUnit subclasses crash due to missing runtime structures. The hybrid approach uses native Objective-C for the AU wrapper while keeping all DSP in Rust via the C-ABI bridge.

## Example

```rust
use beamer::prelude::*;
use beamer_au::{export_au, AuConfig, ComponentType};

// Shared configuration
pub static CONFIG: Config = Config::new("My Plugin")
    .with_vendor("My Company")
    .with_version(env!("CARGO_PKG_VERSION"));

// AU-specific configuration
pub static AU_CONFIG: AuConfig = AuConfig::new(
    ComponentType::Effect,
    "Myco",  // Manufacturer code
    "mypg",  // Subtype code
);

// Export Audio Unit
export_au!(CONFIG, AU_CONFIG, MyPlugin);
```

## Building

```bash
# Build AUv2 bundle (creates .component, recommended)
cargo xtask bundle my-plugin --auv2 --release

# Build AUv3 bundle (creates .appex)
cargo xtask bundle my-plugin --auv3 --release

# Build and install to system location
cargo xtask bundle my-plugin --auv2 --release --install

# Validate with Apple's auval tool
auval -v aufx mypg Myco
```

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)

## License

MIT
