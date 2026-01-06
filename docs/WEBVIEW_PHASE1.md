# WebView Phase 1: Core Platform Support

Detailed implementation plan for Phase 1 of WebView GUI integration.

**Goal**: Get WebView windows showing static HTML in VST3 plugins on macOS and Windows.

## Objectives

1. Create `beamer-webview` crate with platform-native implementations
2. Implement `IPlugView` for macOS (WKWebView) and Windows (WebView2)
3. Integrate with existing `EditorDelegate` trait
4. Update `Vst3Processor::createView()` to return WebView instance
5. Build working example plugin demonstrating WebView GUI

## Architecture

### Crate Structure

```
crates/beamer-webview/
├── Cargo.toml
├── build.rs                     # Platform-specific build config
└── src/
    ├── lib.rs                   # Public API, platform selection
    ├── view.rs                  # IPlugView wrapper (shared)
    ├── error.rs                 # Error types
    └── platform/
        ├── mod.rs               # Platform abstraction
        ├── macos.rs             # macOS WKWebView implementation
        └── windows.rs           # Windows WebView2 implementation
```

### Dependencies

**Cargo.toml**:
```toml
[dependencies]
beamer-core = { workspace = true }
log = { workspace = true }
vst3 = { workspace = true }

[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.5"
objc2-foundation = { version = "0.2", features = ["NSString", "NSBundle"] }
objc2-app-kit = { version = "0.2", features = ["NSView"] }
objc2-web-kit = { version = "0.2", features = ["WKWebView", "WKWebViewConfiguration", "WKNavigationDelegate"] }

[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.58", features = [
    "Win32_Foundation",
    "Win32_UI_WindowsAndMessaging",
    "Win32_Web_WebView2",
] }
```

## Implementation Tasks

### 1. Crate Setup
**Files**: `Cargo.toml`, `src/lib.rs`

- [ ] Create `crates/beamer-webview/` directory
- [ ] Add to workspace members in root `Cargo.toml`
- [ ] Set up platform-conditional compilation
- [ ] Define public API surface

**Public API** (`lib.rs`):
```rust
pub use view::WebViewPlugView;
pub use error::{WebViewError, Result};

/// Configuration for WebView creation
pub struct WebViewConfig {
    /// Initial HTML content
    pub html: String,
    /// Whether to enable dev tools (macOS/Windows)
    pub dev_tools: bool,
}

impl WebViewConfig {
    pub fn with_html(html: impl Into<String>) -> Self;
}
```

### 2. Error Handling
**Files**: `src/error.rs`

- [ ] Define `WebViewError` enum covering platform-specific errors
- [ ] Implement conversions to/from `tresult` (VST3)
- [ ] Add logging integration

```rust
#[derive(Debug)]
pub enum WebViewError {
    PlatformNotSupported,
    InvalidParentWindow,
    CreationFailed(String),
    // Platform-specific variants
}

pub type Result<T> = std::result::Result<T, WebViewError>;
```

### 3. macOS Implementation
**Files**: `src/platform/macos.rs`

**Subtasks**:

- [ ] **3.1**: Create `MacOSWebView` struct wrapping `Id<WKWebView>`
- [ ] **3.2**: Implement `attach_to_parent()` - cast `void*` to `NSView*`, create WKWebView, add as subview
- [ ] **3.3**: Implement `load_html()` - call `loadHTMLString:baseURL:`
- [ ] **3.4**: Implement resize handling - update WKWebView frame
- [ ] **3.5**: Implement `detach()` - remove from superview, cleanup
- [ ] **3.6**: Memory management - proper `retain`/`release` via `objc2`

**Key APIs**:
```rust
pub struct MacOSWebView {
    webview: Id<WKWebView>,
    parent: Id<NSView>,
}

impl MacOSWebView {
    pub unsafe fn attach_to_parent(
        parent: *mut c_void,
        config: &WebViewConfig,
    ) -> Result<Self>;

    pub fn set_frame(&mut self, x: i32, y: i32, width: i32, height: i32);
    pub fn detach(&mut self);
}
```

**Technical Details**:
- VST3 parent is `NSView*`, not `NSWindow*`
- WKWebView must be added to parent's view hierarchy
- Frame coordinates are in parent's coordinate system
- Retain parent to prevent premature deallocation

### 4. Windows Implementation
**Files**: `src/platform/windows.rs`

**Subtasks**:

- [ ] **4.1**: Create `WindowsWebView` struct wrapping WebView2 controller
- [ ] **4.2**: Implement `attach_to_parent()` - cast `void*` to `HWND`, create WebView2 environment
- [ ] **4.3**: Implement async WebView2 initialization (CreateCoreWebView2EnvironmentWithOptions)
- [ ] **4.4**: Implement `load_html()` - call `NavigateToString()`
- [ ] **4.5**: Implement resize handling - update WebView2 bounds
- [ ] **4.6**: Implement `detach()` - close controller, cleanup
- [ ] **4.7**: Handle WebView2 runtime detection/installation

