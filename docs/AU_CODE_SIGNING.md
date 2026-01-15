# AU Code Signing in Beamer

Investigation date: 2026-01-14

## Overview

Code signing for the Beamer AUv3 plugins is implemented entirely through Rust tooling and direct command-line execution. There is **no XCode involvement** and **no xcodebuild** usage. All code signing is performed by the `cargo xtask bundle` command.

## Code Signing Implementation

### Where It Happens

**File**: [xtask/src/main.rs](../xtask/src/main.rs) lines 613-661

Code signing occurs **after** the AU bundle structure is fully created and compiled. It uses direct `codesign` command-line tool invocations, not xcodebuild.

### The Three Signing Steps

The code signs three nested components in dependency order:

#### 1. Framework Signing (lines 613-621)

```rust
println!("Code signing framework...");
let framework_sign_status = Command::new("codesign")
    .args(["--force", "--sign", "-", framework_dir.to_str().unwrap()])
    .status();
```

- **Target**: The dynamic framework containing the plugin dylib
- **Location**: `BeamerGain.app/Contents/Frameworks/BeamerGainAU.framework/`
- **Flags**: `--force --sign -`
- **Entitlements**: None

#### 2. App Extension (Appex) Signing (lines 623-633)

```rust
println!("Code signing appex...");
let entitlements_path = workspace_root.join("crates/beamer-au/resources/appex.entitlements");
let appex_sign_status = Command::new("codesign")
    .args([
        "--force",
        "--sign", "-",
        "--entitlements", entitlements_path.to_str().unwrap(),
        appex_dir.to_str().unwrap()
    ])
    .status();
```

- **Target**: The AUv3 app extension bundle
- **Location**: `BeamerGain.app/Contents/PlugIns/BeamerGain.appex/`
- **Flags**: `--force --sign -`
- **Entitlements**: Applied via `--entitlements` flag
- **Entitlements File**: [crates/beamer-au/resources/appex.entitlements](../crates/beamer-au/resources/appex.entitlements)

#### 3. Container App Signing (lines 635-641)

```rust
println!("Code signing container app...");
let app_sign_status = Command::new("codesign")
    .args(["--force", "--sign", "-", bundle_dir.to_str().unwrap()])
    .status();
```

- **Target**: The container application bundle
- **Location**: `BeamerGain.app/`
- **Flags**: `--force --sign -`
- **Entitlements**: None
- **Note**: This is the top-level bundle that contains both the framework and appex

### Signing Identity

**Ad-hoc signing** is used exclusively: The `-` argument to the `--sign` flag means:
- No Apple Developer Team ID
- No paid Apple Developer account required
- Valid for local testing and distribution
- May not be accepted by some commercial tools (e.g., Logic Pro may require Developer ID signing)

### Error Handling

Code signing failures are **non-fatal**:

```rust
match framework_sign_status {
    Ok(status) if status.success() => println!("Framework code signing successful"),
    Ok(_) => println!("Warning: Framework code signing failed"),
    Err(e) => println!("Warning: Could not run codesign on framework: {}", e),
}
```

The build continues even if signing fails, only printing warnings. This allows the bundle to be created even on systems without proper codesign configuration.

## Complete Build & Signing Flow

### Phase 1: Dependency Compilation (lines 155-170)

```rust
fn ensure_beamer_au_built(workspace_root: &Path, target: &str, release: bool) -> Result<(), String>
```

- Compiles `beamer-au` crate to static libraries
- Outputs: `libbeamer_au_objc.a` and `libbeamer_au_extension.a`
- Located in: `target/{x86_64,aarch64}-apple-darwin/{debug,release}/build/beamer-au-*/out/`

### Phase 2: Universal Binary Building (lines 173-248)

```rust
fn build_universal(package: &str, release: bool, workspace_root: &Path) -> Result<PathBuf, String>
```

- Builds plugin for x86_64 and arm64 architectures separately
- Links against the static libraries from Phase 1
- Combines with `lipo` tool into universal dylib
- Output: `target/{debug,release}/lib{package}.dylib`

### Phase 3: Bundle Creation (lines 489-599 in `bundle_au`)

Creates the AUv3 bundle structure:

```
BeamerGain.app/                               # Container app
├── Contents/
│   ├── Info.plist                           # App plist (LSBackgroundOnly=true)
│   ├── MacOS/
│   │   └── BeamerGain                       # Host app stub executable
│   ├── Frameworks/
│   │   └── BeamerGainAU.framework/          # Framework bundle
│   │       ├── BeamerGainAU                 # Plugin dylib (universal)
│   │       └── Info.plist                   # Framework plist
│   ├── PlugIns/
│   │   └── BeamerGain.appex/                # App extension bundle
│   │       └── Contents/
│   │           ├── Info.plist               # Appex plist with NSExtension
│   │           ├── MacOS/
│   │           │   └── BeamerGain           # Appex executable (universal)
│   │           └── Resources/
│   ├── Resources/
│   └── PkgInfo                              # "APPL????"
```

#### Framework Creation (lines 520-545)

