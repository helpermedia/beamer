# Beamer Framework - Example Coverage & Testing Roadmap

**Purpose:** This document tracks which framework features are tested by example plugins and provides a roadmap for comprehensive feature coverage. Examples serve as both documentation and integration tests - implementing features in examples helps discover bugs early.

**Last Updated:** 2026-04-06
**Current Examples:** gain, compressor, equalizer, delay, synthesizer, midi-transform, drums, webview-demo

---

## Feature Coverage Matrix

| Feature Category | Feature | Gain | Compressor | Equalizer | Delay | Synthesizer | MIDI Transform | Drums | WebView Demo | Notes |
|-----------------|---------|------|------------|-----------|-------|-------------|----------------|-------|--------------|-------|
| **Parameters** | FloatParameter | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Core parameter type |
| | IntParameter | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ❌ | Transpose (synthesizer), note/CC numbers (midi-transform) |
| | BoolParameter | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | Enable toggles, bypass, soft knee |
| | EnumParameter | ❌ | ✅ | ❌ | ✅ | ✅ | ✅ | ❌ | ❌ | Waveform, sync, ratio |
| **Smoothing** | Exponential | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | Feedback, mix, cutoff |
| | Linear | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Attack/release smoothing |
| **Range Mapping** | LinearMapper | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | Default mapping |
| | PowerMapper | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Threshold (db_log) |
| | LogMapper | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | Frequency parameters (kind = "hz") |
| | LogOffsetMapper | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| **Organization** | Units (parameter groups) | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ | VST3 units (works in Cubase, see notes) |
| | Nested groups (`#[nested]`) | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | Rust code organization only? |
| | Flat groups (group = "...") | ❌ | ❌ | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ | Equalizer (3 groups), Synthesizer (4 groups) |
| | Hz Formatter | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | Frequency display via kind = "hz" |
| | bypass attribute | ❌ | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | Special bypass parameter marker |
| | Factory Presets | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | Presets.toml file |
| **Processing** | f32 processing | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | All support f32 |
| | f64 processing | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | All support f64 |
| | tail_samples | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ❌ | ❌ | Delay decay, envelope release |
| | latency_samples | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | set_active | ❌ | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | Reset state on activation |
| **Bypass** | BypassHandler | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Split API (begin/finish) |
| | CrossfadeCurve | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | EqualPower curve |
| | bypass_ramp_samples | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | Reports ramp to host |
| **Buses** | Stereo main | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ | Drums uses mono |
| | Mono bus | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | 4 mono outputs (drums) |
| | Sidechain input (AuxInput) | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | External key |
| | Aux output (AuxOutput) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | 3 mono aux buses (drums) |
| **Transport** | tempo access | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | Used for tempo sync |
| | is_playing | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | samples_per_beat | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | ❌ | Delay tempo sync |
| **MIDI - Basic** | NoteOn/NoteOff | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ✅ | ❌ | Synthesizer voices, drum triggering |
| | PitchBend | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | Synth ±2 semitones |
| | ControlChange (CC) | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ❌ | Mod wheel, transform |
| | MidiCcConfig | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | VST3 CC emulation |
| | PolyPressure | ❌ | ❌ | ❌ | ❌ | ✅ | ✅ | ❌ | ❌ | Per-note vibrato, transform |
| | ChannelPressure | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ | Global vibrato (synthesizer) |
| | ProgramChange | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| **MIDI - Advanced** | Note Expression | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** (MPE) |
| | Keyswitch Controller | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** (orchestral) |
| | Physical UI Mapping | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** (MPE) |
| | MPE Support | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | MIDI Learn | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | MIDI Mapping | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | SysEx | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | RpnTracker | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | 14-bit CC | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | MIDI 2.0 | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | ChordInfo/ScaleInfo | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| **GUI** (macOS only) | WebViewHandler | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | Bidirectional IPC |
| | has_gui / gui_size | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | Config.toml GUI config |
| | gui_background_color | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | Prevents white flash |
| | Embedded assets | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | webview/dist/ via custom URL scheme |
| | Parameter sync (JS ↔ Rust) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | __BEAMER__.invoke() and polling |
| | DAW undo grouping | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ✅ | beginEdit/endEdit |

**Legend:**
- ✅ Tested/Used
- ❌ Not tested
- 🚧 Partially tested

---

## Planned Examples

