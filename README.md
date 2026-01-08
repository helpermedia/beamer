# BEAMER

A Rust framework for building VST3 audio plugins.

Named after the beams that connect notes in sheet music, Beamer links your DSP logic and WebView interface together, then projects them onto any surface through modern web UI. It bridges VST3's C++ COM interfaces with safe Rust abstractions.

## Why Beamer?

Audio plugin development has traditionally meant wrestling with C++ memory management, threading bugs, and cryptic SDK interfaces—time spent debugging instead of creating. Beamer changes this.

**Built on Rust's guarantees.** Memory leaks, dangling pointers, and data races are caught at compile time, not discovered during a live session. Your plugin won't crash someone's mix because of a subtle threading bug.

**No SDK hassle.** The [VST3 SDK](https://github.com/steinbergmedia/vst3sdk) is now MIT licensed (as of v3.8), making it available as a standard Rust dependency—no separate SDK downloads or licensing agreements required. Beamer uses [Coupler's vst3 crate](https://github.com/coupler-rs/vst3-rs) for Rust bindings.

**Derive macros do the heavy lifting.** Define your parameters with `#[derive(Parameters)]` and Beamer generates VST3 integration, state persistence, and DAW automation. Use `#[derive(HasParameters)]` to eliminate repetitive accessor boilerplate. Focus on your DSP, not boilerplate.

**Web developers build your UI.** Beamer's WebView architecture (planned) lets frontend developers create modern plugin interfaces using familiar tools—HTML, CSS, JavaScript—while your audio code stays in safe Rust. Each team does what they do best.

**For creative developers.** Whether you're an audio engineer learning Rust or a Rust developer exploring audio, Beamer handles the VST3 plumbing so you can focus on what matters: making something that sounds great.

## Quick Start

```rust
use beamer::prelude::*;
use beamer::{HasParameters, Parameters};

// Declarative parameters - macro generates Default, VST3 integration, state persistence
#[derive(Parameters)]
struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParam,
}

// Plugin (unprepared state) - holds parameters before audio config is known
// HasParameters derive eliminates parameters()/parameters_mut() boilerplate
#[derive(Default, HasParameters)]
struct GainPlugin {
    #[parameters]
    parameters: GainParameters,
}

impl Plugin for GainPlugin {
    type Config = NoConfig;  // Simple gain doesn't need sample rate
    type Processor = GainProcessor;

    fn prepare(self, _config: NoConfig) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

// Processor (prepared state) - ready for audio processing
#[derive(HasParameters)]
struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
}

impl AudioProcessor for GainProcessor {
    type Plugin = GainPlugin;

    fn unprepare(self) -> GainPlugin {
        GainPlugin { parameters: self.parameters }
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        let gain = self.parameters.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}
```

## Two-Phase Lifecycle

Beamer uses a type-safe two-phase initialization that eliminates placeholder values:

```text
Plugin::default() → Plugin (unprepared, holds parameters)
                         │
                         ▼  prepare(config)
                         │
                    AudioProcessor (prepared, ready for audio)
                         │
                         ▼  unprepare()
                         │
                    Plugin (parameters preserved)
```

**Why?** Audio plugins need sample rate for buffer allocation, filter coefficients, and envelope timing—but the sample rate isn't known until the host calls `setupProcessing()`. A common approach is using placeholder values, with Beamer `Plugin` holds parameters, `prepare()` transforms it into an `AudioProcessor` with real configuration. No placeholders.

| Config Type | Use Case |
|-------------|----------|
| `NoConfig` | Stateless plugins (gain, pan) |
| `AudioSetup` | Most plugins (delay, compressor, synth) |
| `FullAudioSetup` | Plugins needing channel layout info |

## Examples

### Effects

| Example | Description |
|---------|-------------|
| **[gain](https://github.com/helpermedia/beamer/tree/main/examples/gain)** | Simple stereo gain with sidechain ducking. Demonstrates `#[derive(Parameters)]`, dB scaling, and multi-bus audio. |
| **[delay](https://github.com/helpermedia/beamer/tree/main/examples/delay)** | Tempo-synced stereo delay with ping-pong mode. Shows `EnumParam`, tempo sync via `ProcessContext`, and parameter smoothing. |
| **[compressor](https://github.com/helpermedia/beamer/tree/main/examples/compressor)** | Feed-forward compressor with soft/hard knee and sidechain input. Demonstrates `BypassHandler` with equal-power crossfade, `set_active()` for state reset, and auto makeup gain. |

### Instruments

| Example | Description |
|---------|-------------|
| **[synth](https://github.com/helpermedia/beamer/tree/main/examples/synth)** | 8-voice polyphonic synthesizer with full ADSR envelope and lowpass filter. Features expressive MIDI: polyphonic aftertouch for per-note vibrato, channel aftertouch, pitch bend, and mod wheel controlling both vibrato depth and filter brightness. |
| **[midi-transform](https://github.com/helpermedia/beamer/tree/main/examples/midi-transform)** | MIDI processor that transforms notes and CC messages. Shows nested parameter groups, `process_midi()`, and various transform modes (transpose, remap, invert). |

See the [examples](https://github.com/helpermedia/beamer/tree/main/examples) for detailed documentation on each plugin.

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
- **VST3 CC emulation** via `MidiCcConfig` - receive MIDI CC in DAWs that don't send raw CC events

## Parameter Attributes

The `#[parameter(...)]` attribute supports:

| Attribute | Description |
|-----------|-------------|
| `id = "..."` | Required. String ID (hashed to u32 for VST3) |
| `name = "..."` | Display name in DAW |
| `default = <value>` | Default value |
| `range = start..=end` | Value range |
| `kind = "..."` | Unit type: `db`, `hz`, `ms`, `seconds`, `percent`, `pan`, `ratio`, `linear`, `semitones` |
| `group = "..."` | Visual grouping in DAW (flat access, grouped display) |
| `smoothing = "exp:5.0"` | Parameter smoothing (`exp` or `linear`) |
| `bypass` | Mark as bypass parameter (BoolParameter only) |

### Visual Grouping

Use `group = "..."` for flat parameter access with DAW grouping:

```rust
#[derive(Parameters)]
struct SynthParameters {
    #[parameter(id = "cutoff", name = "Cutoff", default = 1000.0, range = 20.0..=20000.0, kind = "hz", group = "Filter")]
    cutoff: FloatParam,

    #[parameter(id = "reso", name = "Resonance", default = 0.5, range = 0.0..=1.0, group = "Filter")]
    resonance: FloatParam,

    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db", group = "Output")]
    gain: FloatParam,
}

// Access: parameters.cutoff, parameters.resonance, parameters.gain (flat)
// DAW shows collapsible "Filter" and "Output" groups
```

For nested structs with separate parameter groups, use `#[nested(group = "...")]`.

## Features

- **Type-safe initialization** - Two-phase lifecycle eliminates placeholder values and sample-rate bugs
- **Format-agnostic core** - Plugin logic is independent of VST3 specifics
- **32-bit and 64-bit audio** - Native f64 support or automatic conversion for f32-only plugins
- **Multi-bus audio** - Main bus + auxiliary buses (sidechain, aux sends, multi-out)
- **Complete MIDI support** - All VST3 SDK 3.8.0 MIDI features including MPE, Note Expression, and MIDI 2.0
- **Real-time safe** - No heap allocations in the audio path
- **State persistence** - Automatic preset/state save and restore
- **WebView GUI** (planned) - Modern web-based plugin interfaces

## Documentation

- [ARCHITECTURE.md](https://github.com/helpermedia/beamer/blob/main/ARCHITECTURE.md) — Design decisions, threading model, guarantees
- [REFERENCE.md](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md) — Detailed API reference
- [EXAMPLE_COVERAGE.md](https://github.com/helpermedia/beamer/blob/main/docs/EXAMPLE_COVERAGE.md) — Example testing roadmap and feature coverage matrix

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
| `beamer-macros` | Derive macros (`#[derive(Parameters)]`, `#[derive(HasParameters)]`, `#[derive(EnumParam)]`) |
| `beamer-utils` | Internal utilities (zero external dependencies) |

## Building & Installation

```bash
# Build all crates
cargo build

# Build release
cargo build --release

# Build, bundle, and install to user VST3 folder (macOS)
cargo xtask bundle gain --release --install

# Or just bundle (output: target/release/BeamerGain.vst3)
cargo xtask bundle gain --release
```

## License

MIT