- Copies dylib to framework directory
- Modifies dylib install name using `install_name_tool`:
  ```bash
  install_name_tool -id @rpath/BeamerGainAU.framework/BeamerGainAU [dylib]
  ```
- Creates framework `Info.plist`
- No code signing at this stage

#### Appex Compilation (lines 548-586)

Compiles the thin wrapper executable that links the framework:

```bash
clang -arch x86_64 \
    -fobjc-arc \
    -framework Foundation \
    -framework AudioToolbox \
    -framework AVFoundation \
    -framework CoreAudio \
    -F [frameworks_dir] \
    -framework BeamerGainAU \
    -Wl,-rpath,@loader_path/../../../../Frameworks \
    -o [appex_binary] \
    appex_main.m
```

- Builds for both x86_64 and arm64
- Sets rpath to find framework at runtime: `@loader_path/../../../../Frameworks`
- Combines with `lipo` into universal binary
- No code signing at this stage

#### Container App Compilation (lines 590-629)

Compiles minimal stub that triggers pluginkit registration:

```bash
clang -arch x86_64 \
    -framework Foundation \
    -o [app_binary] \
    stub_main.c
```

- Builds for both x86_64 and arm64
- Marked `LSBackgroundOnly` in plist (exits immediately)
- Combines with `lipo` into universal binary
- No code signing at this stage

#### Info.plist Generation

Two plists are generated from Rust string templates:

**Appex Info.plist** (via `create_appex_info_plist()` lines 814-882):
- `NSExtension` with principal class `BeamerAuExtension`
- `AudioComponents` array with:
  - Component type (aufx, aumu, aumi, aumf)
  - Subtype fourcc code
  - Manufacturer fourcc code
  - Sandbox safety flag
  - Version number
  - **Tags** (currently hardcoded as "Effects" for all plugins)
- `AudioComponentBundle` pointing to framework bundle ID

**Container App Info.plist** (via `create_app_info_plist()` lines 785-812):
- `LSBackgroundOnly` set to true
- Basic app metadata
- Version from workspace Cargo.toml

### Phase 4: Code Signing (lines 613-661)

Three-step signing process (detailed above):
1. Framework with ad-hoc signature
2. Appex with ad-hoc signature + entitlements
3. Container app with ad-hoc signature

### Phase 5: Installation (lines 676-704 in `install_au`)

If `--install` flag is provided:

```rust
fn install_au(bundle_dir: &Path, bundle_name: &str) -> Result<(), String>
```

- Copies signed bundle to `~/Applications/`
- Launches app via `open` command to trigger pluginkit registration
- Kills app process and AudioComponentRegistrar cache service
- System registers the AU extension

## Entitlements

**File**: [crates/beamer-au/resources/appex.entitlements](../crates/beamer-au/resources/appex.entitlements)

Only applied to the appex during signing. Contains sandbox declarations necessary for:
- Audio unit execution in app sandbox
- User file access for preferences/state
- Audio framework access

## Build Script

**File**: [crates/beamer-au/build.rs](../crates/beamer-au/build.rs)

Runs during `cargo build` for beamer-au crate:

```rust
cc::Build::new()
    .file("objc/BeamerAuWrapper.m")
    .flag("-fobjc-arc")
    .flag("-fmodules")
    .compile("beamer_au_objc");

cc::Build::new()
    .file("objc/BeamerAuExtension.m")
    .flag("-fobjc-arc")
    .flag("-fmodules")
    .compile("beamer_au_extension");
```

- Uses `cc` crate to compile ObjC code
- Outputs static libraries: `libbeamer_au_objc.a` and `libbeamer_au_extension.a`
- Links frameworks: AudioToolbox, AVFoundation, Foundation, CoreAudio
- No signing involved in build script

## What Is NOT Used

### XCode

- No Xcode project files (.xcodeproj)
- No Xcode build phases
- No Xcode code signing configuration
- Not required for building or signing

### xcodebuild

- Not used anywhere in the build pipeline
- Everything is done with Rust tools and system utilities

### Provisioning Profiles

- Not needed for ad-hoc signing
- Would only be needed for App Store or enterprise signing

### Team ID or Developer ID

- Ad-hoc signing requires neither
- Can sign on any Mac without Developer Program membership
- However, Logic Pro may reject ad-hoc signed plugins (potential compatibility issue)

### codesign --deep

- Not used explicitly
- Individual components are signed separately instead

## Known Issues Related to Signing

### 1. Code Signing Is Probably NOT the Issue

**Evidence**: Apple's FilterDemo AUv3 example uses identical ad-hoc signing (no Team ID) and works in Logic Pro. Beamer's ad-hoc signed AU also uses the same signing method. Therefore, **code signing identity is unlikely to be the cause of Logic Pro compatibility issues.**

The original [AU_LOGIC_COMPATIBILITY_INVESTIGATION.md](AU_LOGIC_COMPATIBILITY_INVESTIGATION.md) suggested:
> **Code signing requirements** - Logic may require proper Team ID signing (not ad-hoc)

