# Examples

Working Beamer plugins demonstrating framework features.

**For a comprehensive feature coverage matrix and roadmap for future examples, see [docs/EXAMPLE_COVERAGE.md](../docs/EXAMPLE_COVERAGE.md).**

## Running Examples

```bash
# Build and install to user VST3 folder
cargo xtask bundle <example> --release --install

# Or just build (output: target/release/<Name>.vst3)
cargo xtask bundle <example> --release
```

## Available Examples

### [Gain](gain/)

Simple stereo gain effect with minimal complexity.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Gain** | Volume adjustment from -60 dB (silent) to +12 dB (boost). 0 dB = no change. |

**Demonstrates:**
- Two-phase lifecycle: `Descriptor` → `Processor` via `prepare()`
- `#[derive(Parameters)]` with declarative attributes
- `#[derive(HasParameters)]` for parameters access boilerplate
- `()` setup for plugins without sample-rate-dependent state
- `FloatParameter` with dB scaling
- Generic f32/f64 processing via `Sample` trait

```bash
cargo xtask bundle gain --release --install
```

---

### [Delay](delay/)

Tempo-synced stereo delay with ping-pong mode and factory presets.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Sync Mode** | How delay time is determined. "Free" uses manual time; note values (1/4, 1/8, 1/16, 1/32) lock to DAW tempo. |
| **Stereo Mode** | "Stereo" delays both channels equally. "Ping-Pong" bounces echoes between left and right for a wider sound. |
| **Time** | Delay time in milliseconds (1-2000ms). Only active when Sync Mode is "Free". |
| **Feedback** | Controls how many repeats you hear. 0% = single echo, higher = more repeats. |
| **Mix** | Blend between dry (original) and wet (delayed) signal. 0% = no effect, 100% = only echoes. |

**Factory Presets:**
- **Slapback**: Quick 80ms echo for doubling effect
- **Eighth Note**: Tempo-synced 1/8 note delay with moderate feedback
- **Ping Pong**: Quarter note bouncing between channels
- **Ambient**: Long 750ms delay with high feedback for textures

**Demonstrates:**
- `SampleRate` setup for sample-rate-dependent initialization
- `set_active()` for clearing delay buffers on reset
- `EnumParameter` for sync mode and stereo mode
- Tempo sync using `ProcessContext.samples_per_beat()`
- Declarative parameter smoothing with `smoothing = "exp:5.0"`
- Ring buffer delay line implementation
- Proper tail length via `tail_samples()`
- `#[derive(Presets)]` with factory presets for enum parameters

```bash
cargo xtask bundle delay --release --install
```

---

### [Compressor](compressor/)

Feed-forward compressor with soft/hard knee and sidechain input.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Threshold** | Level at which compression begins (-60 to 0 dB). Uses logarithmic mapping for finer control near 0 dB. |
| **Ratio** | Compression intensity. 2:1 (gentle) through 20:1 (near-limiting). |
| **Attack** | How fast compression engages (0.1-200 ms). Slower preserves transients. |
| **Release** | How fast compression releases (10-2000 ms). Faster = punchier, slower = smoother. |
| **Soft Knee** | Toggle between soft knee (gradual onset) and hard knee (abrupt). |
| **Auto Makeup** | Automatically compensate for volume reduction from compression. |
| **Makeup Gain** | Manual output boost (0-24 dB). Only active when Auto Makeup is off. |
| **Sidechain** | Use external sidechain input for detection instead of main input. |

**Typical Settings:**
- **Transparent leveling**: Low ratio (2:1), slow attack (50ms), medium release (200ms), soft knee
- **Punchy drums**: 4:1 ratio, fast attack (5ms), fast release (50ms), hard knee
- **Vocal control**: 4:1-8:1 ratio, medium attack (20ms), medium release (150ms), soft knee
- **Sidechain ducking**: Enable sidechain, route kick to sidechain input, high ratio, fast attack/release

**Demonstrates:**
- `SampleRate` setup for sample-rate-dependent envelope coefficients
- `BypassHandler` with `CrossfadeCurve::EqualPower` for click-free bypass
- `set_active()` for resetting envelope state on activation
- `kind = "db_log"` for logarithmic-feel threshold control
- Linear parameter smoothing (`smoothing = "linear:50.0"`)
- `EnumParameter` for discrete ratio values
- Multi-bus audio (main + sidechain input)
- dB-domain envelope processing

```bash
cargo xtask bundle compressor --release --install
```

---

### [Synthesizer](synthesizer/)

8-voice polyphonic synthesizer with expressive MIDI controls and parameter groups.

**Parameters** (organized in groups):

