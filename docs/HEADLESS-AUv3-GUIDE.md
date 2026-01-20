# Headless AUv3 Plugin Development Guide

A comprehensive guide to building headless (no GUI) AUv3 audio plugins for macOS. This document is written to help developers, including those using Rust, understand the architecture and requirements for creating AUv3 plugins without a custom user interface.

## Table of Contents

1. [Overview](#overview)
2. [AUv3 Architecture](#auv3-architecture)
3. [Bundle Structure](#bundle-structure)
4. [Info.plist Configuration](#infoplist-configuration)
5. [Entitlements](#entitlements)
6. [Code Signing](#code-signing)
7. [Audio Processing Implementation](#audio-processing-implementation)
8. [Headless vs GUI Plugin Differences](#headless-vs-gui-plugin-differences)
9. [Plugin Registration](#plugin-registration)
10. [Building from Rust](#building-from-rust)
11. [Validation and Testing](#validation-and-testing)
12. [Troubleshooting](#troubleshooting)

---

## Overview

AUv3 (Audio Unit version 3) is Apple's modern audio plugin format based on App Extensions. Unlike older formats, AUv3 plugins must be delivered as part of a host application.

By default, AUv3 plugins run **out-of-process** (in a separate sandboxed process). On macOS, hosts can request **in-process** loading for lower latency, but the plugin must be packaged to support it (with `AudioComponentBundle` pointing to a framework containing the `AUAudioUnit` implementation). The `sandboxSafe` key in Info.plist indicates whether the plugin can be loaded directly into a sandboxed host process.

A **headless** AUv3 plugin has no custom GUI - it relies on the host DAW to provide a generic parameter interface. This simplifies development significantly while still providing full audio processing capabilities.

### Key Characteristics of Headless AUv3

- No custom view controller implementation needed
- No graphics framework dependencies
- Smaller binary size
- Simpler codebase
- Host provides generic parameter UI (sliders, knobs)
- Full audio processing capabilities retained

### Limitations

- **Audio Unit extensions cannot perform recording** - They only provide real-time audio processing (generation or modification)
- Each extension contains exactly one audio unit

---

## AUv3 Architecture

An AUv3 plugin on macOS consists of three nested bundles:

```
Gain.app/                               # Host Application
├── Contents/
│   ├── Info.plist                      # App metadata
│   ├── MacOS/
│   │   └── gain                        # App executable
│   ├── Frameworks/
│   │   └── GainAU.framework/           # Shared framework
│   │       └── Versions/A/
│   │           ├── GainAU              # Framework binary
│   │           └── Resources/
│   │               └── Info.plist      # Framework metadata
│   ├── PlugIns/
│   │   └── Gain.appex/                 # App Extension
│   │       └── Contents/
│   │           ├── Info.plist          # Extension metadata (CRITICAL)
│   │           ├── MacOS/
│   │           │   └── gain            # Extension binary
│   │           └── _CodeSignature/
│   └── Resources/
│       └── Gain.icns                   # App icon
└── _CodeSignature/
```

### Component Roles

| Component | Purpose |
|-----------|---------|
| **Host App** (.app) | Container that hosts the extension. Required for distribution. Can be minimal. |
| **App Extension** (.appex) | The actual plugin. Loaded by DAWs. Contains NSExtension metadata. |
| **Framework** (.framework) | Shared code between app and extension. Contains the AUAudioUnit subclass. |

### Why Three Components?

1. **App Store Requirement**: Extensions must be delivered inside an app
2. **Code Sharing**: Framework allows app and extension to share code
3. **Sandboxing**: Extension runs in its own sandboxed process
4. **Discovery**: macOS discovers plugins by scanning app extensions

---

## Bundle Structure

### Detailed File Tree

```
Gain.app/
├── Contents/
│   ├── Info.plist
│   ├── PkgInfo                         # Contains "BNDL????" or "APPL????"
│   ├── MacOS/
│   │   └── gain                        # Mach-O executable (arm64/x86_64)
│   ├── Frameworks/
│   │   └── GainAU.framework/
│   │       ├── Versions/
│   │       │   ├── A/
│   │       │   │   ├── GainAU          # Mach-O dylib
│   │       │   │   ├── Resources/
│   │       │   │   │   └── Info.plist
│   │       │   │   └── _CodeSignature/
│   │       │   │       └── CodeResources
│   │       │   └── Current -> A        # Symlink
│   │       ├── GainAU -> Versions/Current/GainAU
│   │       └── Resources -> Versions/Current/Resources
│   ├── PlugIns/
│   │   └── Gain.appex/
│   │       └── Contents/
│   │           ├── Info.plist          # MOST IMPORTANT FILE
│   │           ├── PkgInfo             # Contains "XPC!????"
│   │           ├── MacOS/
│   │           │   └── gain
│   │           └── _CodeSignature/
│   │               └── CodeResources
│   ├── Resources/
│   │   └── Gain.icns
│   └── _CodeSignature/
│       └── CodeResources
```

### Bundle Identifiers (Must Be Consistent)

Beamer uses the following bundle identifier pattern:

```
App:        com.beamer.{package}
Extension:  com.beamer.{package}.audiounit
Framework:  com.beamer.{package}.framework
```

For example, a plugin named `gain`:

```
App:        com.beamer.gain
Extension:  com.beamer.gain.audiounit
Framework:  com.beamer.gain.framework
```

The extension's `AudioComponentBundle` key must exactly match the framework's `CFBundleIdentifier`.

---

## Info.plist Configuration

### Host Application Info.plist

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <!-- Required keys -->
    <key>CFBundleExecutable</key>
    <string>gain</string>

    <key>CFBundleIdentifier</key>
    <string>com.beamer.gain</string>

    <key>CFBundleName</key>
    <string>Gain</string>

    <key>CFBundlePackageType</key>
    <string>APPL</string>

    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>

    <key>CFBundleVersion</key>
    <string>1.0.0</string>

    <!-- Recommended keys -->
    <key>LSMinimumSystemVersion</key>
    <string>10.13</string>

    <key>LSApplicationCategoryType</key>
    <string>public.app-category.music</string>

    <key>NSMicrophoneUsageDescription</key>
    <string>This app needs mic access to process audio.</string>
</dict>
</plist>
```

### App Extension Info.plist (CRITICAL)

This is the most important configuration file. It tells macOS this is an Audio Unit.

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <!-- Standard bundle keys -->
    <key>CFBundleExecutable</key>
    <string>gain</string>

    <key>CFBundleIdentifier</key>
    <string>com.beamer.gain.audiounit</string>

    <key>CFBundleName</key>
    <string>Gain</string>

    <key>CFBundlePackageType</key>
    <string>XPC!</string>  <!-- MUST be XPC! for app extensions -->

    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>

    <key>CFBundleVersion</key>
    <string>1.0.0</string>

    <key>LSMinimumSystemVersion</key>
    <string>10.13</string>

    <!-- NSExtension - Audio Unit Configuration -->
    <key>NSExtension</key>
    <dict>
        <!-- Extension point identifier -->
        <key>NSExtensionPointIdentifier</key>
        <string>com.apple.AudioUnit</string>
        <!-- Use "com.apple.AudioUnit" for headless -->
        <!-- Use "com.apple.AudioUnit-UI" for plugins with custom UI -->

        <!-- Principal class - must conform to AUAudioUnitFactory protocol -->
        <!-- For headless: subclass NSObject -->
        <!-- For UI: subclass AUViewController (from CoreAudioKit) -->
        <key>NSExtensionPrincipalClass</key>
        <string>BeamerAuViewController_vGain</string>

        <key>NSExtensionAttributes</key>
        <dict>
            <!-- Links to the framework containing the AUAudioUnit -->
            <key>AudioComponentBundle</key>
            <string>com.beamer.gain.framework</string>

            <!-- Audio Unit component description -->
            <key>AudioComponents</key>
            <array>
                <dict>
                    <!-- Four-character codes (FourCC) -->
                    <key>type</key>
                    <string>aufx</string>  <!-- Effect type -->

                    <key>subtype</key>
                    <string>Gain</string>  <!-- Your unique 4-char code -->

                    <key>manufacturer</key>
                    <string>Beam</string>  <!-- Your 4-char manufacturer code -->

                    <!-- Display information -->
                    <key>name</key>
                    <string>Beamer: Gain</string>

                    <key>description</key>
                    <string>Gain</string>

                    <!-- Version as integer: 0x00010000 = 1.0.0 -->
                    <key>version</key>
                    <integer>65536</integer>

                    <!-- Sandbox safety: true = can load in-process into sandboxed hosts -->
                    <!-- If false, host needs com.apple.security.temporary-exception.audio-unit-host entitlement -->
                    <key>sandboxSafe</key>
                    <true/>

                    <!-- Tags for categorization -->
                    <key>tags</key>
                    <array>
                        <string>Effects</string>
                    </array>
                </dict>
            </array>
        </dict>
    </dict>
</dict>
</plist>
```

### Audio Unit Types (FourCC)

| Type Code | Description | Example Use |
|-----------|-------------|-------------|
| `aufx` | Audio Effect | EQ, compressor, reverb |
| `aumu` | Music Device (Instrument) | Synthesizer, sampler |
| `augn` | Generator | Test tone, noise generator |
| `aumf` | Music Effect | MIDI-controlled effect |
| `aufc` | Format Converter | Sample rate converter |
| `auol` | Offline Effect | Time-stretch, pitch-shift |
| `aumi` | MIDI Processor | MIDI effect |

### Framework Info.plist

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>GainAU</string>

    <key>CFBundleIdentifier</key>
    <string>com.beamer.gain.framework</string>
    <!-- MUST match AudioComponentBundle in extension Info.plist -->

    <key>CFBundleName</key>
    <string>GainAU</string>

    <key>CFBundlePackageType</key>
    <string>FMWK</string>

    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>

    <key>CFBundleVersion</key>
    <string>1.0.0</string>
</dict>
</plist>
```

---

## Entitlements

### App Extension Entitlements (REQUIRED)

The app extension **must** have sandbox entitlements to be registered by macOS.

**Minimum required** (`appex.entitlements`):
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
</dict>
</plist>
```

### Host Application Entitlements

**Minimum for development** (`app.entitlements`):
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
</dict>
</plist>
```

### Additional Entitlements (Optional)

For production apps, you may need:

```xml
<!-- Audio input access -->
<key>com.apple.security.device.audio-input</key>
<true/>

<!-- File access -->
<key>com.apple.security.files.user-selected.read-write</key>
<true/>

<!-- Network access (for license validation, etc.) -->
<key>com.apple.security.network.client</key>
<true/>

<!-- App groups (for sharing data between app and extension) -->
<key>com.apple.security.application-groups</key>
<array>
    <string>group.com.beamer.gain</string>
</array>
```

**Note**: `application-groups` requires a valid Apple Developer Team ID and cannot be used with ad-hoc signing.

---

## Code Signing

### Why Code Signing Matters

AUv3 plugins **require** valid code signatures because:
1. App extensions run in sandboxed processes
2. macOS verifies signatures before loading extensions
3. All components must share the same signing identity

### Signing Order (Critical)

Components must be signed in this order (inside-out):
1. Framework
2. App Extension
3. Host Application

### Ad-hoc Signing Script

```bash
#!/bin/bash
APP_PATH="/path/to/Gain.app"

# Create entitlements for appex
cat > /tmp/appex.entitlements << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
</dict>
</plist>
EOF

# Create entitlements for app
cat > /tmp/app.entitlements << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
</dict>
</plist>
EOF

# 1. Sign framework first
codesign --force --sign - \
    "$APP_PATH/Contents/Frameworks/GainAU.framework"

# 2. Sign app extension with entitlements
codesign --force --sign - \
    --entitlements /tmp/appex.entitlements \
    "$APP_PATH/Contents/PlugIns/Gain.appex"

# 3. Sign host app with entitlements
codesign --force --sign - \
    --entitlements /tmp/app.entitlements \
    "$APP_PATH"

echo "Signing complete"
```

### Common Signing Mistakes

| Mistake | Symptom | Solution |
|---------|---------|----------|
| Using `--deep` | Strips entitlements | Sign components individually |
| Wrong order | Library load failures | Sign framework → appex → app |
| Missing entitlements | Plugin not discovered | Add `app-sandbox` to appex |
| Mismatched identities | "Different Team IDs" error | Use same identity for all |

---

## Audio Processing Implementation

### Minimal AUAudioUnit Subclass

For a headless plugin, you need to implement:

1. **AUAudioUnit subclass** - Core audio processing
2. **AUAudioUnitFactory protocol** - Creates instances (principal class must conform)
3. **Parameter tree** - Exposes parameters to host

### Required AUAudioUnit Overrides

Your `AUAudioUnit` subclass **must** override these methods:

| Method | Purpose |
|--------|---------|
| `inputBusses` (getter) | Returns the array of input buses |
| `outputBusses` (getter) | Returns the array of output buses |
| `internalRenderBlock` (getter) | Returns the block that processes audio |
| `allocateRenderResourcesAndReturnError:` | Called when plugin loads; allocate buffers here |
| `deallocateRenderResources` | Called when plugin unloads; free resources here |

### Core Audio Callbacks

```
┌─────────────────────────────────────────────────────────────┐
│                        Host DAW                              │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    AUAudioUnit                               │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  allocateRenderResources()                          │    │
│  │  - Called when plugin is loaded                     │    │
│  │  - Allocate buffers, initialize DSP                 │    │
│  └─────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  internalRenderBlock                                │    │
│  │  - Called for each audio buffer                     │    │
│  │  - Process audio samples                            │    │
│  │  - Handle parameter changes                         │    │
│  └─────────────────────────────────────────────────────┘    │
│  ┌─────────────────────────────────────────────────────┐    │
│  │  deallocateRenderResources()                        │    │
│  │  - Called when plugin is unloaded                   │    │
│  │  - Free buffers, cleanup                            │    │
│  └─────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────┘
```

### Simple Gain Effect (Pseudocode)

```rust
// Parameter definition
struct Parameters {
    gain: f64,  // 0.0 to 1.0
}

// Called once when plugin loads
fn allocate_render_resources(sample_rate: f64, max_frames: u32) {
    // Initialize any DSP state
    // Allocate any buffers needed
}

// Called for each audio buffer (MUST be realtime-safe)
fn process_block(
    inputs: &[&[f32]],      // Input channels
    outputs: &mut [&mut [f32]], // Output channels
    frame_count: u32,
    params: &Parameters
) {
    let gain = params.gain as f32;

    for channel in 0..outputs.len() {
        for frame in 0..frame_count as usize {
            outputs[channel][frame] = inputs[channel][frame] * gain;
        }
    }
}

// Called when plugin unloads
fn deallocate_render_resources() {
    // Free any allocated resources
}
```

### Realtime Safety Requirements

The render block **must not**:
- Allocate memory (`malloc`, `new`, `Vec::push`)
- Acquire locks (`Mutex`, `RwLock`)
- Perform I/O (file, network)
- Call Objective-C runtime (in some cases)
- Block or wait

The render block **can**:
- Read/write to pre-allocated buffers
- Use lock-free data structures
- Read atomic parameters
- Perform math operations

---

## Headless vs GUI Plugin Differences

### Info.plist Differences

| Key | Headless | With GUI |
|-----|----------|----------|
| `NSExtensionPointIdentifier` | `com.apple.AudioUnit` | `com.apple.AudioUnit-UI` |
| `NSExtensionPrincipalClass` | Minimal view controller | Full view controller |

### Code Differences

| Aspect | Headless | With GUI |
|--------|----------|----------|
| View Controller | Minimal/stub | Full implementation |
| Graphics Framework | Not needed | Required (Metal, OpenGL, etc.) |
| Custom Views | None | NSView/UIView subclasses |
| Resource Files | Minimal | Icons, images, XIBs |
| Binary Size | Smaller | Larger |

### What You Don't Need for Headless

1. **No custom NSView/UIView** - Host provides generic UI
2. **No graphics framework** - No Metal, OpenGL, Skia, NanoVG
3. **No image assets** - No knob images, backgrounds
4. **No XIB/Storyboard files** - No Interface Builder layouts
5. **No font files** - Unless needed for non-UI purposes

### What You Still Need

1. **AUAudioUnit subclass** - Core processing
2. **Parameter tree** - For host to display controls
3. **Audio buffer handling** - Process audio
4. **Info.plist configuration** - Plugin metadata
5. **Entitlements** - Sandbox requirements
6. **Code signing** - All bundles must be signed

---

## Plugin Registration

### Automatic Registration

When a properly configured app is launched, macOS automatically registers its extensions. The system:

1. Reads the app's `Contents/PlugIns/` directory
2. Parses each `.appex` bundle's `Info.plist`
3. Registers extensions with `pluginkit`
4. Makes Audio Units available to hosts

### Manual Registration

```bash
# Register a plugin
pluginkit -a /path/to/Gain.app/Contents/PlugIns/Gain.appex

# List registered Audio Units
pluginkit -m -p com.apple.AudioUnit
pluginkit -m -p com.apple.AudioUnit-UI

# Remove registration
pluginkit -r /path/to/Gain.appex
```

### Verifying Registration

```bash
# List all Audio Units
auval -a

# Find your plugin
auval -a | grep "Beamer"

# Validate specific plugin
auval -v aufx Gain Beam
#        │    │    └── Manufacturer code
#        │    └── Subtype code
#        └── Type code
```

### Troubleshooting Registration

| Issue | Cause | Solution |
|-------|-------|----------|
| Plugin not visible | Missing entitlements | Add `app-sandbox` to appex |
| "Cannot find component" | Not registered | Run app or `pluginkit -a` |
| Validation fails | Code signing issue | Re-sign all components |
| Wrong Team ID error | Inconsistent signing | Sign with same identity |

---

## Building from Rust

### Recommended Approach

For Rust developers, the recommended approach is:

1. **Use `cargo-bundle`** or similar to create the app structure
2. **Write Objective-C/Swift glue code** for AUAudioUnit
3. **Call Rust DSP code** via FFI from the render block

### Project Structure

```
gain/
├── Cargo.toml
├── src/
│   └── lib.rs              # Rust DSP code
├── capi/
│   └── bridge.h            # C API for FFI
├── objc/
│   ├── GainAU.h
│   ├── GainAudioUnit.h
│   └── GainAudioUnit.m     # AUAudioUnit subclass
├── resources/
│   ├── App-Info.plist
│   ├── Appex-Info.plist
│   └── Framework-Info.plist
└── build.rs
```

### Rust C API Example

```rust
// src/lib.rs

#[repr(C)]
pub struct PluginState {
    gain: f32,
}

#[no_mangle]
pub extern "C" fn plugin_create() -> *mut PluginState {
    Box::into_raw(Box::new(PluginState { gain: 1.0 }))
}

#[no_mangle]
pub extern "C" fn plugin_destroy(state: *mut PluginState) {
    if !state.is_null() {
        unsafe { drop(Box::from_raw(state)); }
    }
}

#[no_mangle]
pub extern "C" fn plugin_set_gain(state: *mut PluginState, gain: f32) {
    if let Some(state) = unsafe { state.as_mut() } {
        state.gain = gain;
    }
}

#[no_mangle]
pub extern "C" fn plugin_process(
    state: *mut PluginState,
    inputs: *const *const f32,
    outputs: *mut *mut f32,
    channels: u32,
    frames: u32,
) {
    let state = match unsafe { state.as_ref() } {
        Some(s) => s,
        None => return,
    };

    let gain = state.gain;

    for ch in 0..channels as usize {
        let input = unsafe { std::slice::from_raw_parts(*inputs.add(ch), frames as usize) };
        let output = unsafe { std::slice::from_raw_parts_mut(*outputs.add(ch), frames as usize) };

        for i in 0..frames as usize {
            output[i] = input[i] * gain;
        }
    }
}
```

### Objective-C Bridge

```objc
// objc/GainAudioUnit.m

#import "GainAudioUnit.h"

// Import Rust functions
extern void* plugin_create(void);
extern void plugin_destroy(void* state);
extern void plugin_set_gain(void* state, float gain);
extern void plugin_process(void* state,
                           const float** inputs,
                           float** outputs,
                           uint32_t channels,
                           uint32_t frames);

@implementation GainAudioUnit {
    void* _pluginState;
    AUAudioUnitBus* _inputBus;
    AUAudioUnitBus* _outputBus;
    AUAudioUnitBusArray* _inputBusArray;
    AUAudioUnitBusArray* _outputBusArray;
}

- (instancetype)initWithComponentDescription:(AudioComponentDescription)componentDescription
                                     options:(AudioComponentInstantiationOptions)options
                                       error:(NSError**)outError {
    self = [super initWithComponentDescription:componentDescription
                                       options:options
                                         error:outError];
    if (self) {
        _pluginState = plugin_create();
        [self setupBuses];
        [self setupParameters];
    }
    return self;
}

- (void)dealloc {
    plugin_destroy(_pluginState);
}

- (AUInternalRenderBlock)internalRenderBlock {
    void* state = _pluginState;

    return ^AUAudioUnitStatus(
        AudioUnitRenderActionFlags* actionFlags,
        const AudioTimeStamp* timestamp,
        AVAudioFrameCount frameCount,
        NSInteger outputBusNumber,
        AudioBufferList* outputData,
        const AURenderEvent* realtimeEventListHead,
        AURenderPullInputBlock pullInputBlock
    ) {
        // Pull input
        AudioBufferList* inputData = /* ... */;
        pullInputBlock(actionFlags, timestamp, frameCount, 0, inputData);

        // Get buffer pointers
        const float* inputs[2] = {
            inputData->mBuffers[0].mData,
            inputData->mBuffers[1].mData
        };
        float* outputs[2] = {
            outputData->mBuffers[0].mData,
            outputData->mBuffers[1].mData
        };

        // Call Rust processing
        plugin_process(state, inputs, outputs, 2, frameCount);

        return noErr;
    };
}

@end
```

### Alternative: Pure Rust with objc2

Using the `objc2` crate, you can write the entire plugin in Rust:

```rust
use objc2::rc::Id;
use objc2::runtime::AnyClass;
use objc2::{declare_class, ClassType, DeclaredClass};
use objc2_foundation::NSObject;

// This is more complex but possible
// See: https://github.com/AugmendTech/au3-rs for inspiration
```

---

## Validation and Testing

### auval (Apple's Validator)

```bash
# List all plugins
auval -a

# Validate your plugin
auval -v aufx Gain Beam

# Verbose validation
auval -v aufx Gain Beam -w

# Strict validation
auval -v aufx Gain Beam -strict
```

### Expected auval Output

```
--------------------------------------------------
VALIDATING AUDIO UNIT: 'aufx' - 'Gain' - 'Beam'
--------------------------------------------------
Manufacturer String: Beamer
AudioUnit Name: Gain
Component Version: 1.0.0 (0x10000)

* * PASS
--------------------------------------------------
TESTING OPEN TIMES:
COLD:
Time to open AudioUnit:    0.123 ms
WARM:
Time to open AudioUnit:    0.045 ms
...
```

### Testing in DAWs

1. **Logic Pro**: Rescan plugins (Logic Pro → Preferences → Plug-in Manager)
2. **GarageBand**: Automatic after Logic scan
3. **REAPER**: Preferences → Plug-ins → AU → Re-scan
4. **Ableton Live**: Preferences → Plug-ins → Rescan

### Common Validation Errors

| Error | Cause | Solution |
|-------|-------|----------|
| "Cannot find component" | Not registered | Register with `pluginkit -a` |
| "Cannot get name strings" | Info.plist error | Check AudioComponents dict |
| "Render failed" | DSP error | Check process block |
| "Parameter error" | Invalid parameter tree | Verify parameter setup |

---

## Troubleshooting

### Instrument (Synth) Produces No Sound

Common causes for AUv3 instruments not producing audio:

#### 1. Wrong Audio Unit Type
- Effects use `aufx`, instruments use `aumu`
- Check `type` in Info.plist matches your plugin type
- Verify with: `auval -v aumu <subtype> <manufacturer>`

#### 2. Channel Configuration
- Instruments typically have no inputs: `PLUG_CHANNEL_IO "0-2"` (0 inputs, 2 outputs)
- Effects have inputs: `PLUG_CHANNEL_IO "1-1 2-2"`
- Mismatch causes the host to not route audio correctly

#### 3. MIDI Not Being Processed
- Ensure `PLUG_DOES_MIDI_IN 1` is set in config.h
- Verify `ProcessMidiMsg()` is implemented and called
- Check that MIDI events reach your DSP code

#### 4. Audio Buffer Not Written
- Verify `ProcessBlock()` actually writes to the output buffers
- Check for silent/zero initialization that never gets overwritten
- Ensure sample rate and block size are handled in `OnReset()`

#### 5. Headless vs UI Plugin Registration
- Headless (no UI): register with `pluginkit -m -p com.apple.AudioUnit`
- With UI: register with `pluginkit -m -p com.apple.AudioUnit-UI`
- Wrong extension point = plugin won't appear in correct host category

#### Debugging Tips for Rust Developers

When building AUv3 instruments in Rust (via FFI to C/C++ or frameworks like `nih-plug`):

1. **Verify FFI boundary**: Ensure audio buffers cross the Rust/C boundary correctly
   - Check pointer alignment and lifetime
   - Verify buffer sizes match between Rust and host expectations

2. **Check render callback timing**:
   - Audio callbacks must be realtime-safe (no allocations, no locks)
   - Use `#[inline]` and avoid `Vec` resizing in the audio path

3. **Validate MIDI parsing**:
   ```bash
   # Monitor MIDI in Console.app or with:
   log stream --predicate 'subsystem == "com.apple.coreaudio"' --level debug
   ```

4. **Test outside DAW first**:
   - Use `auval` to validate: `auval -v aumu <subtype> <mfr>`
   - Use the standalone app to test audio generation independently

5. **Memory/Thread issues**:
   - Rust's ownership model can conflict with AU's threading expectations
   - Audio thread != main thread - ensure `Send`/`Sync` bounds are correct
   - Use atomics for parameter communication between threads

6. **Sample format mismatch**:
   - macOS AUv3 typically uses `Float32` (f32 in Rust)
   - Ensure your DSP processes and outputs the correct sample type

---

## Build Commands Reference

### Building with Beamer

```bash
# Bundle a plugin (creates .app with .appex and .framework)
cargo xtask bundle gain

# Bundle with release optimizations
cargo xtask bundle gain --release

# Build all crates without bundling
cargo build

# Run tests
cargo test

# Run lints
cargo clippy
```

### Manual Build Steps (for Rust/Custom)

```bash
# 1. Compile framework
clang -framework AudioToolbox -framework AVFoundation \
    -dynamiclib -o GainAU.framework/GainAU \
    GainAudioUnit.m -L. -lgain

# 2. Compile app extension
clang -framework AudioToolbox \
    -o Gain.appex/Contents/MacOS/gain \
    AppexMain.m

# 3. Compile host app
clang -framework Cocoa \
    -o Gain.app/Contents/MacOS/gain \
    AppMain.m

# 4. Sign everything
./sign.sh
```

---

## Summary

Creating a headless AUv3 plugin requires:

1. **Three nested bundles**: App → Appex → Framework
2. **Correct Info.plist**: Especially `NSExtension` with `AudioComponents`
3. **Sandbox entitlements**: `com.apple.security.app-sandbox` on appex
4. **Proper code signing**: Framework → Appex → App order
5. **AUAudioUnit implementation**: Parameter tree + render block
6. **Registration**: Via app launch or `pluginkit -a`

For headless plugins, you can skip all GUI code, graphics frameworks, and custom views. The host DAW provides a generic parameter interface automatically.

---

## References

- [Audio Unit App Extensions (Apple Archive)](https://developer.apple.com/library/archive/documentation/General/Conceptual/ExtensibilityPG/AudioUnit.html) - Official guide for AUv3 extensions
- [AUAudioUnit Class Reference](https://developer.apple.com/documentation/audiotoolbox/auaudiounit)
- [Creating Custom Audio Effects](https://developer.apple.com/documentation/avfaudio/creating-custom-audio-effects)
- [TN2312: Audio Unit Host Sandboxing Guide](https://developer.apple.com/library/archive/technotes/tn2312/_index.html) - In-process loading and sandbox requirements
- [App Extension Programming Guide](https://developer.apple.com/library/archive/documentation/General/Conceptual/ExtensibilityPG/)
- [Code Signing Guide](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/)
