# AUv3 Discovery Issue - Research Status

## Problem Statement

AUv3 plugin is **registered** in LaunchServices but **not discovered** by `AudioComponentFindNext` or `AVAudioUnitComponentManager`. The `auval` tool finds the component by type/subtype/manufacturer but cannot query its properties (error -50 paramErr).

## Current Bundle Structure

```
BeamerSimpleGain.app/
├── Contents/
│   ├── Info.plist                    # Container app plist (com.beamer.simple-gain)
│   ├── MacOS/
│   │   └── BeamerSimpleGain          # Host app executable (beamer-au-host)
│   ├── Frameworks/
│   │   └── BeamerSimpleGainAU.framework/
│   │       ├── BeamerSimpleGainAU    # Plugin dylib with all AU code
│   │       ├── Info.plist            # Framework plist (com.beamer.simple-gain.framework)
│   │       └── _CodeSignature/
│   ├── PlugIns/
│   │   └── BeamerSimpleGain.appex/
│   │       └── Contents/
│   │           ├── Info.plist        # NSExtension + AudioComponents config
│   │           ├── MacOS/
│   │           │   └── BeamerSimpleGain  # Thin executable linking framework
│   │           └── _CodeSignature/
│   ├── Resources/
│   └── PkgInfo
```

## What Works

1. **Bundle structure** - Correct .app with embedded .appex
2. **ObjC classes exported** - `BeamerAuExtension` and `BeamerAuWrapper` are in framework with `S` (global) symbols
3. **Appex binary type** - Now a proper Mach-O executable (not dylib)
4. **Bundle ID hierarchy** - `com.beamer.simple-gain.audiounit` is child of `com.beamer.simple-gain`
5. **Appex can load** - When run directly, the appex executable starts successfully
6. **LaunchServices registration** - Extension is registered with correct NSExtension configuration
7. **Framework rpath** - Fixed to `@executable_path/../../../../Frameworks`

## What Doesn't Work

1. **AudioComponentFindNext** - Returns NULL for our component
2. **AVAudioUnitComponentManager** - Doesn't list our component (only shows 98 AUv2 plugins)
3. **auval validation** - Finds component but can't query properties:
   ```
   VALIDATING AUDIO UNIT: 'aufx' - 'siga' - 'Bmer'
   ERROR: Cannot get Component's Name strings
   ERROR: Error from retrieving Component Version: -50
   FATAL ERROR: didn't find the component
   ```

## Key Configuration

### Appex Info.plist (NSExtension)
```xml
<key>NSExtension</key>
<dict>
    <key>NSExtensionPointIdentifier</key>
    <string>com.apple.AudioUnit</string>
    <key>NSExtensionPrincipalClass</key>
    <string>BeamerAuExtension</string>
    <key>NSExtensionAttributes</key>
    <dict>
        <key>AudioComponents</key>
        <array>
            <dict>
                <key>type</key><string>aufx</string>
                <key>subtype</key><string>siga</string>
                <key>manufacturer</key><string>Bmer</string>
                <key>name</key><string>Beamer: BeamerSimpleGain</string>
                <key>sandboxSafe</key><true/>
                <key>version</key><integer>131072</integer>
                <key>description</key><string>BeamerSimpleGain Audio Unit</string>
                <key>factoryFunction</key><string>BeamerAuExtensionFactory</string>
            </dict>
        </array>
        <key>AudioComponentBundle</key>
        <string>com.beamer.simple-gain.framework</string>
    </dict>
</dict>
```

### Framework Info.plist
```xml
<key>CFBundleIdentifier</key>
<string>com.beamer.simple-gain.framework</string>
<key>CFBundlePackageType</key>
<string>FMWK</string>
```

## Research Sources

