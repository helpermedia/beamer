# beamer-core

Format-agnostic core abstractions for building audio plugins with Beamer.

This crate provides the shared traits and types used by all plugin formats (AU, VST3):

- **Plugin traits**: `Descriptor`, `Processor`, `HasParameters`, `Parameters`
- **Audio buffers**: `Buffer<S>`, `AuxiliaryBuffers<S>` with real-time safety guarantees
- **MIDI types**: Complete MIDI event handling including MPE and Note Expression
- **Parameter types**: `FloatParameter`, `IntParameter`, `BoolParameter`, `EnumParameter` with smoothing
- **Transport info**: DAW tempo, time signature, and position data

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports everything from `beamer-core` along with the format-specific integration layers.

Use `beamer-core` directly only if you're:
- Building a plugin format adapter (e.g., for CLAP or AAX)
- Creating a custom plugin framework on top of Beamer's abstractions

## Advanced: Manual Parameters Implementation

Most plugins should use `#[derive(Parameters)]`. However, the `Parameters` trait can be implemented manually for advanced scenarios like dynamic parameter counts, custom storage strategies, or shared parameter pools across plugin instances.

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)
- [Architecture Documentation](https://github.com/helpermedia/beamer/blob/main/ARCHITECTURE.md)

## License

MIT
