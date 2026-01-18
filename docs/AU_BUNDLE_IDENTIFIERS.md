# AU Bundle Identifiers and Naming

## Overview

This document explains bundle identifier conventions for Audio Unit plugins in the Beamer framework, covering both AUv3 (current) and AUv2 (planned future support).

## What is a Bundle Identifier?

A **bundle identifier** (`CFBundleIdentifier`) is a unique reverse-DNS string that identifies a bundle in macOS. It serves as a globally unique ID for your app or extension.

### Format

Bundle identifiers follow reverse DNS notation:
```
com.company.product.type
```

For example:
- `com` - Top-level domain
- `beamer` - Company/framework name
- `synth` - Product/package name
- `audiounit` - Component type

This ensures global uniqueness across all macOS applications and plugins.

## Audio Unit Bundle Architecture

### AUv3 Plugin Structure

An AUv3 plugin consists of **three separate bundles**, each with its own identifier:

```
BeamerSynth.app/                                    # Container app
├── CFBundleIdentifier: com.beamer.synth
├── Contents/
│   ├── Frameworks/
│   │   └── BeamerSynthAU.framework/               # Shared framework
│   │       └── CFBundleIdentifier: com.beamer.synth.framework
│   └── PlugIns/
│       └── BeamerSynth.appex/                     # App extension
│           ├── CFBundleIdentifier: com.beamer.synth.audiounit
│           └── AudioComponentBundle: com.beamer.synth.framework
```

#### Bundle Roles

1. **Container App** (`BeamerSynth.app`)
   - Bundle ID: `com.beamer.{package}`
   - Purpose: Host application that contains the plugin
   - Installation: `~/Applications/` or `/Applications/`

2. **App Extension** (`BeamerSynth.appex`)
   - Bundle ID: `com.beamer.{package}.audiounit`
   - Purpose: The actual AU plugin that loads into the DAW
   - Location: Inside container app at `Contents/PlugIns/`

3. **Framework** (`BeamerSynthAU.framework`)
   - Bundle ID: `com.beamer.{package}.framework`
   - Purpose: Contains DSP code, Rust bindings, and native wrapper
   - Location: Inside container app at `Contents/Frameworks/`

## AudioComponentBundle

The `AudioComponentBundle` key in the app extension's Info.plist tells macOS where to find the plugin's executable code.

### Purpose

AUv3 plugins use a **split architecture**:
- **App Extension** (`.appex`) - Small loader that hosts the UI
- **Framework** (`.framework`) - Contains the actual DSP code

The `AudioComponentBundle` key points from the extension to the framework:

```xml
<!-- In BeamerSynth.appex/Contents/Info.plist -->
<key>NSExtensionAttributes</key>
<dict>
    <key>AudioComponents</key>
    <array>
        <dict>
            <key>type</key>
            <string>aumu</string>
            <!-- ... other keys ... -->
        </dict>
    </array>
    <key>AudioComponentBundle</key>
    <string>com.beamer.synth.framework</string>  <!-- Points to framework -->
</dict>
```

### Benefits

1. **Code sharing** - Multiple extensions can share one framework
2. **Better organization** - Separates UI from DSP logic
3. **Reduced duplication** - Common code in one location
4. **Easier updates** - Update framework once for all extensions

### Loading Flow

When a DAW loads the plugin:

1. DAW finds `BeamerSynth.appex`
2. Reads `AudioComponentBundle` from its Info.plist
3. Loads code from `com.beamer.synth.framework`
4. Framework's Rust DSP code executes
5. Plugin appears in DAW

## Current Beamer Naming Scheme (AUv3)

Beamer uses a clear, consistent naming scheme for AUv3 plugins:

| Component | Bundle Identifier | Example |
|-----------|------------------|---------|
| Container App | `com.beamer.{package}` | `com.beamer.synth` |
| App Extension | `com.beamer.{package}.audiounit` | `com.beamer.synth.audiounit` |
| Framework | `com.beamer.{package}.framework` | `com.beamer.synth.framework` |
| AudioComponentBundle | `com.beamer.{package}.framework` | `com.beamer.synth.framework` |

### Implementation

Defined in `xtask/src/main.rs`:

