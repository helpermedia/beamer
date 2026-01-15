# AU Logic Pro Compatibility Investigation

Investigation date: 2026-01-14 (updated 2026-01-15)

## Recent Fix: ObjC Class Name Collisions (Resolved)

**Note:** A separate issue was discovered and fixed on 2026-01-15 where multiple AU plugins loaded in Reaper showed incorrect parameters due to ObjC class name collisions. This was caused by all plugins using identical class names (`BeamerAuWrapper`, `BeamerAuExtension`).

**Solution:** ObjC code is now generated per-plugin by xtask with unique class names (e.g., `BeamerSimpleGainAuWrapper`, `BeamerCompressorAuWrapper`). This fix is unrelated to the Logic Pro XPC timeout issue documented below.

## Problem Statement

All Beamer AU plugins show "not compatible" in Logic Pro 11.2.2, while they work correctly in Reaper and pass `auval` validation.

## Environment

- macOS 26.2 (Build 25C56)
- Logic Pro 11.2.2
- Beamer version 0.1.6

## Plugins Tested

| Plugin | Type | Subtype | Manufacturer | Logic Status | Reaper Status | auval |
|--------|------|---------|--------------|--------------|---------------|-------|
| BeamerSimpleGain | aufx (effect) | siga | Bmer | not compatible | works | PASS |
| BeamerCompressor | aufx (effect) | comp | Bmer | not compatible | works | PASS |
| BeamerSynth | aumu (instrument) | synt | Bmer | not compatible | works | PASS |

## Symptoms

1. All plugins appear in Logic's Plugin Manager but show "not compatible"
2. Plugins are grayed out when inserted on tracks
3. Rescanning plugins in Logic takes very long (~20 seconds per plugin)
4. Logic's AU scan log shows **all Beamer plugins fail with timeout**:
   ```
   BeamerCompressor start
   BeamerCompressor failed with timeout
   BeamerCompressor took 0.000000 seconds

   BeamerSimpleGain start
   BeamerSimpleGain failed with timeout
   BeamerSimpleGain took 0.000000 seconds

   BeamerSynth start
   BeamerSynth failed with timeout
   BeamerSynth took 0.000000 seconds
   ```
5. Apple's FilterDemo loads instantly: `AUV3FilterDemo took 0.000000 seconds` (no timeout)

## Critical Finding: In-Process vs XPC Loading

| Host | Loading Method | Result |
|------|---------------|--------|
| **auval** | In-process (calls `BeamerAuExtensionFactory` directly) | **PASS** |
| **Reaper** | In-process | **Works** |
| **Logic Pro** | Out-of-process XPC (spawns appex, communicates via XPC) | **TIMEOUT** |

This indicates the **XPC communication path is broken**, while in-process loading works fine.

### Additional Evidence
- The appex binary starts and runs correctly when launched manually
- Console.app shows no error messages during Logic AU scan
- The timeout is ~20 seconds per plugin, suggesting Logic gives up waiting for XPC response

---

## Detailed Comparison: Beamer vs Apple FilterDemo

### Important Note on UI Differences

**Beamer plugins are intentionally headless (no UI).** Apple's FilterDemo has a UI. Therefore, certain Info.plist differences related to UI are expected and correct:

- `NSExtensionPointIdentifier`: `com.apple.AudioUnit` (Beamer) vs `com.apple.AudioUnit-UI` (FilterDemo)
- `NSExtensionPrincipalClass`: `BeamerAuExtension` (factory class) vs `FilterDemoViewController` (view controller)
- `NSExtensionServiceRoleType`: Not present in Beamer (UI-related key)

Per Apple documentation, `com.apple.AudioUnit` is the correct extension point for headless AUs.

### Complete Info.plist Comparison (Appex)

