# WebView GUI Roadmap (Phase 2)

Platform-native WebView embedding for plugin GUIs (VST3 and AU).

## Approach

Direct platform APIs (not `wry`). Both VST3 and AU require attaching to
host-provided window handles (`NSView` on macOS, `HWND` on Windows).

`beamer-webview` is a pure platform layer with no format-specific code. Each
format wrapper uses it symmetrically:

- **VST3**: `beamer-vst3` depends on `beamer-webview`, implements `IPlugView`
  using `MacosWebView`/`WindowsWebView`
- **AU (AUv3)**: generated ObjC template calls into `beamer-webview` via C-ABI
  from `requestViewControllerWithCompletionHandler:`
- **AU (AUv2)**: generated ObjC template calls into `beamer-webview` via C-ABI
  from `kAudioUnitProperty_CocoaUI` view factory

This keeps `beamer-webview` free of `vst3` or AU dependencies and ensures
Phase 2C IPC features are available to both formats automatically.

| Platform | Backend | Crate | Phase |
|----------|---------|-------|-------|
| macOS | WKWebView | `objc2` + `objc2-web-kit` | 2A |
| Windows | WebView2 | `webview2-com` + `windows` | 2A |

### Note on `objc2`

The AU wrapper originally used `objc2` but had to abandon it due to ABI
incompatibility with `AUAudioUnit` subclassing.
The WebView case is different - we only need to *instantiate and configure*
`WKWebView`, not subclass a complex framework class. Tauri's `wry` crate uses
`objc2` 0.6 for WKWebView in production without issues.

## Phases

### Phase 2A: Core Platform Support
- Platform WebView creation and lifecycle (macOS/Windows)
- Static HTML loading
- `EditorDelegate` integration
- VST3 `IPlugView` wrapper (in `beamer-vst3`)
- AUv3 `NSViewController` integration (in `auv3_wrapper.m`, calling
  `beamer-webview` via C-ABI)
- AUv2 `kAudioUnitProperty_CocoaUI` integration (in `auv2_wrapper.c`, calling
  `beamer-webview` via C-ABI)

### Phase 2B: Resource Loading
- Embedded assets (`include_str!`)
- Dev server support (hot reload)
- `cargo xtask` integration

### Phase 2C: IPC & Parameter Binding
- JS API (`window.__BEAMER__`)
- Invoke pattern (JS -> Rust)
- Event emission (Rust -> JS)
- Parameter synchronization

## Plugin Directory Convention

Plugins place web UI assets in a `webview/` subdirectory. The name matches the
rendering technology, leaving room for alternative UI approaches (e.g. `egui/`,
`iced/`) in forks or future extensions.

**Plain HTML (no build step):**

```
examples/gain/
├── Cargo.toml
├── Config.toml
├── src/
│   └── lib.rs
└── webview/
    └── index.html
```

**Framework-based (TypeScript, React, Svelte, etc.):**

```
examples/equalizer/
├── Cargo.toml
├── Config.toml
├── src/
│   └── lib.rs
└── webview/
    ├── package.json
    ├── vite.config.ts
    ├── index.html
    └── src/
        └── App.tsx
```

**Detection rules for `cargo xtask bundle`:**

- `webview/package.json` exists: run build, embed `webview/dist/index.html`
- `webview/index.html` exists (no package.json): embed directly
- No `webview/` directory: no editor

## Crate Structure

```
beamer-webview/                    # Platform layer only, no format deps
├── src/
│   ├── lib.rs
│   ├── platform/
│   │   ├── macos.rs               # WKWebView
│   │   └── windows.rs             # WebView2
│   └── error.rs
```

Format-specific integration:
- VST3 `IPlugView` impl lives in `beamer-vst3`
- AUv3 view controller lives in `xtask/src/au_codegen/auv3_wrapper.m`
- AUv2 Cocoa UI view factory lives in `xtask/src/au_codegen/auv2_wrapper.c`

Note: `resources.rs` is Phase 2B scope.

## References

- [VST3 IPlugView](https://steinbergmedia.github.io/vst3_doc/base/classSteinberg_1_1IPlugView.html)
- [AUv3 requestViewController](https://developer.apple.com/documentation/audiotoolbox/auaudiounit/1583904-requestviewcontroller)
- [AUv2 kAudioUnitProperty_CocoaUI](https://developer.apple.com/documentation/audiotoolbox/kaudiounitproperty_cocoaui)
- [Tauri wry](https://github.com/tauri-apps/wry) - reference for objc2 + WKWebView usage
- [webview2-com](https://github.com/wravery/webview2-rs) - WebView2 Rust bindings
- [vstwebview](https://github.com/rdaum/vstwebview)

## Status

**Current**: Planning
**Next**: Phase 2A - [WEBVIEW_PHASE2A.md](./WEBVIEW_PHASE2A.md)
