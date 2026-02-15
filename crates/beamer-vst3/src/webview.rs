//! IPlugView wrapper for WebView editors.

use std::cell::UnsafeCell;
use std::ffi::c_void;

use beamer_core::{EditorConstraints, EditorDelegate, Size};
use beamer_webview::platform::PlatformWebView;
pub use beamer_webview::WebViewConfig;
use vst3::Steinberg::*;
use vst3::Class;

/// VST3 IPlugView implementation backed by a platform WebView.
pub struct WebViewPlugView {
    platform: UnsafeCell<Option<PlatformWebView>>,
    config: WebViewConfig,
    delegate: UnsafeCell<Box<dyn EditorDelegate>>,
    size: UnsafeCell<Size>,
    frame: UnsafeCell<*mut IPlugFrame>,
}

// SAFETY: VST3 IPlugView methods are called from the UI thread only.
unsafe impl Send for WebViewPlugView {}
// SAFETY: VST3 IPlugView methods are called from the UI thread only.
unsafe impl Sync for WebViewPlugView {}

impl WebViewPlugView {
    /// Create a new WebView plug view with the given delegate.
    ///
    /// Initial size is obtained from `delegate.editor_size()`.
    pub fn new(config: WebViewConfig, delegate: Box<dyn EditorDelegate>) -> Self {
        let size = delegate.editor_size();
        Self {
            platform: UnsafeCell::new(None),
            config,
            delegate: UnsafeCell::new(delegate),
            size: UnsafeCell::new(size),
            frame: UnsafeCell::new(std::ptr::null_mut()),
        }
    }
}

impl Class for WebViewPlugView {
    type Interfaces = (IPlugView,);
}