| Group | Parameter | Description |
|-------|-----------|-------------|
| **Oscillator** | Waveform | Oscillator shape: Sine, Saw, Square, or Triangle |
| **Envelope** | Attack | Envelope attack time (1-2000ms) |
| | Decay | Envelope decay time (1-2000ms) |
| | Sustain | Envelope sustain level (0-100%) |
| | Release | Envelope release time (1-5000ms) |
| **Filter** | Cutoff | Lowpass filter cutoff frequency (20-20000Hz, smoothed) |
| | Resonance | Filter resonance amount (0-95%, smoothed) |
| **Global** | Transpose | Pitch transpose (±2 octaves, -24 to +24 semitones) |
| | Gain | Master output level (-60 to +6 dB) |

**MIDI Controls:**
- **Pitch Bend** - ±2 semitones (standard range)
- **Mod Wheel (CC1)** - Controls both vibrato depth AND filter brightness (additive modulation)
- **Polyphonic Aftertouch** - Per-note vibrato control (requires poly aftertouch keyboard)
- **Channel Aftertouch** - Global vibrato control for all notes

**Expressive Performance:**
- **Vibrato depth**: Base controlled by mod wheel, enhanced by aftertouch (pressure)
- **Combined control**: Mod wheel + pressure = up to 2x vibrato depth (±2 semitones max)
- **Priority logic**: Polyphonic aftertouch overrides channel aftertouch per-note
- **Filter modulation**: Mod wheel opens cutoff by up to +8000 Hz for brightness

**Typical Settings:**
- **Pad**: Sine wave, slow attack (200ms), high sustain, long release (1000ms)
- **Bass**: Saw wave, fast attack (5ms), low cutoff (400Hz), short release, transpose -12
- **Lead**: Square wave, medium attack, high cutoff, add expression via mod wheel + aftertouch
- **Pluck**: Triangle wave, instant attack, short decay, low sustain, medium release

**Why MidiCcConfig?** VST3 doesn't pass pitch bend and CC messages directly to plugins. Instead, DAWs use `IMidiMapping` to convert them to parameter changes. `MidiCcConfig` tells the framework which controllers to enable - it creates hidden parameters that receive these values and converts them back to MIDI events for your plugin. No manual state management needed!

**Demonstrates:**
- `SampleRate` setup for sample-rate-dependent filter calculations
- `IntParameter` for transpose (±2 octaves in semitones)
- Flat parameter groups (`group = "..."`) - works in Cubase
- `MidiCcConfig` for pitch bend/mod wheel via IMidiMapping (framework manages state)
- Polyphonic aftertouch (PolyPressure) for per-note vibrato
- Channel aftertouch (ChannelPressure) for global vibrato
- Mod wheel controlling multiple parameters (vibrato + filter)
- Sample-accurate MIDI event processing
- Voice allocation with oldest-note stealing
- ADSR envelope generator
- One-pole lowpass filter with resonance
- Parameter smoothing (exponential for filter cutoff/resonance)
- Generic f32/f64 processing

```bash
cargo xtask bundle synthesizer --release --install
```
---

### [MIDI Transform](midi-transform/)

MIDI instrument that transforms notes and CC messages.

**Note Transform Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Enabled** | Toggle note processing on/off |
| **Mode** | How notes are transformed (see below) |
| **Transpose** | Semitones to shift (-24 to +24), only for Transpose mode |
| **Input Note** | Source note for Remap mode (0-127) |
| **Output Note** | Target note for Remap mode (0-127) |
| **Velocity** | Velocity scaling (0-200%). 100% = no change |

**Note Modes:**
- **Through** - Pass notes unchanged (with optional velocity scaling)
- **Transpose** - Shift all notes by semitones
- **Octave Up/Down** - Shift all notes by one octave
- **Remap Note** - Change one specific note to another (e.g., kick on C1 → D1)
- **Invert** - Mirror pitches around middle C (C4)

**CC Transform Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Enabled** | Toggle CC processing on/off |
| **Mode** | How CC messages are transformed (see below) |
| **Input CC** | Source CC number for Remap modes (0-127) |
| **Output CC** | Target CC number for Remap modes (0-127) |
| **Scale** | Value scaling (0-200%). 100% = no change |

**CC Modes:**
- **Through** - Pass CC messages unchanged
- **Remap CC** - Change CC number (e.g., Mod Wheel → Expression)
- **Scale** - Multiply all CC values
- **Invert** - Flip CC values (0→127, 127→0)
- **Remap + Scale** - Remap CC number AND scale value

**Demonstrates:**
- `SampleRate` setup for parameter smoothing
- Nested parameter groups with `#[nested(group = "...")]`
- `EnumParameter` for discrete choices
- `IntParameter` for note/CC selection
- `BoolParameter` for enable toggles
- `process_midi()` for MIDI processing

```bash
cargo xtask bundle midi-transform --release --install
```
