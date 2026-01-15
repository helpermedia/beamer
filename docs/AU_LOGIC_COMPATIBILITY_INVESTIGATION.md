# AU Logic Pro Compatibility Investigation

Investigation date: 2026-01-14

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
3. Rescanning plugins in Logic takes very long (vs instant for Apple's FilterDemo)
4. Logic's AU scan log shows:
   - Effects: "took 0.000000 seconds" (suspiciously fast)
   - BeamerSynth: "failed with timeout" after ~23 seconds

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

### Bundle Structure
AUv3 app extension structure is correct:
```
BeamerSimpleGain.app/
├── Contents/
│   ├── Info.plist
│   ├── MacOS/BeamerSimpleGain (universal binary stub)
│   ├── Frameworks/
│   │   └── BeamerSimpleGainAU.framework/
│   │       ├── BeamerSimpleGainAU (universal dylib)
│   │       └── Info.plist
│   ├── PlugIns/
│   │   └── BeamerSimpleGain.appex/
│   │       └── Contents/
│   │           ├── Info.plist (with AudioComponents)
│   │           └── MacOS/BeamerSimpleGain (universal)
│   └── Resources/
```

### Bundle ID Matching
- Appex `AudioComponentBundle`: `com.beamer.simple-gain.framework`
- Framework `CFBundleIdentifier`: `com.beamer.simple-gain.framework`
- These match correctly.

### Symbol Exports
Framework exports all required ObjC symbols:
```
_BeamerAuExtensionFactory
_OBJC_CLASS_$_BeamerAuExtension
_OBJC_CLASS_$_BeamerAuWrapper
_OBJC_METACLASS_$_BeamerAuExtension
_OBJC_METACLASS_$_BeamerAuWrapper
```

### Library Linking
Appex correctly links to framework via rpath:
- Link: `@rpath/BeamerSimpleGainAU.framework/BeamerSimpleGainAU`
- Rpath: `@loader_path/../../../../Frameworks`
- Resolves to: `BeamerSimpleGain.app/Contents/Frameworks/`

### Entitlements
Appex has correct sandbox entitlements:
```xml
<key>com.apple.security.app-sandbox</key>
<true/>
<key>com.apple.security.files.user-selected.read-write</key>
<true/>
```

### Code Signing
All components are ad-hoc signed:
- Container app: signed, adhoc, no Team ID
- Framework: signed, adhoc
- Appex: signed with entitlements, adhoc

## Issues Found

### 1. Hardcoded Tags in Info.plist (Confirmed Bug)

**Location**: `xtask/src/main.rs:1027-1030`

```xml
<key>tags</key>
<array>
    <string>Effects</string>  <!-- Hardcoded for ALL plugins -->
</array>
```

**Problem**: All plugins get "Effects" tag regardless of component type:
- `aufx` (effect) → should be "Effects" ✓
- `aumu` (instrument) → should be "Synth" or "Instrument" ✗
- `aumi` (MIDI processor) → should be "MIDI" ✗

**Impact**: Logic uses tags for categorization. Mismatched tags may cause validation issues.

### 2. Slow Initialization Time

auval reports:
```
Time to open AudioUnit: 21.266 ms (cold)
Time to open AudioUnit: 0.298 ms (warm)
```

Apple's built-in AUs typically open in <5ms. This slower initialization might trigger Logic's timeout during batch validation.

### 3. VST3 Version Hardcoded (Unrelated but Found)

**Location**: `xtask/src/main.rs:924-927`

VST3 Info.plist has hardcoded version `0.2.0` instead of using `get_version_info()` like AU does.

## Tests Performed

### 1. auval Tests
```bash
# Basic validation
auval -v aufx siga Bmer  # PASS
auval -v aumu synt Bmer  # PASS

# Strict mode
auval -v aufx siga Bmer -strict  # PASS

# All tests pass including:
# - Format tests (various sample rates, channel configs)
# - Render tests (different buffer sizes)
# - Parameter tests
# - State persistence
# - MIDI handling
```

### 2. Bundle Structure Verification
```bash
# Check appex Info.plist
plutil -p ~/Applications/BeamerSimpleGain.app/Contents/PlugIns/BeamerSimpleGain.appex/Contents/Info.plist

# Check framework Info.plist
plutil -p ~/Applications/BeamerSimpleGain.app/Contents/Frameworks/BeamerSimpleGainAU.framework/Info.plist

# Verify AudioComponentBundle matches framework CFBundleIdentifier
# Result: Both are "com.beamer.simple-gain.framework" ✓
```

### 3. Symbol Export Verification
```bash
# Check framework exports
nm -gU ~/Applications/BeamerSimpleGain.app/Contents/Frameworks/BeamerSimpleGainAU.framework/BeamerSimpleGainAU | grep -E "BeamerAu|OBJC_CLASS"

# Result: All required symbols exported ✓
```

### 4. Library Dependency Verification
```bash
# Check appex dependencies
otool -L ~/Applications/BeamerSimpleGain.app/Contents/PlugIns/BeamerSimpleGain.appex/Contents/MacOS/BeamerSimpleGain

# Check rpath
otool -l ... | grep -A2 LC_RPATH

# Result: Framework correctly linked via @rpath ✓
```

### 5. Code Signing Verification
```bash
# Check app signing
codesign -dvvv ~/Applications/BeamerSimpleGain.app

# Check appex signing and entitlements
codesign -dvvv ~/Applications/BeamerSimpleGain.app/Contents/PlugIns/BeamerSimpleGain.appex
codesign -d --entitlements - ~/Applications/BeamerSimpleGain.app/Contents/PlugIns/BeamerSimpleGain.appex

# Result: Ad-hoc signed with correct entitlements ✓
```

### 6. Logic AU Scan Log Analysis
```bash
plutil -p ~/Library/Caches/AudioUnitCache/Logs/AUScan*.plist | grep -i beamer

# Results:
# BeamerCompressor: "took 0.000000 seconds"
# BeamerSimpleGain: "took 0.000000 seconds"
# BeamerSynth: "failed with timeout"
```

### 7. Plugin Registration Check
```bash
pluginkit -m -v -p com.apple.AudioUnit | grep -i beamer

# Result: All three plugins registered correctly ✓
```

## Comparison: Beamer vs Apple FilterDemo

| Aspect | Beamer | Apple FilterDemo |
|--------|--------|------------------|
| Logic rescan time | Very slow | Instant |
| auval pass | Yes | Yes |
| In-process loading | Yes | Yes |
| Init time | ~21ms | <5ms |

## Potential Causes (Theories)

1. **Tags mismatch** - Wrong categorization tags in Info.plist
2. **Initialization timeout** - Logic may have stricter timeout than auval
3. **Logic-specific validation** - Logic performs checks beyond auval
4. **Code signing requirements** - Logic may require proper Team ID signing (not ad-hoc)
5. **Sandbox restrictions** - Something in AU init blocked in Logic's sandbox

## Fixes Applied

### ✅ Fixed: Tags in Info.plist

**Status**: Fixed in commit (pending)

**Location**: `xtask/src/main.rs:973-983`

Added `get_au_tags()` function that maps component types to correct tags:
```rust
fn get_au_tags(component_type: &str) -> &'static str {
    match component_type {
        "aufx" => "Effects",           // Audio effect
        "aumu" => "Synth",             // Music device/instrument
        "aumi" => "MIDI",              // MIDI processor
        "aumf" => "Effects",           // Music effect
        _ => "Effects",                // Default fallback
    }
}
```

**Verification**:
- BeamerSimpleGain (aufx): tags = "Effects" ✓
- BeamerSynth (aumu): tags = "Synth" ✓
- Both plugins still pass auval ✓

## Recommended Next Steps

### High Priority

1. **Test in Logic Pro** - Verify if corrected tags resolve "not compatible" issue
2. **Profile initialization** - Find what's causing 21ms cold start

### Medium Priority

3. **Test with proper code signing** - Sign with Developer ID to rule out signing issues
4. **Test in GarageBand** - Determine if Logic-specific or all Apple hosts
5. **Compare with working AUv3** - Find a working third-party AUv3 and compare plists/structure

### Low Priority

6. **Fix VST3 version hardcoding** - Use `get_version_info()` for consistency

## Files Examined

- `xtask/src/main.rs` - Bundle creation and Info.plist generation
- `crates/beamer-au/objc/BeamerAuWrapper.m` - ObjC AU wrapper
- `crates/beamer-au/objc/BeamerAuExtension.m` - AUv3 extension principal class
- `crates/beamer-au/objc/appex_main.m` - Appex entry point
- `crates/beamer-au/resources/appex.entitlements` - Sandbox entitlements
- `examples/simple-gain/src/lib.rs` - Example plugin source
- `examples/synth/src/lib.rs` - Synth plugin source

## Next Steps for Further Investigation

1. Test plugins on a different Mac to rule out environment-specific issues
2. Test in GarageBand to determine if Logic-specific
3. Open Console.app and filter by process during Logic rescan to capture real-time errors
4. Compare Info.plist structure with a known-working third-party AUv3 plugin
5. Try building with proper Apple Developer signing (not ad-hoc)
6. Profile AU initialization to find performance bottleneck

## Related Documentation

- [AU_DEBUG_INFO.md](AU_DEBUG_INFO.md) - Debug procedures
- [AU_ARCHITECTURE_REVIEW.md](AU_ARCHITECTURE_REVIEW.md) - Architecture overview