#[allow(non_snake_case)]
impl IPlugViewTrait for WebViewPlugView {
    unsafe fn isPlatformTypeSupported(&self, r#type: FIDString) -> tresult {
        if r#type.is_null() {
            return kResultFalse;
        }
        // SAFETY: type_ is non-null and host provides null-terminated C string.
        let type_str = unsafe { std::ffi::CStr::from_ptr(r#type) };

        #[cfg(target_os = "macos")]
        // SAFETY: kPlatformTypeNSView is a static null-terminated byte literal.
        let supported = type_str == unsafe { std::ffi::CStr::from_ptr(kPlatformTypeNSView) };

        #[cfg(target_os = "windows")]
        // SAFETY: kPlatformTypeHWND is a static null-terminated byte literal.
        let supported = type_str == unsafe { std::ffi::CStr::from_ptr(kPlatformTypeHWND) };

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let supported = false;

        if supported { kResultOk } else { kResultFalse }
    }

    unsafe fn attached(&self, parent: *mut c_void, r#type: FIDString) -> tresult {
        // SAFETY: r#type is forwarded from the host and we are in an unsafe fn.
        if unsafe { self.isPlatformTypeSupported(r#type) } != kResultOk {
            return kResultFalse;
        }

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let platform = unsafe { &mut *self.platform.get() };
        if platform.is_some() {
            return kResultFalse;
        }

        // SAFETY: parent is a valid platform handle provided by the host.
        match unsafe { PlatformWebView::attach_to_parent(parent, &self.config) } {
            Ok(webview) => {
                *platform = Some(webview);
                // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
                let delegate = unsafe { &mut *self.delegate.get() };
                delegate.editor_opened();
                kResultOk
            }
            Err(e) => {
                log::error!("Failed to create WebView: {e}");
                kResultFalse
            }
        }
    }

    unsafe fn removed(&self) -> tresult {
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let delegate = unsafe { &mut *self.delegate.get() };
        delegate.editor_closed();

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let platform = unsafe { &mut *self.platform.get() };
        if let Some(webview) = platform.as_mut() {
            webview.detach();
        }
        *platform = None;
        kResultOk
    }

    unsafe fn onWheel(&self, _distance: f32) -> tresult {
        kResultFalse // Let WebView handle scroll events
    }

    unsafe fn onKeyDown(&self, _key: char16, _keyCode: int16, _modifiers: int16) -> tresult {
        kResultFalse // Let WebView handle key events
    }

    unsafe fn onKeyUp(&self, _key: char16, _keyCode: int16, _modifiers: int16) -> tresult {
        kResultFalse // Let WebView handle key events
    }

    unsafe fn getSize(&self, size: *mut ViewRect) -> tresult {
        if size.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let current = unsafe { *self.size.get() };
        // SAFETY: size is non-null (checked above) and host guarantees validity.
        let rect = unsafe { &mut *size };
        rect.left = 0;
        rect.top = 0;
        rect.right = current.width as i32;
        rect.bottom = current.height as i32;
        kResultOk
    }

    unsafe fn onSize(&self, newSize: *mut ViewRect) -> tresult {
        if newSize.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: newSize is non-null (checked above) and host guarantees validity.
        let rect = unsafe { &*newSize };
        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;

        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let size = unsafe { &mut *self.size.get() };
        size.width = width;
        size.height = height;

        // Notify delegate of resize
        let new_size = Size::new(width, height);
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let delegate = unsafe { &mut *self.delegate.get() };
        delegate.editor_resized(new_size);

        // Update platform webview frame
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let platform = unsafe { &*self.platform.get() };
        if let Some(webview) = platform.as_ref() {
            #[cfg(target_os = "macos")]
            webview.set_frame(0, 0, width as i32, height as i32);
            #[cfg(target_os = "windows")]
            webview.set_bounds(0, 0, width as i32, height as i32);
        }

        kResultOk
    }

    unsafe fn onFocus(&self, _state: TBool) -> tresult {
        kResultOk
    }

    unsafe fn setFrame(&self, frame: *mut IPlugFrame) -> tresult {
        let frame_ptr = self.frame.get();
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let old_frame = unsafe { *frame_ptr };

        // Release old frame reference
        if !old_frame.is_null() {
            // SAFETY: old_frame is a valid COM object. IPlugFrame inherits FUnknown,
            // so its vtbl starts with the FUnknownVtbl base.
            unsafe {
                let unknown = old_frame as *mut FUnknown;
                ((*(*unknown).vtbl).release)(unknown);
            };
        }

        // AddRef new frame
        if !frame.is_null() {
            // SAFETY: frame is a valid COM object provided by the host.
            unsafe {
                let unknown = frame as *mut FUnknown;
                ((*(*unknown).vtbl).addRef)(unknown);
            };
        }

        // SAFETY: Single-threaded access guaranteed by VST3.
        unsafe { *frame_ptr = frame };
        kResultOk
    }

    unsafe fn canResize(&self) -> tresult {
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let delegate = unsafe { &*self.delegate.get() };
        if delegate.editor_constraints().resizable { kResultOk } else { kResultFalse }
    }

    unsafe fn checkSizeConstraint(&self, rect: *mut ViewRect) -> tresult {
        if rect.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: VST3 guarantees single-threaded access for IPlugView methods.
        let delegate = unsafe { &*self.delegate.get() };
        let constraints = delegate.editor_constraints();

        // SAFETY: rect is non-null (checked above) and host guarantees validity.
        let r = unsafe { &mut *rect };
        let mut width = (r.right - r.left).max(0) as u32;
        let mut height = (r.bottom - r.top).max(0) as u32;

        width = width.clamp(constraints.min.width, constraints.max.width);
        height = height.clamp(constraints.min.height, constraints.max.height);

        r.right = r.left + width as i32;
        r.bottom = r.top + height as i32;
        kResultOk
    }
}

// Release IPlugFrame reference when dropped.
impl Drop for WebViewPlugView {
    fn drop(&mut self) {
        let frame = *self.frame.get_mut();
        if !frame.is_null() {
            // SAFETY: frame is a valid COM object. We hold a reference from setFrame.
            unsafe {
                let unknown = frame as *mut FUnknown;
                ((*(*unknown).vtbl).release)(unknown);
            }
        }
    }
}

/// Simple `EditorDelegate` backed by fixed size and constraints.
///
/// Used when the plugin doesn't provide its own delegate (the common case
/// for Config-driven editor setup).
pub struct StaticEditorDelegate {
    size: Size,
    constraints: EditorConstraints,
}

impl StaticEditorDelegate {
    /// Create a new static delegate with the given size and constraints.
    pub fn new(size: Size, constraints: EditorConstraints) -> Self {
        Self { size, constraints }
    }
}

impl EditorDelegate for StaticEditorDelegate {
    fn editor_size(&self) -> Size {
        self.size
    }

    fn editor_constraints(&self) -> EditorConstraints {
        self.constraints
    }
}