**Key APIs**:
```rust
pub struct WindowsWebView {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
}

impl WindowsWebView {
    pub unsafe fn attach_to_parent(
        parent: *mut c_void,
        config: &WebViewConfig,
    ) -> Result<Self>;

    pub fn set_bounds(&mut self, x: i32, y: i32, width: i32, height: i32);
    pub fn detach(&mut self);
}
```

**Technical Details**:
- WebView2 requires async initialization (may not complete before `attached()` returns)
- Show loading state until WebView2 ready
- Check for WebView2 runtime, fail gracefully if missing
- Handle window messages for proper integration

### 5. IPlugView Wrapper
**Files**: `src/view.rs`

**Subtasks**:

- [ ] **5.1**: Create `WebViewPlugView` struct implementing `IPlugViewTrait`
- [ ] **5.2**: Implement `isPlatformTypeSupported()` - check for platform-specific types
- [ ] **5.3**: Implement `attached()` - create platform WebView, pass parent handle
- [ ] **5.4**: Implement `removed()` - cleanup WebView
- [ ] **5.5**: Implement `onSize()` - resize WebView
- [ ] **5.6**: Implement `getSize()` - return current size from `EditorDelegate`
- [ ] **5.7**: Implement `canResize()` - check constraints from `EditorDelegate`
- [ ] **5.8**: Implement `setFrame()` - store `IPlugFrame` reference
- [ ] **5.9**: Add `EditorDelegate` integration - call lifecycle hooks
- [ ] **5.10**: Handle thread safety - WebView operations must be on UI thread

**Structure**:
```rust
pub struct WebViewPlugView {
    #[cfg(target_os = "macos")]
    platform: Option<MacOSWebView>,

    #[cfg(target_os = "windows")]
    platform: Option<WindowsWebView>,

    config: WebViewConfig,
    size: Size,
    frame: Option<*mut IPlugFrame>,
    // TODO: Add EditorDelegate reference in Phase 1.5
}

impl IPlugViewTrait for WebViewPlugView {
    unsafe fn isPlatformTypeSupported(&self, type_: FIDString) -> tresult {
        #[cfg(target_os = "macos")]
        if type_ == kPlatformTypeNSView { return kResultOk; }

        #[cfg(target_os = "windows")]
        if type_ == kPlatformTypeHWND { return kResultOk; }

        kResultFalse
    }

    unsafe fn attached(&mut self, parent: *mut c_void, type_: FIDString) -> tresult {
        // Platform-specific attachment
    }

    // ... other methods
}
```

### 6. Integration with beamer-vst3
**Files**: `crates/beamer-vst3/src/processor.rs`

**Subtasks**:

- [ ] **6.1**: Add `beamer-webview` dependency to `beamer-vst3/Cargo.toml`
- [ ] **6.2**: Update `Vst3Processor` to optionally hold `EditorDelegate` (trait object)
- [ ] **6.3**: Update `createView()` to create `WebViewPlugView` if `config.has_editor`
- [ ] **6.4**: Pass default HTML or config to `WebViewPlugView`
- [ ] **6.5**: Handle `createView()` being called multiple times (ref counting?)

**Changes to `processor.rs`**:
```rust
// Line ~1812 - Update createView implementation
unsafe fn createView(&self, name: *const c_char) -> *mut IPlugView {
    if name.is_null() {
        return std::ptr::null_mut();
    }

    let name_str = std::ffi::CStr::from_ptr(name).to_str().unwrap_or("");
    if name_str != "editor" {
        return std::ptr::null_mut();
    }

    // Check if plugin has editor enabled
    if !self.config.has_editor {
        return std::ptr::null_mut();
    }

    // Create WebView with default HTML
    let html = "<html><body><h1>Beamer Plugin</h1></body></html>".to_string();
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

### 7. Example Plugin
**Files**: `examples/webview-demo/`

**Subtasks**:

- [ ] **7.1**: Create new example plugin `examples/webview-demo/`
- [ ] **7.2**: Simple gain plugin with WebView UI
- [ ] **7.3**: Static HTML showing plugin name and basic info
- [ ] **7.4**: Configure `PluginConfig::with_editor()`
- [ ] **7.5**: Test in DAW (Logic Pro, Ableton, Reaper)
- [ ] **7.6**: Document any platform-specific quirks

**Example structure**:
```rust
static CONFIG: PluginConfig = PluginConfig::new("WebView Demo", UID)
    .with_vendor("Beamer")
    .with_category("Fx")
    .with_editor(); // Enable WebView

impl Plugin for WebViewDemo {
    // ... standard implementation
}

