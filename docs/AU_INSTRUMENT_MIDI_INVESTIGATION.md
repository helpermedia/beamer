# AU Instrument (aumu) MIDI Investigation

**Status: ✅ RESOLVED (2026-01-20)**

## Problem Summary

AU instruments (`aumu` component type) produced no audio because **MIDI events were not being delivered** to the render block. The same synth plugin worked correctly as VST3.

**Solution**: Two fixes were required:
1. Create render block eagerly in `init` (like JUCE) - enables base class MIDI forwarding
2. Convert absolute `eventSampleTime` to relative buffer offset (like iPlug2)

## Symptoms

- BeamerSynth AU loads successfully in Logic Pro and Reaper
- Output buffers are correctly collected (2 channels, stereo)
- `auval -v aumu synt Bmer` passes all tests including "Test MIDI"
- **No MIDI events received** - `realtimeEventListHead` is always null
- VST3 version of the same synth works perfectly in Reaper and Cubase

## Key Finding: auval MIDI Test vs Real Host Usage

**Critical observation:**

| Host | Loading Mode | MIDI Works? |
|------|--------------|-------------|
| auval | In-process | ✅ Yes (auval's synthetic MIDI test) |
| Logic Pro | Out-of-process (XPC) | ❌ No |
| Reaper | In-process | ❌ No |

**Important**: Reaper loads AU plugins in-process, yet MIDI still doesn't work. This rules out XPC as the cause.

The auval "Test MIDI" passes, but this is auval's own synthetic test - it doesn't mean real hosts will deliver MIDI the same way. The issue is likely:
- How real hosts route MIDI to AU instruments differs from auval's test
- There may be something hosts check before deciding to send MIDI to an AU
- The render block is being called, but hosts aren't populating `realtimeEventListHead`

## Debug Findings

### Log Output
```
[render] event_list null=true, midi_count=0
[render] after collect_outputs: out_ch=2, in_ch=0
[render] process done: midi=0, outputs=2, first_sample=0
```

### What Works
- AU plugin loads and initializes correctly
- Output bus configuration: 1 bus, stereo (2 channels)
- Input bus configuration: 0 buses (correct for instrument)
- Audio output buffers are properly allocated and collected
- Parameters work correctly
- auval validation passes completely (including "Test MIDI")

### What Doesn't Work
- MIDI events never arrive in real hosts (Logic Pro, Reaper)
- The `realtimeEventListHead` pointer is always null
- No note-on/note-off events reach the plugin
- This happens regardless of in-process or out-of-process loading

## Technical Details

### AU Render Block Signature
```objc
^AUAudioUnitStatus(
    AudioUnitRenderActionFlags* actionFlags,
    const AudioTimeStamp* timestamp,
    AUAudioFrameCount frameCount,
    NSInteger outputBusNumber,
    AudioBufferList* outputData,
    const AURenderEvent* realtimeEventListHead,  // <-- Always null when out-of-process
    AURenderPullInputBlock pullInputBlock
)
```

### Component Description
- Type: `aumu` (Music Device / Instrument)
- Subtype: `synt`
- Manufacturer: `Bmer`

### Info.plist AudioComponents Entry
```xml
<key>AudioComponents</key>
<array>
    <dict>
        <key>type</key>
        <string>aumu</string>
        <key>subtype</key>
        <string>synt</string>
        <key>manufacturer</key>
        <string>Bmer</string>
        <key>name</key>
        <string>Beamer: BeamerSynth</string>
        <key>sandboxSafe</key>
        <true/>
        <key>tags</key>
        <array>
            <string>Synth</string>
        </array>
    </dict>
</array>
```

## Comparison with Working Implementations

### VST3 (Works)
- MIDI events come through `process_data.inputEvents`
- Host provides `IEventList` interface with `getEventCount()` and `getEvent()`

### iPlug2 AUv3 (Works)
- Also uses `realtimeEventListHead` parameter
- Same render block signature
- MIDI events are received and processed
- Uses `AUViewController` as extension class (we use `NSObject`)

## Attempted Fixes

### Fix 1: Implement channelCapabilities

**Problem**: Our `channelCapabilities` property returned `nil`.

**iPlug2 approach**: Returns an array of input/output channel pairs like `[@0, @2]` for instruments (0 inputs, 2 outputs).

**Fix applied**: Changed `channelCapabilities` to return actual configuration based on bus state.

**Result**: ❌ **Caused auval to fail** - The dynamic implementation read from bus format which changes during auval format tests, causing "Unit now reports it cannot support default channel configuration" error.

**Resolution**: Reverted to returning `nil` (means "any configuration supported").

---

### Fix 2: Implement MIDIOutputNames

**Problem**: We didn't implement `MIDIOutputNames` property that iPlug2 implements.

**Fix applied**: Added empty array return:
```objc
- (NSArray<NSString*>*)MIDIOutputNames {
    return @[];
}
```

**Rationale**: Even though we don't have MIDI output, implementing this property might signal to the host that we're MIDI-aware.

**Result**: ❌ **Did not fix MIDI input issue** - auval still passes, but MIDI events still not received in Logic/Reaper.

---

### Fix 3: Implement virtualMIDICableCount

**Problem**: We didn't implement `virtualMIDICableCount` property.

**Fix applied**: Added override that returns 1 for instruments:
```objc
- (NSInteger)virtualMIDICableCount {
    return beamer_au_accepts_midi(_rustInstance) ? 1 : 0;
}
```

**Result**: ❌ **Did not fix MIDI input issue** - Property implemented correctly but events still not received.

---

### Fix 4: Override musicDeviceOrEffect

**Problem**: The base class `AUAudioUnit` uses `musicDeviceOrEffect` to determine if MIDI should be enabled.

**Fix applied**: Added explicit override:
```objc
- (BOOL)musicDeviceOrEffect {
    return beamer_au_accepts_midi(_rustInstance);
}
```

**Result**: ❌ **Did not fix MIDI input issue** - Property returns YES but events still not received.

---

### Fix 5: Cache internalRenderBlock Instance

**Problem**: We were creating a new block each time `internalRenderBlock` getter was called. AUAudioUnit may wire MIDI event forwarding (`scheduleMIDIEventBlock` → `realtimeEventListHead`) to the **first** block it obtains. Returning different block instances on subsequent calls could break the forwarding chain.

**Fix applied**: Added caching ivar and early return:
```objc
@interface WrapperClass : AUAudioUnit {
    // ... other ivars ...
    AUInternalRenderBlock _cachedInternalRenderBlock;
}

- (AUInternalRenderBlock)internalRenderBlock {
    // CRITICAL: Cache the render block instance.
    if (_cachedInternalRenderBlock != nil) {
        return _cachedInternalRenderBlock;
    }

    // ... block creation ...
    _cachedInternalRenderBlock = ^AUAudioUnitStatus(...) { ... };

    return _cachedInternalRenderBlock;
}
```

Also updated to access host blocks (`musicalContextBlock`, `transportStateBlock`, `scheduleMIDIEventBlock`) dynamically inside the block rather than capturing them at creation time, since they may be set after the block is created.

**Result**: ❌ **Did not fix MIDI input issue** - Log still shows:
```
[render] event_list null=true, midi_count=0
```

The cached block receives render calls but `realtimeEventListHead` is still always null.

---

### Fix 6: Custom MIDI Event Collection (SOLUTION)

**Problem**: The base class `AUAudioUnit` is supposed to collect events from `scheduleMIDIEventBlock` and deliver them via `realtimeEventListHead` in `internalRenderBlock`. This mechanism was completely broken for us - `realtimeEventListHead` was always null despite hosts calling `scheduleMIDIEventBlock` with valid MIDI data.

**Root Cause Analysis**: We never determined exactly WHY the base class forwarding mechanism fails. However, we confirmed:
1. Hosts DO call `scheduleMIDIEventBlock` with MIDI events
2. The base class mechanism that should forward these to `realtimeEventListHead` does not work
3. This is NOT caused by missing properties (we implemented `virtualMIDICableCount`, `musicDeviceOrEffect`, etc.)
4. This is NOT caused by render block caching issues

The base class `AUAudioUnit`'s internal event collection/forwarding is simply not compatible with our AUv3 App Extension architecture for unknown reasons.

**Solution**: Bypass the base class entirely by implementing custom MIDI event collection:

```objc
// Buffered MIDI event structure
typedef struct {
    AUEventSampleTime sampleTime;
    uint8_t cable;
    uint8_t length;
    uint8_t data[3];
} BufferedMIDIEvent;

@interface WrapperClass : AUAudioUnit {
    // ... other ivars ...
    BufferedMIDIEvent _midiEvents[MIDI_BUFFER_CAPACITY];
    volatile uint32_t _midiEventCount;
    os_unfair_lock _midiLock;
}

// Override scheduleMIDIEventBlock getter to return our custom collector
- (AUScheduleMIDIEventBlock)scheduleMIDIEventBlock {
    __unsafe_unretained typeof(self) blockSelf = self;

    return ^(AUEventSampleTime eventSampleTime, uint8_t cable,
             NSInteger length, const uint8_t* midiBytes) {
        // Store event in our buffer (with lock for thread safety)
        os_unfair_lock_lock(&blockSelf->_midiLock);
        if (blockSelf->_midiEventCount < MIDI_BUFFER_CAPACITY) {
            BufferedMIDIEvent* event = &blockSelf->_midiEvents[blockSelf->_midiEventCount++];
            event->sampleTime = eventSampleTime;
            event->cable = cable;
            event->length = (uint8_t)length;
            memcpy(event->data, midiBytes, length);
        }
        os_unfair_lock_unlock(&blockSelf->_midiLock);
    };
}

// In internalRenderBlock: build AURenderEvent linked list from our buffer
- (AUInternalRenderBlock)internalRenderBlock {
    // ... in the block ...
    AURenderEvent midiEventStorage[MIDI_BUFFER_CAPACITY];
    const AURenderEvent* eventListHead = realtimeEventListHead; // Fallback

    os_unfair_lock_lock(&blockSelf->_midiLock);
    if (blockSelf->_midiEventCount > 0) {
        // Build linked list from buffered events
        for (uint32_t i = 0; i < blockSelf->_midiEventCount; i++) {
            // Copy to AURenderEvent format, link events together
            midiEventStorage[i].head.eventType = AURenderEventMIDI;
            midiEventStorage[i].head.next = (i + 1 < count) ? &midiEventStorage[i + 1] : NULL;
            // ... copy MIDI data ...
        }
        eventListHead = &midiEventStorage[0];
        blockSelf->_midiEventCount = 0; // Clear for next cycle
    }
    os_unfair_lock_unlock(&blockSelf->_midiLock);

    // Pass our event list to Rust
    return beamer_au_render(..., eventListHead, ...);
}
```

**Result**: ✅ **WORKS IN REAPER** - Synth produces sound when playing MIDI notes!

**Remaining Issues**:
- ❌ Still no sound in Logic Pro (needs investigation - Logic may handle `scheduleMIDIEventBlock` differently)

---

## Critical Discovery: scheduleMIDIEventBlock IS Being Called

**Major finding**: By wrapping `scheduleMIDIEventBlock`, we confirmed that **hosts ARE sending MIDI events**:

```
[ObjC] scheduleMIDIEventBlock CALLED: time=-4294967296, cable=0, len=3, status=0x90  // Note On
[ObjC] scheduleMIDIEventBlock CALLED: time=-4294967296, cable=0, len=3, status=0x80  // Note Off
```

This proves:
1. ✅ Host recognizes us as a MIDI-capable instrument
2. ✅ Host is using `scheduleMIDIEventBlock` to send MIDI events
3. ❌ Events scheduled via `scheduleMIDIEventBlock` are NOT appearing in `realtimeEventListHead`

**The disconnect**: The base class's `AUAudioUnit` should collect events from `scheduleMIDIEventBlock` and deliver them via `realtimeEventListHead` in `internalRenderBlock`. This mechanism is broken for us.

---

## Render Block Investigation

Both `renderBlock` and `internalRenderBlock` getters are being called:
```
[ObjC] renderBlock getter called
[ObjC] internalRenderBlock getter called
```

This confirms the host is using the standard `renderBlock` path (not bypassing to `internalRenderBlock` directly), which means the base class SHOULD be forwarding events.

**Key observations**:
1. `scheduleMIDIEventBlock` is non-nil after `[super init]`
2. Host reads both `renderBlock` and `internalRenderBlock`
3. Host calls `scheduleMIDIEventBlock` with MIDI data
4. But `realtimeEventListHead` is always null in our `internalRenderBlock`

---

## Solution Summary (RESOLVED)

**The Problem**: AU instruments produced no sound because MIDI events weren't reaching the audio processing code. Two issues were identified:

### Issue 1: Render Block Creation Timing

The base class `AUAudioUnit` sets up MIDI forwarding (wiring `scheduleMIDIEventBlock` → `realtimeEventListHead`) during initialization. If `internalRenderBlock` doesn't exist at that point, the forwarding can't be established.

**Comparison of frameworks:**

| Framework | When Block Created | Cached | MIDI Works |
|-----------|-------------------|--------|------------|
| JUCE | Eagerly in init() | Yes | Yes |
| iPlug2 | Lazily in getter | No | Yes |
| Beamer (old) | Lazily in getter | Yes | No |
| Beamer (fixed) | Eagerly in init() | Yes | Yes |

**Fix**: Create render block eagerly during `init` (like JUCE):

```objc
- (instancetype)initWithComponentDescription:... {
    self = [super initWithComponentDescription:desc options:options error:outError];
    if (self) {
        // ... other init code ...

        // CRITICAL: Create render block eagerly during init, like JUCE does.
        // The base class AUAudioUnit may set up MIDI forwarding (wiring
        // scheduleMIDIEventBlock -> realtimeEventListHead) during initialization.
        // If internalRenderBlock doesn't exist at that point, forwarding fails.
        (void)[self internalRenderBlock];
    }
    return self;
}
```

### Issue 2: Absolute vs Relative Sample Offsets

Even after fixing Issue 1, MIDI events still weren't being processed. Debug logging revealed the problem:

```
MIDI1: status=0x90 ch=0 d1=36 d2=108 offset=145346
```

The sample offset `145346` was way larger than the buffer size (~512 samples). This caused MIDI events to fall outside the processing window.

**Root Cause**: AU's `eventSampleTime` is an **absolute** sample position (like the transport), not a relative offset within the buffer.

**How iPlug2 handles this** (IPlugAUv3.mm:151):
```objc
midiMsg = {static_cast<int>(midiEvent.eventSampleTime - now), ...};
```

iPlug2 subtracts `now` (the timestamp's `sample_time`, i.e., buffer start position) from `eventSampleTime` to get a relative offset.

**Fix**: Convert absolute sample times to relative buffer offsets:

```rust
// In process_impl, after extracting MIDI events:
let buffer_start_sample = unsafe {
    if !timestamp.is_null() {
        (*timestamp).sample_time as i64
    } else {
        0
    }
};

for event in midi_buffer.events.iter_mut() {
    let absolute_time = event.sample_offset as i64;
    let relative_offset = absolute_time - buffer_start_sample;

    // Clamp to buffer bounds
    event.sample_offset = relative_offset.clamp(0, (num_samples - 1) as i64) as u32;
}
```

### Final Status

✅ **RESOLVED** - Both Reaper and Logic Pro now work correctly:
- ✅ **Reaper**: MIDI works, synth produces sound with full polyphony
- ✅ **Logic Pro**: MIDI works, synth produces sound with full polyphony
- ✅ **Polyphony**: Notes get unique IDs for voice tracking (pitch as note_id)

## What We've Ruled Out

1. ~~Component type wrong~~ - `aumu` is correct for instruments
2. ~~Output buffers not working~~ - Buffers are correctly collected (2 channels)
3. ~~Plugin not receiving render calls~~ - Render block is called, just without MIDI
4. ~~auval validation issues~~ - AU passes all auval tests
5. ~~channelCapabilities needed~~ - Returning `nil` works fine
6. ~~MIDIOutputNames needed~~ - Implementing it didn't help
7. ~~virtualMIDICableCount needed~~ - Implemented, returns 1, didn't help
8. ~~musicDeviceOrEffect needed~~ - Override returns YES, didn't help
9. ~~Host not sending MIDI~~ - Confirmed hosts DO call scheduleMIDIEventBlock
10. ~~renderBlock override issue~~ - Removing override didn't help
11. ~~internalRenderBlock caching~~ - Caching the block instance didn't fix event forwarding
12. ~~Base class event forwarding~~ - **BYPASSED** with custom MIDI collection
13. ~~Polyphony/voice allocation bug~~ - **FIXED**: Use pitch as note_id (AU/MIDI 1.0 has no native note IDs)

## Next Steps

All major issues are now resolved:

1. ~~**Fix Logic Pro MIDI delivery**~~ - ✅ **FIXED**: Two issues found and resolved:
   - **Render block timing**: Create render block eagerly in `init` (like JUCE) so base class can wire MIDI forwarding
   - **Sample offset conversion**: Convert absolute `eventSampleTime` to relative buffer offset (like iPlug2)

2. ~~**Fix polyphony bug**~~ - ✅ **FIXED**: Use MIDI pitch as `note_id` since AU/MIDI 1.0 doesn't have native note IDs. Fixed in:
   - `beamer-au/src/render.rs` - AU: use pitch as note_id
   - `beamer-vst3/src/processor.rs` - VST3: use pitch as note_id when host sends -1

3. ~~**Test with iPlug2 AU instrument in Logic**~~ - No longer needed, our implementation now works in both Reaper and Logic Pro.

## Related Files

- [xtask/src/main.rs](../xtask/src/main.rs) - ObjC wrapper generation
- [crates/beamer-au/src/render.rs](../crates/beamer-au/src/render.rs) - Render block and MIDI extraction
- [crates/beamer-au/src/bridge.rs](../crates/beamer-au/src/bridge.rs) - C bridge functions

## Related Documentation

- [AU_LOGIC_COMPATIBILITY_INVESTIGATION.md](AU_LOGIC_COMPATIBILITY_INVESTIGATION.md) - Original Logic Pro compatibility work
- [Apple AUAudioUnit Class Reference](https://developer.apple.com/documentation/audiotoolbox/auaudiounit)
- [Audio Unit Hosting Guide](https://developer.apple.com/library/archive/documentation/MusicAudio/Conceptual/AudioUnitHostingGuide_iOS/)
- [Apple Developer Forums: AUv3 MIDI events](https://developer.apple.com/forums/thread/46328)

---

## Build & Test Instructions

### Building the AU Plugin

```bash
# Build synth AU plugin (release, clean caches, install to ~/Applications)
cargo xtask bundle synth --release --au --clean --install

# Build gain effect AU plugin
cargo xtask bundle gain --release --au --clean --install
```

**Flags:**
- `--release` - Build in release mode (optimized)
- `--au` - Build only AU format (skip VST3)
- `--clean` - Remove cached build artifacts (important when changing ObjC wrapper code)
- `--install` - Install to `~/Applications` and register with `pluginkit`

### Clearing Build Caches

When modifying the ObjC wrapper code in `xtask/src/main.rs`, you **must** clear the build caches:

```bash
# Option 1: Use --clean flag (recommended)
cargo xtask bundle synth --release --au --clean --install

# Option 2: Manual cache removal
rm -rf target/release/build/beamer-au-*
rm -rf target/release/BeamerSynth.app
```

The `--clean` flag removes:
- `target/release/build/beamer-au-*` - Compiled ObjC wrapper
- `target/release/BeamerSynth.app` - Previous bundle

### Debug Logging

Debug logs are written to `/tmp/beamer-au.log` (NSLog doesn't work in sandboxed extensions).

```bash
# Clear log before testing
rm -f /tmp/beamer-au.log

# Monitor log in real-time
tail -f /tmp/beamer-au.log

# View recent entries
cat /tmp/beamer-au.log
```

### Validation

```bash
# Validate AU instrument
auval -v aumu synt Bmer

# Validate AU effect
auval -v aufx gain Bmer

# List registered AUs
pluginkit -m -v -p com.apple.audio.AudioUnit
```

### Forcing Host Rescan

After installing a new build, force hosts to rescan:

- **Logic Pro**: Hold Option while launching, or delete `~/Library/Caches/AudioUnitCache`
- **Reaper**: Options → Preferences → Plug-ins → AU → Re-scan
- **System-wide**: `killall -9 AudioComponentRegistrar` (aggressive)
