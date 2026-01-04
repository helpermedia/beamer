# Examples

Working Beamer plugins demonstrating framework features.

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
