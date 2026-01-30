# BEAMER

A Rust framework for building VST3, Audio Unit (AU), and CLAP audio plugins.

Named after the beams that connect notes in sheet music, Beamer links your DSP logic and WebView interface together, then projects them onto any surface through modern web UI. Write your plugin once, export to VST3 (all platforms), AU (macOS), and CLAP (planned).

> [!NOTE]
> Beamer is pre-1.0 and under active development. Expect breaking changes between minor versions.

## Why Beamer?

**Built on Rust's guarantees.** Where most plugin frameworks use C++, Beamer uses Rust. Memory and threading bugs become compile-time errors, not runtime crashes.

**Derive macros do the heavy lifting.** Define your parameters with `#[derive(Parameters)]` and Beamer generates host integration, state persistence, DAW automation, and parameter access traits. Focus on your DSP, not boilerplate.

**Web developers build your UI.** Beamer's WebView architecture (planned) lets frontend developers create modern plugin interfaces using familiar tools (HTML, CSS, JavaScript) while your audio code stays in safe Rust. Each team does what they do best.

**For creative developers.** Whether you're an audio engineer learning Rust or a Rust developer exploring audio, Beamer handles the plugin format plumbing so you can focus on what matters: making something that sounds great.

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
#[derive(Default, HasParameters)]
struct GainDescriptor {
    #[parameters]
    parameters: GainParameters,
}

impl Descriptor for GainDescriptor {
    // No setup needed for simple effects,
    // but for more complex plugins use SampleRate or MaxBufferSize here
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

// Export with: export_vst3!(GainDescriptor, "Gain", "Bmer");
```

## Plugin Structure

Beamer plugins use three structs: **Parameters** (data), **Descriptor** (configuration), and **Processor** (audio). The host calls `prepare(setup)` to transition from Descriptor to Processor when sample rate becomes available, ensuring audio buffers are properly allocated before `process()` runs.

See [ARCHITECTURE.md](ARCHITECTURE.md#plugin-lifecycle) for detailed rationale.

## Examples

### Effects

| Example | Description |
|---------|-------------|
| **[gain](https://github.com/helpermedia/beamer/tree/main/examples/gain)** | Simple stereo gain plugin |
| **[compressor](https://github.com/helpermedia/beamer/tree/main/examples/compressor)** | Feed-forward compressor with sidechain input |
| **[equalizer](https://github.com/helpermedia/beamer/tree/main/examples/equalizer)** | 3-band parametric EQ |
| **[delay](https://github.com/helpermedia/beamer/tree/main/examples/delay)** | Tempo-synced stereo delay with ping-pong mode |

### Instruments

| Example | Description |
|---------|-------------|
| **[synthesizer](https://github.com/helpermedia/beamer/tree/main/examples/synthesizer)** | 8-voice polyphonic synth with ADSR and filter |
| **[midi-transform](https://github.com/helpermedia/beamer/tree/main/examples/midi-transform)** | MIDI processor for note/CC transformation |

See the [examples](https://github.com/helpermedia/beamer/tree/main/examples) for detailed documentation on each plugin.

## Features

- **Multi-format** - VST3 (all platforms), AU (macOS), and CLAP (planned)
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
| macOS (arm64) | Tested |
| Windows | Untested |
| Linux | Untested |

Contributions for testing and fixes on Windows and Linux are welcome.

## Crates

| Crate | Description |
|-------|-------------|
| `beamer` | Main facade crate (re-exports everything) |
| `beamer-core` | Platform-agnostic traits and types |
| `beamer-vst3` | VST3 wrapper implementation |
| `beamer-au` | AU wrapper (macOS) - AUv2 and AUv3 via shared C-ABI bridge |
| `beamer-clap` | CLAP wrapper (planned) |
| `beamer-macros` | Derive macros for parameters and presets |
| `beamer-utils` | Internal utilities (zero external dependencies) |

## Building & Installation

```bash
cargo xtask bundle gain --vst3 --release            # Build VST3
cargo xtask bundle gain --vst3 --release --install  # Build and install
```

For AU formats, add `--auv2` or `--auv3`. For universal binaries (x86_64 + arm64), add `--arch universal`.

## License

MIT
