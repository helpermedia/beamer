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
objc2 = "0.5"
objc2-foundation = { version = "0.2", features = ["NSString", "NSBundle"] }
objc2-app-kit = { version = "0.2", features = ["NSView"] }
objc2-web-kit = { version = "0.2", features = ["WKWebView", "WKWebViewConfiguration"] }

[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Web_WebView2",
] }
```

## Public API

```rust
pub use view::WebViewPlugView;
pub use error::{WebViewError, Result};

pub struct WebViewConfig {
    pub html: String,
    pub dev_tools: bool,
}
```

## Platform Implementations

### macOS (`platform/macos.rs`)

```rust
pub struct MacOSWebView {
    webview: Id<WKWebView>,
    parent: Id<NSView>,
}

impl MacOSWebView {
    pub unsafe fn attach_to_parent(parent: *mut c_void, config: &WebViewConfig) -> Result<Self>;
    pub fn set_frame(&mut self, x: i32, y: i32, width: i32, height: i32);
    pub fn detach(&mut self);
}
```

**Notes**:
- Parent is `NSView*` (not `NSWindow*`)
- Coordinate origin: bottom-left
- WKWebView must be created on main thread

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
- WebView2 initialization is async
- Runtime required (built into Win11, separate install for Win10)
- Parent is `HWND`

## IPlugView Wrapper (`view.rs`)

```rust
pub struct WebViewPlugView {
    #[cfg(target_os = "macos")]
    platform: Option<MacOSWebView>,
    #[cfg(target_os = "windows")]
    platform: Option<WindowsWebView>,

    config: WebViewConfig,
    size: Size,
    frame: Option<*mut IPlugFrame>,
}

impl IPlugViewTrait for WebViewPlugView {
    unsafe fn isPlatformTypeSupported(&self, type_: FIDString) -> tresult;
    unsafe fn attached(&mut self, parent: *mut c_void, type_: FIDString) -> tresult;
    unsafe fn removed(&mut self) -> tresult;
    unsafe fn onSize(&mut self, new_size: *mut ViewRect) -> tresult;
    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult;
    unsafe fn canResize(&self) -> tresult;
    unsafe fn setFrame(&mut self, frame: *mut IPlugFrame) -> tresult;
}
```

## beamer-vst3 Integration

Update `Vst3Processor::createView()`:

```rust
unsafe fn createView(&self, name: *const c_char) -> *mut IPlugView {
    let name_str = std::ffi::CStr::from_ptr(name).to_str().unwrap_or("");
    if name_str != "editor" || !self.config.has_editor {
        return std::ptr::null_mut();
    }

    let config = beamer_webview::WebViewConfig::with_html(html);
    match beamer_webview::WebViewPlugView::new(config) {
        Ok(view) => Box::into_raw(Box::new(view)) as *mut IPlugView,
        Err(e) => {
            log::error!("Failed to create WebView: {:?}", e);
            std::ptr::null_mut()
        }
    }
}
```

## Example Plugin

```rust
static CONFIG: PluginConfig = PluginConfig::new("WebView Demo")
    .with_vendor("Beamer")
    .with_editor();
```

## Tasks

- [ ] Create `beamer-webview` crate
- [ ] Implement `MacOSWebView`
- [ ] Implement `WindowsWebView`
- [ ] Implement `WebViewPlugView` (IPlugView)
- [ ] Update `Vst3Processor::createView()`
- [ ] Create example plugin

## References

- [objc2](https://docs.rs/objc2/) / [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview)
- [windows-rs](https://docs.rs/windows/) / [WebView2](https://learn.microsoft.com/en-us/microsoft-edge/webview2/)
- [VST3 IPlugView](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+Module+Architecture/IPlugView.html)
- [vstwebview](https://github.com/rdaum/vstwebview)