```rust
// Framework bundle ID (line 1825)
let framework_bundle_id = format!("com.beamer.{}.framework", package);

// App extension bundle ID (line 2395)
CFBundleIdentifier: com.beamer.{package}.audiounit

// AudioComponentBundle reference (line 2448)
<key>AudioComponentBundle</key>
<string>{framework_bundle_id}</string>
```

## Comparison with iPlug2

### iPlug2 Naming Convention

iPlug2 uses a more verbose scheme:

| Component | iPlug2 | Beamer |
|-----------|--------|--------|
| Container App | `com.AcmeInc.app.IPlugEffectHeadless` | `com.beamer.synth` |
| App Extension | `com.AcmeInc.app.IPlugEffectHeadless.AUv3` | `com.beamer.synth.audiounit` |
| Framework | `com.AcmeInc.app.IPlugEffectHeadless.AUv3Framework` | `com.beamer.synth.framework` |

### Key Differences

1. **Namespace Structure**
   - iPlug2: Uses `.app.PluginName` pattern (explicit hierarchy)
   - Beamer: Uses `.package` pattern (simpler, cleaner)

2. **Extension Suffix**
   - iPlug2: Uses `.AUv3` (version-specific)
   - Beamer: Uses `.audiounit` (more descriptive, version-agnostic)

3. **Framework Suffix**
   - iPlug2: Uses `.AUv3Framework` (verbose)
   - Beamer: Uses `.framework` (simpler)

4. **Framework Name**
   - iPlug2: Generic `AUv3Framework.framework` (same for all plugins)
   - Beamer: Unique `{Package}AU.framework` (per-plugin)

### Assessment

Both approaches are valid and follow Apple's guidelines. Beamer's scheme is:
- ✓ More concise and readable
- ✓ Easier to understand at a glance
- ✓ Less redundant
- ✓ Version-agnostic (works for future AU versions)

## Future AUv2 Support

When adding AUv2 (`.component`) support, bundle identifiers must be distinct from AUv3 to avoid conflicts.

### Proposed AUv2 Naming

| Component | Bundle Identifier | Location |
|-----------|------------------|----------|
| AUv2 Component | `com.beamer.{package}.component` | `~/Library/Audio/Plug-Ins/Components/` |
| Shared Framework | `com.beamer.{package}.framework` | Inside `.component` bundle |

### Bundle Identifier Conflict Prevention

**Problem**: Both AUv2 and AUv3 cannot use the same bundle identifier.

**Solution**: Use different suffixes:
- AUv2: `com.beamer.{package}.component`
- AUv3: `com.beamer.{package}.audiounit` (current)

This prevents macOS from confusing the two bundles while maintaining clear semantics.

### AudioComponentDescription (Critical!)

While bundle identifiers must differ, the **AudioComponentDescription** (type/manufacturer/subtype) **must be identical** for both formats:

```rust
// In both AUv2 and AUv3
ComponentType::MusicDevice,     // Type:  'aumu'
fourcc!(b"Bmer"),              // Manufacturer: 'Bmer'
fourcc!(b"synt"),              // Subtype: 'synt'
```

This allows DAWs to recognize them as the **same plugin**, just different implementations. Users see one plugin that works in both AUv2 and AUv3 hosts.

### Framework Sharing Between AUv2 and AUv3

AUv2 can also use `AudioComponentBundle` to reference an external framework (just like AUv3):

```xml
<!-- In AUv2 .component/Contents/Info.plist -->
<key>AudioComponents</key>
<array>
    <dict>
        <key>type</key>
        <string>aumu</string>
        <key>subtype</key>
        <string>synt</string>
        <key>manufacturer</key>
        <string>Bmer</string>
        <key>AudioComponentBundle</key>
        <string>com.beamer.synth.framework</string>  <!-- Same framework! -->
    </dict>
</array>
```

#### Benefits of Sharing

- **Single DSP codebase** - One Rust implementation for both formats
- **Smaller disk footprint** - Framework included only once
- **Easier updates** - Update one framework, both formats benefit
- **Consistency** - Identical behavior across AUv2 and AUv3

#### Deployment Options

