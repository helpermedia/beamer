# TODO: Audio Unit (AU) Support

> **Goal:** Plugins load and run correctly in Logic Pro and other AU hosts on macOS.
>
> **Approach:** Native Rust implementation using `objc2` bindings — not Steinberg's C++ wrapper.
>
> **Prerequisite:** Phase 1 (Format-Agnostic Core) should be complete first.

---

## Overview

Create a `beamer-au` crate that wraps Beamer's `Plugin` and `AudioProcessor` traits for the Audio Unit v3 (AUv3) format.

**Key insight:** AUv3 plugins are App Extensions that subclass `AUAudioUnit`. We use `objc2`'s `declare_class!` macro to create this subclass in Rust.

---

## Reference Material

- [AU_SUPPORT_ANALYSIS.md](AU_SUPPORT_ANALYSIS.md) — Architecture decisions and rationale
- [Apple AUv3 Documentation](https://developer.apple.com/documentation/audiotoolbox/audio_unit_v3_plug-ins)
- [objc2 GitHub](https://github.com/madsmtm/objc2)
- [Steinberg AUv3 Wrapper](https://github.com/steinbergmedia/vst3sdk/tree/master/public.sdk/source/vst/auv3wrapper) — MIT licensed reference

---

## Tasks

### 1. Create beamer-au crate structure

```bash
mkdir -p crates/beamer-au/src/platform
```

**Create files:**

- [ ] `crates/beamer-au/Cargo.toml`
- [ ] `crates/beamer-au/src/lib.rs`
- [ ] `crates/beamer-au/src/audio_unit.rs` — AUAudioUnit subclass
- [ ] `crates/beamer-au/src/parameters.rs` — AUParameterTree mapping
- [ ] `crates/beamer-au/src/buffers.rs` — AU buffer translation
- [ ] `crates/beamer-au/src/midi.rs` — AU MIDI event handling
- [ ] `crates/beamer-au/src/factory.rs` — Component registration
- [ ] `crates/beamer-au/src/error.rs` — Error types

**Cargo.toml:**
```toml
[package]
name = "beamer-au"
version = "0.1.0"
edition = "2021"

[dependencies]
beamer-core = { path = "../beamer-core" }
log = { workspace = true }

[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.5"
objc2-foundation = { version = "0.2", features = ["NSString", "NSError", "NSArray"] }
objc2-audio-toolbox = { version = "0.2", features = ["AUAudioUnit", "AUParameterTree", "AUParameter"] }
block2 = "0.5"
```

---

### 2. Research objc2-audio-toolbox bindings

Before implementing, verify what's available in the crate.

```bash
# Check available types
cargo doc -p objc2-audio-toolbox --open
```

**Key types needed:**
- [ ] `AUAudioUnit` — Base class to subclass
- [ ] `AUParameterTree` — Parameter hierarchy
- [ ] `AUParameter` — Individual parameters
- [ ] `AUAudioUnitBus` — Audio bus representation
- [ ] `AUAudioUnitBusArray` — Bus collections
- [ ] `AVAudioFormat` — Audio format description
- [ ] `AURenderBlock` — The render callback type

**If bindings are missing:** May need to add feature flags or contribute to objc2-audio-toolbox.

---

### 3. Implement AUAudioUnit subclass

**File:** `crates/beamer-au/src/audio_unit.rs`

Use `declare_class!` to subclass `AUAudioUnit`:

```rust
use objc2::declare_class;
use objc2::runtime::AnyClass;
use objc2_audio_toolbox::AUAudioUnit;
use beamer_core::{Plugin, AudioProcessor, HasParameters};

declare_class!(
    pub struct BeamerAudioUnit<P: Plugin> {
        // Ivars
        plugin: Option<P>,
        processor: Option<P::Processor>,
        // ... other state
    }

    unsafe impl<P: Plugin> ClassType for BeamerAudioUnit<P> {
        type Super = AUAudioUnit;
        const NAME: &'static str = "BeamerAudioUnit";
    }

    // Required overrides
    unsafe impl<P: Plugin> BeamerAudioUnit<P> {
        #[method(initWithComponentDescription:options:error:)]
        fn init_with_component(
            &self,
            desc: AudioComponentDescription,
            options: AudioComponentInstantiationOptions,
            error: *mut *mut NSError,
        ) -> Option<&Self>;

        #[method(allocateRenderResourcesAndReturnError:)]
        fn allocate_render_resources(&self, error: *mut *mut NSError) -> bool;

        #[method(deallocateRenderResources)]
        fn deallocate_render_resources(&self);

        #[method(internalRenderBlock)]
        fn internal_render_block(&self) -> AURenderBlock;

        #[method(parameterTree)]
        fn parameter_tree(&self) -> *mut AUParameterTree;
    }
);
```

**Key lifecycle mapping:**

| AU Method | Beamer Equivalent |
|-----------|-------------------|
| `init` | `Plugin::default()` |
| `allocateRenderResources` | `Plugin::prepare()` → `AudioProcessor` |
| `deallocateRenderResources` | `AudioProcessor::unprepare()` → `Plugin` |
| `internalRenderBlock` | Returns block that calls `AudioProcessor::process()` |

---

### 4. Implement parameter tree mapping

**File:** `crates/beamer-au/src/parameters.rs`

Map Beamer's `Parameters` trait to `AUParameterTree`:

```rust
pub fn build_parameter_tree<P: Parameters>(params: &P) -> Id<AUParameterTree> {
    let mut au_params = Vec::new();

    for i in 0..params.count() {
        if let Some(info) = params.info(i) {
            let au_param = create_au_parameter(info);
            au_params.push(au_param);
        }
    }

    AUParameterTree::createTreeWithChildren(&au_params)
}

fn create_au_parameter(info: &ParameterInfo) -> Id<AUParameter> {
    // Map ParameterInfo to AUParameter
    // - info.id → address
    // - info.name → identifier/displayName
    // - info.default_normalized → default value
    // - info.flags → AU flags
}
```

**Parameter change handling:**
- AU uses KVO (Key-Value Observing) for parameter changes
- Set up observers to call `Parameters::set_normalized()`

---

### 5. Implement audio buffer translation

**File:** `crates/beamer-au/src/buffers.rs`

Translate AU's `AudioBufferList` to Beamer's `Buffer`:

```rust
pub unsafe fn au_buffers_to_beamer<'a>(
    input_data: *const AudioBufferList,
    output_data: *mut AudioBufferList,
    frame_count: u32,
) -> (Buffer<'a, f32>, AuxiliaryBuffers<'a, f32>) {
    // Extract channel pointers from AudioBufferList
    // Build Buffer and AuxiliaryBuffers
}
```

**Considerations:**
- AU uses `AudioBufferList` with interleaved or non-interleaved options
- Check `mNumberBuffers` and `mNumberChannels` for layout
- Handle both float32 and float64 formats

---

### 6. Implement MIDI event translation

**File:** `crates/beamer-au/src/midi.rs`

AU MIDI comes through `AUMIDIEvent` or `MIDIPacketList`:

```rust
pub fn translate_midi_events(
    events: &AURenderEvent,  // Linked list of events
    output: &mut MidiBuffer,
) {
    // Walk the event list
    // Convert each AU MIDI event to MidiEvent
    // Push to MidiBuffer
}
```

**Event types to handle:**
- Note On/Off
- Control Change
- Pitch Bend
- Aftertouch (Poly and Channel)
- Program Change
- SysEx

---

### 7. Implement the render block

The render block is the audio callback. It must be real-time safe.

```rust
fn create_render_block<P: Plugin>(
    processor: Arc<Mutex<Option<P::Processor>>>,
) -> AURenderBlock {
    block2::RcBlock::new(move |
        action_flags: *mut AudioUnitRenderActionFlags,
        timestamp: *const AudioTimeStamp,
        frame_count: AUAudioFrameCount,
        output_bus: NSInteger,
        output_data: *mut AudioBufferList,
        pull_input: AURenderPullInputBlock,
    | -> AUAudioUnitStatus {
        // 1. Pull input if needed
        // 2. Translate buffers
        // 3. Call processor.process()
        // 4. Return noErr or error
    })
}
```

**Critical:** The render block captures state. Use appropriate synchronization (or lock-free patterns) for the processor reference.

---

### 8. Implement component registration

**File:** `crates/beamer-au/src/factory.rs`

AUv3 plugins need an `AUAudioUnitFactory`:

```rust
pub fn register_audio_unit<P: Plugin>(
    component_description: AudioComponentDescription,
) {
    // Register the BeamerAudioUnit class with the AU system
}
```

**The factory creates instances when the host loads the plugin.**

---

### 9. Create export macro

**File:** `crates/beamer-au/src/lib.rs`

```rust
#[macro_export]
macro_rules! export_au {
    ($config:expr, $plugin:ty) => {
        // Generate the entry point and registration code
        // Similar pattern to export_vst3!
    };
}
```

**Usage:**
```rust
beamer_au::export_au!(CONFIG, MyPlugin);
```

---

### 10. Update xtask for AU bundling

**File:** `xtask/src/main.rs` (or equivalent)

AU plugins are App Extensions with specific bundle structure:

```
MyPlugin.appex/
├── Contents/
│   ├── Info.plist
│   ├── MacOS/
│   │   └── MyPlugin          # Rust binary
│   └── Resources/
```

**Info.plist requirements:**
```xml
<key>NSExtension</key>
<dict>
    <key>NSExtensionPointIdentifier</key>
    <string>com.apple.AudioUnit-UI</string>
    <key>NSExtensionPrincipalClass</key>
    <string>BeamerAudioUnit</string>
</dict>
<key>AudioComponents</key>
<array>
    <dict>
        <key>type</key>
        <string>aufx</string>  <!-- or aumu for instruments -->
        <key>subtype</key>
        <string>test</string>  <!-- 4-char code -->
        <key>manufacturer</key>
        <string>Demo</string>  <!-- 4-char code -->
        <key>name</key>
        <string>Beamer: My Plugin</string>
    </dict>
</array>
```

**Add to xtask:**
```bash
cargo xtask bundle gain --au
# Produces: target/bundled/BeamerGain.appex
```

---

### 11. Create minimal test plugin

Before testing all examples, create a minimal AU-only test:

**File:** `examples/au-test/src/lib.rs`

```rust
use beamer::prelude::*;
use beamer_au::export_au;

#[derive(Parameters)]
pub struct TestParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,
}

#[derive(Default, HasParameters)]
pub struct TestPlugin {
    #[parameters]
    parameters: TestParameters,
}

impl Plugin for TestPlugin {
    type Config = NoConfig;
    type Processor = TestProcessor;

    fn prepare(self, _config: NoConfig) -> TestProcessor {
        TestProcessor { parameters: self.parameters }
    }
}

#[derive(HasParameters)]
pub struct TestProcessor {
    #[parameters]
    parameters: TestParameters,
}

impl AudioProcessor for TestProcessor {
    type Plugin = TestPlugin;

    fn unprepare(self) -> TestPlugin {
        TestPlugin { parameters: self.parameters }
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &ProcessContext) {
        let gain = self.parameters.gain.as_linear() as f32;
        for (input, output) in buffer.zip_channels() {
            for (i, o) in input.iter().zip(output.iter_mut()) {
                *o = *i * gain;
            }
        }
    }
}

export_au!(CONFIG, TestPlugin);

static CONFIG: AuConfig = AuConfig::new("AU Test", "aufx", "test", "Demo");
```

---

### 12. Run auval validation

Apple's `auval` tool validates AU plugins:

```bash
# List registered AUs
auval -a

# Validate a specific AU
auval -v aufx test Demo

# Verbose output
auval -v aufx test Demo -de
```

**Common validation failures:**
- Incorrect bundle structure
- Missing Info.plist keys
- Parameter tree issues
- Render block crashes
- Threading violations

**Fix issues until validation passes.**

---

### 13. Test in Logic Pro

Once `auval` passes:

1. Copy `.appex` to `~/Library/Audio/Plug-Ins/Components/`
2. Open Logic Pro
3. Create new project
4. Add AU instrument or effect
5. Find plugin in list
6. Verify:
   - [ ] Plugin loads without crash
   - [ ] Parameters appear in UI
   - [ ] Parameter automation works
   - [ ] Audio passes through correctly
   - [ ] Save/load project preserves state

---

### 14. Port existing examples

Once the minimal test works:

- [ ] `examples/gain` — Simple effect
- [ ] `examples/delay` — Effect with tempo sync
- [ ] `examples/synth` — Instrument with MIDI
- [ ] `examples/compressor` — Effect with sidechain

**For each example:**
1. Add AU export alongside VST3 export
2. Build with `cargo xtask bundle <name> --au`
3. Test in Logic Pro

---

## Verification Checklist

- [ ] `beamer-au` crate compiles on macOS
- [ ] `auval -v aufx test Demo` passes
- [ ] Plugin loads in Logic Pro
- [ ] Parameters visible and automatable
- [ ] Audio processes correctly
- [ ] MIDI events received (for instruments)
- [ ] State saves and restores
- [ ] No crashes on repeated open/close
- [ ] gain, delay, synth, compressor examples work

---

## Known Challenges

| Challenge | Mitigation |
|-----------|------------|
| objc2-audio-toolbox may have gaps | Check docs first; contribute bindings if needed |
| AUv3 is App Extension architecture | Follow Apple's extension guidelines |
| Render block threading | Use lock-free patterns; test with Thread Sanitizer |
| Parameter KVO complexity | Study Apple's sample code |
| auval is strict | Use Steinberg wrapper as reference for correct behavior |

---

## Dependencies on Other Work

- **Phase 1 (Format-Agnostic):** Must be complete — AU needs `Parameters` trait, not `Vst3Parameters`
- **objc2 ecosystem:** Verify bindings exist for all needed types

---

## References

- [AU_SUPPORT_ANALYSIS.md](AU_SUPPORT_ANALYSIS.md) — Design decisions
- [Apple AUv3 Sample Code](https://developer.apple.com/documentation/audiotoolbox/audio_unit_v3_plug-ins)
- [objc2-audio-toolbox](https://docs.rs/objc2-audio-toolbox)
- [Steinberg AUv3 Wrapper](https://github.com/steinbergmedia/vst3sdk) — MIT reference
