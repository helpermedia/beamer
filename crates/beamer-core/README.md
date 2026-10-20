# beamer-core

Format-agnostic core abstractions for building audio plugins with Beamer.

This crate provides the shared traits and types used by all plugin formats (AU, VST3):

- **Plugin traits**: `Model`, `Processor`, `Parameters`, `Saved`, `WebViewEvents`
- **Audio buffers**: `Buffer<S>`, `AuxiliaryBuffers<S>` with real-time safety guarantees
- **MIDI types**: Complete MIDI event handling including MPE and Note Expression
- **Parameter types**: `FloatParameter`, `IntParameter`, `BoolParameter`, `EnumParameter`, with smoothing for `FloatParameter`
- **State and presets**: `State<T>` fields saved with the parameters, `Handover` for values handed to the audio thread, factory `Preset`s
- **Buses**: `BusInfo`, `ActiveBuses` and the negotiation of buses that hosts switch on and off
- **Transport info**: DAW tempo, time signature and position data

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports everything from `beamer-core` along with the format-specific integration layers.

Use `beamer-core` directly only if you're:
- Building a plugin format adapter (e.g., for CLAP or AAX)
- Creating a custom plugin framework on top of Beamer's abstractions

## Advanced: Manual Implementation

Most plugins should use `#[beamer::model]`. For advanced scenarios like dynamic parameter counts or custom storage strategies, a model can implement `Parameters`, `ParameterGroups`, `Saved`, `WebViewEvents` and `Default` by hand. Empty impls of `ParameterGroups`, `Saved` and `WebViewEvents` are enough for a model without groups, `State` fields or an event queue. See [Low-Level Traits](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md#low-level-traits).

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)
- [Architecture Documentation](https://github.com/helpermedia/beamer/blob/main/ARCHITECTURE.md)

## License

MIT
