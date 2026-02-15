# Phase 2A: Core Platform Support

WebView windows showing static HTML in VST3 plugins on macOS and Windows.

## Crate Structure

```
crates/beamer-webview/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── view.rs                  # IPlugView wrapper
│   ├── error.rs
│   └── platform/
│       ├── mod.rs
│       ├── macos.rs             # WKWebView
│       └── windows.rs           # WebView2
```

## Dependencies

```toml
[dependencies]
beamer-core = { workspace = true }
log = { workspace = true }
vst3 = { workspace = true }

[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-foundation = { version = "0.3", features = ["NSString"] }
objc2-app-kit = { version = "0.3", features = ["NSView"] }
objc2-web-kit = { version = "0.3", features = ["WKWebView", "WKWebViewConfiguration"] }

[target.'cfg(target_os = "windows")'.dependencies]
webview2-com = "0.38"
windows = { version = "0.62", features = [
    "Win32_Foundation",
    "Win32_System_Com",
    "Win32_UI_WindowsAndMessaging",
] }
```

### Note on objc2

The AU wrapper tried `objc2` and abandoned it due to `define_class!` ABI
incompatibility with `AUAudioUnit` subclassing. The WebView case is different -
we only *instantiate and configure* `WKWebView`, not subclass it. Tauri's `wry`
crate uses `objc2` 0.6 with `objc2-web-kit` 0.3 for WKWebView in production.

## Public API

```rust
pub use view::WebViewPlugView;
pub use error::{WebViewError, Result};

pub struct WebViewConfig {
    pub html: &'static str,
    pub dev_tools: bool,
}
```

## Platform Implementations

### macOS (`platform/macos.rs`)

```rust
pub struct MacosWebView {
    webview: Retained<WKWebView>,
    parent: Retained<NSView>,
}

impl MacosWebView {
    pub unsafe fn attach_to_parent(parent: *mut c_void, config: &WebViewConfig) -> Result<Self>;
    pub fn set_frame(&mut self, x: i32, y: i32, width: i32, height: i32);
    pub fn detach(&mut self);
}
```

**Notes**:
- Parent is `NSView*` (not `NSWindow*`) - VST3 hosts provide an `NSView`
- Coordinate origin: bottom-left
- WKWebView must be created on main thread (VST3 hosts typically call
  `IPlugView::attached()` on the main thread, but verify per-host)
- `objc2` 0.6 uses `Retained<T>` (formerly `Id<T>`)

### Windows (`platform/windows.rs`)

```rust
pub struct WindowsWebView {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
}

impl WindowsWebView {
    pub unsafe fn attach_to_parent(parent: *mut c_void, config: &WebViewConfig) -> Result<Self>;
    pub fn set_bounds(&mut self, x: i32, y: i32, width: i32, height: i32);
    pub fn detach(&mut self);
}
```

**Notes**:
- WebView2 initialization is async (callback-based via
  `CreateCoreWebView2EnvironmentWithOptions`). Need to bridge this into the
  synchronous `IPlugView::attached()` return - likely by pumping the message
  loop or using a completion event.
- Runtime required (built into Win11, separate install for Win10)
- Parent is `HWND`
- Use `webview2-com` crate instead of raw `windows` features for WebView2 APIs

## IPlugView Wrapper (`view.rs`)

The wrapper implements `IPlugViewTrait` from the `vst3` crate, matching how
`Vst3Processor` implements other VST3 COM traits (e.g., `IEditControllerTrait`,
`IAudioProcessorTrait`).

```rust
pub struct WebViewPlugView {
    #[cfg(target_os = "macos")]
    platform: Option<MacosWebView>,
    #[cfg(target_os = "windows")]
    platform: Option<WindowsWebView>,

    config: WebViewConfig,
    constraints: EditorConstraints,
    size: Size,
    frame: Option<*mut IPlugFrame>,
}

impl IPlugViewTrait for WebViewPlugView {
    unsafe fn isPlatformTypeSupported(&self, type_: FIDString) -> tresult;
    unsafe fn attached(&mut self, parent: *mut c_void, type_: FIDString) -> tresult;
    unsafe fn removed(&mut self) -> tresult;
    unsafe fn onSize(&mut self, new_size: *mut ViewRect) -> tresult;
    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult;
    unsafe fn canResize(&self) -> tresult; // uses EditorConstraints.resizable
    unsafe fn setFrame(&mut self, frame: *mut IPlugFrame) -> tresult;
}
```

### EditorDelegate integration

`WebViewPlugView::new()` takes an `EditorDelegate` reference to:
- Get initial size via `editor_size()`
- Get constraints via `editor_constraints()` (used by `canResize`)
- Call `editor_opened()` in `attached()`
- Call `editor_closed()` in `removed()`
- Call `editor_resized()` in `onSize()`

## beamer-vst3 Integration

Update `Vst3Processor::createView()` in [processor.rs:2275](../crates/beamer-vst3/src/processor.rs#L2275):

```rust
unsafe fn createView(&self, name: *const c_char) -> *mut IPlugView {
    let name_str = std::ffi::CStr::from_ptr(name).to_str().unwrap_or("");
    if name_str != "editor" || !self.config.has_editor {
        return std::ptr::null_mut();
    }

    // TODO: get html content and EditorDelegate from plugin
    let config = beamer_webview::WebViewConfig { html, dev_tools: false };
    let constraints = /* from EditorDelegate */;
    let size = /* from EditorDelegate */;
    match beamer_webview::WebViewPlugView::new(config, size, constraints) {
        Ok(view) => /* return as COM pointer via vst3 crate machinery */,
        Err(e) => {
            log::error!("Failed to create WebView: {:?}", e);
            std::ptr::null_mut()
        }
    }
}
```

**Open question**: The exact COM pointer return mechanism needs to match how the
`vst3` crate expects `IPlugView` implementors to be returned. Investigate how
`Vst3Processor` itself is exposed as a COM object to determine the right
pattern (likely via `Class` / `ComRef` from the `vst3` crate).

## Example Plugin

```rust
static CONFIG: Config = Config::new("WebView Demo")
    .with_vendor("Beamer")
    .with_editor();
```

## Tasks

- [ ] Create `beamer-webview` crate with Cargo.toml
- [ ] Implement `MacosWebView` (attach, resize, detach)
- [ ] Implement `WindowsWebView` (attach with async bridging, resize, detach)
- [ ] Implement `WebViewPlugView` (IPlugView) with `EditorDelegate` integration
- [ ] Wire up `Vst3Processor::createView()` (COM pointer return mechanism)
- [ ] Create example plugin

## References

- [objc2](https://docs.rs/objc2/) / [objc2-web-kit](https://docs.rs/objc2-web-kit/)
- [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview)
- [webview2-com](https://github.com/wravery/webview2-rs) / [WebView2 docs](https://learn.microsoft.com/en-us/microsoft-edge/webview2/)
- [Tauri wry](https://github.com/tauri-apps/wry) - reference implementation using objc2 + WKWebView
- [VST3 IPlugView](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+Module+Architecture/IPlugView.html)
- [vstwebview](https://github.com/rdaum/vstwebview)
