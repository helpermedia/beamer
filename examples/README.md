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

### [gain](gain/)

Simple stereo gain effect with sidechain ducking.

**Demonstrates:**
- `#[derive(Params)]` with declarative attributes
- `FloatParam` with dB scaling
- Multi-bus audio (main + sidechain input)
- Generic f32/f64 processing via `Sample` trait
- Transport info access

```bash
cargo xtask bundle gain --release --install
```

### [midi-transform](midi-transform/)

MIDI instrument that transforms notes and CC messages.

**Demonstrates:**
- Nested parameter groups with `#[nested(group = "...")]`
- `EnumParam` for discrete choices (transform modes)
- `IntParam` for note/CC selection
- `BoolParam` for enable toggles
- `process_midi()` for MIDI processing

```bash
cargo xtask bundle midi-transform --release --install
```
