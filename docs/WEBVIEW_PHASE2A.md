# Phase 2A: Core Platform Support

WebView windows showing static HTML in VST3 and AU plugins on macOS (and
Windows for VST3).

## Crate Structure

`beamer-webview` is a pure platform layer with no format-specific dependencies:

```
crates/beamer-webview/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── error.rs
│   └── platform/
│       ├── mod.rs
│       ├── macos.rs             # WKWebView
│       └── windows.rs           # WebView2
```

Format-specific integration lives in each format's own crate/template:
- VST3 `IPlugView` impl in `beamer-vst3` (uses `MacosWebView`/`WindowsWebView`)
- AUv3 view controller in `xtask/src/au_codegen/auv3_wrapper.m` (calls
  `beamer-webview` via C-ABI)
- AUv2 Cocoa UI view factory in `xtask/src/au_codegen/auv2_wrapper.c` (calls
  `beamer-webview` via C-ABI)

## Dependencies

```toml
# beamer-webview - no vst3 or AU dependencies
[dependencies]
beamer-core = { workspace = true }
log = { workspace = true }

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
pub use platform::PlatformWebView;
pub use error::{WebViewError, Result};

pub struct WebViewConfig {
    pub html: &'static str,
    pub dev_tools: bool,
}
```

The crate exposes the platform WebView types and config. Format-specific
wrappers (`IPlugView`, `NSViewController`) are built on top by each format
crate.

## Platform Implementations

### macOS (`platform/macos.rs`)

```rust
pub struct MacosWebView {
    webview: Retained<WKWebView>,
    parent: Retained<NSView>,
}

impl MacosWebView {
    pub unsafe fn attach_to_parent(parent: *mut c_void, config: &WebViewConfig) -> Result<Self>;
    pub fn set_frame(&self, x: i32, y: i32, width: i32, height: i32);
    pub fn detach(&mut self);
}
```

**Notes**:
- Parent is `NSView*` (not `NSWindow*`) - both VST3 and AU hosts provide an
  `NSView` as the parent container
- Coordinate origin: bottom-left
- WKWebView must be created on main thread (VST3 hosts typically call
  `IPlugView::attached()` on the main thread; AU hosts call
  `requestViewController` on the main thread)
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
- AU is macOS-only, so this backend is only used by VST3

### C-ABI exports (for AU, both AUv2 and AUv3)

`beamer-webview` exports C-ABI functions so both AU wrapper templates can use
the platform layer without reimplementing WKWebView setup:

```rust
/// Create a WebView attached to the given parent NSView.
/// Returns an opaque handle, or null on failure.
#[no_mangle]
pub extern "C" fn beamer_webview_create(
    parent: *mut c_void,
    html: *const c_char,
    dev_tools: bool,
) -> *mut c_void;

/// Update the WebView frame.
#[no_mangle]
pub extern "C" fn beamer_webview_set_frame(
    handle: *mut c_void,
    x: i32, y: i32, width: i32, height: i32,
);

/// Detach and destroy the WebView.
#[no_mangle]
pub extern "C" fn beamer_webview_destroy(handle: *mut c_void);
```

Both AU formats call into the same Rust platform code that VST3 uses, keeping
WebView creation logic in one place.

## beamer-vst3 Integration

The `IPlugView` implementation lives in `beamer-vst3`, using `beamer-webview`'s
platform types. This is analogous to how `Vst3Processor` implements other VST3
COM traits.

```rust
// In beamer-vst3/src/webview.rs (new file)
pub struct WebViewPlugView {
    #[cfg(target_os = "macos")]
    platform: UnsafeCell<Option<beamer_webview::MacosWebView>>,
    #[cfg(target_os = "windows")]
    platform: UnsafeCell<Option<beamer_webview::WindowsWebView>>,

    config: beamer_webview::WebViewConfig,
    delegate: UnsafeCell<Box<dyn EditorDelegate>>,
    size: UnsafeCell<Size>,
    frame: UnsafeCell<*mut IPlugFrame>,
}

impl IPlugViewTrait for WebViewPlugView {
    unsafe fn isPlatformTypeSupported(&self, type_: FIDString) -> tresult;
    unsafe fn attached(&self, parent: *mut c_void, type_: FIDString) -> tresult;
    unsafe fn removed(&self) -> tresult;
    unsafe fn onSize(&self, new_size: *mut ViewRect) -> tresult;
    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult;
    unsafe fn canResize(&self) -> tresult;
    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult;
}
```

