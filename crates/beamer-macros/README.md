# beamer-macros

Derive macros for the Beamer VST3 framework.

This crate provides procedural macros that generate boilerplate code for plugins:

- **`#[derive(Parameters)]`**: Generates parameter traits, VST3 integration, state persistence, and `Default` implementation
- **`#[derive(HasParameters)]`**: Generates `parameters()` and `parameters_mut()` accessors for Plugin and AudioProcessor types
- **`#[derive(EnumParameter)]`**: Generates enum parameter variants with display names
- **Declarative attributes**: Configure parameters with `#[parameter(id, name, default, range, kind)]`
- **Compile-time validation**: ID collision detection and hash generation

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports these macros with the `derive` feature (enabled by default).

```rust
use beamer::prelude::*;

#[derive(Parameters)]
struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,
}

#[derive(Default, HasParameters)]
struct GainPlugin {
    #[parameters]
    parameters: GainParameters,
}
```

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Parameter Documentation](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md#13-parameters)
- [Declarative Attributes Guide](https://github.com/helpermedia/beamer#parameter-attributes)
- [Examples](https://github.com/helpermedia/beamer/tree/main/examples)

## License

MIT
