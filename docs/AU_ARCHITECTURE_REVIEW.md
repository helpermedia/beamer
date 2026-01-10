# Audio Unit Architecture

**Date**: 2026-01-10
**Status**: Working implementation using AUv2 .component bundles

---

## Architecture Overview

Beamer uses **AUv2 .component bundles** with a **v3 AUAudioUnit** implementation internally.

```
┌─────────────────────────────────────────────────────────────────┐
│  DAW Host (Logic Pro, Ableton, etc.)                            │
│  Scans /Library/Audio/Plug-Ins/Components/                      │
│  Reads Info.plist, calls factoryFunction                        │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  BeamerAudioUnitFactory(const AudioComponentDescription*)       │
│  1. Calls registerSubclass() once                               │
│  2. Returns AudioComponentPlugInInterface*                      │
└─────────────────────────────────────────────────────────────────┘
                              │
          ┌───────────────────┴───────────────────┐
          ▼                                       ▼
    Open() callback                        Lookup() → NULL
    Creates BeamerAuWrapper                (defer to v3 API)
          │
          ▼
┌─────────────────────────────────────────────────────────────────┐
│  BeamerAuWrapper : AUAudioUnit                                  │
│  Pure v3-style implementation                                   │
│  - internalRenderBlock for audio processing                     │
│  - parameterTree for parameters                                 │
│  - inputBusses/outputBusses for I/O                             │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  Rust Bridge (bridge.rs FFI functions)                          │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│  beamer_core::Plugin implementation                             │
└─────────────────────────────────────────────────────────────────┘
```

---

## Key Design Decisions

### Why AUv2 .component + v3 AUAudioUnit?

1. **Simple distribution**: `.component` bundles install to `/Library/Audio/Plug-Ins/Components/`
2. **No special signing**: Works with ad-hoc code signing
3. **Modern API internally**: Uses AUAudioUnit for parameters, buses, rendering
4. **Wide compatibility**: Works with all DAWs that support Audio Units

### How Lookup() Returning NULL Works

When `Lookup()` returns `NULL`, the AU framework uses the modern AUAudioUnit API:
- `parameterTree` for parameters
- `internalRenderBlock` for audio processing
- `inputBusses`/`outputBusses` for I/O configuration

This is documented behavior, not a hack.

### Why registerSubclass() in the Factory

The `registerSubclass:` call tells the framework how to create AUAudioUnit instances.
It must be called after the Rust factory is registered, so we call it from
`BeamerAudioUnitFactory()` which runs after all initializers.

---

## Safety Guarantees

### FFI Boundary Safety

All bridge functions in `bridge.rs` use `catch_unwind` to prevent Rust panics from
unwinding across the FFI boundary into Objective-C/host code. Panics are caught and
converted to appropriate error codes (e.g., `kAudioUnitErr_FailedInitialization`).

### Factory Registration

The plugin factory uses `OnceLock` for thread-safe, single registration. This enforces
the **one plugin per binary** constraint - attempting to register multiple plugins
will fail. The `beamer_au_ensure_factory_registered()` function allows the ObjC wrapper
to verify registration before creating instances.

### Render Block Safety

Objective-C callback blocks (e.g., `pullInputBlock`, `transportStateBlock`) are treated
as **render-scoped only**:
- Blocks are called within the render function, never stored beyond that scope
- This avoids lifetime/ownership issues with block captures
- The pattern is documented in `render.rs` with explicit safety invariants

### Thread Safety

- **Render thread**: No allocations, no locks that could block
- **Parameter access**: Atomic storage for thread-safe host/UI access
- **Render block**: Protected by `RwLock` with `try_read()` for real-time safety

---

## Bundle Structure

```
BeamerGain.component/
├── Contents/
│   ├── Info.plist       ← Contains factoryFunction key
│   ├── MacOS/
│   │   └── BeamerGain   ← The plugin dylib
│   ├── Resources/
│   └── PkgInfo
```

### Info.plist Structure

```xml
<key>AudioComponents</key>
<array>
    <dict>
        <key>type</key>
        <string>aufx</string>
        <key>subtype</key>
        <string>gain</string>
        <key>manufacturer</key>
        <string>Demo</string>
        <key>factoryFunction</key>
        <string>BeamerAudioUnitFactory</string>
        ...
    </dict>
</array>
```

---

## Future: AUv3 App Extensions

For Mac App Store distribution, AUv3 App Extensions would be needed:

```
BeamerGain.app/
├── Contents/
│   └── PlugIns/
│       └── BeamerGainAU.appex/
```

This requires:
- Proper Apple Developer signing
- Container app
- NSExtensionPrincipalClass in Info.plist

The current `AUAudioUnitFactory` protocol conformance in BeamerAuWrapper
is already in place for future AUv3 support.