#### FilterDemo Appex Info.plist
```xml
CFBundleDevelopmentRegion: en
CFBundleDisplayName: FilterDemoAppExtensionOSX
CFBundleExecutable: FilterDemoAppExtension
CFBundleIconFile: icon
CFBundleIdentifier: com.example.apple-samplecode.FilterDemoAppOSXQ5WYU5N7GL.FilterDemoAppExtensionOSX
CFBundleInfoDictionaryVersion: 6.0
CFBundleName: FilterDemoAppExtension
CFBundlePackageType: XPC!
CFBundleShortVersionString: 1.6
CFBundleSignature: ????
CFBundleSupportedPlatforms: [MacOSX]
CFBundleVersion: 1
LSMinimumSystemVersion: 11.0

NSExtension:
  NSExtensionPointIdentifier: com.apple.AudioUnit-UI          # UI extension (has GUI)
  NSExtensionPrincipalClass: FilterDemoViewController         # ViewController for UI
  NSExtensionAttributes:
    NSExtensionServiceRoleType: NSExtensionServiceRoleTypeEditor  # UI-related
    AudioComponentBundle: com.example.apple-samplecode.FilterDemoFrameworkOSX
    AudioComponents:
      - type: aufx
        subtype: f1tR
        manufacturer: Demo
        name: "Demo: AUV3FilterDemo"
        description: AUV3FilterDemo
        sandboxSafe: true
        tags: [Effects]
        version: 67072
```

#### Beamer Appex Info.plist
```xml
CFBundleDevelopmentRegion: English
CFBundleExecutable: BeamerSimpleGain
CFBundleIdentifier: com.beamer.simple-gain.audiounit
CFBundleInfoDictionaryVersion: 6.0
CFBundleName: BeamerSimpleGain
CFBundlePackageType: XPC!
CFBundleShortVersionString: 0.1.6
CFBundleSignature: ????
CFBundleVersion: 0.1.6
LSMinimumSystemVersion: 10.13

NSExtension:
  NSExtensionPointIdentifier: com.apple.AudioUnit              # Headless extension (no GUI) - CORRECT
  NSExtensionPrincipalClass: BeamerSimpleGainAuExtension       # Plugin-specific factory class - CORRECT
  NSExtensionAttributes:
    AudioComponentBundle: com.beamer.simple-gain.framework
    AudioComponents:
      - type: aufx
        subtype: siga
        manufacturer: Bmer
        name: "Beamer: BeamerSimpleGain"
        description: "BeamerSimpleGain Audio Unit"
        sandboxSafe: true
        tags: [Effects]
        version: 262
```

#### Key Differences Summary

| Key | FilterDemo | Beamer | Issue? |
|-----|-----------|--------|--------|
| `NSExtensionPointIdentifier` | `com.apple.AudioUnit-UI` | `com.apple.AudioUnit` | **Expected** (UI vs headless) |
| `NSExtensionPrincipalClass` | `FilterDemoViewController` | `BeamerSimpleGainAuExtension` | **Expected** (UI vs headless) |
| `NSExtensionServiceRoleType` | `NSExtensionServiceRoleTypeEditor` | *missing* | **Expected** (UI-only key) |
| `CFBundleDisplayName` | Present | *missing* | Minor |
| `CFBundleSupportedPlatforms` | `[MacOSX]` | *missing* | Minor |
| `CFBundleIconFile` | Present | *missing* | Minor |
| `LSMinimumSystemVersion` | `11.0` | `10.13` | Should update to 11.0 |

### Framework Structure Comparison

| Aspect | FilterDemo | Beamer |
|--------|-----------|--------|
| Structure | **Versioned** (`Versions/A/`, symlinks) | **Flat** (no Versions directory) |
| Binary path | `.../Versions/A/FilterDemoFramework` | `.../BeamerSimpleGainAU` |
| Resources | `Versions/A/Resources/Info.plist` | `Info.plist` (root level) |

**FilterDemo Framework Structure:**
```
FilterDemoFramework.framework/
├── FilterDemoFramework -> Versions/Current/FilterDemoFramework
├── Resources -> Versions/Current/Resources
└── Versions/
    ├── A/
    │   ├── FilterDemoFramework (binary)
    │   ├── Resources/
    │   │   └── Info.plist
    │   └── _CodeSignature/
    └── Current -> A
```

**Beamer Framework Structure:**
```
BeamerSimpleGainAU.framework/
├── BeamerSimpleGainAU (binary)
├── Info.plist
└── _CodeSignature/
```

### Library Linking Comparison

| Aspect | FilterDemo | Beamer |
|--------|-----------|--------|
| Framework link | `@rpath/.../Versions/A/FilterDemoFramework` | `@rpath/.../BeamerSimpleGainAU` |
| rpaths | `@executable_path/../Frameworks` **AND** `@executable_path/../../../../Frameworks` | Only `@loader_path/../../../../Frameworks` |

