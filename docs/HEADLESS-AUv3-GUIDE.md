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

---

## Overview

AUv3 (Audio Unit version 3) is Apple's modern audio plugin format based on App Extensions. Unlike older formats (VST2, AUv2), AUv3 plugins are sandboxed and must be delivered as part of a host application.

A **headless** AUv3 plugin has no custom GUI - it relies on the host DAW to provide a generic parameter interface. This simplifies development significantly while still providing full audio processing capabilities.

### Key Characteristics of Headless AUv3

- No custom view controller implementation needed
- No graphics framework dependencies
- Smaller binary size
- Simpler codebase
- Host provides generic parameter UI (sliders, knobs)
- Full audio processing capabilities retained

---

## AUv3 Architecture

An AUv3 plugin on macOS consists of three nested bundles:

```
MyPlugin.app/                           # Host Application
├── Contents/
│   ├── Info.plist                      # App metadata
│   ├── MacOS/
│   │   └── MyPlugin                    # App executable
│   ├── Frameworks/
│   │   └── AUv3Framework.framework/    # Shared framework
│   │       └── Versions/A/
│   │           ├── AUv3Framework       # Framework binary
│   │           └── Resources/
│   │               └── Info.plist      # Framework metadata
│   ├── PlugIns/
│   │   └── MyPlugin.appex/             # App Extension
│   │       └── Contents/
│   │           ├── Info.plist          # Extension metadata (CRITICAL)
│   │           ├── MacOS/
│   │           │   └── MyPlugin        # Extension binary
│   │           └── _CodeSignature/
│   └── Resources/
│       └── MyPlugin.icns               # App icon
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
BeamerEffectHeadless.app/
├── Contents/
│   ├── Info.plist
│   ├── PkgInfo                         # Contains "BNDL????" or "APPL????"
│   ├── MacOS/
│   │   └── BeamerEffectHeadless         # Mach-O executable (arm64/x86_64)
│   ├── Frameworks/
│   │   └── AUv3Framework.framework/
│   │       ├── Versions/
│   │       │   ├── A/
│   │       │   │   ├── AUv3Framework   # Mach-O dylib
│   │       │   │   ├── Resources/
│   │       │   │   │   └── Info.plist
│   │       │   │   └── _CodeSignature/
│   │       │   │       └── CodeResources
│   │       │   └── Current -> A        # Symlink
│   │       ├── AUv3Framework -> Versions/Current/AUv3Framework
│   │       └── Resources -> Versions/Current/Resources
│   ├── PlugIns/
│   │   └── BeamerEffectHeadless.appex/
│   │       └── Contents/
│   │           ├── Info.plist          # MOST IMPORTANT FILE
│   │           ├── PkgInfo             # Contains "XPC!????"
│   │           ├── MacOS/
│   │           │   └── BeamerEffectHeadless
│   │           └── _CodeSignature/
│   │               └── CodeResources
│   ├── Resources/
│   │   └── BeamerEffectHeadless.icns
│   └── _CodeSignature/
│       └── CodeResources
```

### Bundle Identifiers (Must Be Consistent)

