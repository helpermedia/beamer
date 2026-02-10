# BEAMER

A Rust framework for building Audio Unit (AU) and VST3 audio plugins.

Named after the beams connecting notes in sheet music and from Dutch where "beamer" means projector. Beamer projects your DSP logic onto AU (macOS) and VST3 (macOS, Windows) through modern web UI (planned) from a single codebase.

> [!NOTE]
> Beamer is pre-1.0 and under active development. Expect breaking changes between minor versions.

## Why Beamer?

**Built on Rust's guarantees.** Where most plugin frameworks use C++, Beamer uses Rust. Memory and threading bugs become compile-time errors, not runtime crashes.

**Derive macros do the heavy lifting.** `#[derive(Parameters)]` generates host integration, state persistence, DAW automation, and parameter access traits automatically. `#[beamer::export]` reads Config.toml at compile time to generate plugin metadata and entry points.

**Clean separation of concerns.** DSP code stays in safe Rust while the planned WebView architecture enables modern interfaces with HTML, CSS, and JavaScript. Plugin configuration lives in TOML files. Beamer bridges them together and handles plugin format complexity.

## Quick Start

```rust
use beamer::prelude::*;

// 1. Parameters - pure data with derive macros
#[derive(Parameters)]
struct GainParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,
}

// 2. Descriptor - holds parameters (and optional state), describes plugin to host
#[beamer::export]
#[derive(Default, HasParameters)]
struct GainDescriptor {
    #[parameters]
    parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    // No setup needed for simple effects; use SampleRate for delays, MaxBufferSize for FFT
    type Setup = ();
    type Processor = GainProcessor;

    fn prepare(self, _: ()) -> GainProcessor {
        GainProcessor { parameters: self.parameters }
    }
}

// 3. Processor - prepared state, ready for audio
#[derive(HasParameters)]
struct GainProcessor {
    #[parameters]
    parameters: GainParameters,
}

impl Processor for GainProcessor {
    type Descriptor = GainDescriptor;

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

**Config.toml** (place in crate root next to Cargo.toml):
```toml
name = "My Gain Plugin"
category = "effect"
subcategories = ["dynamics"]
manufacturer_code = "Manu"
plugin_code = "gain"
vendor = "My Company"
url = "https://example.com"
email = "support@example.com"
```

## Plugin Structure

Beamer plugins use three structs: **Parameters** (data), **Descriptor** (plugin blueprint), and **Processor** (audio and MIDI). The host calls `prepare(setup)` to transition from Descriptor to Processor when sample rate becomes available, ensuring audio buffers are properly allocated before `process()` runs.

See [ARCHITECTURE.md](ARCHITECTURE.md#plugin-lifecycle) for detailed rationale.

## Examples

### Effects

| Example | Description |
|---------|-------------|
| **[gain](https://github.com/helpermedia/beamer/tree/main/examples/gain)** | Simple stereo gain plugin |
| **[compressor](https://github.com/helpermedia/beamer/tree/main/examples/compressor)** | Feed-forward compressor with sidechain input |
| **[equalizer](https://github.com/helpermedia/beamer/tree/main/examples/equalizer)** | 3-band parametric EQ |
| **[delay](https://github.com/helpermedia/beamer/tree/main/examples/delay)** | Tempo-synced stereo delay with ping-pong mode |

### Instruments & MIDI

| Example | Description |
|---------|-------------|
| **[synthesizer](https://github.com/helpermedia/beamer/tree/main/examples/synthesizer)** | 8-voice polyphonic synth with ADSR and filter |
| **[drums](https://github.com/helpermedia/beamer/tree/main/examples/drums)** | Drum synthesizer with multi-output buses |
| **[midi-transform](https://github.com/helpermedia/beamer/tree/main/examples/midi-transform)** | MIDI effect for note/CC transformation |

See the [examples](https://github.com/helpermedia/beamer/tree/main/examples) for detailed documentation on each plugin.

## Features

- **Multi-format** - AU (macOS) and VST3 (macOS, Windows)
- **Declarative parameters** - `#[derive(Parameters)]` with attributes for units, smoothing, and more
- **Type-safe initialization** - `prepare()` lifecycle eliminates placeholder values and sample-rate bugs
- **Format-agnostic core** - Plugin logic is independent of format specifics
- **32-bit and 64-bit audio** - Native f64 support or automatic conversion for f32-only plugins
- **Multi-bus audio** - Main bus + auxiliary buses (sidechain, aux sends, multi-out)
- **Complete MIDI support** - Full MIDI 1.0/2.0, MPE, Note Expression, SysEx
- **Real-time safe** - No heap allocations in the audio path
- **State persistence** - Automatic preset/state save and restore
- **WebView GUI** (planned) - Modern web-based plugin interfaces

## Documentation

- [ARCHITECTURE.md](https://github.com/helpermedia/beamer/blob/main/ARCHITECTURE.md) - Design decisions, threading model, guarantees
- [REFERENCE.md](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md) - Detailed API reference
- [EXAMPLE_COVERAGE.md](https://github.com/helpermedia/beamer/blob/main/docs/EXAMPLE_COVERAGE.md) - Example testing roadmap and feature coverage matrix

## Platform Support

| Platform | Status |
|----------|--------|
| macOS | Tested (arm64) |
| Windows | Untested |

Contributions for testing and fixes on Windows are welcome.

## Crates

| Crate | Description |
|-------|-------------|
| `beamer` | Main facade crate (re-exports everything) |
| `beamer-core` | Platform-agnostic traits and types |
| `beamer-macros` | Derive macros for parameters, `#[beamer::export]` for config and presets |
| `beamer-utils` | Internal utilities (zero external dependencies) |
| `beamer-au` | AU wrapper (macOS) - AUv2 and AUv3 via shared C-ABI bridge |
| `beamer-vst3` | VST3 wrapper implementation |

## Building & Installation

```bash
cargo xtask bundle gain --auv3 --vst3 --release            # Build both formats
cargo xtask bundle gain --auv3 --vst3 --release --install  # Build and install
```

Use `--auv2` for AUv2 instead of AUv3. For universal binaries (x86_64 + arm64), add `--arch universal`.

## License

MIT
