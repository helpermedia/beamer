# AU Plugin Crash Analysis: PC=0 During Instantiation

## Executive Summary

AU plugins built with beamer-au crash during `auval` validation with a SIGSEGV at address 0x0 (null function pointer call). The crash occurs inside Apple's `AudioToolboxCore` framework during AU instantiation, specifically in `APComponent::newInstance()` at offset +628, **after** our factory function successfully returns a valid AU instance.

## Problem Description

### Symptoms
- `auval -v aufx gain Bemr` crashes with exit code 139 (SIGSEGV)
- Crash location: PC=0x0 (null function pointer dereference)
- Crash timing: During "TESTING OPEN TIMES: COLD:" phase
- Our factory function (`create_audio_unit_instance`) completes successfully before the crash

### Stack Trace
```
* thread #1, queue = 'com.apple.main-thread', stop reason = EXC_BAD_ACCESS (code=1, address=0x0)
  * frame #0: 0x0000000000000000
    frame #1: AudioToolboxCore`APComponent::newInstance(...) + 628
    frame #2: AudioToolboxCore`instantiate(...) + 388
    frame #3: AudioToolboxCore`__AudioComponentInstanceNew_block_invoke + 120
    frame #4: AudioToolboxCore`Synchronously + 132
    frame #5: AudioToolboxCore`AudioComponentInstanceNew + 248
```

### What Works
- Component registration passes (`* * PASS` in auval output)
- Class allocation succeeds
- `initWithComponentDescription:error:` returns a valid instance
- AU metadata (name, manufacturer, version) is correctly read

### What Fails
- After our factory returns, Apple's `APComponent::newInstance` continues processing
- At offset +628, it calls a null function pointer and crashes

## Technical Investigation

### Architecture Overview

The beamer-au crate uses:
- **objc2 v0.6** - Rust bindings to Objective-C runtime
- **block2 v0.6** - Rust implementation of Objective-C blocks
- **define_class! macro** - To create `BeamerAudioUnit` as a subclass of `AUAudioUnit`

The `BeamerAudioUnit` class overrides several methods including `internalRenderBlock`, which returns an Objective-C block for audio processing.

### Key Finding: The Crash is Triggered by Method Presence

Through systematic testing, we discovered:

| Scenario | Result |
|----------|--------|
| `internalRenderBlock` not defined (superclass used) | Crash at PC=0 |
| `internalRenderBlock` returns `nil` via `OpaqueBlock(std::ptr::null())` | Crash at PC=0 |
| `internalRenderBlock` returns valid block pointer | Crash at PC=0 |
| `internalRenderBlock` with different return type (`Option<Retained<AnyObject>>`) | Crash at PC=0 |
| Manual method addition via `class_replaceMethod` | Crash at PC=0 |

**Critical observation**: Our `internalRenderBlock` method is **never called** before the crash. We added extensive logging including file-based logging, and the method body never executes.

### What This Means

The crash is not caused by:
- The block content we return
- The block's invoke function pointer
- Our method implementation logic
- Block encoding metadata

The crash appears to be triggered by something in the **class structure or method dispatch table** that Apple's code accesses after our factory returns, but before any method is actually invoked on the instance.

### Block Encoding Investigation

We investigated whether the issue was related to block type encoding:

**AURenderBlock/AUInternalRenderBlock Signature:**
```c
typedef AUAudioUnitStatus (^AUInternalRenderBlock)(
    AudioUnitRenderActionFlags *actionFlags,
    const AudioTimeStamp *timestamp,
    AUAudioFrameCount frameCount,
    NSInteger outputBusNumber,
    AudioBufferList *outputData,
    const AURenderEvent *realtimeEventListHead,
    AURenderPullInputBlock pullInputBlock
);
```

**Our Encoding:**
```rust
const ENCODING_CSTR: &'static CStr =
    c"i64@?0^I8^{AudioTimeStamp}16I24q32^{AudioBufferList}40^{AURenderEvent}48@?56";
