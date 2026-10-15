//! The component handler the host set, shared by the edit controller and
//! its editors.

use std::cell::UnsafeCell;

use vst3::Steinberg::FUnknown;
use vst3::Steinberg::Vst::IComponentHandler;

/// The `IComponentHandler` the host set last, with a reference of its own.
///
/// The edit controller and every editor it created hold the same slot, so
/// that a handler the host sets after an editor opened reaches that editor
/// too. The reference is released when the host sets another handler or
/// none, and when the last holder goes away.
///
/// The slot is used by controller calls and by the editors, which the host
/// makes on its UI thread and never concurrently.
pub struct ComponentHandler {
    handler: UnsafeCell<*mut IComponentHandler>,
}

// SAFETY: The slot is never used concurrently (see above). A host that
// creates a plugin on one thread and uses it on another hands it over with
// its own synchronization.
unsafe impl Send for ComponentHandler {}
// SAFETY: See above.
unsafe impl Sync for ComponentHandler {}

impl ComponentHandler {
    /// An empty slot.
    pub fn new() -> Self {
        Self {
            handler: UnsafeCell::new(std::ptr::null_mut()),
        }
    }

    /// Keep `handler`, or no handler for null, and release the one kept
    /// before.
    ///
    /// # Safety
    ///
    /// `handler` must be a valid COM pointer or null, and no other use of
    /// the slot may run at the same time.
    pub unsafe fn set(&self, handler: *mut IComponentHandler) {
        // SAFETY: handler is a valid COM pointer or null per the contract.
        unsafe { add_ref(handler) };
        // SAFETY: Nothing else uses the slot at the same time (see the
        // contract).
        let old = unsafe { std::mem::replace(&mut *self.handler.get(), handler) };
        // SAFETY: The slot added a reference to old when it was set.
        unsafe { release(old) };
    }

    /// The handler, or null. Valid until the next `set()`.
    pub fn get(&self) -> *mut IComponentHandler {
        // SAFETY: The slot is never used concurrently, so nothing writes it now.
        unsafe { *self.handler.get() }
    }
}

impl Default for ComponentHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ComponentHandler {
    fn drop(&mut self) {
        // SAFETY: The slot added a reference to the kept handler when it was
        // set.
        unsafe { release(*self.handler.get_mut()) };
    }
}

/// Add a reference to a COM object, if there is one.
///
/// # Safety
///
/// `handler` must be a valid COM pointer or null.
unsafe fn add_ref(handler: *mut IComponentHandler) {
    if !handler.is_null() {
        let unknown = handler as *mut FUnknown;
        // SAFETY: handler is non-null and a valid COM pointer per the contract.
        unsafe { ((*(*unknown).vtbl).addRef)(unknown) };
    }
}

/// Release a reference to a COM object, if there is one.
///
/// # Safety
///
/// `handler` must be a valid COM pointer or null.
unsafe fn release(handler: *mut IComponentHandler) {
    if !handler.is_null() {
        let unknown = handler as *mut FUnknown;
        // SAFETY: handler is non-null and a valid COM pointer per the contract.
        unsafe { ((*(*unknown).vtbl).release)(unknown) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use vst3::Steinberg::Vst::{IComponentHandlerTrait, ParamID, ParamValue};
    use vst3::Steinberg::{int32, kResultOk, tresult};
    use vst3::{Class, ComWrapper};

    /// A host's component handler that records when it is freed.
    struct Handler {
        dropped: Arc<AtomicBool>,
    }

    impl Drop for Handler {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::Relaxed);
        }
    }

    impl Class for Handler {
        type Interfaces = (IComponentHandler,);
    }

    impl IComponentHandlerTrait for Handler {
        unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
            kResultOk
        }

        unsafe fn performEdit(&self, _id: ParamID, _value: ParamValue) -> tresult {
            kResultOk
        }

        unsafe fn endEdit(&self, _id: ParamID) -> tresult {
            kResultOk
        }

        unsafe fn restartComponent(&self, _flags: int32) -> tresult {
            kResultOk
        }
    }

    #[test]
    fn test_slot_holds_one_reference_and_releases_it() {
        let dropped = Arc::new(AtomicBool::new(false));
        let host = ComWrapper::new(Handler {
            dropped: Arc::clone(&dropped),
        });
        let pointer = host
            .as_com_ref::<IComponentHandler>()
            .expect("the handler is an IComponentHandler")
            .as_ptr();

        let slot = Arc::new(ComponentHandler::new());
        let editor = Arc::clone(&slot);
        // SAFETY: pointer is a valid COM pointer, and the test is one thread.
        unsafe { slot.set(pointer) };
        assert_eq!(editor.get(), pointer);

        // The host lets go of its own reference: the slot keeps the handler.
        drop(host);
        assert!(!dropped.load(Ordering::Relaxed));

        // Setting none releases it.
        // SAFETY: Null is allowed.
        unsafe { slot.set(std::ptr::null_mut()) };
        assert!(editor.get().is_null());
        assert!(dropped.load(Ordering::Relaxed));
    }

    #[test]
    fn test_last_holder_releases_the_handler() {
        let dropped = Arc::new(AtomicBool::new(false));
        let host = ComWrapper::new(Handler {
            dropped: Arc::clone(&dropped),
        });
        let pointer = host
            .as_com_ref::<IComponentHandler>()
            .expect("the handler is an IComponentHandler")
            .as_ptr();

        let slot = Arc::new(ComponentHandler::new());
        let editor = Arc::clone(&slot);
        // SAFETY: pointer is a valid COM pointer, and the test is one thread.
        unsafe { slot.set(pointer) };
        drop(host);

        drop(slot);
        assert!(!dropped.load(Ordering::Relaxed));
        drop(editor);
        assert!(dropped.load(Ordering::Relaxed));
    }
}
