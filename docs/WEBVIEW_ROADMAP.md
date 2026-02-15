# WebView GUI Roadmap (Phase 2)

Platform-native WebView embedding for VST3 plugin GUIs.

## Approach

Direct platform APIs (not `wry`) - VST3 requires attaching to host-provided window handles.

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
- `IPlugView` implementation (macOS/Windows)
- WebView creation and lifecycle
- Static HTML loading
- `EditorDelegate` integration

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
beamer-webview/
├── src/
│   ├── lib.rs
│   ├── platform/
│   │   ├── macos.rs
│   │   └── windows.rs
│   ├── view.rs         # IPlugView wrapper
│   └── error.rs
```

Note: `resources.rs` is Phase 2B scope.

## References

- [VST3 IPlugView](https://steinbergmedia.github.io/vst3_doc/base/classSteinberg_1_1IPlugView.html)
- [Tauri wry](https://github.com/tauri-apps/wry) - reference for objc2 + WKWebView usage
- [webview2-com](https://github.com/wravery/webview2-rs) - WebView2 Rust bindings
- [vstwebview](https://github.com/rdaum/vstwebview)

## Status

**Current**: Planning
**Next**: Phase 2A - [WEBVIEW_PHASE2A.md](./WEBVIEW_PHASE2A.md)