```

The encoding appears correct:
- `i` = int32 return (AUAudioUnitStatus/OSStatus)
- `64` = total frame size
- `@?0` = block self at offset 0
- Parameters at correct offsets for ARM64 ABI

### objc2 Validation

objc2 performs runtime validation of method signatures. When we intentionally used a wrong return type (`*const c_void` with encoding `^v` instead of `@?`), objc2 correctly panicked:

```
defined invalid method -[BeamerAudioUnit internalRenderBlock]:
expected return to have type code '@?', but found '^v'
```

This confirms objc2's validation works, and our `OpaqueBlock` type with `Encoding::Block` passes validation.

### Block Memory Layout Verification

We verified the block structure created by block2:

```
Block ABI Layout (ARM64):
- Offset 0:  isa pointer (8 bytes) - Points to _NSConcreteMallocBlock
- Offset 8:  flags (4 bytes) - Block flags including BLOCK_HAS_SIGNATURE
- Offset 12: reserved (4 bytes)
- Offset 16: invoke pointer (8 bytes) - Function pointer for block invocation
- Offset 24: descriptor pointer (8 bytes) - Points to block descriptor
```

When we logged the no-op block structure, the invoke pointer was non-null and valid. The block structure appears correct.

## Hypotheses

### Hypothesis 1: objc2 Class Metadata Issue

The most likely cause is an issue with how objc2's `define_class!` macro generates the class metadata when subclassing `AUAudioUnit`.

Possible issues:
- Method implementation pointer (IMP) is stored incorrectly
- Method type encoding in the class structure is malformed
- Superclass pointer or method lookup chain is corrupted
- Some AUAudioUnit-specific class metadata is not properly inherited

### Hypothesis 2: AUAudioUnit Initialization Side Effects

`AUAudioUnit`'s `initWithComponentDescription:error:` may perform internal setup that accesses method implementations or caches certain properties. If this caching reads corrupted metadata, it could store a null pointer that's later called.

### Hypothesis 3: KVO or Property Observer Issue

`AUAudioUnit` uses Key-Value Observing internally. If objc2's class doesn't properly support KVO, observers might receive corrupted data leading to the null call.

### Hypothesis 4: ARM64 ABI Mismatch

There could be a calling convention mismatch between:
- How objc2 generates method trampolines
- How AUAudioUnit/AudioToolbox expects methods to be called

This could cause the return value register to contain 0 even though our method returns a valid pointer.

## What We Ruled Out

1. **Block content** - Returning nil doesn't fix the crash
2. **Block invoke pointer** - Verified non-null in our blocks
3. **Block encoding metadata** - Using `with_encoding` doesn't help
4. **Our method implementation** - Method is never called
5. **Manual method addition** - Using raw `class_replaceMethod` still crashes
6. **Return type variations** - Different return types all crash

## Recommendations

### Short-Term Workarounds

1. **Hybrid Architecture**: Write the AUAudioUnit subclass in Objective-C/Swift and bridge to Rust only for DSP processing. This is the approach used by [SwiftRustAudioExample](https://github.com/cornedriesprong/SwiftRustAudioExample) and guarantees ABI compatibility.

2. **Try objc crate**: The older `objc` crate (predecessor to objc2) has different class declaration mechanics. It might not have this issue.

3. **Minimal objc2 Class**: Create the AU class with absolutely no method overrides to verify if basic subclassing works, then add methods one by one.

### Investigation Paths

1. **Disassemble APComponent::newInstance**: Use Hopper or IDA to understand what happens at offset +628. This would reveal exactly what null pointer is being called.

2. **Compare with Working AU**: Find a working Rust AU implementation (if any exists) and compare the class structure byte-by-byte.

3. **objc2 Source Analysis**: Deep dive into `define_class!` macro expansion to understand exactly how method implementations are registered.

4. **Test with Older macOS**: Try on older macOS versions to see if this is a recent regression in AudioToolbox.

### Diagnostic Code

To help narrow down the issue, consider adding this diagnostic before returning from the factory:

```rust
// Inspect the class method list
let class = BeamerAudioUnit::class();
unsafe {
    // Get method for internalRenderBlock
    let sel = objc2::sel!(internalRenderBlock);
    let method = class_getInstanceMethod(class as *const _ as *mut _, sel);
    if method.is_null() {
        log::error!("internalRenderBlock method not found in class!");
    } else {
        let imp = method_getImplementation(method);
        log::debug!("internalRenderBlock IMP: {:p}", imp);
        if imp.is_null() {
            log::error!("internalRenderBlock has NULL implementation!");
        }
    }
}
```

## Environment

- **macOS**: Darwin 25.2.0 (macOS 26/Tahoe)
- **Architecture**: ARM64 (Apple Silicon)
- **Rust**: Latest stable
- **objc2**: 0.6.x
- **block2**: 0.6.x
- **auval**: Version 1.10.0

## Files Involved

- `crates/beamer-au/src/audio_unit.rs` - AU class definition, `internalRenderBlock` method
- `crates/beamer-au/src/render.rs` - Block creation, `AuRenderBlockEncoding`
- `crates/beamer-au/src/factory.rs` - AU component registration
- `crates/beamer-au/src/export.rs` - Entry point macros

## Investigation Results (January 2026)

### Tests Performed

We performed extensive testing to isolate the crash cause:

#### 1. Diagnostic Code Added
Added runtime inspection of class metadata before returning from factory:
```
=== AU CLASS DIAGNOSTICS ===
Class pointer: 0xa08c243c0
Class name: BeamerAudioUnit
Superclass: AUAudioUnit
internalRenderBlock method: 0x8000000104b7f61a
internalRenderBlock IMP: 0x1170e4ecc
internalRenderBlock encoding: @?@:
allocateRenderResourcesAndReturnError: method: 0x8000000104b7d9aa
allocateRenderResourcesAndReturnError: IMP: 0x1170e4f9c
...
Instance created at: 0x104b82c30
Instance responds to internalRenderBlock: true
Factory returning now...
```

**Finding**: All method implementations (IMPs) are valid, non-null pointers. Method encodings are correct (`@?@:` for block-returning methods).

#### 2. Used objc2-audio-toolbox Proper Bindings
Replaced manual `extern_class!` declaration with proper `AUAudioUnit` from `objc2-audio-toolbox` crate.

**Finding**: Same crash. Using official framework bindings doesn't help.

#### 3. Minimal Class Test (Critical Finding)
Created a completely minimal AUAudioUnit subclass with:
- NO instance variables (ivars)
- NO method overrides
- ONLY relies on superclass

```rust
define_class!(
    #[unsafe(super(AUAudioUnit))]
    #[name = "MinimalBeamerAU"]
    pub struct MinimalBeamerAU;
    // INTENTIONALLY NO METHOD IMPLEMENTATIONS
);
```

**Finding**: **STILL CRASHES**. Even a minimal subclass with absolutely no customization crashes in `APComponent::newInstance + 628`.

### Conclusions from Testing

1. **Not our method implementations**: The minimal class with no overrides crashes
2. **Not our ivars**: The minimal class has no ivars and still crashes
3. **Not incorrect AUAudioUnit declaration**: Using objc2-audio-toolbox's proper binding doesn't help
4. **Not method encoding issues**: All IMPs are valid, encodings are correct

**Root Cause**: The issue is fundamental to how `objc2`'s `define_class!` macro generates class metadata. Something in the generated class structure is incompatible with what Apple's `AudioToolboxCore` framework expects.

### Related Issues

- [objc2 Issue #606](https://github.com/madsmtm/objc2/issues/606): Similar crash reports with delegate classes using `declare_class!`
- The objc2 documentation notes fundamental safety concerns: "Fundamentally cannot be made safe, since you're calling into unknown Objective-C classes"

## Recommended Path Forward

Given that even a minimal objc2 subclass crashes, the options are:

### 1. Hybrid Architecture (Recommended)

Write the `AUAudioUnit` subclass in **Objective-C or Swift**, then bridge to Rust only for DSP:

```
┌──────────────────────────────────────┐
│  Objective-C/Swift AU Wrapper        │
│  (AUAudioUnit subclass)              │
│  - inputBusses/outputBusses          │
│  - internalRenderBlock               │
│  - parameterTree                     │
└───────────────┬──────────────────────┘
                │ C-ABI calls
                ▼