**Option 1: Separate Bundles (Simpler)**
```
~/Library/Audio/Plug-Ins/Components/
└── BeamerSynth.component/
    ├── CFBundleIdentifier: com.beamer.synth.component
    └── Contents/MacOS/
        └── BeamerSynth      # All code embedded (no external framework)

~/Applications/BeamerSynth.app/
└── (AUv3 with framework, as before)
```

**Option 2: Shared Framework (Advanced)**
```
~/Library/Audio/Plug-Ins/Components/
└── BeamerSynth.component/
    ├── CFBundleIdentifier: com.beamer.synth.component
    ├── AudioComponentBundle: com.beamer.synth.framework
    └── Contents/Frameworks/
        └── BeamerSynthAU.framework/  # Shared with AUv3

~/Applications/BeamerSynth.app/
└── (uses same framework)
```

## Current Implementation Status

### AUv3 ✓ Implemented

File: `xtask/src/main.rs`

- Container app Info.plist generation: Lines 2277-2346
- App extension Info.plist generation: Lines 2358-2467
- Framework Info.plist generation: Lines 1843-1878
- Bundle identifier logic:
  - Framework: Line 1825
  - App extension: Line 2395
  - AudioComponentBundle: Line 2448

### AUv2 ⏳ Planned

Currently, Beamer only generates AUv3 (`.appex`) bundles. Support for AUv2 (`.component`) is planned for the future.

**Note**: Legacy `BeamerSimpleGain.component` exists but uses `com.beamer.simple-gain.audiounit` which would conflict with future AUv3. This should be updated to use `.component` suffix for consistency.

## Distribution Considerations

### AUv2 (.component)

- **Install location**: `~/Library/Audio/Plug-Ins/Components/` or `/Library/Audio/Plug-Ins/Components/`
- **Registration**: Automatic by system scan
- **Code signing**: Works with ad-hoc signing
- **DAW compatibility**: Universal (all AU-capable hosts)

### AUv3 (.appex in .app)

- **Install location**: `~/Applications/` or `/Applications/`
- **Registration**: By `pluginkit` when container app launches (must run app once)
- **Code signing**: Requires proper entitlements
- **DAW compatibility**: Modern hosts (Logic Pro 10.4.5+, some third-party)

### Dual Distribution Strategy

To support both AUv2-only hosts (Live, older Logic) and modern AUv3 hosts:

1. Build both formats with different bundle IDs
2. Share framework code between them
3. Package both in installer:
   - `BeamerSynth.component` → `/Library/Audio/Plug-Ins/Components/`
   - `BeamerSynth.app` → `/Applications/`
4. Same AudioComponentDescription ensures they appear as one plugin

## References

### Apple Documentation

- [Audio Unit v3 Plug-ins](https://developer.apple.com/documentation/audiotoolbox/audio_unit_v3_plug-ins)
- [Migrating Your Audio Unit Host to the AUv3 API](https://developer.apple.com/documentation/audiotoolbox/audio_unit_v3_plug-ins/migrating_your_audio_unit_host_to_the_auv3_api)

### Community Resources

- [JUCE Forum: AUv3 Customize Bundle ID](https://forum.juce.com/t/auv3-customize-bundle-id/53147)
- [JUCE Forum: Bundle Identifier per Format](https://forum.juce.com/t/shouldnt-bundle-identifier-be-unique-per-plug-in-format/19183)
- [Apple Developer Forums: Audio Unit Extension LoadInProcess](https://developer.apple.com/forums/thread/66557)
- [KVR Forum: AUv3 in Xcode Setup](https://www.kvraudio.com/forum/viewtopic.php?t=482362)

## Summary

### Current AUv3 Naming (Implemented)

```
com.beamer.{package}            # Container app
com.beamer.{package}.audiounit  # App extension (the plugin)
com.beamer.{package}.framework  # Framework (DSP code)
```

✓ **Correct and future-proof**

### Future AUv2 Naming (Planned)

```
com.beamer.{package}.component  # AUv2 component
com.beamer.{package}.framework  # Shared framework (optional)
```

✓ **No conflicts, enables dual distribution**

### Key Principles

1. **Distinct bundle IDs** prevent conflicts between formats
2. **Identical AudioComponentDescription** makes them appear as one plugin
3. **Shared framework** reduces duplication and ensures consistency
4. **Clear suffixes** (`.audiounit`, `.component`, `.framework`) improve maintainability