#### Lookahead Limiter
**Goal:** Test latency reporting and advanced dynamics

**Features to test:**
- ✅ `latency_samples()` - Report lookahead buffer size
- ✅ `BoolParameter` - True peak mode on/off
- ✅ Delay buffer - Lookahead implementation
- ✅ Advanced smoothing - Gain reduction smoothing

**Implementation notes:**
- Brick-wall limiter with configurable lookahead (0-10ms)
- True peak detection option
- Reports latency to host based on lookahead time
- Smooth gain reduction using exponential smoothing

**Files to create:**
- `examples/limiter/src/lib.rs`
- `examples/limiter/Cargo.toml`

---

#### MPE Synthesizer
**Goal:** Test MPE, note expression, physical UI mapping

**Features to test:**
- ✅ Note Expression Controller - Per-note volume, pan, brightness
- ✅ Physical UI Mapping - X-axis → pan, Y-axis → brightness, Pressure → volume
- ✅ MPE Support - `enable_mpe_input_processing`, `set_mpe_input_device_settings`
- ✅ Per-note expression events
- ✅ Multi-dimensional per-note control

**Implementation notes:**
- MPE-capable polyphonic synthesizer
- Supports slide (X), slide (Y), pressure (Z)
- Map physical gestures to timbral parameters
- Each voice responds to its own note expression
- Compatible with ROLI Seaboard, Linnstrument, etc.

**Files to create:**
- `examples/mpe-synthesizer/src/lib.rs`
- `examples/mpe-synthesizer/Cargo.toml`

---

#### Orchestral Sampler
**Goal:** Test keyswitch controller, program change

**Features to test:**
- ✅ Keyswitch Controller - Articulation switching
- ✅ `keyswitch_count()`, `keyswitch_info()`
- ✅ `ProgramChange` - Preset switching
- ✅ Sample playback - Basic sampler functionality

**Implementation notes:**
- Simple sampler with 3-4 articulations (sustain, staccato, pizzicato)
- Keyswitches for articulation selection (C0, C#0, D0)
- Program change support for preset switching
- Basic sample playback (could use sine waves as "samples" for demo)

**Files to create:**
- `examples/orchestral-sampler/src/lib.rs`
- `examples/orchestral-sampler/Cargo.toml`

---

#### MIDI Processor
**Goal:** Test RPN/NRPN, 14-bit CC, MIDI learn, PolyPressure

**Features to test:**
- ✅ `RpnTracker` - RPN/NRPN message assembly
- ✅ 14-bit CC utilities - High-res parameter control
- ✅ MIDI Learn - `on_midi_learn()`, `on_midi1_learn()`
- ✅ MIDI Mapping - `midi_cc_to_parameter()`, `midi1_assignments()`
- ✅ `PolyPressure` - Per-note aftertouch
- ✅ `ChannelPressure` - Channel aftertouch
- ✅ `SysEx` - Custom device messages

**Implementation notes:**
- MIDI effects processor/utility
- RPN/NRPN tracking and display
- Convert 14-bit CC to parameters
- MIDI learn mode for CC mapping
- Pass-through with optional transformations
- Poly aftertouch → CC conversion

**Files to create:**
- `examples/midi-processor/src/lib.rs`
- `examples/midi-processor/Cargo.toml`

---

---

#### Multi-Bus Router
**Goal:** Test multiple aux buses with stereo outputs and complex routing

**Features to test:**
- ✅ `AuxOutput` - Multiple output buses (mono tested in drums, stereo needed)
- ✅ Multiple aux input/output buses
- ✅ Complex bus routing
- ✅ `output_bus_info()` - Custom output configuration

**Implementation notes:**
- Audio router with multiple inputs and outputs
- Route/mix any input to any output
- Demonstrates complex bus configurations with stereo aux buses
- Gain control per route
- **Note:** Mono aux outputs already tested in drums example

**Files to create:**
- `examples/router/src/lib.rs`
- `examples/router/Cargo.toml`

---

## Notes

- **Bug Discovery:** Implementing examples has helped find bugs in MidiCcConfig and smoothing
- **Real-World Testing:** Examples should reflect actual use cases, not contrived scenarios
- **Keep Simple:** Examples should be minimal while demonstrating features effectively
- **Cross-Reference:** Link examples in REFERENCE.md feature documentation
- **Document Maintenance:** Update coverage matrix after each new example
