# WebView GUI Roadmap (Phase 2)

Platform-native WebView embedding for VST3 plugin GUIs.

## Approach

Direct platform APIs (not `wry`) — VST3 requires attaching to host-provided window handles.

| Platform | Backend | Crate | Phase |
|----------|---------|-------|-------|
| macOS | WKWebView | `objc2` + `icrate` | 2A |
| Windows | WebView2 | `windows` | 2A |
| Linux | TBD | TBD | 2D |

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
- Invoke pattern (JS → Rust)
- Event emission (Rust → JS)
- Parameter synchronization

### Phase 2D: Linux Support
- Evaluate GTK conflict mitigation
- IPC-isolated process vs WebKitGTK

## Crate Structure

```
beamer-webview/
├── src/
│   ├── lib.rs
│   ├── platform/
│   │   ├── macos.rs
│   │   ├── windows.rs
│   │   └── linux.rs
│   ├── view.rs         # IPlugView wrapper
│   └── resources.rs
```

## References

- [REFERENCE.md §4](./REFERENCE.md#4-future-phases) — WebView API details
- [VST3 IPlugView](https://steinbergmedia.github.io/vst3_doc/base/classSteinberg_1_1IPlugView.html)
- [vstwebview](https://github.com/rdaum/vstwebview)

## Status

**Current**: Planning
**Next**: Phase 2A — [WEBVIEW_PHASE2A.md](./WEBVIEW_PHASE2A.md)