This has been **invalidated by testing** — both FilterDemo and Beamer use ad-hoc signing successfully.

If Logic Pro rejects Beamer AU, the issue is **not code signing**, but rather:
- AudioComponents metadata (component type, subtype, manufacturer, tags)
- Info.plist structure or values
- AU registration/discovery
- Framework binary compatibility
- Semantic entitlements differences

### 2. Tags Hardcoded as "Effects"

**File**: [xtask/src/main.rs](../xtask/src/main.rs) line 871

All plugins get the tag "Effects" regardless of component type:
- `aufx` (effect) → should be "Effects" ✓
- `aumu` (instrument) → should be "Synth" or "Instrument" ✗
- `aumi` (MIDI processor) → should be "MIDI" ✗

Logic uses tags for categorization and validation. This mismatch may contribute to compatibility issues.

## Version Handling

Version is read from workspace `Cargo.toml` and converted to Apple's integer format:

```rust
fn get_version_info(workspace_root: &Path) -> Result<(String, u32), String>
```

- Parses major.minor.patch format
- Converts to integer: `(major << 16) | (minor << 8) | patch`
- Used in both appex and app Info.plist files

Note: VST3 Info.plist has hardcoded version "0.2.0" instead of using this function (inconsistency).

## Command-Line Usage

```bash
# Build and sign, no installation
cargo xtask bundle gain --au --release

# Build, sign, and install to ~/Applications/
cargo xtask bundle gain --au --release --install

# Force rebuild by clearing caches, then build and install
cargo xtask bundle gain --au --release --clean --install

# Build VST3 instead (default format)
cargo xtask bundle gain --vst3 --release --install
```

## Related Documentation

- [AU_LOGIC_COMPATIBILITY_INVESTIGATION.md](AU_LOGIC_COMPATIBILITY_INVESTIGATION.md) - Logic Pro compatibility issues (may be related to ad-hoc signing)
- [AU_DEBUG_INFO.md](AU_DEBUG_INFO.md) - Debug procedures
- [AU_ARCHITECTURE_REVIEW.md](AU_ARCHITECTURE_REVIEW.md) - Architecture overview

## Appendix: Entitlements Comparison with Apple FilterDemo

### Investigation Context

To determine if code signing issues are contributing to Logic Pro compatibility problems, we compared the entitlements used by Apple's official FilterDemo AUv3 example (which works in Logic Pro) with Beamer's AU implementation.

### Apple FilterDemo Entitlements

**Location**: `~/Applications/FilterDemo.app/Contents/PlugIns/FilterDemo AppExtension.appex/`

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
    <key>com.apple.security.get-task-allow</key>
    <true/>
</dict>
</plist>
```

**Interpretation**:
- `com.apple.security.app-sandbox` — Required for app extensions
- `com.apple.security.get-task-allow` — Allows LLDB debugger attachment (debug/development feature)

### Beamer Compressor Entitlements (Current)

**File**: [crates/beamer-au/resources/appex.entitlements](../crates/beamer-au/resources/appex.entitlements)

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.app-sandbox</key>
    <true/>
    <key>com.apple.security.get-task-allow</key>
    <true/>
</dict>
</plist>
```

**Status**: ✓ Identical to FilterDemo

### Entitlements History

#### Previous Configuration (Before Investigation)

Beamer originally had:
```xml
<key>com.apple.security.app-sandbox</key>
<true/>
<key>com.apple.security.files.user-selected.read-write</key>
<true/>
```

**Rationale**: This entitlement was likely added to support debug file logging — writing diagnostic output to user-selected files.

#### Investigation Findings

During testing:
1. **Removed `files.user-selected.read-write`** — Reduced to only `app-sandbox`
2. **Added `get-task-allow`** — To match FilterDemo exactly
3. **Result**: No impact on Logic Pro compatibility

**Conclusion**: The entitlements difference (file access vs debug mode) is **not** the root cause of Logic Pro issues, since:
- FilterDemo uses `get-task-allow` (more permissive)
- Beamer used `files.user-selected.read-write` (more restrictive)
- FilterDemo works in Logic Pro; Beamer doesn't
- Even matching entitlements exactly didn't solve the problem

### Code Signing Verification

Both use **ad-hoc signing** (no Team ID):

| Component | FilterDemo | BeamerCompressor |
|-----------|-----------|------------------|
| **Signature Type** | adhoc | adhoc |
| **TeamIdentifier** | not set | not set |
| **Universal Binary** | arm64 only | x86_64 + arm64 |

### Next Steps

Since entitlements have been verified as identical to a working reference implementation, other factors likely cause Logic Pro incompatibility:

1. **AudioComponents metadata** — Component type, tags, sandboxSafe flag
2. **Info.plist structure** — Appex registration details
3. **AU discovery/registration** — Pluginkit integration
4. **Framework binary linking** — How the AU is loaded at runtime

Recommended: Compare with a iPlug2-based (headless, meaning no GUI) AUv3 example (production-tested in Logic Pro) to isolate the actual incompatibility.
