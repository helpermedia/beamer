# Examples

Working Beamer plugins demonstrating framework features.

**For a comprehensive feature coverage matrix and roadmap for future examples, see [docs/EXAMPLE_COVERAGE.md](../docs/EXAMPLE_COVERAGE.md).**

## Running Examples

A format flag is required: `--auv2`, `--auv3` or `--vst3`.

```bash
cargo xtask bundle <example> --auv2 --release --install
cargo xtask bundle <example> --vst3 --release --install
cargo xtask bundle <example> --auv2 --vst3 --release    # Multiple formats at once
```

See [docs/BUILDING.md](../docs/BUILDING.md) for prerequisites, all options and testing.

The examples turn on the `allocation-check` feature: a debug build logs any memory that `process()` allocates or frees (see [Allocation Check](../docs/REFERENCE.md#allocation-check)). The gain example has a test of it.

## Available Examples

### [Gain](gain/)

Simple gain effect for stereo and mono tracks with minimal complexity.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Gain** | Volume adjustment from -60 dB to +12 dB (boost). 0 dB = no change. |

**Demonstrates:**
- `Model` and `Processor` lifecycle via `prepare()`
- `#[beamer::model]` with declarative attributes
- `()` setup for plugins without sample-rate-dependent state
- `Model::LAYOUTS` with a stereo and a mono layout
- `FloatParameter` with dB scaling
- Generic f32/f64 processing via `Sample` trait

---

### [Compressor](compressor/)

Feed-forward compressor with soft/hard knee, lookahead, sidechain input and an editor in React.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Threshold** | Level at which compression begins (-60 to 0 dB). Uses logarithmic mapping for finer control near 0 dB. |
| **Ratio** | Compression intensity. 2:1 (gentle) through 20:1 (near-limiting). |
| **Attack** | How fast compression engages (0.1-200 ms). Slower preserves transients. |
| **Release** | How fast compression releases (10-2000 ms). Faster = punchier, slower = smoother. |
| **Soft Knee** | Toggle between a soft knee (a gradual onset, 6 dB wide and centered on the threshold) and a hard knee (abrupt). |
| **Lookahead** | Delays the audio (0-10 ms) so that compression starts before a peak arrives. It is the plugin's latency, which the host compensates. |
| **Auto Makeup** | Automatically compensate for volume reduction from compression. |
| **Makeup Gain** | Manual output boost (0-24 dB), added to the auto makeup when that is on. |
| **Sidechain** | Use external sidechain input for detection instead of main input, once the host has switched the sidechain bus on. |

**Typical Settings:**
- **Transparent leveling**: Low ratio (2:1), slow attack (50ms), medium release (200ms), soft knee
- **Punchy drums**: 4:1 ratio, fast attack (5ms), fast release (50ms), hard knee
- **Vocal control**: 4:1-8:1 ratio, medium attack (20ms), medium release (150ms), soft knee
- **Sidechain ducking**: Enable sidechain, route kick to sidechain input, high ratio, fast attack/release

**Editor:**

A bar with the plugin's name and the Beamer logo, then the transfer curve: threshold, knee, ratio and makeup from the parameters, the area the compressor turns down in blue, the level the detector keys on as a bar along the input axis and as a dot on the curve. Beside it a gain reduction meter, then knobs, buttons and switches for every parameter but the bypass, which the host's own bypass sets. Below them the choice of an internal or external sidechain and the latency of the lookahead in samples. The curve's input axis says which of the two the compressor hears. The displays dim while the plugin is bypassed. The editor scales with its window, side by side in a wide one and the displays above the controls in a tall one. It has the look of the Beamer brand (ink, paper and one blue, the font Inter, embedded from fontsource with no request to the internet). Built with React, Vite and Tailwind.

**Demonstrates:**
- A tuple setup `(SampleRate, MaxBufferSize, MainInputChannels, ActiveBuses)`: the sample rate for the envelope coefficients, the channels and block size for the bypass handler and the active buses
- `BypassHandler` with `CrossfadeCurve::Linear` for click-free bypass, its dry signal delayed by the lookahead and a warm-up (lookahead plus three attack times) after a bypass
- `latency_samples()` that follows a parameter at runtime
- `set_active()` for resetting envelope and lookahead on activation
- `kind = "db_log"` for logarithmic-feel threshold control
- Linear parameter smoothing (`smoothing = "linear:50.0"`)
- `EnumParameter` for discrete ratio values
- A sidechain input (`BusInfo::aux()`) that stays off until the host switches it on, read from `ActiveBuses` in `prepare()`
- `Model::LAYOUTS` with mono and stereo main buses and sidechains
- dB-domain envelope processing
- Two value senders from the audio thread: the meters (held peaks that fall by 20 dB a second, sent with every block until they rest, so that the editor lets them fall on its own when the host stops calling the plugin) and the status (sample rate, whether the detector keys on the sidechain), whose first value `prepare()` sets
- An editor that works out from the parameters what it can (the transfer curve, the latency in samples), so it follows a knob at once, also while the host does not run the processor
- The packages for web UIs: `beamer-react` hooks for parameters and sender values, `beamer-gestures` for the knobs (vertical drag, wheel, keys, double click to reset), used from the bun workspace
- Enum choices from the parameter (`valueTexts`) as buttons for the ratio, from which the editor also reads the ratio for its curve

---

### [Equalizer](equalizer/)

3-band parametric EQ with peak (bell) filters.

**Parameters:**

| Group | Parameter | Description |
|-------|-----------|-------------|
| **Low** | Frequency | Center frequency (20-2000 Hz) |
| | Gain | Boost/cut (-12 to +12 dB) |
| | Width | Bandwidth (0.1-10.0, higher = wider = more audible) |
| **Mid** | Frequency | Center frequency (200-8000 Hz) |
| | Gain | Boost/cut (-12 to +12 dB) |
| | Width | Bandwidth (0.1-10.0, higher = wider = more audible) |
| **High** | Frequency | Center frequency (2000-20000 Hz) |
| | Gain | Boost/cut (-12 to +12 dB) |
| | Width | Bandwidth (0.1-10.0, higher = wider = more audible) |

**Typical Settings:**
- **Presence boost**: Mid frequency 3000 Hz, gain +3 dB, width 1.0
- **Warmth**: Low frequency 200 Hz, gain +2 dB, width 2.0
- **Air**: High frequency 12000 Hz, gain +4 dB, width 3.0
- **Mud cut**: Mid frequency 300 Hz, gain -4 dB, width 0.5

**Demonstrates:**
- `FloatParameter` with Hz scaling via `kind = "hz"` (LogMapper + frequency formatter)
- `FloatParameter` with dB scaling via `kind = "db"`
- Flat parameter groups via `group = "..."` attribute
- `Model::LAYOUTS` with a stereo and a mono layout, the filter state sized from `MainOutputChannels`
- Biquad peak filters using standard bilinear transform
- Width parameter (inverted Q: higher = wider = more audible)
- Generic f32/f64 processing via `Sample` trait

---

### [Delay](delay/)

Tempo-synced delay for stereo and mono tracks with ping-pong mode, factory presets and an editor in plain HTML and JavaScript.

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

**Editor:**

The delay time in effect and where it comes from (free or a note value at the host's tempo), the echoes of a short sound over time (fading by the feedback, alternating sides in ping-pong mode), buttons for the sync and stereo modes and knobs for time, feedback and mix. The files in `webview/` are embedded as they are: no package manager, no build step.

**Demonstrates:**
- A `(SampleRate, MainOutputChannels)` setup: the sample rate for the delay lines, the output channels for the editor
- `set_active()` for clearing delay buffers on reset
- `EnumParameter` for sync mode and stereo mode
- Tempo sync using `ProcessContext.samples_per_beat()`
- Declarative parameter smoothing with `smoothing = "exp:5.0"`
- Ring buffer delay line implementation
- Proper tail length via `tail_samples()`
- Factory presets in code that set only some parameters
- `Model::LAYOUTS` with stereo, mono and mono to stereo layouts (a mono input feeds both delay lines)
- A web UI in plain HTML, CSS and JavaScript modules, working on `__BEAMER__` directly
- A `WebViewValueSender` that sends the host's tempo from the audio thread only when it changes
- Knobs with drag, fine drag, wheel, arrow keys, reset on double click and typed values (`_beamer/paramTextToNormalized`)
- Buttons for the choices of an enum parameter, named by its `valueTexts`

---

### [Synthesizer](synthesizer/)

8-voice polyphonic synthesizer with expressive MIDI controls and parameter groups.

**Parameters** (organized in groups):

| Group | Parameter | Description |
|-------|-----------|-------------|
| **Oscillator** | Waveform | Oscillator shape: Sine, Saw, Square or Triangle |
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

**Factory Presets:**
- **Init**: The declared defaults
- **Soft Pad**: Triangle wave, slow attack (600ms), high sustain, long release (1500ms)
- **Pluck**: Saw wave, instant attack, short decay, no sustain, resonant filter
- **Sub Bass**: Sine wave, fast attack, low cutoff (400Hz), transpose -12
- **Square Lead**: Square wave, high cutoff, add expression via mod wheel + aftertouch

Each preset starts from the defaults with `reset_parameters()`, so it sounds the same whatever was loaded before.

**Why MidiCcConfig?** VST3 doesn't pass pitch bend and CC messages directly to plugins. Instead, DAWs use `IMidiMapping` to convert them to parameter changes. `MidiCcConfig` tells the framework which controllers to enable - it creates hidden parameters that receive these values and converts them back to MIDI events for your plugin. No manual state management needed!

**Demonstrates:**
- `SampleRate` setup for sample-rate-dependent filter calculations
- `IntParameter` for transpose (±2 octaves in semitones)
- Flat parameter groups (`group = "..."`) for organizing automation lanes
- `MidiCcConfig` for pitch bend/mod wheel via IMidiMapping (framework manages state)
- Polyphonic aftertouch (PolyPressure) for per-note vibrato
- Channel aftertouch (ChannelPressure) for global vibrato
- Mod wheel controlling multiple parameters (vibrato + filter)
- Sample-accurate MIDI event processing
- Voice allocation with oldest-note stealing
- ADSR envelope generator
- One-pole lowpass filter with resonance
- Parameter smoothing (exponential for filter cutoff/resonance)
- Factory presets that start from the defaults (`reset_parameters()`)
- Generic f32/f64 processing

---

### [Drums](drums/)

MIDI-triggered drum synthesizer with multi-output buses. Each drum type routes to its own mono output bus for independent mixing in the DAW.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Kick** | Kick output level (-60 to +6 dB) |
| **Snare** | Snare output level (-60 to +6 dB) |
| **HiHat** | Hi-hat output level (-60 to +6 dB) |
| **Crash** | Crash cymbal output level (-60 to +6 dB) |

**MIDI Note Mapping (GM Standard):**

| MIDI Note | Drum |
|-----------|------|
| 36 (C1) | Kick |
| 38 (D1) | Snare |
| 42 (F#1) | Hi-Hat |
| 49 (C#2) | Crash |

**Demonstrates:**
- Multi-output auxiliary buses (`LAYOUTS`, `output_bus_info()`, `AuxiliaryBuffers`)
- Mono buses via `BusLayout::MONO`, named with `BusInfo::main()` and `BusInfo::aux()`
- MIDI input from the `instrument` category, without any setting
- Sample-accurate automation (`sample_accurate_automation` in `Config.toml`), since the levels are read once per hit
- Sample-accurate note triggering from `context.midi_input()` in `process()`
- 16-voice polyphony (4 per drum type) with oldest-note stealing
- Per-drum synthesis algorithms (sine, triangle, noise, metallic oscillators)
- Velocity-sensitive response
- Generic f32/f64 processing

---

### [MIDI Transform](midi-transform/)

MIDI effect that transforms notes and CC messages.

**Note Transform Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Enabled** | Toggle note processing on/off |
| **Mode** | How notes are transformed (see below) |
| **Transpose** | Semitones to shift (-24 to +24), only for Transpose mode |
| **Input Note** | Source note for Remap mode (0-127) |
| **Output Note** | Target note for Remap mode (0-127) |
| **Velocity** | Velocity scaling (0.0-2.0). 1.0 = no change |

**Note Modes:**
- **Through** - Pass notes unchanged (with optional velocity scaling)
- **Transpose** - Shift all notes by semitones
- **Octave Up/Down** - Shift all notes by one octave
- **Remap Note** - Change one specific note to another (e.g., kick on C1 to D1)
- **Invert** - Mirror pitches around middle C (C4)

**CC Transform Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Enabled** | Toggle CC processing on/off |
| **Mode** | How CC messages are transformed (see below) |
| **Input CC** | Source CC number for Remap modes (0-127) |
| **Output CC** | Target CC number for Remap modes (0-127) |
| **Scale** | Value scaling (0.0-2.0). 1.0 = no change |

A **Bypass** parameter passes all MIDI through unchanged.

**CC Modes:**
- **Through** - Pass CC messages unchanged
- **Remap CC** - Change CC number (e.g., Mod Wheel to Expression)
- **Scale** - Multiply all CC values
- **Invert** - Flip CC values (0 becomes 127, 127 becomes 0)
- **Remap + Scale** - Remap CC number AND scale value

**Demonstrates:**
- `()` setup: a MIDI transformer needs nothing from the audio setup
- Nested groups with `#[beamer::group]` and `#[nested(group = "...")]`
- `EnumParameter` for discrete choices
- `IntParameter` for note/CC selection
- `BoolParameter` for enable toggles
- `FloatParameter` for velocity and value scaling
- `process_midi()` for MIDI processing
- MIDI event modification with the `with()` method
- Held-note tracking, so a note-off ends the note its note-on started when the settings change while the note is held
- Sample-accurate automation (`sample_accurate_automation` in `Config.toml`), since the settings apply to each note

---

### [WebView Demo](webview-demo/)

Gain and pan effect with a web-based editor built with React, Vite and Tailwind.

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| **Gain** | Volume adjustment from -60 dB to +12 dB |
| **Pan** | Stereo panning |

**State beyond parameters:**

| State | Description |
|-------|-------------|
| **Pan law** | Constant power or linear, set in the web UI and saved with the project |

**Demonstrates:**
- `WebViewHandler::on_invoke()` on the model for request/response calls from JavaScript
- `WebViewEventQueue` for events from Rust to JavaScript: the editor shows the host's sample rate and follows it
- `__BEAMER__.invoke()` for request/response calls from JS to Rust
- A `State` field: read and set from the web UI with `useStateField`, read on the audio thread with a `HandoverReader`, saved with the plugin state
- `#[beamer::export]` macro embedding `webview/dist/` assets at compile time
- The packages for web UIs: `beamer-react` hooks for parameters, the `State` field and the meter, `beamer-gestures` for the knobs (the handle follows the pointer around the arc, vertical drag elsewhere, wheel, keys, Cmd-click or Ctrl-click to reset), used from the bun workspace
- Circular knob drawn in SVG, its input from `useGestures`
- Bipolar arc rendering for pan knob
- DAW undo grouping: each drag, scroll, key, reset or typed value is one undo step
