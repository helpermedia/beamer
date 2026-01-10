# Hybrid Objective-C/Rust AU Implementation Plan

## Status: ✅ IMPLEMENTED (January 2026)

The hybrid architecture has been fully implemented. The `beamer-au` crate now uses a native Objective-C `AUAudioUnit` subclass (`BeamerAuWrapper`) that bridges to Rust via C-ABI functions.

## Background

The `beamer-au` crate originally attempted to create an `AUAudioUnit` subclass using `objc2`'s `define_class!` macro. This crashed in Apple's `AudioToolboxCore` framework at `APComponent::newInstance + 628` with a null function pointer dereference (PC=0).

**Critical finding**: Even a completely minimal subclass with NO ivars and NO method overrides crashed. This proved the issue was fundamental to how `objc2` generates Objective-C class metadata - not our implementation.

See `docs/AU_CRASH_ANALYSIS.md` for the full investigation details and resolution.

## Solution: Hybrid Architecture

Write the `AUAudioUnit` subclass in native Objective-C, then bridge to Rust via C-ABI functions. This guarantees Apple's runtime accepts our class while keeping all DSP in Rust.

```
┌─────────────────────────────────────────┐
│  Objective-C: BeamerAuWrapper           │
│  (Native AUAudioUnit subclass)          │
│  - inputBusses / outputBusses           │
│  - internalRenderBlock                  │
│  - parameterTree                        │
│  - allocateRenderResources              │
└──────────────────┬──────────────────────┘
                   │ extern "C" calls
                   ▼
┌─────────────────────────────────────────┐
│  Rust: beamer-au crate                  │
│  - Plugin instance management           │
│  - Audio processing (render block)      │
│  - Parameter handling                   │
│  - MIDI processing                      │
└─────────────────────────────────────────┘
```

## File Structure

```
crates/beamer-au/
├── Cargo.toml              # Add cc build dependency
├── build.rs                # NEW: Compile Objective-C
├── objc/                   # NEW: Objective-C sources
│   ├── BeamerAuWrapper.h
│   ├── BeamerAuWrapper.m
│   └── BeamerAuBridge.h    # C-ABI function declarations
├── src/
│   ├── lib.rs
│   ├── bridge.rs           # NEW: extern "C" function implementations
│   ├── audio_unit.rs       # MODIFY: Remove objc2 class, keep helpers
│   └── ... (existing files)
```

## Implementation Tasks

### Phase 1: Build System Setup ✅

- [x] **1.1** Add `cc` crate to `[build-dependencies]` in `Cargo.toml`
- [x] **1.2** Create `build.rs` that compiles `.m` files with:
  - `-fobjc-arc` flag for automatic reference counting
  - Link frameworks: `AudioToolbox`, `AVFoundation`, `Foundation`
  - Only compile on `target_os = "macos"`
- [x] **1.3** Create `objc/` directory structure

### Phase 2: C-ABI Bridge Definition ✅

- [x] **2.1** Create `objc/BeamerAuBridge.h` with C function declarations:
  ```c
  // Instance lifecycle
  void* beamer_au_create_instance(void);
  void beamer_au_destroy_instance(void* instance);

  // Render resources
  int32_t beamer_au_allocate_render_resources(
      void* instance,
      double sample_rate,
      uint32_t max_frames,
      uint32_t input_channels,
      uint32_t output_channels
  );
  void beamer_au_deallocate_render_resources(void* instance);

  // Render block (called from audio thread)
  int32_t beamer_au_render(
      void* instance,
      uint32_t* action_flags,
      const AudioTimeStamp* timestamp,
      uint32_t frame_count,
      int32_t output_bus_number,
      AudioBufferList* output_data,
      const AURenderEvent* events,
      AURenderPullInputBlock pull_input_block
  );

  // Parameters
  void* beamer_au_create_parameter_tree(void* instance);
  void beamer_au_parameter_changed(void* instance, uint64_t address, float value);

  // State
  uint32_t beamer_au_get_state_size(void* instance);
  void beamer_au_get_state(void* instance, uint8_t* buffer, uint32_t size);
  int32_t beamer_au_set_state(void* instance, const uint8_t* buffer, uint32_t size);

  // Properties
  uint32_t beamer_au_get_latency_samples(void* instance);
  uint32_t beamer_au_get_tail_samples(void* instance);
  ```

- [x] **2.2** Create `src/bridge.rs` implementing these as `#[no_mangle] pub extern "C"` functions
  - Use existing `AuPluginInstance` trait and `factory::create_instance()`
  - Store instance as `Box<dyn AuPluginInstance>` behind the `void*`
  - Reuse existing render block logic from `src/render.rs`

### Phase 3: Objective-C AUAudioUnit Subclass ✅

- [x] **3.1** Create `objc/BeamerAuWrapper.h`:
  ```objc
  #import <AudioToolbox/AudioToolbox.h>

  @interface BeamerAuWrapper : AUAudioUnit
  @end
  ```

