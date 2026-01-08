# beamer-core

Core abstractions for the Beamer VST3 framework.

This crate provides platform-agnostic traits and types for building VST3 audio plugins in Rust:

- **Plugin traits**: `Plugin`, `AudioProcessor`, `HasParameters`, `Parameters`
- **Audio buffers**: `Buffer<S>`, `AuxiliaryBuffers<S>` with real-time safety guarantees
- **MIDI types**: Complete MIDI event handling including MPE and Note Expression
- **Parameter types**: `FloatParam`, `IntParam`, `BoolParam`, `EnumParam` with smoothing
- **Transport info**: DAW tempo, time signature, and position data

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports everything from `beamer-core` along with the VST3 integration layer.

Use `beamer-core` directly only if you're:
- Building a plugin format adapter for a non-VST3 format
- Creating a custom plugin framework on top of Beamer's abstractions

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)
- [Architecture Documentation](https://github.com/helpermedia/beamer/blob/main/ARCHITECTURE.md)

## License

MIT
