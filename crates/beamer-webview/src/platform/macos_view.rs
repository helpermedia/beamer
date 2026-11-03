//! The WKWebView subclass of the editor.
//!
//! A click in a window that is not active normally only activates it: the
//! view under the mouse does not get the click unless it accepts the first
//! mouse event. A host's plugin window is often not active (Logic Pro opens
//! it without activating it), so a button in the web UI would need two
//! clicks. The subclass accepts the first click, as native plugin views do.
//!
//! A right click or a Control click on a page that does not cancel the
//! `contextmenu` event opens the menu of WebKit: Reload, plus Copy, Look Up,
//! Translate and Share over text. A plugin editor offers none of these.
//! Reload also throws away what only the page knew. The subclass closes that
//! menu before it shows. With dev tools on, it keeps Reload and Inspect
//! Element. A page with a menu of its own cancels the event, so WebKit never
//! builds its menu.
//!
//! The class is registered at runtime under a name that is unique to this
//! binary (see `macos_class.rs`).

use std::ffi::CStr;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{msg_send, sel, ClassType, MainThreadMarker};
use objc2_foundation::NSString;
use objc2_web_kit::WKWebView;

use crate::platform::macos_class::class_name;

/// Ivar name for whether dev tools are on in this web view.
const DEV_TOOLS_IVAR: &CStr = c"_beamerDevTools";

/// Menu items of WebKit that stay with dev tools on.
const DEV_TOOLS_MENU_ITEMS: [&str; 2] = [
    "WKMenuItemIdentifierReload",
    "WKMenuItemIdentifierInspectElement",
];

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

    builder.add_ivar::<Bool>(DEV_TOOLS_IVAR);

    // SAFETY: the signatures match NSView's `acceptsFirstMouse:` and
    // `willOpenMenu:withEvent:`.
    unsafe {
        builder.add_method(
            sel!(acceptsFirstMouse:),
            accepts_first_mouse
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *const AnyObject) -> Bool,
        );
        builder.add_method(
            sel!(willOpenMenu:withEvent:),
            will_open_menu
                as unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, *mut AnyObject),
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

/// `willOpenMenu:withEvent:` implementation: closes WebKit's context menu,
/// or keeps only its dev tools items when dev tools are on.
unsafe extern "C-unwind" fn will_open_menu(
    this: *mut AnyObject,
    _cmd: Sel,
    menu: *mut AnyObject,
    event: *mut AnyObject,
) {
    // SAFETY: AppKit provides a valid receiver, an instance of the subclass.
    let this: &AnyObject = unsafe { &*this };
    // SAFETY: WKWebView implements `willOpenMenu:withEvent:` with this
    // signature; menu and event come from AppKit unchanged.
    let _: () = unsafe {
        msg_send![super(this, WKWebView::class()), willOpenMenu: menu, withEvent: event]
    };

    let dev_tools = web_view_class()
        .instance_variable(DEV_TOOLS_IVAR)
        // SAFETY: the receiver is an instance of the class that declares
        // the ivar as a Bool, written only by `set_dev_tools`.
        .is_some_and(|ivar| unsafe { ivar.load::<Bool>(this) }.as_bool());

    if !dev_tools {
        // The menu is already tracking the mouse when this runs: emptied but
        // not cancelled, it would stay open, unseen, until the next click.
        // SAFETY: menu is a valid NSMenu on the main thread.
        unsafe {
            let _: () = msg_send![menu, removeAllItems];
            let _: () = msg_send![menu, cancelTrackingWithoutAnimation];
        }
        return;
    }

    // SAFETY: menu is a valid NSMenu on the main thread. Going backwards
    // keeps the indices of the items still to visit.
    unsafe {
        let count: isize = msg_send![menu, numberOfItems];
        for index in (0..count).rev() {
            let item: *mut AnyObject = msg_send![menu, itemAtIndex: index];
            let identifier: Option<Retained<NSString>> = msg_send![item, identifier];
            let keep = identifier
                .is_some_and(|id| DEV_TOOLS_MENU_ITEMS.contains(&id.to_string().as_str()));
            if !keep {
                let _: () = msg_send![menu, removeItemAtIndex: index];
            }
        }
    }
}

/// Allocate a web view of the subclass, to be initialized like a WKWebView.
pub fn alloc_web_view(_mtm: MainThreadMarker) -> Allocated<WKWebView> {
    // SAFETY: the class is a subclass of WKWebView, so the object is a
    // WKWebView. The marker proves that this runs on the main thread, where
    // a view must be created.
    unsafe { msg_send![web_view_class(), alloc] }
}

/// Keep the menu items of dev tools in a web view of the subclass.
pub fn set_dev_tools(web_view: &WKWebView, _mtm: MainThreadMarker) {
    let ivar = web_view_class()
        .instance_variable(DEV_TOOLS_IVAR)
        .expect("dev tools ivar must exist");
    // SAFETY: the web view is an instance of the subclass, which has the
    // ivar as a Bool. The marker proves that this runs on the main thread,
    // where the menu reads it.
    unsafe { *ivar.load_ptr::<Bool>(web_view) = Bool::YES };
}