- [x] **3.2** Create `objc/BeamerAuWrapper.m` implementing:
  - **Instance variables**: `void* _rustInstance`, bus arrays, cached values
  - **`-initWithComponentDescription:options:error:`**: Call `beamer_au_create_instance()`
  - **`-dealloc`**: Call `beamer_au_destroy_instance()`
  - **`-inputBusses`** / **`-outputBusses`**: Return cached bus arrays (KVO requirement)
  - **`-allocateRenderResourcesAndReturnError:`**:
    - Call super
    - Extract format info from buses
    - Call `beamer_au_allocate_render_resources()`
  - **`-deallocateRenderResources`**: Call `beamer_au_deallocate_render_resources()` then super
  - **`-internalRenderBlock`**: Return block that calls `beamer_au_render()`
  - **`-parameterTree`**: Build from Rust via `beamer_au_get_parameter_info()`
  - **`-fullState`** / **`-setFullState:`**: State persistence via bridge functions

### Phase 4: Factory Function Update ✅

- [x] **4.1** Create new factory in Objective-C that instantiates `BeamerAuWrapper`:
  ```objc
  void* BeamerAudioUnitFactory(const AudioComponentDescription* desc) {
      NSError* error = nil;
      BeamerAuWrapper* au = [[BeamerAuWrapper alloc]
          initWithComponentDescription:*desc
          options:0
          error:&error];
      return (__bridge_retained void*)au;
  }
  ```

- [x] **4.2** Update `src/export.rs` macro to use the ObjC factory (it's already named `BeamerAudioUnitFactory`)

### Phase 5: Parameter Tree Bridge ✅

- [x] **5.1** Implement `beamer_au_get_parameter_info()` in Rust:
  - Query plugin parameters via existing `Parameters` trait
  - Return parameter info structs that ObjC uses to build `AUParameterTree`

- [x] **5.2** Implement ObjC parameter tree construction:
  - Create `AUParameterTree` from Rust parameter definitions
  - Set up KVO observers that call `beamer_au_set_parameter_value()`

### Phase 6: Cleanup and Testing ✅

- [x] **6.1** Remove the `objc2`-based `BeamerAudioUnit` class (`audio_unit.rs`, `ivar_arc.rs`)
- [x] **6.2** Remove `minimal_test.rs` module
- [x] **6.3** Update `docs/AU_CRASH_ANALYSIS.md` with resolution
- [ ] **6.4** Test with `auval -v aufx gain Bemr` (requires bundled plugin)
- [ ] **6.5** Test in Logic Pro / GarageBand (requires bundled plugin)
- [ ] **6.6** Verify all example plugins work as AU (requires bundled plugin)

## Key Design Decisions

### Memory Management
- Rust owns the plugin instance (`Box<dyn AuPluginInstance>`)
- ObjC wrapper holds a `void*` pointer to the Rust instance
- `dealloc` must call Rust to free the instance

### Thread Safety
- Render block runs on audio thread - no allocations, no locks
- Parameter changes may come from any thread - use atomic/lock-free where possible
- Existing `Mutex<Box<dyn AuPluginInstance>>` pattern from current code

### Bus Configuration
- ObjC wrapper creates `AUAudioUnitBusArray` for inputs/outputs
- Must return same instance each time (KVO requirement)
- Query Rust for bus count/format via bridge

### Render Block
- ObjC creates the `AUInternalRenderBlock`
- Block captures `void* _rustInstance`
- Calls `beamer_au_render()` which delegates to existing Rust render logic

## Files to Reference

Understanding these existing files is essential:

1. **`src/audio_unit.rs`** - Current objc2-based implementation (what to replace)
2. **`src/render.rs`** - Render block creation, `RenderBlockTrait`, encoding
3. **`src/instance.rs`** - `AuPluginInstance` trait definition
4. **`src/processor.rs`** - `AuProcessor<P>` generic wrapper
5. **`src/parameters.rs`** - Parameter tree building logic
6. **`src/factory.rs`** - Plugin factory registration
7. **`src/export.rs`** - The `export_au!` macro

## Success Criteria

1. `auval -v aufx gain Bemr` passes all tests (no crash)
2. Plugin loads in Logic Pro and processes audio
3. Parameters are visible and controllable in host
4. State save/load works (presets)
5. All existing example plugins (`gain`, `delay`, `synth`, etc.) work as AU

## Estimated Complexity

- **Build system**: Simple (~50 lines in build.rs)
- **C bridge header**: ~50 lines
- **Rust bridge impl**: ~300 lines (mostly delegating to existing code)
- **ObjC wrapper**: ~400-500 lines
- **Total new code**: ~800-900 lines
- **Reused existing code**: Most of render.rs, parameters.rs, instance.rs, processor.rs