### EditorDelegate integration

`WebViewPlugView::new()` takes an `EditorDelegate` to:
- Get initial size via `editor_size()`
- Get constraints via `editor_constraints()` (used by `canResize`)
- Call `editor_opened()` in `attached()`
- Call `editor_closed()` in `removed()`
- Call `editor_resized()` in `onSize()`

### createView() in Vst3Processor

Update `Vst3Processor::createView()` in [processor.rs:2275](../crates/beamer-vst3/src/processor.rs#L2275):

```rust
unsafe fn createView(&self, name: *const c_char) -> *mut IPlugView {
    let name_str = std::ffi::CStr::from_ptr(name).to_str().unwrap_or("");
    if name_str != "editor" || !self.config.has_editor {
        return std::ptr::null_mut();
    }

    let config = beamer_webview::WebViewConfig { html, dev_tools: cfg!(debug_assertions) };
    let delegate = Box::new(StaticEditorDelegate::new(size, constraints));
    let view = WebViewPlugView::new(config, delegate);
    // Return as COM pointer via vst3::ComWrapper
}
```

## beamer-au Integration

Both AUv3 and AUv2 use the same C-ABI bridge functions and the same
`beamer-webview` platform layer. The only difference is how the host requests
the editor view:

- **AUv3**: `requestViewControllerWithCompletionHandler:` returns an
  `NSViewController`
- **AUv2**: `kAudioUnitProperty_CocoaUI` returns an `AUCocoaUIBase` view
  factory class that provides an `NSView`

### C-ABI bridge additions (`BeamerAuBridge.h`)

New functions to expose editor config from the Rust instance:

```c
/// Whether the plugin has a custom editor.
bool beamer_au_has_editor(BeamerAuInstanceHandle instance);

/// Get the editor HTML content. Returns NULL if no editor.
const char* beamer_au_get_editor_html(BeamerAuInstanceHandle instance);

/// Get the initial editor size.
void beamer_au_get_editor_size(BeamerAuInstanceHandle instance,
                               uint32_t* width, uint32_t* height);
```

These read directly from the `Config` fields (`has_editor`, `editor_html`,
`editor_width`, `editor_height`) that are already populated by the
`#[beamer::export]` macro.

### AUv3 wrapper changes (`auv3_wrapper.m`)

Override `requestViewControllerWithCompletionHandler:` to provide an
`NSViewController` hosting a WebView created via `beamer-webview` C-ABI:

```objc
- (void)requestViewControllerWithCompletionHandler:
    (void (^)(NSViewController* _Nullable))completionHandler {
    if (!beamer_au_has_editor(_rustInstance)) {
        completionHandler(nil);
        return;
    }

    const char* html = beamer_au_get_editor_html(_rustInstance);
    if (html == NULL) {
        completionHandler(nil);
        return;
    }

    uint32_t width = 0, height = 0;
    beamer_au_get_editor_size(_rustInstance, &width, &height);

    NSViewController* vc = [[NSViewController alloc] init];
    NSView* container = [[NSView alloc]
        initWithFrame:NSMakeRect(0, 0, width, height)];
    vc.view = container;
    vc.preferredContentSize = NSMakeSize(width, height);

    // Create WebView via beamer-webview C-ABI (shared platform layer)
    void* webviewHandle = beamer_webview_create(
        (__bridge void*)container, html, /* dev_tools */ false);
    if (webviewHandle == NULL) {
        completionHandler(nil);
        return;
    }

    // Store handle for later cleanup
    _webviewHandle = webviewHandle;
    completionHandler(vc);
}
```

**Notes**:
- The completion handler is called synchronously here. AU hosts handle both
  sync and async responses.
- WebView creation goes through the same Rust `MacosWebView` code that VST3
  uses, via the `beamer_webview_create` C-ABI function. This ensures both
  formats share identical WebView setup, and Phase 2C IPC will work for both.
- Dev tools (`setInspectable:`) should be enabled in debug builds. The generated
  template can pass a bool or check a preprocessor flag.
- `autoresizingMask` is set inside `MacosWebView::attach_to_parent()`, so the
  AU host's view controller resizing works automatically.
- The wrapper stores the opaque handle and calls `beamer_webview_destroy` on
  dealloc.

### AUv2 wrapper changes (`auv2_wrapper.c`)

AUv2 provides custom UIs via the `kAudioUnitProperty_CocoaUI` property. The
host queries this property, gets a bundle URL and an ObjC class name that
conforms to `AUCocoaUIBase`, then calls `uiViewForAudioUnit:withSize:` to get
an `NSView`.

The generated wrapper needs:

1. An `AUCocoaUIBase`-conforming view factory class
2. Property handler for `kAudioUnitProperty_CocoaUI` returning
   `AudioUnitCocoaViewInfo` with the factory class
3. The factory's `uiViewForAudioUnit:withSize:` creates a container `NSView`
   and calls `beamer_webview_create` to attach a WebView

```objc
@interface BeamerCocoaViewFactory : NSObject <AUCocoaUIBase>
@end

@implementation BeamerCocoaViewFactory
- (NSView *)uiViewForAudioUnit:(AudioUnit)audioUnit
                      withSize:(NSSize)preferredSize {
    // Query editor config from Rust via bridge functions
    // Create container NSView
    // Call beamer_webview_create to attach WebView
    // Return the container view
}
@end
```

**Notes**:
- The view factory class is generated per-plugin (like the AUv3 extension class)
- WebView lifecycle: create in `uiViewForAudioUnit:withSize:`, destroy when the
  view is removed from its superview (override `viewDidMoveToWindow:` or use a
  weak reference pattern)
- Same `beamer_webview_*` C-ABI functions as AUv3, same Rust platform code
- `autoresizingMask` is set inside `MacosWebView::attach_to_parent()`, so host
  resizing works automatically

## Example Plugin

```rust
static CONFIG: Config = Config::new("WebView Demo")
    .with_vendor("Beamer")
    .with_editor();
```

## Tasks

### beamer-webview crate (platform layer)
- [ ] Create `beamer-webview` crate with Cargo.toml (no `vst3` dependency)
- [ ] Implement `MacosWebView` (attach, resize, detach)
- [ ] Implement `WindowsWebView` (attach with async bridging, resize, detach)
- [ ] Add C-ABI exports (`beamer_webview_create`, `beamer_webview_set_frame`, `beamer_webview_destroy`)

### VST3 integration (in beamer-vst3)
- [ ] Move `WebViewPlugView` (`IPlugView` impl) to `beamer-vst3/src/webview.rs`
- [ ] Wire up `Vst3Processor::createView()` (COM pointer return mechanism)

### AU integration (AUv3)
- [ ] Add C-ABI bridge functions (`beamer_au_has_editor`, `beamer_au_get_editor_html`, `beamer_au_get_editor_size`)
- [ ] Implement `requestViewControllerWithCompletionHandler:` in `auv3_wrapper.m`

### AU integration (AUv2)
- [ ] Add `AUCocoaUIBase` view factory class to `auv2_wrapper.c`
- [ ] Handle `kAudioUnitProperty_CocoaUI` in the property dispatcher
- [ ] Implement `uiViewForAudioUnit:withSize:` using `beamer_webview_create`

### Example & testing
- [ ] Create example plugin
- [ ] Verify in a VST3 host (e.g., REAPER)
- [ ] Verify in an AU host (e.g., Logic, GarageBand)

## References

- [objc2](https://docs.rs/objc2/) / [objc2-web-kit](https://docs.rs/objc2-web-kit/)
- [WKWebView](https://developer.apple.com/documentation/webkit/wkwebview)
- [webview2-com](https://github.com/wravery/webview2-rs) / [WebView2 docs](https://learn.microsoft.com/en-us/microsoft-edge/webview2/)
- [Tauri wry](https://github.com/tauri-apps/wry) - reference implementation using objc2 + WKWebView
- [VST3 IPlugView](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+Module+Architecture/IPlugView.html)
- [AUv3 requestViewController](https://developer.apple.com/documentation/audiotoolbox/auaudiounit/1583904-requestviewcontroller)
- [AUv2 kAudioUnitProperty_CocoaUI](https://developer.apple.com/documentation/audiotoolbox/kaudiounitproperty_cocoaui)
- [AUCocoaUIBase protocol](https://developer.apple.com/documentation/audiotoolbox/aucocoauibase)
- [vstwebview](https://github.com/rdaum/vstwebview)
