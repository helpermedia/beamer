# AU Stereo Channel Bug - Debug TODO

## Problem
- Reaper AU: Only left channel audio passes through on first insert
- Re-inserting plugin fixes stereo (workaround)
- Logic Pro: Shows "not compatible"

## Root Cause (investigating)
ObjC writes `output_buses[0].channel_count = 2`, but Rust reads `1`.
Suspected struct layout mismatch between C and Rust.

## Current Debug Logging
Debug output writes to `/tmp/beamer_au_debug.log`

Expected output format:
```
[OBJC] sizeof_config=X sizeof_info=Y in[0]=2 out[0]=2
[RUST] sizeof_config=X sizeof_info=Y in[0]=2 out[0]=?
```

If sizes differ between OBJC and RUST lines, that confirms struct layout mismatch.

## Build & Test (One Command)

```bash
# Clean build, install to ~/Applications, register AU, clear log
rm /tmp/beamer_au_debug.log; cargo xtask bundle simple-gain --release --au --clean --install
```

This single command does everything:
1. Cleans cc cache and previous build artifacts
2. Builds universal binary (x86_64 + arm64)
3. Creates and signs the .app bundle
4. Installs to `~/Applications/BeamerSimpleGain.app`
5. **Automatically launches the app** (required to register AU with macOS)
6. Clears AU cache

No manual steps needed - the `--install` flag handles app execution.

After running, just:
1. Open Reaper and insert the plugin
2. Check log: `cat /tmp/beamer_au_debug.log`

## Validate with auval

```bash
# List all Beamer AUs
auval -a | grep -i beamer

# Validate the plugin
auval -v aufx siga Bmer
```

Component codes: `aufx` (effect), `Bmer` (manufacturer), `siga` (simple-gain)

## Quick Reference

| Action | Command |
|--------|---------|
| Full rebuild & install | `cargo xtask bundle simple-gain --release --au --clean --install` |
| Check registration | `pluginkit -m -v 2>/dev/null \| grep -i beamer` |
| Validate AU | `auval -v aufx siga Bmer` |
| Check log | `cat /tmp/beamer_au_debug.log` |
| Clear log | `rm /tmp/beamer_au_debug.log` |

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