### Code Signing Comparison

| Aspect | FilterDemo | Beamer |
|--------|-----------|--------|
| Flags | `0x10002 (adhoc,runtime)` | `0x2 (adhoc)` |
| Hardened Runtime | **Yes** | **No** |
| Team ID | not set | not set |

### Binary Size Comparison

| Component | FilterDemo | Beamer |
|-----------|-----------|--------|
| Appex binary | 117 KB | 118 KB |
| Framework binary | 338 KB | **1.2 MB** |

The larger Beamer framework includes both AU and VST3 code (same cdylib exports both).

---

## Things We Tried (Did NOT Fix the Issue)

### 1. ❌ Changed NSExtensionPointIdentifier to `com.apple.AudioUnit-UI`

**What we tried:** Changed from `com.apple.AudioUnit` to `com.apple.AudioUnit-UI` to match FilterDemo.

**Result:** Did not fix the Logic Pro issue.

**Conclusion:** The extension point identifier is not the root cause. `com.apple.AudioUnit` is correct for headless AUs per Apple documentation.

### 2. ❌ Embedded Framework Inside Appex (like JUCE)

**What we tried:** Instead of having the framework in `Contents/Frameworks/`, embedded the framework code directly inside the appex (similar to how JUCE structures its AU plugins).

**Result:**
- Reaper stopped working
- auval **hangs** at `TESTING OPEN TIMES: COLD:` - never completes

**Conclusion:** This approach breaks even in-process loading. There may be initialization order issues when the Rust code is embedded directly in the appex.

### 3. ❌ Console.app Logging During Logic Scan

**What we tried:** Monitored Console.app with filters for "Beamer", "AudioUnit", "extension", "error" during Logic's AU rescan.

**Result:** No relevant log messages appear.

**Conclusion:** The failure is silent - no errors are logged to the system console.

### 4. ❌ Manually Launching Appex

**What we tried:** Launched the appex binary directly from terminal to see if it starts correctly.

**Result:** The appex starts and runs fine (NSRunLoop runs until killed).

**Conclusion:** The appex itself initializes correctly. The issue is in XPC communication with Logic, not appex startup.

---

## What Was Verified Working

### auval Validation
All plugins pass complete auval validation including strict mode:
```bash
auval -v aufx siga Bmer        # PASS
auval -v aufx comp Bmer        # PASS
auval -v aumu synt Bmer        # PASS
auval -v aufx siga Bmer -strict # PASS
```

### Plugin Registration
Plugins are properly registered via pluginkit:
```
com.beamer.compressor.audiounit(0.1.6)
com.beamer.simple-gain.audiounit(0.1.6)
com.beamer.synth.audiounit(0.1.6)
```

### NSExtension Configuration (Correct per Apple Docs)
The headless AU configuration is correct (example for simple-gain):
```xml
<key>NSExtensionPointIdentifier</key>
<string>com.apple.AudioUnit</string>              <!-- Correct for headless AU -->
<key>NSExtensionPrincipalClass</key>
<string>BeamerSimpleGainAuExtension</string>      <!-- Plugin-specific factory class -->
```
- ✅ Uses `NSExtensionPrincipalClass` (not `NSExtensionViewController` or `NSExtensionMainStoryboard`)
- ✅ Principal class implements `AUAudioUnitFactory` protocol
- ✅ No storyboard entries (which would cause crashes for headless AUs)
- ✅ Each plugin has unique class names to avoid collisions

### Bundle ID Matching
- Appex `AudioComponentBundle`: `com.beamer.simple-gain.framework`
- Framework `CFBundleIdentifier`: `com.beamer.simple-gain.framework`
- These match correctly ✓

### Symbol Exports
Each plugin's framework exports plugin-specific ObjC symbols (example for simple-gain):
```
_BeamerSimpleGainAuExtensionFactory
_OBJC_CLASS_$_BeamerSimpleGainAuExtension
_OBJC_CLASS_$_BeamerSimpleGainAuWrapper
_OBJC_METACLASS_$_BeamerSimpleGainAuExtension
_OBJC_METACLASS_$_BeamerSimpleGainAuWrapper
```

