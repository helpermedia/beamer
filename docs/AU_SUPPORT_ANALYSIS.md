# Audio Unit Support Analysis

This document analyzes options for adding Audio Unit (AU) support to Beamer.

## Executive Summary

**Recommended approach:** Native AU implementation in Rust using `objc2` bindings, with Steinberg's MIT-licensed wrapper code as reference material.

This gives us:
- Pure Rust codebase (no C++ dependency)
- Direct wrapping of Plugin/AudioProcessor traits (no VST3 intermediate)
- Consistent architecture with `beamer-vst3` and future `beamer-clap`
- Smaller bundles (single binary, not wrapper + embedded VST3)

**Estimated effort:** Medium

---

## Audio Unit Versions

| Version | Introduced | Platforms | Status |
|---------|------------|-----------|--------|
| **AUv2** | macOS 10.0+ | macOS only | Legacy but widely supported |
| **AUv3** | macOS 10.11+, iOS 9+ | macOS, iOS, iPadOS | Modern, App Extension based |

**Target:** AUv3 first (covers both desktop and mobile), AUv2 compatibility layer if needed.

---

## Approach: Native AU via Rust + objc2

### Why This Approach

The [objc2 project](https://github.com/madsmtm/objc2) provides Rust bindings to Apple frameworks:

| Crate | Purpose |
|-------|---------|
| [`objc2-audio-toolbox`](https://docs.rs/objc2-audio-toolbox/latest/objc2_audio_toolbox/) | AUAudioUnit, AUParameterTree, etc. |
| [`objc2-avf-audio`](https://docs.rs/objc2-avf-audio) | AVAudioUnit integration |
| [`objc2-core-audio`](https://docs.rs/objc2-core-audio) | Low-level CoreAudio types |

The `objc2` crate's `declare_class!` macro allows subclassing Objective-C classes from Rust - exactly what AUv3 requires.

### Steinberg Wrapper as Reference

The [VST3 SDK 3.8+](https://www.steinberg.net/press/2025/vst-3-8/) is MIT licensed. We can use Steinberg's AU wrapper code (`public.sdk/source/vst/auv3wrapper/`) as reference material for:

| Wrapper File | Reference For |
|--------------|---------------|
| `AUv3Wrapper.mm` | AUAudioUnit subclass structure |
| Parameter handling code | AUParameterTree mapping patterns |
| Buffer management | AU ↔ internal buffer translation |
| MIDI code | AU MIDI event handling quirks |

This is not a port - we're writing native Rust code that wraps Beamer's traits directly, using Steinberg's battle-tested code to understand AU requirements and edge cases.

---

## Architecture

### Crate Structure

```
beamer-au/
├── Cargo.toml
├── src/
│   ├── lib.rs              # Public API, export macro
│   ├── audio_unit.rs       # AUAudioUnit subclass via declare_class!
│   ├── parameters.rs       # Plugin parameters → AUParameterTree
│   ├── buffers.rs          # AU buffers → beamer Buffer
│   ├── midi.rs             # AU MIDI → beamer MidiEvent
│   ├── factory.rs          # AUAudioUnitFactory, component registration
│   └── view_controller.rs  # AUViewController for UI (Phase 2)
```

### How It Fits

```
┌─────────────────────────────────────────────────────────────┐
│                      AU Host (Logic, GarageBand)            │
├─────────────────────────────────────────────────────────────┤
│                         AUAudioUnit                         │
│                  (BeamerAudioUnit subclass)                 │
├─────────────────────────────────────────────────────────────┤
│                        beamer-au                            │
│         Translates AU calls → Plugin/AudioProcessor         │
├─────────────────────────────────────────────────────────────┤
│                       beamer-core                           │
│              Plugin, AudioProcessor, Parameters             │
└─────────────────────────────────────────────────────────────┘
```

Same pattern as `beamer-vst3` - format-specific wrapper around format-agnostic core.

### Code Sketch

```rust
// beamer-au/src/audio_unit.rs

use beamer_core::{AudioProcessor, HasParameters, Plugin};
use objc2::declare_class;
use objc2::runtime::AnyClass;
use objc2_audio_toolbox::AUAudioUnit;

declare_class!(
    pub struct BeamerAudioUnit<P: Plugin> {
        processor: Option<P::Processor>,
        // ...
    }

    unsafe impl<P: Plugin> ClassType for BeamerAudioUnit<P> {
        type Super = AUAudioUnit;
        const NAME: &'static str = "BeamerAudioUnit";
    }

    // AUAudioUnit overrides
    unsafe impl<P: Plugin> BeamerAudioUnit<P> {
        #[method(allocateRenderResourcesAndReturnError:)]
        fn allocate_render_resources(&self, error: *mut *mut NSError) -> bool {
            // Call Plugin::prepare() here
        }

        #[method(deallocateRenderResources)]
        fn deallocate_render_resources(&self) {
            // Call AudioProcessor::unprepare() here
        }

        #[method(internalRenderBlock)]
        fn internal_render_block(&self) -> AURenderBlock {
            // Return block that calls AudioProcessor::process()
        }
    }
);
```

---

## Implementation Plan

### Phase 1: Core AUv3 Support

| Task | Description | Effort |
|------|-------------|--------|
| AUAudioUnit subclass | `declare_class!` with lifecycle methods | Medium |
| Parameter tree | Map `Parameters` trait → `AUParameterTree` | Small |
| Audio rendering | Render block calling `process()` | Small |
| MIDI events | AU MIDI → `MidiEvent` translation | Small |
| Bundle structure | App Extension packaging | Small |
| auval validation | Fix issues until validation passes | Medium |

### Phase 2: Full Feature Parity

| Task | Description | Effort |
|------|-------------|--------|
| State persistence | `fullState` / `fullStateForDocument` | Small |
| Presets | Factory + user preset support | Small |
| View controller | `AUViewController` for WebView UI | Medium |
| iOS support | ARM build, App Store considerations | Medium |

### Phase 3: Polish

| Task | Description | Effort |
|------|-------------|--------|
| AUv2 compatibility | Optional legacy support | Medium |
| Host quirk handling | Logic, GarageBand, MainStage testing | Medium |
| Documentation | Usage guide, examples | Small |

---

## Export Macro

```rust
// User's plugin crate

use beamer::prelude::*;
use beamer_au::export_au;

export_au!(CONFIG, AuProcessor<MyPlugin>);
```

Or unified with other formats:

```rust
beamer::export_plugin! {
    config: CONFIG,
    plugin: MyPlugin,
    formats: [vst3, au, clap],
}
```

---

## Bundle Structure

AUv3 plugins are App Extensions:

```
MyPlugin.appex/
├── Contents/
│   ├── Info.plist
│   │   ├── NSExtension
│   │   │   ├── NSExtensionPointIdentifier: com.apple.AudioUnit-UI
│   │   │   └── NSExtensionPrincipalClass: BeamerAudioUnit
│   │   ├── AudioComponents
│   │   │   ├── type: aufx/aumu/aumi
│   │   │   ├── subtype: (4-char code)
│   │   │   └── manufacturer: (4-char code)
│   ├── MacOS/
│   │   └── MyPlugin              # Rust binary
│   └── Resources/
│       └── (assets, if any)
```

The `xtask` tool would generate this structure:

```bash
cargo xtask bundle my-plugin --au
# Produces: target/bundled/MyPlugin.appex
```

---

## Comparison with Alternatives

| Aspect | VST3 SDK Wrapper | Native Rust (Recommended) |
|--------|------------------|---------------------------|
| Build dependencies | C++ toolchain | Pure Rust + objc2 |
| Runtime architecture | AU → wrapper → VST3 → plugin | AU → beamer-au → plugin |
| Bundle contents | .appex containing .vst3 | Single binary |
| Bundle size | Larger | Smaller |
| Maintenance | Track Steinberg updates | Own codebase |
| AU-specific features | Limited by wrapper | Full access |
| iOS support | Complex | Native |

---

## Licensing

| Component | License | Impact |
|-----------|---------|--------|
| Apple AudioToolbox/AVFoundation | System frameworks | None - dynamic linking |
| objc2 crates | MIT | ✅ Compatible |
| Steinberg wrapper (as reference) | MIT (since v3.8) | ✅ Compatible |
| Your beamer-au code | MIT | ✅ No issues |

**Beamer can remain fully MIT licensed.**

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| objc2 bindings incomplete | Bindings are auto-generated from Xcode SDK; gaps can be filled |
| auval validation failures | Use Steinberg wrapper as reference for correct behavior |
| Host-specific quirks | Test early with Logic, GarageBand, MainStage |
| iOS App Store rules | Follow Apple guidelines, may need containing app |

---

## Dependencies

```toml
# beamer-au/Cargo.toml

[dependencies]
beamer-core = { path = "../beamer-core" }
objc2 = "0.5"
objc2-foundation = "0.2"
objc2-audio-toolbox = "0.2"
objc2-avf-audio = "0.2"
block2 = "0.5"

[target.'cfg(target_os = "macos")'.dependencies]
# macOS-specific if needed

[target.'cfg(target_os = "ios")'.dependencies]
# iOS-specific if needed
```

---

## References

- [objc2 GitHub](https://github.com/madsmtm/objc2) - Rust bindings to Apple frameworks
- [objc2-audio-toolbox docs](https://docs.rs/objc2-audio-toolbox/latest/objc2_audio_toolbox/)
- [VST 3.8 MIT License Announcement](https://www.steinberg.net/press/2025/vst-3-8/)
- [Apple Audio Unit Documentation](https://developer.apple.com/documentation/audiotoolbox/audio_unit_v3_plug-ins)
- [AUv3 App Extension Guide](https://developer.apple.com/documentation/avfaudio/audio_engine/building_an_audio_unit_extension)