┌──────────────────────────────────────┐
│  Rust DSP Core                       │
│  - beamer-core traits                │
│  - Audio processing                  │
│  - Parameter handling                │
└──────────────────────────────────────┘
```

This approach:
- Guarantees ABI compatibility with macOS
- Keeps all DSP in Rust for performance and safety
- Is the pattern used by [SwiftRustAudioExample](https://github.com/cornedriesprong/SwiftRustAudioExample)

### 2. File an Issue with objc2

Report this issue to the objc2 maintainers at [github.com/madsmtm/objc2](https://github.com/madsmtm/objc2) with:
- The minimal reproduction case
- The crash stack trace
- The finding that even empty subclasses crash

### 3. Try the Older objc Crate

The predecessor `objc` crate uses different class declaration mechanics and might work. However, this would require significant refactoring and the crate is less actively maintained.

## Conclusion

This appears to be a fundamental incompatibility between objc2's `define_class!` and Apple's AUAudioUnit class. The crash occurs in Apple's code after we return a valid instance, suggesting the class metadata itself is somehow malformed or incompatible with what AudioToolbox expects.

**The hybrid architecture approach is the recommended path forward** - it guarantees compatibility while still allowing the core audio processing to be written in Rust. This is also the approach used by other successful Rust audio plugin projects.
