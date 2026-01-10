# AU Hybrid Implementation - Remaining Issues

## Status: Build succeeds, runtime fails

The hybrid Objective-C/Rust AU architecture builds successfully, but the plugin fails to open at runtime with error `-1` (0xFFFFFFFF) during `auval` testing.

```
VALIDATING AUDIO UNIT: 'aufx' - 'gain' - 'Bemr'
Manufacturer String: Beamer
AudioUnit Name: BeamerGain
Component Version: 2.0.0 (0x20000)
* * PASS
--------------------------------------------------
TESTING OPEN TIMES:
COLD:
FATAL ERROR: OpenAComponent: result: -1,0xFFFFFFFF
```

## Root Cause Analysis

The error occurs during `beamer_au_create_instance()` which returns NULL. This happens because:

1. **Factory not registered**: The `export_au!` macro uses `__DATA,__mod_init_func` section to register the factory at dylib load time
2. **Module init may not run**: The module initializer might not be executing before `BeamerAudioUnitFactory` is called
3. **Order of operations issue**: macOS calls `BeamerAudioUnitFactory` → ObjC calls `beamer_au_create_instance()` → Rust checks `factory::create_instance()` → returns None if factory not registered

## Investigation Tasks

- [ ] **1. Verify module initializer runs**
  - Add logging to `__beamer_au_register()` in export.rs
  - Check if `log::debug!` in `register_factory()` outputs anything
  - May need to use `eprintln!` or write to file for debugging

- [ ] **2. Check symbol visibility**
  - Verify `BeamerAudioUnitFactory` is exported from the dylib
  - Run `nm -g target/release/libgain.dylib | grep -i beamer`
  - Check if both ObjC factory and Rust functions are visible

- [ ] **3. Verify link order**
  - The ObjC code is compiled by `cc` crate and linked
  - Rust code is compiled separately
  - Check if link order affects module init timing

- [ ] **4. Test manual factory registration**
  - Add an ObjC `+load` method to `BeamerAuWrapper` that calls a Rust init function
  - This guarantees registration before any instance creation

## Potential Fixes

### Option A: Use ObjC +load for initialization

Add to `BeamerAuWrapper.m`:
```objc
+ (void)load {
    // Call Rust to register factory before any AU instantiation
    beamer_au_init_factory();
}
```

Add to `bridge.rs`:
```rust
#[no_mangle]
pub extern "C" fn beamer_au_init_factory() {
    // The export_au! macro's __beamer_au_manual_init() does this
    // But we need a way to call it from ObjC
}
```

### Option B: Lazy factory registration in create_instance

Modify `beamer_au_create_instance()` to handle missing factory gracefully:
```rust
pub extern "C" fn beamer_au_create_instance() -> BeamerAuInstanceHandle {
    // If factory not registered, log error and return null
    if !factory::is_registered() {
        log::error!("AU factory not registered - module init may not have run");
        return ptr::null_mut();
    }
    // ... rest of function
}
```

### Option C: Use constructor attribute

Change the module init to use `#[ctor]` crate which is more reliable:
```rust
#[ctor::ctor]
fn init_au_factory() {
    factory::register_factory(...);
}
```

## Files to Modify

| File | Change |
|------|--------|
| `src/export.rs` | Add alternative initialization mechanism |
| `src/bridge.rs` | Add `beamer_au_init_factory()` function |
| `objc/BeamerAuWrapper.m` | Add `+load` method to call Rust init |
| `Cargo.toml` | Possibly add `ctor` crate |

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

## Related Files

- `crates/beamer-au/src/export.rs` - Module init macro
- `crates/beamer-au/src/factory.rs` - Factory registration
- `crates/beamer-au/src/bridge.rs` - C-ABI bridge
- `crates/beamer-au/objc/BeamerAuWrapper.m` - ObjC wrapper
- `docs/HYBRID_AU_IMPLEMENTATION_PLAN.md` - Implementation plan
- `docs/AU_CRASH_ANALYSIS.md` - Original crash analysis
