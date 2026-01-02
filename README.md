# BEAMR

A Rust framework for building VST3 audio plugins.

Like beams connect notes in musical notation, BEAMR bridges VST3's C++ COM interfaces with safe Rust abstractions and your plugin logic with WebView GUIs.

## Overview

BEAMR provides a clean separation between plugin logic and the VST3 format details. You implement simple traits for your audio processing and parameters, and BEAMR handles the rest.

The [VST3 SDK](https://github.com/steinbergmedia/vst3sdk) is now MIT licensed (as of v3.8). BEAMR uses the [vst3](https://github.com/coupler-rs/vst3-rs) crate for Rust bindings.

## Features

- **Format-agnostic core** - Plugin logic is independent of VST3 specifics
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
| `beamr` | Main facade crate (re-exports everything) |
| `beamr-core` | Platform-agnostic traits and types |
| `beamr-vst3` | VST3 wrapper implementation |

## Quick Start

```rust
use beamr::prelude::*;

// Parameters with thread-safe atomic storage
struct MyParams {
    gain: AtomicU64,
}

impl Parameters for MyParams {
    fn count(&self) -> usize { 1 }
    fn get_normalized(&self, id: u32) -> f64 {
        f64::from_bits(self.gain.load(Ordering::Relaxed))
    }
    fn set_normalized(&self, id: u32, value: f64) {
        self.gain.store(value.to_bits(), Ordering::Relaxed);
    }
    // ... other required methods
}

// Plugin with DSP logic
struct MyPlugin {
    params: MyParams,
}

impl AudioProcessor for MyPlugin {
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers) {
        let gain = f64::from_bits(self.params.gain.load(Ordering::Relaxed)) as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

impl Plugin for MyPlugin {
    type Params = MyParams;

    fn params(&self) -> &Self::Params { &self.params }

    fn create() -> Self {
        Self { params: MyParams { gain: AtomicU64::new(1.0f64.to_bits()) } }
    }
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
cargo xtask bundle br-gain --release --install

# Or just bundle (output: target/release/BrGain.vst3)
cargo xtask bundle br-gain --release
```

## Examples

- **br-gain** - Simple gain effect plugin
- **br-midi-transform** - MIDI instrument that transposes notes

## Multi-Bus Audio

BEAMR separates main bus and auxiliary buses for sidechain and multi-output plugins:

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

BEAMR provides comprehensive MIDI support:

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