### Entitlements
Appex has correct sandbox entitlements:
```xml
<key>com.apple.security.app-sandbox</key>
<true/>
<key>com.apple.security.files.user-selected.read-write</key>
<true/>
```

---

## Remaining Differences to Investigate

### High Priority (Potential Causes)

1. **Framework Structure** - Beamer uses flat structure, FilterDemo uses versioned structure with symlinks
2. **Hardened Runtime** - FilterDemo has it enabled, Beamer doesn't
3. **Missing rpath** - FilterDemo has `@executable_path/../Frameworks`, Beamer doesn't
4. **VST3 Code in AU Binary** - Beamer's AU framework contains VST3 entry points (`_GetPluginFactory`), though this shouldn't cause issues

### Lower Priority (Probably Not Causes)

5. **Missing Info.plist keys** - `CFBundleDisplayName`, `CFBundleSupportedPlatforms` (likely cosmetic)
6. **LSMinimumSystemVersion** - 10.13 vs 11.0 (unlikely to cause Logic issues)
7. **Binary size** - 1.2MB vs 338KB (shouldn't matter for loading)

---

## Technical Architecture Notes

### How the AU Loading Works

**In-Process Loading (auval, Reaper):**
1. Host loads framework directly into its process
2. Calls `BeamerAuExtensionFactory(desc)` function
3. Factory calls `beamer_au_ensure_factory_registered()`
4. Returns `BeamerAuWrapper` instance

**Out-of-Process XPC (Logic Pro):**
1. Logic spawns appex process
2. Appex runs `main()` which calls `[[NSRunLoop mainRunLoop] run]`
3. System looks up `NSExtensionPrincipalClass` = `BeamerAuExtension`
4. Logic sends XPC request to create AU instance
5. `BeamerAuExtension.createAudioUnitWithComponentDescription:error:` called
6. Returns `BeamerAuWrapper` instance via XPC

**The XPC path (step 3-6) appears to be failing silently.**

### Rust Static Initialization

The `export_au!` macro creates a static initializer in `__DATA,__mod_init_func`:
```rust
#[used]
#[link_section = "__DATA,__mod_init_func"]
static __BEAMER_AU_INIT: extern "C" fn() = __beamer_au_register;
```

This runs when the binary loads, registering the plugin factory in an `OnceLock`.

When embedded in appex (failed attempt), this static initialization may have caused issues with initialization order or the ObjC runtime.

---

## Files Examined

- `xtask/src/main.rs` - Bundle creation, Info.plist generation, and **ObjC code generation** (generates plugin-specific wrapper/extension classes)
- `crates/beamer-au/objc/BeamerAuBridge.h` - C bridge declarations for Rust FFI
- `crates/beamer-au/src/bridge.rs` - Rust FFI bridge (~1700 lines)
- `crates/beamer-au/src/factory.rs` - Factory registration (83 lines)
- `crates/beamer-au/src/export.rs` - Export macro (112 lines)
- `crates/beamer-au/resources/appex.entitlements` - Sandbox entitlements
- `examples/simple-gain/src/lib.rs` - Example plugin source

**Note:** ObjC source files (`BeamerAuWrapper.m`, `BeamerAuExtension.m`, `appex_main.m`) are now **generated by xtask** per-plugin with unique class names. Generated files are placed in `target/au-gen/<plugin-name>/`.

---

## Recommended Next Step

1. **Compare with a working headless iPlug2-based AUv3** - Compare with a iPlug2-based (headless, meaning no GUI) AUv3 example (production-tested in Logic Pro) to isolate the actual incompatibility.

---

## Related Documentation

- [AU_DEBUG_INFO.md](AU_DEBUG_INFO.md) - Debug procedures
- [AU_CODE_SIGNING.md](AU_CODE_SIGNING.md) - Code signing details
- [AU_ARCHITECTURE_REVIEW.md](AU_ARCHITECTURE_REVIEW.md) - Architecture overview
- [Apple Developer Forums: Audio Extension without UI](https://developer.apple.com/forums/thread/22121)
- [App Extension Programming Guide: Audio Unit](https://developer.apple.com/library/archive/documentation/General/Conceptual/ExtensibilityPG/AudioUnit.html)
