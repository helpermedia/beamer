# BEAMER

A Rust framework for building VST3 audio plugins.

Named after the beams that connect notes in sheet music, Beamer links your DSP logic and WebView interface together, then projects them onto any surface through modern web UI. It bridges VST3's C++ COM interfaces with safe Rust abstractions.

## Overview

Beamer provides a clean separation between plugin logic and the VST3 format details. You implement simple traits for your audio processing and parameters, and Beamer handles the rest.

The [VST3 SDK](https://github.com/steinbergmedia/vst3sdk) is now MIT licensed (as of v3.8), making it available as a standard Rust dependency—no separate SDK downloads or licensing agreements required. Beamer uses the [vst3](https://github.com/coupler-rs/vst3-rs) crate for Rust bindings.

## Documentation

- [ARCHITECTURE.md](https://github.com/helpermedia/beamer/blob/main/ARCHITECTURE.md) — Design decisions, threading model, guarantees
- [REFERENCE.md](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md) — Detailed API reference
- [EXAMPLE_COVERAGE.md](https://github.com/helpermedia/beamer/blob/main/docs/EXAMPLE_COVERAGE.md) — Example testing roadmap and feature coverage matrix

## Features

- **Format-agnostic core** - Plugin logic is independent of VST3 specifics
- **32-bit and 64-bit audio** - Native f64 support or automatic conversion for f32-only plugins
- **Multi-bus audio** - Main bus + auxiliary buses (sidechain, aux sends, multi-out)
- **Complete MIDI support** - All VST3 SDK 3.8.0 MIDI features including MPE, Note Expression, and MIDI 2.0
- **Real-time safe** - No heap allocations in the audio path
- **State persistence** - Automatic preset/state save and restore
- **WebView GUI** (planned) - Modern web-based plugin interfaces

## Platform Support

| Platform | Status |
|----------|--------|
| macOS (arm64) | Tested |
| Windows | Untested |
| Linux | Untested |

## Crates

| Crate | Description |
|-------|-------------|
| `beamer` | Main facade crate (re-exports everything) |
| `beamer-core` | Platform-agnostic traits and types |
| `beamer-vst3` | VST3 wrapper implementation |

## Quick Start

```rust
use beamer::prelude::*;
use beamer::Params;

// Declarative parameters - macro generates Default, VST3 integration, state persistence
#[derive(Params)]
struct GainParams {
    #[param(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParam,
}

// Plugin with DSP logic
struct GainPlugin {
    params: GainParams,
}

impl AudioProcessor for GainPlugin {
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &ProcessContext) {
        let gain = self.params.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

impl Plugin for GainPlugin {
    type Params = GainParams;

    fn params(&self) -> &Self::Params { &self.params }
    fn params_mut(&mut self) -> &mut Self::Params { &mut self.params }
    fn create() -> Self { Self { params: GainParams::default() } }
}
```

See the [examples](https://github.com/helpermedia/beamer/tree/main/examples) for complete working plugins.

## Parameter Attributes

The `#[param(...)]` attribute supports:

| Attribute | Description |
|-----------|-------------|
| `id = "..."` | Required. String ID (hashed to u32 for VST3) |
| `name = "..."` | Display name in DAW |
| `default = <value>` | Default value |
| `range = start..=end` | Value range |
| `kind = "..."` | Unit type: `db`, `hz`, `ms`, `seconds`, `percent`, `pan`, `ratio`, `linear`, `semitones` |
| `group = "..."` | Visual grouping in DAW (flat access, grouped display) |
| `smoothing = "exp:5.0"` | Parameter smoothing (`exp` or `linear`) |
| `bypass` | Mark as bypass parameter (BoolParam only) |

### Visual Grouping

Use `group = "..."` for flat parameter access with DAW grouping:

```rust
#[derive(Params)]
struct SynthParams {
    #[param(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
    cutoff: FloatParam,

    #[param(id = "reso", name = "Resonance", default = 0.5, range = 0.0..=1.0, group = "Filter")]
    resonance: FloatParam,

    #[param(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db", group = "Output")]
    gain: FloatParam,
}

// Access: params.cutoff, params.resonance, params.gain (flat)
// DAW shows collapsible "Filter" and "Output" groups
```

For nested structs with separate parameter groups, use `#[nested(group = "...")]`.

## Building

```bash
# Build all crates
cargo build

# Build release
cargo build --release

# Run clippy
cargo clippy --workspace
```

## Bundling for DAW

```bash
# Build, bundle, and install to user VST3 folder (macOS)
cargo xtask bundle gain --release --install

# Or just bundle (output: target/release/BeamerGain.vst3)
cargo xtask bundle gain --release
```

## Examples

- **gain** - Simple gain effect plugin
- **midi-transform** - MIDI instrument that transposes notes

## Multi-Bus Audio

Beamer separates main bus and auxiliary buses for sidechain and multi-output plugins:

```rust
fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers) {
    // Sidechain input (e.g., for compression keying)
    if let Some(sidechain) = aux.sidechain() {
        let level = sidechain.rms(0);
        // Use sidechain level...
    }

    // Main bus processing
    buffer.copy_to_output();
}
```

## MIDI Support

Beamer provides comprehensive MIDI support:

- Note On/Off with velocity, tuning, and note length
- Control Change with 14-bit resolution helpers
- Pitch Bend, Channel Pressure, Poly Pressure
- Program Change
- SysEx (configurable buffer size)
- Note Expression (MPE)
- RPN/NRPN decoding
- Chord and Scale info from DAW

## License

MIT
