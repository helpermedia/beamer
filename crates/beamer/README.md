# Beamer

Beamer is a Rust framework for building Audio Unit (AU) and VST3 audio plugins.

Named after the beams connecting notes in sheet music and from Dutch where "beamer" means projector. Beamer projects your DSP logic onto AU (macOS) and VST3 (macOS; Windows planned) through modern web UI from a single codebase.

**Beamer is pre-1.0** and under active development. Expect breaking changes between minor versions.

## This crate

`beamer` is the one dependency a plugin needs. It brings the framework's crates together under one path:

- `beamer::prelude::*` imports the traits and types a plugin works with, from `beamer-core` (model, processor, parameters, buffers, MIDI, state) and `beamer-webview` (the web UI's events and handler)
- `#[beamer::model]`, `#[beamer::group]`, `#[beamer::export]` and `#[derive(EnumParameter)]` come from `beamer-macros`
- The format wrappers `beamer-au` and `beamer-vst3` come in through features
- The crates themselves stay reachable as `beamer::core` and `beamer::webview`

| Feature | Description |
|---------|-------------|
| `derive` (default) | The attribute and derive macros |
| `au` | AU export (macOS) |
| `vst3` | VST3 export |
| `allocation-check` | Debug builds check that `process()` neither allocates nor frees memory; not for a plugin with its own global allocator |
| `diagnostics` | Writes what the host does to the system log, for test builds |

A plugin crate is a dynamic library with one feature per format, named `au` and `vst3`, which turn on the same features of `beamer`:

```toml
[lib]
crate-type = ["cdylib"]

[features]
au = ["beamer/au"]
vst3 = ["beamer/vst3"]
```

`cargo xtask bundle` turns on the feature for each format it builds and `diagnostics` with `--diagnostics`. See [BUILDING.md](https://github.com/helpermedia/beamer/blob/main/docs/BUILDING.md) for the rest of a plugin's setup.

## Why Beamer?

**Built on Rust's guarantees.** Where most plugin frameworks use C++, Beamer uses Rust. Memory and threading bugs become compile-time errors, not runtime crashes.

**Macros do the heavy lifting.** `#[beamer::model]` generates host integration, state persistence, DAW automation and parameter access traits automatically. `#[beamer::export]` reads Config.toml at compile time to generate plugin metadata and entry points.

**Clean separation of concerns.** DSP code stays in safe Rust while the WebView architecture enables modern interfaces with HTML, CSS and JavaScript. Plugin configuration lives in Config.toml. Beamer bridges them together and handles plugin format complexity.

## Features

- **Multi-format** - AU (macOS) and VST3 (macOS; Windows planned)
- **Declarative parameters** - `#[beamer::model]` with attributes for units, smoothing and more
- **Type-safe initialization** - `prepare()` lifecycle eliminates placeholder values and sample-rate bugs
- **Format-agnostic core** - Plugin logic is independent of format specifics
- **32-bit and 64-bit audio** - Native f64 support or automatic conversion for f32-only plugins
- **Multi-bus audio** - Main bus + auxiliary buses (sidechain, aux sends, multi-out) that hosts switch on and off
- **Complete MIDI support** - Full MIDI 1.0/2.0, MPE, Note Expression, SysEx
- **Real-time safe** - No heap allocations in the audio path
- **State persistence** - Automatic preset/state save and restore, also for data beyond parameters
- **Factory presets in code** - `Model::PRESETS` of typed `Preset::new(name, |m| ...)` closures, checked by the compiler
- **Sample-accurate automation** - Opt in with `sample_accurate_automation = true` in Config.toml
- **WebView editor** - Web-based plugin editors with WKWebView, bidirectional IPC, realtime-safe data from the audio thread (meters, streams) and built-in React/Vite tooling (macOS)

## Getting Started

The crate documentation starts with a complete gain plugin. The [main repository](https://github.com/helpermedia/beamer) has the rest:

- [Quick Start](https://github.com/helpermedia/beamer#quick-start) - A plugin with its Config.toml
- [Examples](https://github.com/helpermedia/beamer/tree/main/examples) - Effects, instruments, MIDI and web UI editors
- [BUILDING.md](https://github.com/helpermedia/beamer/blob/main/docs/BUILDING.md) - Build, install and test plugins
- [REFERENCE.md](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md) - API reference: configuration, parameters, traits, MIDI and format notes
- [WEBVIEW.md](https://github.com/helpermedia/beamer/blob/main/docs/WEBVIEW.md) - WebView editor, JavaScript API and the packages for web UIs

## License

MIT
