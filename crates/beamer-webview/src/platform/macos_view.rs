//! The WKWebView subclass of the editor.
//!
//! A click in a window that is not active normally only activates it: the
//! view under the mouse does not get the click unless it accepts the first
//! mouse event. A host's plugin window is often not active (Logic Pro opens
//! it without activating it), so a button in the web UI would need two
//! clicks. The subclass accepts the first click, as native plugin views do.
//!
//! The class is registered at runtime under a name that is unique to this
//! binary (see `macos_class.rs`).

use objc2::rc::Allocated;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{msg_send, sel, ClassType, MainThreadMarker};
use objc2_web_kit::WKWebView;

use crate::platform::macos_class::class_name;

/// Get or register the WKWebView subclass of this binary.
fn web_view_class() -> &'static AnyClass {
    let c_name = class_name("BeamerWebView", accepts_first_mouse as *const () as usize);
    let c_name = c_name.as_c_str();

    if let Some(existing) = AnyClass::get(c_name) {
        return existing;
    }

    let superclass = WKWebView::class();
    let mut builder = match ClassBuilder::new(c_name, superclass) {
        Some(b) => b,
        None => {
            return AnyClass::get(c_name)
                .expect("class must exist after ClassBuilder::new returned None");
        }
    };

    // SAFETY: the signature matches NSView's `acceptsFirstMouse:`.
    unsafe {
        builder.add_method(
            sel!(acceptsFirstMouse:),
            accepts_first_mouse
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyObject) -> Bool,
        );
    }

    builder.register()
}

/// `acceptsFirstMouse:` implementation: every click reaches the web UI,
/// also the one that activates the window.
unsafe extern "C-unwind" fn accepts_first_mouse(
    _this: *mut AnyObject,
    _cmd: Sel,
    _event: *const AnyObject,
) -> Bool {
    Bool::YES
}

/// Allocate a web view of the subclass, to be initialized like a WKWebView.
pub fn alloc_web_view(_mtm: MainThreadMarker) -> Allocated<WKWebView> {
    // SAFETY: the class is a subclass of WKWebView, so the object is a
    // WKWebView. The marker proves that this runs on the main thread, where
    // a view must be created.
    unsafe { msg_send![web_view_class(), alloc] }
}
