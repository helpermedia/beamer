# Beamer Framework - Example Coverage & Testing Roadmap

**Purpose:** This document tracks which framework features are tested by example plugins and provides a roadmap for comprehensive feature coverage. Examples serve as both documentation and integration tests - implementing features in examples helps discover bugs early.

**Last Updated:** 2026-01-06
**Current Examples:** gain, delay, synth, midi-transform, compressor

---

## Table of Contents

- [Feature Coverage Matrix](#feature-coverage-matrix)
- [Untested Features](#untested-features)
- [Planned Examples](#planned-examples)
- [Example Enhancement Opportunities](#example-enhancement-opportunities)
- [Testing Strategy](#testing-strategy)

---

## Feature Coverage Matrix

| Feature Category | Feature | Gain | Delay | Synth | MIDI Transform | Compressor | Notes |
|-----------------|---------|------|-------|-------|----------------|------------|-------|
| **Parameters** | FloatParameter | ✅ | ✅ | ✅ | ✅ | 🚧 | Core parameter type |
| | IntParameter | ❌ | ❌ | ✅ | ✅ | ❌ | Transpose (synth), note/CC numbers (midi-transform) |
| | BoolParameter | ❌ | ❌ | ❌ | ✅ | 🚧 | Enable toggles, bypass, soft knee |
| | EnumParameter | ❌ | ✅ | ✅ | ✅ | 🚧 | Waveform, sync, ratio |
| **Smoothing** | Exponential | ❌ | ✅ | ✅ | ❌ | ❌ | Feedback, mix, cutoff |
| | Linear | ❌ | ❌ | ❌ | ❌ | 🚧 | Attack/release smoothing |
| **Range Mapping** | LinearMapper | ✅ | ✅ | ✅ | ✅ | 🚧 | Default mapping |
| | PowerMapper | ❌ | ❌ | ❌ | ❌ | 🚧 | Threshold (db_log) |
| | LogOffsetMapper | ❌ | ❌ | ❌ | ❌ | ❌ | Available but not used |
| **Organization** | Units (parameter groups) | ❌ | ❌ | ✅ | ❌ | ❌ | VST3 units (works in Cubase, see notes) |
| | Nested groups (#[nested]) | ❌ | ❌ | ❌ | ✅ | ❌ | Rust code organization only? |
| | Flat groups (group = "...") | ❌ | ❌ | ✅ | ❌ | ❌ | Synth uses 4 groups (works in Cubase) |
| | Custom Formatter | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | bypass attribute | ❌ | ❌ | ❌ | ✅ | 🚧 | Special bypass parameter marker |
| **Processing** | f32 processing | ✅ | ✅ | ✅ | ✅ | 🚧 | All support f32 |
| | f64 processing | ✅ | ✅ | ✅ | ✅ | 🚧 | All support f64 |
| | tail_samples | ❌ | ✅ | ✅ | ❌ | ❌ | Delay decay, envelope release |
| | latency_samples | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | set_active | ❌ | ❌ | ❌ | ❌ | 🚧 | Reset envelope on activation |
| **Bypass** | BypassHandler | ❌ | ❌ | ❌ | ❌ | 🚧 | Split API (begin/finish) |
| | CrossfadeCurve | ❌ | ❌ | ❌ | ❌ | 🚧 | EqualPower curve |
| | bypass_ramp_samples | ❌ | ❌ | ❌ | ❌ | 🚧 | Reports ramp to host |
| **Buses** | Stereo main | ✅ | ✅ | ✅ | ✅ | 🚧 | All use stereo |
| | Mono bus | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | Sidechain input (AuxInput) | ✅ | ❌ | ❌ | ❌ | 🚧 | Gain ducking, external key |
| | Aux output (AuxOutput) | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| **Transport** | tempo access | ✅ | ✅ | ❌ | ❌ | ❌ | Used for tempo sync |
| | is_playing | ✅ | ❌ | ❌ | ❌ | ❌ | Read but unused |
| | samples_per_beat | ❌ | ✅ | ❌ | ❌ | ❌ | Delay tempo sync |
| **MIDI - Basic** | NoteOn/NoteOff | ❌ | ❌ | ✅ | ✅ | ❌ | Synth voices |
| | PitchBend | ❌ | ❌ | ✅ | ❌ | ❌ | Synth ±2 semitones |
| | ControlChange (CC) | ❌ | ❌ | ✅ | ✅ | ❌ | Mod wheel, transform |
| | MidiCcConfig | ❌ | ❌ | ✅ | ❌ | ❌ | VST3 CC emulation |
| | PolyPressure | ❌ | ❌ | ✅ | ✅ | ❌ | Per-note vibrato, transform |
| | ChannelPressure | ❌ | ❌ | ✅ | ❌ | ❌ | Global vibrato (synth) |
| | ProgramChange | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| **MIDI - Advanced** | Note Expression | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** (MPE) |
| | Keyswitch Controller | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** (orchestral) |
| | Physical UI Mapping | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** (MPE) |
| | MPE Support | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | MIDI Learn | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | MIDI Mapping | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | SysEx | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | RpnTracker | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | 14-bit CC | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | MIDI 2.0 | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| | ChordInfo/ScaleInfo | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |
| **Editor** | EditorDelegate | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** (WebView) |
| | EditorConstraints | ❌ | ❌ | ❌ | ❌ | ❌ | **UNTESTED** |

**Legend:**
- ✅ Tested/Used
- ❌ Not tested
- 🚧 Partially tested

---

## Untested Features

### High Priority (Core Functionality)

1. **Parameter Types**
   - ✅ ~~`IntParameter`~~ - Tested in midi-transform
   - ✅ ~~`BoolParameter`~~ - Tested in midi-transform
   - `LogMapper` - Logarithmic parameter scaling

2. **Bypass Handling**
   - 🚧 `BypassHandler` - Implemented in compressor (split API: begin/finish), needs DAW testing
   - 🚧 `CrossfadeCurve` - Implemented in compressor (EqualPower), needs DAW testing
   - 🚧 `bypass_ramp_samples()` - Implemented in compressor, needs DAW testing

3. **Bus Configuration**
   - Mono buses (all examples use stereo)
   - `AuxOutput` - Auxiliary output buses
   - Multiple aux buses

4. **Processing Callbacks**
   - `latency_samples()` - Lookahead reporting
   - 🚧 `set_active()` - Implemented in compressor (reset envelope on activation), needs DAW testing

### Medium Priority (Advanced Features)

5. **Parameter Organization**
   - ✅ ~~Units system~~ - Tested in synth (4 flat groups: Oscillator, Envelope, Filter, Global - **works in Cubase**)
   - ✅ ~~Nested groups~~ - Tested in midi-transform (`#[nested]` - **may be just Rust organization, not DAW-visible**)
   - ✅ ~~Flat groups (`group = "..."`)~~ - Tested in synth (**works in Cubase, verified with screenshot**)
   - Custom `Formatter` - Parameter display formatting
   - 🚧 Linear smoothing - Implemented in compressor (attack/release parameters), needs DAW testing
   - ✅ ~~`bypass` attribute~~ - Tested in midi-transform; also in compressor (needs DAW testing)

6. **MIDI - Message Types**
   - ✅ ~~`PolyPressure`~~ - Tested in midi-transform (event transform) and synth (per-note vibrato)
   - ✅ ~~`ChannelPressure`~~ - Tested in synth (global vibrato control)
   - `ProgramChange` - Patch selection
   - `SysEx` - System exclusive messages

7. **MIDI - Utilities**
   - `RpnTracker` - RPN/NRPN message assembly
   - 14-bit CC utilities
   - `ChordInfo`, `ScaleInfo` - VST3 chord/scale events

### Low Priority (Specialized Features)

8. **MPE & Note Expression**
   - Note Expression Controller (per-note volume, pan, tuning)
   - Physical UI Mapping (MPE controller mappings)
   - MPE Support methods
   - Multi-dimensional per-note control

9. **MIDI - Learning & Mapping**
   - MIDI Learn (`on_midi_learn`, `on_midi1_learn`, `on_midi2_learn`)
   - MIDI Mapping (`midi_cc_to_parameter`, assignments)

10. **Orchestral Features**
    - Keyswitch Controller - Articulation switching

11. **MIDI 2.0**
    - `Midi2Controller` support
    - `Midi2Assignment`
    - MIDI 2.0 event handling

12. **Editor/GUI**
    - `EditorDelegate` - WebView-based GUI
    - `EditorConstraints` - GUI sizing

---

## Planned Examples

### Priority 1: Core Feature Coverage

#### 1. **Compressor** (High Priority)
**Goal:** Test bypass, LogMapper, and advanced bus features

**Features to test:**
- ✅ `IntParameter` - Ratio (2:1, 4:1, 8:1, 10:1, 20:1) - *Also tested in midi-transform*
- ✅ `BoolParameter` - Auto-makeup gain on/off, Knee type (hard/soft) - *Also tested in midi-transform*
- ✅ `BypassHandler` - Smooth bypass with equal-power crossfade - **NEW**
- ✅ `bypass_ramp_samples()` - Proper bypass reporting - **NEW**
- ✅ Sidechain input - External sidechain
- ✅ `set_active()` - Reset envelope followers on activation - **NEW**
- ✅ `LogMapper` - Threshold and ratio with logarithmic scaling - **NEW**
- ✅ Linear smoothing - Attack/release time smoothing - **NEW**

**Implementation notes:**
- Classic compressor with threshold, ratio, attack, release, knee, makeup gain
- External sidechain input for ducking/pumping
- Gain reduction metering (could be exposed as output parameter)
- RMS envelope follower with smoothing
- Soft/hard knee using BoolParameter or EnumParameter

**Files to create:**
- `examples/compressor/src/lib.rs`
- `examples/compressor/Cargo.toml`

---

#### 2. **EQ** (High Priority)
**Goal:** Test Units system, custom formatting, mono buses

**Features to test:**
- ✅ Units system - Group parameters by band (Low/Mid/High units)
- ✅ Custom `Formatter` - Frequency display (e.g., "1.2k", "440 Hz")
- ✅ Mono bus option - Mono in/out for certain use cases
- ✅ `LogMapper` - Frequency parameters with logarithmic mapping
- ✅ `IntParameter` - Filter type selection (bell, shelf, notch)

**Implementation notes:**
- 3-band parameteretric EQ (low shelf, mid bell, high shelf)
- Each band: frequency, gain, Q factor
- Organize parameters into units: Low Band, Mid Band, High Band
- Custom frequency formatter: "20 Hz", "1.5k", "10k"
- Biquad filters (or simple one-pole for demo)

**Files to create:**
- `examples/eq/src/lib.rs`
- `examples/eq/Cargo.toml`

---

#### 3. **Lookahead Limiter** (Medium Priority)
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

### Priority 2: MIDI Advanced Features

#### 4. **MPE Synth** (Medium Priority)
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
- `examples/mpe-synth/src/lib.rs`
- `examples/mpe-synth/Cargo.toml`

---

#### 5. **Orchestral Sampler** (Low Priority)
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

#### 6. **MIDI Processor** (Medium Priority)
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

### Priority 3: GUI & Advanced

#### 7. **WebView Plugin** (Low Priority - Phase 2)
**Goal:** Test EditorDelegate, WebView GUI

**Features to test:**
- ✅ `EditorDelegate` - WebView integration
- ✅ `EditorConstraints` - GUI sizing
- ✅ Parameter communication - GUI ↔ DSP
- ✅ Custom UI rendering

**Implementation notes:**
- Simple plugin with WebView-based GUI
- Real-time parameter updates from GUI
- Visual waveform display or spectrum analyzer
- Demonstrates bidirectional communication

**Files to create:**
- `examples/webview-demo/src/lib.rs`
- `examples/webview-demo/Cargo.toml`
- `examples/webview-demo/gui/` - HTML/CSS/JS

**Note:** Requires Phase 2 WebView implementation to be complete.

---

#### 8. **Multi-Bus Router** (Low Priority)
**Goal:** Test multiple aux buses, AuxOutput

**Features to test:**
- ✅ `AuxOutput` - Multiple output buses
- ✅ Multiple aux input/output buses
- ✅ Complex bus routing
- ✅ `output_bus_info()` - Custom output configuration

**Implementation notes:**
- Audio router with multiple inputs and outputs
- Route/mix any input to any output
- Demonstrates complex bus configurations
- Gain control per route

**Files to create:**
- `examples/router/src/lib.rs`
- `examples/router/Cargo.toml`

---

## Example Enhancement Opportunities

### Existing Examples - Potential Improvements

#### **gain** (Current)
**Could add:**
- ✅ `BypassHandler` - Add smooth bypass instead of just gain control
- ✅ `BoolParameter` - Add "Invert Phase" toggle
- ✅ `IntParameter` - Add "Oversampling" (1x, 2x, 4x, 8x) selector
- ✅ Units - Group "Input" and "Output" parameters

#### **delay** (Current)
**Could add:**
- ✅ `BoolParameter` - Add "Freeze" mode (infinite feedback)
- ✅ `IntParameter` - Add "Tap Count" for multi-tap delay
- ✅ `BypassHandler` - Add smooth bypass
- ✅ `latency_samples()` - Report minimum delay time as latency

#### **synth** (Current)
**Recently added:**
- ✅ `IntParameter` - Transpose parameter (±2 octaves, -24 to +24 semitones)
- ✅ Flat parameter groups - "Oscillator", "Envelope", "Filter", "Global" groups (works in Cubase)
- ✅ `PolyPressure` - Per-note aftertouch → vibrato depth (polyphonic expression)
- ✅ `ChannelPressure` - Channel aftertouch → vibrato depth (global expression)
- ✅ Mod wheel - Controls both vibrato depth AND filter cutoff modulation

**Could still add:**
- ❌ `BoolParameter` - Add "Legato Mode" toggle
- ❌ "Voice Count" parameter (1-16 voices) using IntParameter

#### **midi-transform** (Current)
**Could add:**
- ✅ `RpnTracker` - Track and display RPN/NRPN messages
- ✅ 14-bit CC - Demonstrate 14-bit CC MSB/LSB handling
- ✅ `ProgramChange` - Add program change filtering/remapping

#### **compressor** (Current)
**Could add:**
- ❌ Look-ahead option - Professional limiters use look-ahead to catch transients before they happen. Requires delay buffer and `latency_samples()` for delay compensation reporting to host.
- ❌ RMS detection mode - Add toggle for RMS averaging instead of peak detection. RMS provides smoother, more musical compression that's less sensitive to individual transients.
- ❌ Gain reduction metering - Expose GR as an output parameter for DAW metering display.

---

## Testing Strategy

### Integration Testing via Examples

**Philosophy:** Examples serve dual purpose:
1. **Documentation** - Show developers how to use features
2. **Integration Tests** - Validate features work in real-world scenarios

**Benefits of example-driven testing:**
- ✅ Bugs discovered during implementation
- ✅ Real-world usage patterns validated
- ✅ Documentation stays in sync with code
- ✅ Examples can be bundled and tested by users

### Development Workflow

1. **Identify Untested Feature** - Review coverage matrix
2. **Design Example** - Choose plugin that naturally uses feature
3. **Implement Example** - Build plugin using feature
4. **Discover Bugs** - Find and fix framework issues
5. **Document** - Update this file and REFERENCE.md
6. **Update Matrix** - Mark feature as tested

### Coverage Goals

- **Phase 1 (Current):** Core parameter types, basic MIDI, audio processing
  - Target: 60% feature coverage
  - Focus: FloatParameter, EnumParameter, basic MIDI, f32/f64

- **Phase 2 (Next):** Advanced parameters, bypass, buses
  - Target: 80% feature coverage
  - Focus: IntParameter, BoolParameter, BypassHandler, Units, multi-bus

- **Phase 3 (Future):** MPE, advanced MIDI, GUI
  - Target: 95% feature coverage
  - Focus: Note Expression, MPE, Keyswitch, WebView

- **Phase 4 (Complete):** Edge cases, MIDI 2.0
  - Target: 100% feature coverage
  - Focus: MIDI 2.0, RPN/NRPN, SysEx, advanced mapping

---

## Implementation Checklist

### Phase 1: Core Coverage (Current)
- [x] gain - FloatParameter, f32/f64, multi-bus, transport
- [x] delay - EnumParameter, smoothing, tempo sync, tail_samples
- [x] synth - MIDI basics, MidiCcConfig, polyphony
- [x] midi-transform - MIDI pass-through, CC transformation

### Phase 2: Advanced Parameters & Processing
- [x] compressor - BoolParameter, EnumParameter, BypassHandler, PowerMapper, linear smoothing, set_active (implemented, needs DAW testing)
- [ ] eq - Units system, custom Formatter, mono buses
- [ ] limiter - latency_samples, lookahead processing

### Phase 3: Advanced MIDI
- [ ] mpe-synth - Note Expression, Physical UI, MPE Support
- [ ] orchestral-sampler - Keyswitch Controller, ProgramChange
- [ ] midi-processor - RpnTracker, 14-bit CC, MIDI Learn, Poly/Channel Pressure

### Phase 4: GUI & Advanced Routing
- [ ] webview-demo - EditorDelegate, WebView GUI
- [ ] router - AuxOutput, multiple aux buses

### Documentation Updates
- [ ] Update REFERENCE.md with tested features
- [ ] Add "Used By" column showing which examples use each feature
- [ ] Create example comparison table
- [ ] Document common patterns discovered

---

## Notes

- **Bug Discovery:** As of 2026-01-05, implementing examples has already helped find bugs in MidiCcConfig and smoothing
- **Real-World Testing:** Examples should reflect actual use cases, not contrived scenarios
- **Keep Simple:** Examples should be minimal while demonstrating features effectively
- **Cross-Reference:** Link examples in REFERENCE.md feature documentation

---

## Appendix A: midi-transform Example - Feature Analysis

### Should We Remove midi-transform?

The midi-transform example may seem "odd" as it's a somewhat contrived MIDI processor, but it currently provides **critical test coverage** for features not used anywhere else.

### Unique Features Only in midi-transform

#### 1. **IntParameter** ⚠️ CRITICAL
```rust
#[parameter(id = "note_transpose", name = "Transpose", default = 0, range = -24..=24, kind = "semitones")]
pub transpose: IntParameter,
```
**Used for:** Transpose amount, note numbers (0-127), CC numbers (0-127)
- ❌ Not used in: gain, delay, synth

#### 2. **BoolParameter** ⚠️ CRITICAL
```rust
#[parameter(id = "note_enabled", name = "Enabled", default = true)]
pub enabled: BoolParameter,

#[parameter(id = "bypass", bypass)]
pub bypass: BoolParameter,
```
**Used for:** Enable/disable toggles, bypass parameter
- ❌ Not used in: gain, delay, synth

#### 3. **Nested Parameter Groups** ⚠️ QUESTIONABLE VALUE
```rust
#[nested(group = "Note Transform")]
pub note: NoteTransformParameters,

#[nested(group = "CC Transform")]
pub cc: CcTransformParameters,
```
**Demonstrates:** Hierarchical parameter organization (`#[nested]` attribute)
- ❌ Not used in: gain, delay, synth

**⚠️ Reality Check:** While the framework implements VST3 `IUnitInfo` for parameter grouping, it's unclear if DAWs actually display these groups. The practical value may be limited to:
- Rust code organization (`parameters.filter.cutoff` vs `parameters.cutoff`)
- State serialization path-based IDs (`"filter/cutoff"`)
- Reusable parameter structs (same struct in multiple groups)

**Needs investigation:** Test in multiple DAWs (Reaper, Logic, Cubase, etc.) to verify if groups are actually visible to users.

#### 4. **PolyPressure (Polyphonic Aftertouch)** ✅ TESTED
```rust
MidiEventKind::PolyPressure(poly) => {
    if let Some(new_pitch) = self.transform_pitch(poly.pitch) {
        output.push(MidiEvent::poly_pressure(
            event.sample_offset,
            poly.channel,
            new_pitch,
            poly.pressure,
            poly.note_id,
        ));
    }
}
```
- ✅ **Also used in synth** - Per-note vibrato depth control via polyphonic aftertouch
- ❌ Not used in: gain, delay

#### 5. **Special `bypass` Attribute**
```rust
#[parameter(id = "bypass", bypass)]
pub bypass: BoolParameter,
```
**Marks parameter as the official bypass parameter**
- ❌ Not used in: gain, delay, synth

#### 6. **`buffer.copy_to_output()`**
```rust
fn process(&mut self, buffer: &mut Buffer, ...) {
    buffer.copy_to_output();
}
```
**Used for:** Pass-through audio processing (MIDI-only plugin)
- ✅ Also used in gain (in bypass handler context)

### Coverage Summary

**If midi-transform is removed, we lose test coverage for:**
- ✅ IntParameter - **Now also tested in synth** (transpose parameter)
- ✅ BoolParameter - **Still unique to midi-transform** (would lose coverage)
- ⚠️ Nested parameter groups (`#[nested]`) - **Still unique to midi-transform** (Rust-only organization)
- ✅ PolyPressure - **Now also tested in synth** (per-note vibrato control)
- ✅ `bypass` attribute - **Still unique to midi-transform** (would lose coverage)

### Recommendations

#### Option 1: Keep and Enhance
Make midi-transform more useful while preserving features:
- Rename to "midi-utility"
- Add MIDI channel filtering
- Add velocity curve remapping
- Add MIDI event logging/display
- Keep all IntParameter, BoolParameter, nested group usage

#### Option 2: Move Features to Compressor
Migrate unique features to the planned **compressor** example:
- Use `IntParameter` for ratio selection (2:1, 4:1, 8:1, 10:1, 20:1)
- Use `BoolParameter` for auto-makeup gain, hard/soft knee toggle
- Use nested groups: "Input", "Compression", "Output" sections
- Use `bypass` attribute for compressor bypass
- **Then** remove midi-transform
- **Note:** Would still lose PolyPressure test coverage

#### Option 3: Keep As-Is
Accept that it's a contrived example but serves an important testing purpose:
- Document clearly that it's a "parameter showcase" example
- Value test coverage over "real-world usefulness"
- Keep until features are tested elsewhere

### Migration Checklist

**Before removing midi-transform, ensure these features are tested elsewhere:**

- [x] IntParameter - ✅ **Added to synth** (transpose parameter)
- [ ] BoolParameter - Add to another example (compressor, eq)
- [ ] Nested parameter groups - Add to another example (eq with bands)
- [x] PolyPressure - ✅ **Added to synth** (per-note vibrato control)
- [ ] `bypass` attribute - Add to any effect example
- [ ] Update coverage matrix after migration
- [ ] Update ARCHITECTURE.md and examples README

**Current Status (Updated 2026-01-06):** midi-transform can now be removed with less impact. IntParameter and PolyPressure are now tested in synth. However, we would still lose BoolParameter, nested groups, and bypass attribute coverage.

---

## Appendix B: VST3 Units & Parameter Grouping - Investigation Needed

### The Claim

The framework implements VST3 parameter grouping via `IUnitInfo`:
- **Flat groups:** `group = "..."` attribute creates VST3 units for "DAW visual grouping"
- **Nested groups:** `#[nested(group = "...")]` creates VST3 units + Rust struct organization

Documentation claims: *"DAW shows collapsible 'Filter' and 'Output' groups"*

### The Reality Check

**Current observation:** Groups are **not visible** in tested DAWs (as of 2026-01-05).

This raises questions:
1. Do ANY DAWs actually display VST3 units as parameter groups?
2. Is the `IUnitInfo` implementation correct/complete?
3. Is this feature documented but unsupported by real-world DAWs?

### What's Actually Implemented

**In the framework:**
```rust
// beamer-vst3/src/processor.rs
impl<P: Plugin> IUnitInfoTrait for Vst3Processor<P> {
    unsafe fn getUnitCount(&self) -> i32 { ... }
    unsafe fn getUnitInfo(&self, unit_index: i32, info: *mut UnitInfo) -> tresult { ... }
}
```

**The VST3 spec supports:**
- Hierarchical parameter organization via `IUnitInfo`
- Parent/child unit relationships
- Unit names and IDs

**But does anyone use it?**

### Investigation Findings (2026-01-05)

**Research into JUCE and VST3 official docs reveals:**

#### Confirmed Working (from Steinberg VST3 docs):
- ✅ **Cubase** - Steinberg's DAW, full VST3 units support (MultibandCompressor example)
- ✅ **Cakewalk** - Shows HALion Sonic SE unit structure in automation lists
- ✅ **PluginTestHost** - Steinberg's test host displays units correctly

#### Confirmed NOT Working:
- ❌ **Logic Pro (AU format)** - JUCE forum: "AUs and AUv3s in Logic are problematic" - parameters sort alphabetically, ignoring group structure

#### Unknown/Untested:
- ❓ **Reaper** - No documentation found
- ❓ **Ableton Live** - No documentation found
- ❓ **Bitwig** - No documentation found
- ❓ **FL Studio** - No documentation found
- ❓ **Logic Pro (VST3)** - May differ from AU behavior, untested

#### Industry Consensus (from JUCE):
- Parameter groups work in **some** VST3 hosts (notably Cubase)
- Support is **inconsistent** across DAWs
- VST3 spec says hosts **"can"** implement units, not **"must"**
- Even major frameworks like JUCE have the same issues
- Developers use workarounds (separator strings like " | ") for unsupported hosts

**Sources:**
- [JUCE Forum: Plug-in parameter groups](https://forum.juce.com/t/plug-in-parameter-groups/29409)
- [VST3 Developer Portal: Units](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+3+Units/Index.html)

### Actual Outcome: Scenario 3 (Partial Support) ✅

**Reality:** VST3 units work in **some** DAWs (Cubase, Cakewalk), but not universally.

This matches the industry standard - even JUCE has the same limitations. The VST3 spec makes units optional for hosts.

**What This Means:**
- Beamer's `IUnitInfo` implementation is **correct**
- Lack of universal DAW support is **not a Beamer bug**
- This is a **VST3 ecosystem limitation**

**Action Items:**
- ✅ Keep the feature (it works in major DAWs like Cubase)
- ✅ Update documentation to set realistic expectations
- ✅ Clearly state which DAWs support it
- ✅ Emphasize code organization benefits of `#[nested]`

### Current Practical Value (Assuming Groups Don't Show)

**Flat groups (`group = "..."`):**
- ❌ No visual grouping in DAW
- ❌ No code organization benefit
- ❌ No practical value
- **Verdict:** Consider deprecating/removing

**Nested groups (`#[nested]`):**
- ✅ Rust code organization (`parameters.filter.cutoff`)
- ✅ Reusable parameter structs
- ✅ Path-based state serialization prevents ID collisions
- ❌ No visual grouping in DAW
- **Verdict:** Useful for large plugins, but not for DAW grouping reasons

### Recommendation

**Immediate action:**
1. Test `#[nested]` in at least 2-3 major DAWs
2. Document findings in this file
3. Update REFERENCE.md to reflect reality
4. Adjust claims about "DAW visual grouping"

**Updated Recommendations (based on research):**

1. **Keep both features** - They work in Cubase/Cakewalk, which is industry-standard support level
2. **Update documentation** to honestly reflect DAW support:
   ```markdown
   ## Parameter Groups (VST3 Units)

   Beamer supports VST3 units for parameter organization:

   **Code Organization (Always Works):**
   - `#[nested]` creates separate structs (parameters.filter.cutoff)
   - Reusable parameter groups
   - Path-based state serialization

   **DAW Visual Grouping (Partial Support):**
   - ✅ Cubase: Full support
   - ✅ Cakewalk: Full support
   - ❌ Logic (AU): Does not work
   - ❓ Other DAWs: May or may not display groups

   If DAW grouping is critical, test in your target DAW.
   ```
3. **Set expectations** - This is a "nice to have" feature, not guaranteed across all DAWs
4. **Document benefits** - Emphasize code organization even if DAW doesn't show groups

**Related files to update:**
- [docs/REFERENCE.md](REFERENCE.md) - Claims "DAW shows collapsible groups"
- [README.md](../README.md) - Visual grouping claims
- [examples/README.md](../examples/README.md) - midi-transform description

---

**Document Maintenance:**
- Update coverage matrix after each new example
- Review and prioritize untested features quarterly
- Add new features to matrix as framework expands
- Track bugs discovered through example development
- **NEW:** Track VST3 unit grouping investigation results
