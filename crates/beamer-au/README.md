# beamer-au

Audio Unit implementation for the Beamer framework (macOS only).

This crate provides **both AUv2 and AUv3** support through a shared C-ABI bridge. A native Objective-C layer handles Apple runtime compatibility, while all DSP and plugin logic remains in Rust.

## Features

- **Dual format**: AUv2 (`.component`) and AUv3 (`.appex`) from the same codebase
- **Full AU lifecycle**: Allocate/deallocate render resources, parameter tree, state persistence
- **Parameter automation**: Complete integration with host callbacks
- **MIDI support**: MIDI 1.0 and MIDI 2.0 UMP event processing
- **Real-time safe**: Zero-allocation render path
- **Auxiliary buses**: Sidechain and multi-bus support
- **Limitation**: No custom UI (uses host generic parameter UI)

## Platform Requirements

- **macOS 10.11+** (AUAudioUnit API minimum)
- **Apple Silicon and Intel** supported

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports everything you need.

Use `beamer-au` directly only if you're:
- Implementing a custom Audio Unit wrapper
- Building macOS-specific tooling

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)

## License

MIT
