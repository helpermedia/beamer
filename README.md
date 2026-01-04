# BEAMER

A Rust framework for building VST3 audio plugins.

A modern framework bridging VST3's C++ COM interfaces with safe Rust abstractions and WebView GUIs.

## Overview

Beamer provides a clean separation between plugin logic and the VST3 format details. You implement simple traits for your audio processing and parameters, and Beamer handles the rest.

The [VST3 SDK](https://github.com/steinbergmedia/vst3sdk) is now MIT licensed (as of v3.8). Beamer uses the [vst3](https://github.com/coupler-rs/vst3-rs) crate for Rust bindings.

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

// Parameters using derive macro - handles atomic storage, VST3 integration, state persistence
#[derive(Params)]
struct GainParams {
    #[param(id = "gain")]
    gain: FloatParam,
}

impl GainParams {
    fn new() -> Self {
        Self {
            // 0 dB default, range -60 to +12 dB
            gain: FloatParam::db("Gain", 0.0, -60.0..=12.0),
        }
    }
}

impl Default for GainParams {
    fn default() -> Self { Self::new() }
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
    fn create() -> Self { Self { params: GainParams::new() } }
}
```

See the [examples](examples/) for complete working plugins.

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
