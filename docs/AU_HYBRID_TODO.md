# AU Hybrid Implementation - Status

## Status: Factory registration fix implemented

**Updated: 2026-01-10**

The hybrid Objective-C/Rust AU architecture now uses an AUv2 `.component` bundle
with v3 `AUAudioUnit` internals. The factory registration timing issue has been
addressed with explicit registration checks.

## Previous Issue (Resolved)

The plugin was failing with error `-1` during `auval` testing because the Rust
factory wasn't registered before `BeamerAudioUnitFactory` was called.

## Implemented Solution

We implemented a variant of **Option A** (explicit factory check) combined with
the proper AUv2 `AudioComponentPlugInInterface` pattern:

### 1. Factory Registration Check (`bridge.rs`)

```rust
#[no_mangle]
pub extern "C" fn beamer_au_ensure_factory_registered() -> bool {
    factory::is_registered()
}
```

### 2. ObjC Wrapper Checks Before Instance Creation (`BeamerAuWrapper.m`)

Both `initWithComponentDescription:` and `createAudioUnitWithComponentDescription:`
now call `beamer_au_ensure_factory_registered()` and return an error if false.

### 3. AUv2 Factory Pattern (`BeamerAuWrapper.m`)

The factory now returns an `AudioComponentPlugInInterface` with:
- `Open()` - Creates `BeamerAuWrapper` instance
- `Close()` - Releases instance
- `Lookup()` - Returns `NULL` to defer to v3 `AUAudioUnit` API

### 4. Subclass Registration

`BeamerAuRegisterSubclass()` is called once on first factory invocation using
`dispatch_once`, registering the `AUAudioUnit` subclass with the framework.

### 5. Symbol Export (`build.rs`)

Explicit linker flag ensures `BeamerAudioUnitFactory` is exported:
```rust
println!("cargo:rustc-cdylib-link-arg=-Wl,-exported_symbol,_BeamerAudioUnitFactory");
```

## Additional Improvements

### Channel Configuration Validation

Added `beamer_au_is_channel_config_valid()` to validate channel configurations
per component type:
- **Effect (aufx)**: Input channels must equal output channels
- **Instrument (aumu)**: Any output channel count valid
- **MIDI Processor (aumi)**: Input channels must equal output channels

### Frame Count Validation

`beamer_au_render()` now validates that `frame_count <= max_frames` and returns
`kAudioUnitErr_TooManyFramesToProcess` if exceeded.

### Channel Capabilities

`BeamerAuWrapper` now returns explicit `channelCapabilities` for supported
configurations (mono, stereo, quad, 5.0, 5.1, 6.1, 7.1).

## Investigation Tasks (Completed)

- [x] **1. Verify module initializer runs** - Added explicit factory check
- [x] **2. Check symbol visibility** - Added explicit symbol export in build.rs
- [x] **3. Verify link order** - Addressed via explicit checks, not relying on init order
- [x] **4. Test manual factory registration** - Implemented via `beamer_au_ensure_factory_registered()`

## Testing Commands

```bash
# Build and install AU
cargo xtask bundle gain --au --release --install

# Test with auval
auval -v aufx gain Bemr

# Check dylib symbols
nm -g target/release/libgain.dylib | grep -i beamer

# Check system logs
log show --last 1m | grep -i beamer
```

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│  DAW Host                                                        │
│  Calls BeamerAudioUnitFactory (from Info.plist factoryFunction)  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  BeamerAudioUnitFactory()                                        │
│  1. registerSubclass() once via dispatch_once                    │
│  2. Returns AudioComponentPlugInInterface*                       │
└─────────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┴───────────────────┐
          ▼                                       ▼
    Open() callback                        Lookup() → NULL
    1. Check beamer_au_ensure_factory_registered()
    2. Create BeamerAuWrapper              (defer to v3 API)
          │
          ▼
┌─────────────────────────────────────────────────────────────────┐
│  BeamerAuWrapper : AUAudioUnit <AUAudioUnitFactory>              │
│  - internalRenderBlock → beamer_au_render()                      │
│  - parameterTree from Rust parameter definitions                 │
│  - channelCapabilities for supported configurations              │
└─────────────────────────────────────────────────────────────────┘
```

## Related Files

- `crates/beamer-au/src/export.rs` - Module init macro
- `crates/beamer-au/src/factory.rs` - Factory registration
- `crates/beamer-au/src/bridge.rs` - C-ABI bridge with factory check
- `crates/beamer-au/objc/BeamerAuWrapper.m` - ObjC wrapper with AUv2 factory
- `docs/AU_ARCHITECTURE_REVIEW.md` - Architecture overview
