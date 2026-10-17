# beamer-macros

Attribute macros and a derive macro for building audio plugins with Beamer.

This crate provides procedural macros that generate boilerplate code for plugins:

- **`#[beamer::export]`**: Reads `Config.toml` at compile time and generates plugin configuration, factory presets (from `Presets.toml`) and format-specific entry points
- **`#[beamer::model]`**: Marks the model of a plugin, the struct every thread shares, and generates its parameter traits, host integration, state persistence and `Default` implementation
- **`#[beamer::group]`**: Marks a nested group of a model, with the same code generation
- **`#[derive(EnumParameter)]`**: Generates enum parameter variants with display names
- **Declarative attributes**: Configure parameters with `#[parameter(id, name, default, range, kind)]`
- **Compile-time validation**: Duplicate parameter ids in a struct are compile errors

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

#[beamer::export]
#[beamer::model]
struct GainModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,
}

impl Model for GainModel {
    // implementation...
}
```

The `#[beamer::export]` macro reads `Config.toml` and generates the plugin configuration and entry points.

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Parameter Documentation](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md#parameters)
- [Declarative Attributes Guide](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md#declarative-attributes)
- [Examples](https://github.com/helpermedia/beamer/tree/main/examples)

## License

MIT
