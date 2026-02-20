# beamer-macros

Derive macros and attribute macros for building audio plugins with Beamer.

This crate provides procedural macros that generate boilerplate code for plugins:

- **`#[beamer::export]`**: Reads `Config.toml` at compile time and generates plugin configuration, factory presets (from `Presets.toml`) and format-specific entry points
- **`#[derive(Parameters)]`**: Generates parameter traits, host integration, state persistence and `Default` implementation
- **`#[derive(HasParameters)]`**: Generates `parameters()`, `parameters_mut()` and `set_parameters()` accessors for Descriptor and Processor types
- **`#[derive(EnumParameter)]`**: Generates enum parameter variants with display names
- **Declarative attributes**: Configure parameters with `#[parameter(id, name, default, range, kind)]`
- **Compile-time validation**: ID collision detection and hash generation

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports these macros with the `derive` feature (enabled by default).

**Config.toml** (place in crate root):
```toml
name = "My Gain Plugin"
category = "effect"
manufacturer_code = "Myco"
plugin_code = "gain"
vendor = "My Company"
```

**Rust code:**
```rust
use beamer::prelude::*;

#[derive(Parameters)]
struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,
}

#[beamer::export]
#[derive(Default, HasParameters)]
struct GainDescriptor {
    #[parameters]
    parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    // implementation...
}
```

The `#[beamer::export]` macro reads `Config.toml` and generates the plugin configuration and entry points.

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Parameter Documentation](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md#13-parameters)
- [Declarative Attributes Guide](https://github.com/helpermedia/beamer#parameter-attributes)
- [Examples](https://github.com/helpermedia/beamer/tree/main/examples)

## License

MIT