```
App:        com.AcmeInc.app.BeamerEffectHeadless
Extension:  com.AcmeInc.app.BeamerEffectHeadless.AUv3
Framework:  com.AcmeInc.app.BeamerEffectHeadless.AUv3Framework
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
    <string>BeamerEffectHeadless</string>

    <key>CFBundleIdentifier</key>
    <string>com.AcmeInc.app.BeamerEffectHeadless</string>

    <key>CFBundleName</key>
    <string>BeamerEffectHeadless</string>

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
    <string>BeamerEffectHeadless</string>

    <key>CFBundleIdentifier</key>
    <string>com.AcmeInc.app.BeamerEffectHeadless.AUv3</string>

    <key>CFBundleName</key>
    <string>BeamerEffectHeadless</string>

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

        <!-- Principal class - your AUViewController subclass -->
        <key>NSExtensionPrincipalClass</key>
        <string>BeamerAUViewController_vBeamerEffectHeadless</string>

        <key>NSExtensionAttributes</key>
        <dict>
            <!-- Links to the framework containing the AUAudioUnit -->
            <key>AudioComponentBundle</key>
            <string>com.AcmeInc.app.BeamerEffectHeadless.AUv3Framework</string>

            <!-- Audio Unit component description -->
            <key>AudioComponents</key>
            <array>
                <dict>
                    <!-- Four-character codes (FourCC) -->
                    <key>type</key>
                    <string>aufx</string>  <!-- Effect type -->

                    <key>subtype</key>
                    <string>Iphl</string>  <!-- Your unique 4-char code -->

                    <key>manufacturer</key>
                    <string>Acme</string>  <!-- Your 4-char manufacturer code -->

                    <!-- Display information -->
                    <key>name</key>
                    <string>AcmeInc: BeamerEffectHeadless</string>

                    <key>description</key>
                    <string>BeamerEffectHeadless</string>

                    <!-- Version as integer: 0x00010000 = 1.0.0 -->
                    <key>version</key>
                    <integer>65536</integer>

                    <!-- Sandbox safety declaration -->
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
    <string>AUv3Framework</string>

    <key>CFBundleIdentifier</key>
    <string>com.AcmeInc.app.BeamerEffectHeadless.AUv3Framework</string>
    <!-- MUST match AudioComponentBundle in extension Info.plist -->

    <key>CFBundleName</key>
    <string>AUv3Framework</string>

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
    <string>group.com.AcmeInc.BeamerEffectHeadless</string>
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
APP_PATH="/path/to/MyPlugin.app"

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
    "$APP_PATH/Contents/Frameworks/AUv3Framework.framework"

# 2. Sign app extension with entitlements
codesign --force --sign - \
    --entitlements /tmp/appex.entitlements \
    "$APP_PATH/Contents/PlugIns/MyPlugin.appex"

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
2. **AUAudioUnitFactory** - Creates instances
3. **Parameter tree** - Exposes parameters to host

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
pluginkit -a /path/to/MyPlugin.app/Contents/PlugIns/MyPlugin.appex

# List registered Audio Units
pluginkit -m -p com.apple.AudioUnit
pluginkit -m -p com.apple.AudioUnit-UI

# Remove registration
pluginkit -r /path/to/MyPlugin.appex
```

### Verifying Registration

```bash
# List all Audio Units
auval -a

# Find your plugin
auval -a | grep "YourManufacturer"

# Validate specific plugin
auval -v aufx Iphl Acme
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
my-plugin/
├── Cargo.toml
├── src/
│   └── lib.rs              # Rust DSP code
├── capi/
│   └── bridge.h            # C API for FFI
├── objc/
│   ├── AUv3Framework.h
│   ├── MyAudioUnit.h
│   └── MyAudioUnit.m       # AUAudioUnit subclass
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
// objc/MyAudioUnit.m

#import "MyAudioUnit.h"

// Import Rust functions
extern void* plugin_create(void);
extern void plugin_destroy(void* state);
extern void plugin_set_gain(void* state, float gain);
extern void plugin_process(void* state,
                           const float** inputs,
                           float** outputs,
                           uint32_t channels,
                           uint32_t frames);

@implementation MyAudioUnit {
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
auval -v aufx Iphl Acme

# Verbose validation
auval -v aufx Iphl Acme -w

# Strict validation
auval -v aufx Iphl Acme -strict
```

### Expected auval Output

```
--------------------------------------------------
VALIDATING AUDIO UNIT: 'aufx' - 'Iphl' - 'Acme'
--------------------------------------------------
Manufacturer String: AcmeInc
AudioUnit Name: BeamerEffectHeadless
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

## Build Commands Reference

### Building with Xcode (Command Line)

```bash
cd /path/to/BeamerEffectHeadless

# Build
xcodebuild -project projects/BeamerEffectHeadless-macOS.xcodeproj \
    -scheme "macOS-APP with AUv3" \
    -configuration Debug \
    CODE_SIGNING_ALLOWED=NO

# Clean
xcodebuild -project projects/BeamerEffectHeadless-macOS.xcodeproj \
    -scheme "macOS-APP with AUv3" \
    clean
```

### Manual Build Steps (for Rust/Custom)

```bash
# 1. Compile framework
clang -framework AudioToolbox -framework AVFoundation \
    -dynamiclib -o AUv3Framework.framework/AUv3Framework \
    MyAudioUnit.m -L. -lmy_rust_dsp

# 2. Compile app extension
clang -framework AudioToolbox \
    -o MyPlugin.appex/Contents/MacOS/MyPlugin \
    AppexMain.m

# 3. Compile host app
clang -framework Cocoa \
    -o MyPlugin.app/Contents/MacOS/MyPlugin \
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

- [Apple Audio Unit Hosting Guide](https://developer.apple.com/documentation/audiotoolbox/audio_unit_hosting_guide)
- [App Extension Programming Guide](https://developer.apple.com/library/archive/documentation/General/Conceptual/ExtensibilityPG/)
- [AUAudioUnit Class Reference](https://developer.apple.com/documentation/audiotoolbox/auaudiounit)
- [Code Signing Guide](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/)
- [iPlug2 Framework](https://github.com/iPlug2/iPlug2)
