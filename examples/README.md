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

Simple stereo gain effect with sidechain ducking.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Gain** | Volume adjustment from -60 dB (silent) to +12 dB (boost). 0 dB = no change. |

**Sidechain Ducking:** When a sidechain input is connected, the plugin automatically reduces gain when the sidechain signal is loud (like a kick drum). This creates the classic "pumping" effect used in EDM and radio production.

**Demonstrates:**
- `#[derive(Params)]` with declarative attributes
- `FloatParam` with dB scaling
- Multi-bus audio (main + sidechain input)
- Generic f32/f64 processing via `Sample` trait

```bash
cargo xtask bundle gain --release --install
```

---

### [Delay](delay/)

Tempo-synced stereo delay with ping-pong mode.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Sync Mode** | How delay time is determined. "Free" uses manual time; note values (1/4, 1/8, 1/16, 1/32) lock to DAW tempo. |
| **Stereo Mode** | "Stereo" delays both channels equally. "Ping-Pong" bounces echoes between left and right for a wider sound. |
| **Time** | Delay time in milliseconds (1-2000ms). Only active when Sync Mode is "Free". |
| **Feedback** | Controls how many repeats you hear. 0% = single echo, higher = more repeats. |
| **Mix** | Blend between dry (original) and wet (delayed) signal. 0% = no effect, 100% = only echoes. |

**Typical Settings:**
- **Slapback**: Free mode, 80-120ms, low feedback (10-20%), mix to taste
- **Rhythmic delay**: 1/8 note, Ping-Pong, feedback 30-50%
- **Ambient tail**: 1/4 note, high feedback (60-80%), low mix (20-30%)

**Demonstrates:**
- `EnumParam` for sync mode and stereo mode
- Tempo sync using `ProcessContext.samples_per_beat()`
- Declarative parameter smoothing with `smoothing = "exp:5.0"`
- Ring buffer delay line implementation
- Proper tail length via `tail_samples()`

```bash
cargo xtask bundle delay --release --install
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
- Nested parameter groups with `#[nested(group = "...")]`
- `EnumParam` for discrete choices
- `IntParam` for note/CC selection
- `BoolParam` for enable toggles
- `process_midi()` for MIDI processing

```bash
cargo xtask bundle midi-transform --release --install
```

---

### [Synth](synth/)

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

**Why MidiCcParams?** VST3 doesn't pass pitch bend and CC messages directly to plugins. Instead, DAWs use `IMidiMapping` to convert them to parameter changes. `MidiCcParams` creates hidden parameters that receive these values and converts them back to MIDI events for your plugin.

**Demonstrates:**
- `IntParam` for transpose (±2 octaves in semitones)
- Flat parameter groups (`group = "..."`) - works in Cubase
- `MidiCcParams` for pitch bend/mod wheel via IMidiMapping
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
cargo xtask bundle synth --release --install
```
