# Audio Unit - AUv3 - Debug Info

## Build & Test

```bash
# Clean build, install to ~/Applications, register AU
cargo xtask bundle gain --release --au --clean --install
```

This single command does everything:
1. Cleans cc cache and previous build artifacts
2. Builds universal binary (x86_64 + arm64)
3. Creates and signs the .app bundle
4. Installs to `~/Applications/BeamerGain.app`
5. **Automatically launches the app** (required to register AU with macOS)
6. Clears AU cache

No manual steps needed - the `--install` flag handles app execution.

## Validate with auval

```bash
# List all Beamer AUs
auval -a | grep -i beamer

# Validate the plugin
auval -v aufx gain Bmer
```

Component codes: `aufx` (effect), `Bmer` (manufacturer), `gain` (gain)

## Quick Reference

| Action | Command |
|--------|---------|
| Full rebuild & install | `cargo xtask bundle gain --release --au --clean --install` |
| Validate AU | `auval -v aufx gain Bmer` |
| List registered AUs | `auval -a \| grep -i beamer` |

> **Note:** `pluginkit -m` shows App Extension registrations, not AudioComponent registrations. Use `auval` to verify AU plugins - if `auval` passes, the plugin is working correctly.

## Build Process Explanation

The AU build has multiple caching layers:

1. **build.rs** uses `cc::Build` to compile ObjC → cached in `target/release/build/beamer-au-*/`
2. **Rust compilation** → cached in `target/release/deps/libbeamer_au*`
3. **xtask bundle** creates .app with .appex

The `--clean` flag removes all these caches to force a full rebuild.
The `--install` flag copies to `~/Applications` and launches to register.

## Key Files
- `crates/beamer-au/objc/BeamerAuWrapper.m` - ObjC AU wrapper, buildBusConfig
- `crates/beamer-au/objc/BeamerAuBridge.h` - C struct definitions (BEAMER_AU_MAX_BUSES=16)
- `crates/beamer-au/src/bridge.rs` - Rust FFI, bus_config_from_c
- `xtask/src/main.rs` - Build tooling (--clean, --install flags)

---

## Debug Logging

Production code has no debug logging. When debugging AU issues, you may need to add temporary logging.

### Why NSLog and os_log Don't Work

AUv3 plugins run in an **App Extension** (`.appex`) which is sandboxed. Both `NSLog` and `os_log` fail because:

1. Sandboxed extensions have restricted console access
2. Output doesn't appear in Console.app or `log stream`
3. Host DAWs don't forward extension logs

### Recommended: File-Based Logging

Write to `/tmp/` which is accessible even from sandboxed extensions:

**Objective-C (BeamerAuWrapper.m):**
```objc
static void debug_log(const char *format, ...) {
    va_list args;
    va_start(args, format);

    FILE *f = fopen("/tmp/beamer_au_debug.log", "a");
    if (f) {
        vfprintf(f, format, args);
        fprintf(f, "\n");
        fclose(f);
    }

    va_end(args);
}

// Usage:
debug_log("[ObjC] allocateRenderResources: sr=%.0f maxFrames=%u",
          self.outputBusses[0].format.sampleRate,
          (unsigned)self.maximumFramesToRender);
```

**Rust (bridge.rs):**
```rust
// Add temporarily for debugging - REMOVE before committing
{
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/beamer_au_debug.log")
    {
        let _ = writeln!(file, "[Rust] some_value={}", some_value);
    }
}
```

**Viewing logs:**
```bash
# Watch log in real-time
tail -f /tmp/beamer_au_debug.log

# Clear log before testing
rm /tmp/beamer_au_debug.log
```

### Important: Don't Commit Debug Logging

File I/O in the audio render path causes:
- Priority inversion (audio thread blocked on I/O)
- Audio dropouts and glitches
- Non-real-time-safe behavior

**Always remove debug logging before committing.** Use `git diff` to verify no logging remains.