1. [Apple: Audio Unit Extension - LoadInProcess](https://developer.apple.com/forums/thread/66557)
   - Key insight: "move the AU factory method from the Extension source file to the framework"
   - AudioComponentBundle must match framework's bundle ID exactly

2. [Apple: Creating an audio unit extension](https://developer.apple.com/documentation/avfaudio/audio_engine/audio_units/creating_an_audio_unit_extension/)
   - Bundle ID hierarchy requirements
   - NSExtension configuration

3. [JUCE Forum: AUv3 discovery issues](https://forum.juce.com/t/help-my-audio-unit-version-3-auv3-is-not-working/45860)
   - auval must pass for Logic Pro to list plugins
   - Name format should be "Manufacturer: ProductName"

4. [iPlug2 AUv3 structure](https://github.com/iPlug2/iPlug2)
   - Uses separate framework for AU code
   - `AudioComponentBundle` points to framework bundle ID

5. [How PlugInKit enables app extensions](https://eclecticlight.co/2025/04/16/how-pluginkit-enables-app-extensions/)
   - PlugInKit derives from LaunchServices database
   - Parent app must run for registration

## Hypotheses to Test

### 1. AUv3 vs AUv2 Discovery Path
The 98 discovered plugins are all AUv2 (`.component` bundles). Maybe on macOS, AUv3 uses a completely different discovery mechanism that we're not triggering.

**Test:** Check if any AUv3 plugins show up on the system, or if all discovered AUs are AUv2.

### 2. Framework Structure Incomplete
Apple's sample projects use a more complete framework structure with `Versions/A/` symlinks:
```
Framework.framework/
├── Versions/
│   └── A/
│       └── Framework
├── Framework -> Versions/A/Framework
└── Info.plist
```

**Test:** Create proper versioned framework structure.

### 3. Missing AudioComponent Registration
For in-process loading, the framework might need to explicitly register the AudioComponent using `AudioComponentRegister()`.

**Test:** Add AudioComponent registration call in framework initialization.

### 4. factoryFunction Signature Wrong
The `factoryFunction` might need a different signature than what we implemented.

**Test:** Check Apple's documentation for exact factory function signature.

### 5. Out-of-Process Only on macOS
Maybe macOS AUv3 only works out-of-process (via XPC) and in-process loading isn't properly supported.

**Test:** Remove `AudioComponentBundle` and `factoryFunction`, rely purely on XPC.

### 6. Entitlements Required
App Store AUv3 plugins require specific entitlements. Maybe even for development we need certain entitlements.

**Test:** Add sandbox entitlements to appex.

## Code Locations

- **xtask bundling:** `xtask/src/main.rs` - `bundle_au()` function
- **ObjC factory class:** `crates/beamer-au/objc/BeamerAuExtension.m`
- **ObjC AU wrapper:** `crates/beamer-au/objc/BeamerAuWrapper.m`
- **Appex entry point:** `crates/beamer-au/objc/appex_main.m`
- **Build script:** `examples/simple-gain/build.rs` (symbol exports)
- **Test programs:** `au_test.c`, `av_test.m` in project root

## Commands for Testing

```bash
# Build and install
cargo xtask bundle simple-gain --au --install

# Test AU discovery (AudioComponent API)
./au_test

# Test AU discovery (AVFoundation API)
./av_test

# Validate with auval
auval -v aufx siga Bmer

# Check LaunchServices registration
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -dump | grep -A30 "com.beamer.simple-gain.audiounit"

# Check pluginkit (doesn't show AudioUnit extensions)
pluginkit -m | grep -i beamer

# Refresh audio cache
killall -9 AudioComponentRegistrar

# Force LaunchServices re-registration
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f ~/Applications/BeamerSimpleGain.app

# Check framework symbols
nm ~/Applications/BeamerSimpleGain.app/Contents/Frameworks/BeamerSimpleGainAU.framework/BeamerSimpleGainAU | grep -E "(OBJC_CLASS|BeamerAu)"

# Test if appex can load
~/Applications/BeamerSimpleGain.app/Contents/PlugIns/BeamerSimpleGain.appex/Contents/MacOS/BeamerSimpleGain &
```

## Commands for Comparing with FilterDemo

```bash
# Find FilterDemo location (built by Xcode)
find ~/Library/Developer/Xcode/DerivedData -name "FilterDemo*.app" 2>/dev/null | head -1

# OR if installed to Applications:
FILTER_APP="<path-to-FilterDemo.app>"
BEAMER_APP=~/Applications/BeamerSimpleGain.app

# 1. Compare code signing
echo "=== FilterDemo signing ===" && codesign -dvv "$FILTER_APP" 2>&1 | head -20
echo "=== Beamer signing ===" && codesign -dvv "$BEAMER_APP" 2>&1 | head -20

echo "=== FilterDemo appex signing ===" && codesign -dvv "$FILTER_APP/Contents/PlugIns/"*.appex 2>&1 | head -20
echo "=== Beamer appex signing ===" && codesign -dvv "$BEAMER_APP/Contents/PlugIns/"*.appex 2>&1 | head -20

# 2. Compare bundle structure
echo "=== FilterDemo structure ===" && ls -laR "$FILTER_APP" | head -50
echo "=== Beamer structure ===" && ls -laR "$BEAMER_APP" | head -50

# 3. Compare Info.plist AudioComponents
echo "=== FilterDemo AudioComponents ===" && /usr/libexec/PlistBuddy -c "Print NSExtension:NSExtensionAttributes:AudioComponents" "$FILTER_APP/Contents/PlugIns/"*.appex/Contents/Info.plist
echo "=== Beamer AudioComponents ===" && /usr/libexec/PlistBuddy -c "Print NSExtension:NSExtensionAttributes:AudioComponents" "$BEAMER_APP/Contents/PlugIns/"*.appex/Contents/Info.plist

# 4. Check if FilterDemo shows in auval
auval -a | grep -i filter

# 5. Test FilterDemo with our test program
./av_test  # Look for FilterDemo in output
```

## Next Steps

### ✅ DONE: Built Apple's FilterDemo - IT WORKS!

### IMMEDIATE: Compare FilterDemo with Our Implementation

**Run the comparison commands above to identify differences:**

1. **Code Signing** - Does FilterDemo have proper Apple ID signature vs our ad-hoc?
   ```bash
   # Check Team ID, entitlements, signature differences
   codesign -dvv <FilterDemo.app> vs codesign -dvv ~/Applications/BeamerSimpleGain.app
   ```

2. **Info.plist** - What keys are different in NSExtension section?
   ```bash
   # Compare AudioComponents, AudioComponentBundle, other keys
   /usr/libexec/PlistBuddy -c "Print NSExtension" <paths>
   ```

3. **Bundle Structure** - Any files we're missing?
   ```bash
   # Look for entitlements, provisioning profiles, etc.
   ls -laR <FilterDemo.app>
   ```

4. **Discovery Test** - Does FilterDemo show in our test programs?
   ```bash
   # If FilterDemo shows in ./av_test but ours doesn't → code signing
   # If neither show → different issue
   ./av_test | grep -i filter
   ```

### If Signing is Confirmed as Issue

Update `xtask/src/main.rs` to support proper code signing:

```rust
// Add environment variable for signing identity
let signing_identity = std::env::var("CODESIGN_IDENTITY")
    .unwrap_or_else(|_| "-".to_string());  // Fallback to ad-hoc

// Sign with identity
Command::new("codesign")
    .args(["--force", "--sign", &signing_identity,
           "--entitlements", entitlements_path,  // May need entitlements
           appex_dir.to_str().unwrap()])
```

Then use:
```bash
# Find your signing identity
security find-identity -v -p codesigning

# Build with proper signing
CODESIGN_IDENTITY="Apple Development: your@email.com (TEAMID)" \
  cargo xtask bundle simple-gain --au --install
```

### Other options
- Build a JUCE AUv3 plugin to verify system capability
- Check Console.app for system errors during AU loading
- Add detailed logging to BeamerAuExtension

## Latest Session Status (2026-01-12)

### ✅ BREAKTHROUGH: Apple's FilterDemo Works!

**FilterDemo is discovered by Logic Pro after running the app once.**
- App IS the plugin host that displays the plugin UI
- After first launch, plugin appears in Logic Pro's plugin list
- **This confirms:** AUv3 discovery WORKS on this system
- **This proves:** The issue is specific to our implementation, not macOS

### ✅ COMPARISON ANALYSIS COMPLETE

Performed systematic comparison between FilterDemo (working) and BeamerSimpleGain (broken):

**1. Code Signing - ✅ NOT THE ISSUE**
- Both are ad-hoc signed (`Signature=adhoc`)
- Neither has Team ID (`TeamIdentifier=not set`)
- Conclusion: Signing is identical

**2. Bundle Structure - ⚠️ MINOR DIFFERENCES**
- FilterDemo uses versioned framework: `Versions/A/` with symlinks
- FilterDemo has `Resources/` folder inside framework with NIB file
- Beamer uses flat framework structure
- FilterDemo appex has 3 sealed resource files vs Beamer's 0
- **Impact:** Likely cosmetic, not the root cause

**3. NSExtension Configuration - 🚨 CRITICAL DIFFERENCES**

| Key | FilterDemo | BeamerSimpleGain | Impact |
|-----|-----------|------------------|--------|
| NSExtensionPointIdentifier | `com.apple.AudioUnit-UI` | `com.apple.AudioUnit` | **CRITICAL** |
| NSExtensionPrincipalClass | `FilterDemoViewController` | `BeamerAuExtension` | **CRITICAL** |
| NSExtensionServiceRoleType | `NSExtensionServiceRoleTypeEditor` | (missing) | **CRITICAL** |
| AudioComponentBundle | `com.example.apple-samplecode.FilterDemoFrameworkOSX` | (missing) | **IMPORTANT** |

**4. Validation Results - 🚨 CONFIRMS THE ISSUE**
- FilterDemo: `auval -v aufx f1tR Demo` → **PASS** ✅
- BeamerSimpleGain: `auval -v aufx siga Bmer` → **FAIL** ❌
  ```
  ERROR: Cannot get Component's Name strings
  ERROR: Error from retrieving Component Version: -50
  FATAL ERROR: didn't find the component
  ```

### 🎯 ROOT CAUSE IDENTIFIED

**FilterDemo is registered as an AudioUnit-UI extension (with user interface), while BeamerSimpleGain is registered as a plain AudioUnit extension (no UI).**

Apple's AUv3 architecture on macOS requires:
1. **Extension point:** `com.apple.AudioUnit-UI` (not `com.apple.AudioUnit`)
2. **Principal class:** Must be a `NSViewController` subclass (for UI hosting)
3. **Service role:** `NSExtensionServiceRoleTypeEditor` (for UI extensions)
4. **AudioComponentBundle:** Framework bundle ID for in-process loading

Without these, the AudioUnit cannot be properly instantiated, hence the -50 (paramErr) when auval tries to query component properties.

### Current Error
```
VALIDATING AUDIO UNIT: 'aufx' - 'siga' - 'Bmer'
ERROR: Cannot get Component's Name strings
ERROR: Error from retrieving Component Version: -50
FATAL ERROR: didn't find the component
```
**Diagnosis:** auval finds the component in LaunchServices (registration works) but cannot instantiate it to query properties because:
- Extension point is `com.apple.AudioUnit` instead of `com.apple.AudioUnit-UI`
- Principal class `BeamerAuExtension` is not a view controller
- Missing `NSExtensionServiceRoleType`
- Missing `AudioComponentBundle` key

### ✅ UPDATE (2026-01-12 23:00): AudioUnit-UI Implementation Complete

**All configuration changes have been implemented:**

1. ✅ Changed NSExtensionPointIdentifier to `com.apple.AudioUnit-UI`
2. ✅ Added NSExtensionServiceRoleType: `NSExtensionServiceRoleTypeEditor`
3. ✅ Added AudioComponentBundle key pointing to framework
4. ✅ Created BeamerAuViewController (NSViewController subclass)
5. ✅ Exported all classes from framework

**Current Configuration (now matches FilterDemo):**

```xml
<key>NSExtension</key>
<dict>
    <key>NSExtensionPointIdentifier</key>
    <string>com.apple.AudioUnit-UI</string>
    <key>NSExtensionPrincipalClass</key>
    <string>BeamerAuViewController</string>
    <key>NSExtensionAttributes</key>
    <dict>
        <key>NSExtensionServiceRoleType</key>
        <string>NSExtensionServiceRoleTypeEditor</string>
        <key>AudioComponentBundle</key>
        <string>com.beamer.simple-gain.framework</string>
        <key>AudioComponents</key>
        <array>
            <!-- component config -->
        </array>
    </dict>
</dict>
```

**LaunchServices Verification:**
- ✅ Extension registered with `com.apple.AudioUnit-UI` extension point
- ✅ AudioComponentBundle present
- ✅ All ObjC classes exported from framework
- ✅ Framework linked with AppKit and Cocoa

**❌ STILL NOT WORKING:**
- `AudioComponentFindNext` returns NULL
- `auval -v aufx siga Bmer` fails with error -50
- Plugin not discovered by AVAudioUnitComponentManager

**Remaining Differences from FilterDemo:**

| Aspect | FilterDemo | BeamerSimpleGain |
|--------|------------|------------------|
| Sandbox entitlements | ✅ Present | ❌ Missing |
| Framework structure | Versioned (Versions/A/) | Flat |
| Framework Resources | Has NIB file | Empty |

**Next Hypotheses to Test:**

1. ✅ **SOLVED: Sandbox Entitlements Required**: FilterDemo has `com.apple.security.app-sandbox = 1` entitlement
2. **Framework Must Be Versioned**: Proper `Versions/A/` structure may be required
3. **Host App Must Actually Display Plugin UI**: Maybe the host needs to instantiate the ViewController?
4. **System Audio Component Cache**: May need to force rebuild of audio component database
5. **macOS Security Policy**: macOS 15+ may have additional requirements

### 🎉 BREAKTHROUGH (2026-01-12 23:06): IT WORKS!

**Adding sandbox entitlements fixed the discovery issue!**

**Test Results:**
- ✅ `auval -v aufx siga Bmer` → **PASS**
- ✅ `AudioComponentFindNext` → **Found component**
- ✅ `AVAudioUnitComponentManager` → **Component discovered**
- ✅ Plugin loads in-process (`out-of-process: false`)
- ✅ Cold open time: 27.964 ms
- ✅ Warm open time: 0.497 ms

**The Missing Piece: Sandbox Entitlements**

Created `resources/appex.entitlements`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
    <key>com.apple.security.files.user-selected.read-write</key>
    <true/>
</dict>
</plist>
```

Updated xtask to sign appex with entitlements:
```rust
let entitlements_path = workspace_root.join("resources/appex.entitlements");
let appex_sign_status = Command::new("codesign")
    .args([
        "--force",
        "--sign", "-",
        "--entitlements", entitlements_path.to_str().unwrap(),
        appex_dir.to_str().unwrap()
    ])
    .status();
```

**Why Sandbox Entitlements Are Required:**

macOS requires AUv3 app extensions to be sandboxed. Without the `com.apple.security.app-sandbox` entitlement:
- LaunchServices registers the extension
- Extension appears in the database
- BUT the Audio Component system refuses to instantiate it
- Result: Error -50 (paramErr) when trying to query component

With sandbox entitlements:
- Extension is properly sandboxed
- Audio Component system trusts and loads it
- Plugin becomes fully discoverable and usable

**Summary of All Required Changes for AUv3 Discovery:**

~~1. ✅ NSExtensionPointIdentifier: `com.apple.AudioUnit-UI`~~
~~2. ✅ NSExtensionPrincipalClass: `BeamerAuViewController` (NSViewController)~~
~~3. ✅ NSExtensionServiceRoleType: `NSExtensionServiceRoleTypeEditor`~~
1. ✅ AudioComponentBundle: Framework bundle ID
2. ✅ **Sandbox entitlements** with `com.apple.security.app-sandbox`

### 🎯 FINAL SOLUTION (2026-01-12 23:17): Minimal Requirements Confirmed

**Tested and confirmed: AudioUnit-UI and ViewController are NOT required!**

**Working configuration with headless (non-UI) extension:**
- ✅ Extension point: `com.apple.AudioUnit` (NOT AudioUnit-UI)
- ✅ Principal class: `BeamerAuExtension` (NSObject-based, NOT NSViewController)
- ✅ No NSExtensionServiceRoleType needed
- ✅ AudioComponentBundle: `com.beamer.simple-gain.framework`
- ✅ Sandbox entitlements with `com.apple.security.app-sandbox`

**Test results with minimal configuration:**
- ✅ `auval -v aufx siga Bmer` → **PASS**
- ✅ `AVAudioUnitComponentManager` → **Component discovered**
- ✅ Plugin loads in-process
- ✅ Cold open: 20.284 ms, Warm open: 0.623 ms

**The actual minimal requirements for macOS AUv3 discovery are:**

1. **AudioComponentBundle key** pointing to framework bundle ID
2. **Sandbox entitlements** with `com.apple.security.app-sandbox = true`

That's it! The AudioUnit-UI extension point and ViewController were red herrings from comparing with FilterDemo. A headless AUv3 plugin works perfectly on macOS with just these two requirements.

### Implementation Files

**Key files for AUv3 support:**
- [crates/beamer-au/objc/BeamerAuExtension.m](crates/beamer-au/objc/BeamerAuExtension.m) - Extension principal class
- [crates/beamer-au/objc/BeamerAuWrapper.m](crates/beamer-au/objc/BeamerAuWrapper.m) - AUAudioUnit wrapper
- [xtask/src/main.rs](xtask/src/main.rs) - Bundle creation and Info.plist generation
- [resources/appex.entitlements](resources/appex.entitlements) - Sandbox entitlements

**Custom UI support:**
When you want to add custom UI later, you can create an AudioUnit-UI extension by:
1. Creating an NSViewController subclass
2. Changing NSExtensionPointIdentifier to `com.apple.AudioUnit-UI`
3. Adding NSExtensionServiceRoleType: `NSExtensionServiceRoleTypeEditor`
4. Linking against Cocoa framework

For now, the headless configuration allows DAWs to use their generic parameter UI.

### ✅ UPDATE (2026-01-12 23:40): Universal Binary + Version Fix

**Added universal binary support for maximum compatibility:**

All AU plugin components now build as universal binaries (x86_64 + arm64):
- Framework: Universal ✅
- Appex executable: Universal ✅
- Host app: Universal ✅

**Changes made to [xtask/src/main.rs](xtask/src/main.rs):**

1. **Universal binary compilation** - Added `build_universal()` function:
   - Builds plugin for both `x86_64-apple-darwin` and `aarch64-apple-darwin`
   - Combines binaries with `lipo -create`
   - Applied to framework dylib, appex executable, and host app

2. **Appex universal build**:
   ```rust
   // Build for both architectures with clang -arch x86_64 and -arch arm64
   // Combine with lipo into single universal binary
   ```

3. **Host app universal build**:
   ```rust
   // cargo build for both targets
   // Combine with lipo into universal binary
   ```

**Version number fix:**

Added `get_version_info()` function to read version from workspace Cargo.toml:
- Parses `[workspace.package] version = "0.1.6"`
- Converts to Apple format: `(major << 16) | (minor << 8) | patch`
- 0.1.6 → 262 (0x106)
- Updated all Info.plist files to use actual version instead of hardcoded values

**Clearing AU caches for Logic Pro:**

When Logic shows "not compatible" or doesn't recognize updated plugins, clear caches:

```bash
# Kill audio component services
killall -9 AudioComponentRegistrar
killall -9 coreaudiod

# Remove AU cache files
rm -rf ~/Library/Caches/AudioUnitCache
rm -rf ~/Library/Caches/com.apple.audiounits.*

# Re-register the plugin
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f ~/Applications/BeamerSimpleGain.app

# IMPORTANT: Completely quit and restart Logic Pro (Cmd+Q, not just close window)
```

**Current Status:**

- ✅ auval validation: **PASS** with version 0.1.6 (0x106)
- ✅ Universal binary: x86_64 + arm64
- ✅ Sandbox entitlements: Applied
- ✅ AudioComponentBundle: Present
- ✅ Version: Correct (0.1.6)
- ❌ Logic Pro compatibility: Still shows "not compatible"

**Possible causes for Logic Pro incompatibility:**

1. **Logic Pro's stricter validation** - Logic may have additional requirements beyond auval
2. **Cache not fully cleared** - Logic may cache plugin validation results elsewhere
3. **Missing metadata** - Logic may require additional Info.plist keys
4. **Plugin format preference** - Logic may prefer specific bundle structures

**Next debugging steps:**

1. Check Console.app for errors when Logic scans plugins:
   ```bash
   log stream --predicate 'process == "Logic Pro"' --level error
   ```

2. Compare with working FilterDemo in Logic's plugin list