// Static HTML embedded in binary
const EDITOR_HTML: &str = r#"
<!DOCTYPE html>
<html>
<head>
    <title>WebView Demo</title>
    <style>
        body {
            font-family: system-ui;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
        }
    </style>
</head>
<body>
    <div>
        <h1>🎵 Beamer WebView</h1>
        <p>Phase 1: Platform Integration</p>
    </div>
</body>
</html>
"#;
```

### 8. Testing & Validation

**Test Cases**:

- [ ] **8.1**: Open/close editor multiple times - no leaks
- [ ] **8.2**: Resize editor - WebView scales correctly
- [ ] **8.3**: Load plugin in Logic Pro (macOS)
- [ ] **8.4**: Load plugin in Ableton Live (macOS/Windows)
- [ ] **8.5**: Load plugin in Reaper (macOS/Windows)
- [ ] **8.6**: Multiple instances - no interference
- [ ] **8.7**: Remove plugin while editor open - graceful cleanup
- [ ] **8.8**: Host resize constraints - honor min/max

**Validation Checklist**:

- [ ] No crashes on editor open/close
- [ ] No memory leaks (use Instruments on macOS)
- [ ] HTML renders correctly
- [ ] Window is properly sized
- [ ] Resize handles work (if enabled)
- [ ] Works in at least 2 DAWs per platform

## Platform-Specific Considerations

### macOS

**Platform Types**:
- `kPlatformTypeNSView` - Standard for macOS VST3

**Coordinate System**:
- Origin is bottom-left (unlike Windows top-left)
- Must convert if needed

**Threading**:
- WKWebView must be created on main thread
- VST3 `attached()` should be called on main thread by host
- If not, may need `dispatch_sync()` to main queue

**View Hierarchy**:
```
Host NSView (parent)
  └── WKWebView (child)
```

### Windows

**Platform Types**:
- `kPlatformTypeHWND` - Standard for Windows VST3

**Async Initialization**:
- WebView2 `CreateCoreWebView2Environment` is async
- May not be ready when `attached()` returns
- Show loading UI or blank window until ready

**Runtime Requirement**:
- WebView2 Runtime must be installed
- Built into Windows 11
- Separate installer for Windows 10
- Gracefully handle missing runtime

**Message Loop**:
- Host controls message loop
- WebView2 integrates via `ICoreWebView2Controller`

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Host doesn't call on UI thread | Crash | Document requirement, add thread assertions |
| WebView2 missing on Windows | No UI | Detect and show helpful error message |
| Memory leaks in Objective-C | Crash/slowdown | Use `objc2` smart pointers, test with Instruments |
| Multiple `createView()` calls | Resource leak | Track view lifetime, ref counting |
| Host doesn't respect size constraints | Layout issues | Clamp sizes, log warnings |

## Dependencies

- **Before starting**: None (standalone phase)
- **Blocks**: Phase 2 (resource loading needs working WebView)

## Success Criteria

Phase 1 is complete when:

1. ✅ `beamer-webview` crate compiles on macOS and Windows
2. ✅ Example plugin opens WebView window showing static HTML
3. ✅ Window resize works correctly
4. ✅ Window close and reopen works without crashes
5. ✅ No memory leaks detected
6. ✅ Tested in at least 2 DAWs per platform
7. ✅ Documentation covers basic usage

## Timeline Estimate

**Complexity**: Medium-High (FFI, platform APIs, VST3 integration)

**Suggested Breakdown**:
- Crate setup & error handling: ~4 hours
- macOS implementation: ~12 hours
- Windows implementation: ~12 hours
- IPlugView wrapper: ~8 hours
- beamer-vst3 integration: ~4 hours
- Example plugin: ~4 hours
- Testing & debugging: ~8 hours

**Total**: ~52 hours (6-7 full days)

**Note**: First-time platform API work may take longer. Budget extra time for debugging platform-specific issues.

## Next Steps After Phase 1

Once Phase 1 is complete:

1. **Phase 1.5** (optional): Add `EditorDelegate` integration for lifecycle hooks
2. **Phase 2**: Implement resource loading (embedded assets, dev server)
3. **Update docs**: Add WebView usage guide to REFERENCE.md
4. **Community feedback**: Share example, gather feedback on API

## References

- [objc2 documentation](https://docs.rs/objc2/)
- [WKWebView Apple docs](https://developer.apple.com/documentation/webkit/wkwebview)
- [windows-rs documentation](https://docs.rs/windows/)
- [WebView2 API Reference](https://learn.microsoft.com/en-us/microsoft-edge/webview2/)
- [VST3 IPlugView documentation](https://steinbergmedia.github.io/vst3_dev_portal/pages/Technical+Documentation/VST+Module+Architecture/IPlugView.html)
- [vstwebview reference implementation](https://github.com/rdaum/vstwebview)
