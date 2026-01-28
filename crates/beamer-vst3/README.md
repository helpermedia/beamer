# beamer-vst3

VST3 implementation layer for the Beamer framework.

This crate provides the VST3 SDK integration that bridges `beamer-core` abstractions to the VST3 plugin format.

## VST3 SDK

The [VST3 SDK](https://github.com/steinbergmedia/vst3sdk) is MIT licensed (as of v3.8), making it available as a standard Rust dependency - no separate SDK downloads or licensing agreements required. Beamer uses [Coupler's vst3 crate](https://github.com/coupler-rs/vst3-rs) for the Rust bindings.

## Features

- **VST3 factory**: Plugin registration and entry points
- **Two-phase lifecycle**: Descriptor ↔ Processor state machine (prepare/unprepare)
- **Edit controller**: Parameter management and host communication
- **MIDI conversion**: Bidirectional mapping between Beamer and VST3 MIDI events
- **Real-time buffer management**: Zero-allocation audio processing

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports everything you need.

Use `beamer-vst3` directly only if you're:
- Implementing a custom plugin wrapper
- Building tooling that needs VST3-specific functionality

## Features

- `sysex-heap-fallback`: Enable heap-backed overflow for SysEx messages (breaks real-time guarantee)

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)
- [VST3 Integration Details](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md#3-vst3-integration)

## License

MIT
