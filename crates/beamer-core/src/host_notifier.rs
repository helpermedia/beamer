//! Telling the host about changes from any thread.
//!
//! Some changes reach the host only on its main thread: a VST3 plugin asks
//! for a restart there, an Audio Unit notifies its property listeners
//! there. The change itself is often noticed on the audio thread, after a
//! block whose latency differs from the one before. A [`HostNotifier`]
//! takes such changes from any thread without waiting and delivers them,
//! collected, on the main thread.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Changes the host has to hear about. Several can be delivered at once.
///
/// The bits are part of the Audio Unit bridge: `BEAMER_AU_HOST_CHANGE_*`
/// in BeamerAuBridge.h has the same values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostChanges(u32);

impl HostChanges {
    /// No change.
    pub const NONE: Self = Self(0);
    /// The latency changed.
    pub const LATENCY: Self = Self(1);
    /// The tail changed.
    pub const TAIL: Self = Self(1 << 1);
    /// Parameter values changed together, for example by a preset.
    pub const PARAMETER_VALUES: Self = Self(1 << 2);

    /// The changes as bits, for a C caller.
    #[inline]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every change of `other` is in `self`.
    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether there is no change.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for HostChanges {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for HostChanges {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// What receives the changes on the main thread.
pub type HostChangeTarget = Box<dyn FnMut(HostChanges) + Send>;

thread_local! {
    /// The notifier whose target runs on this thread, if any.
    static DELIVERING: Cell<*const Shared> = const { Cell::new(std::ptr::null()) };
}

/// What the notifier and its pending deliveries share.
struct Shared {
    /// Changes noted since the last delivery.
    pending: AtomicU32,
    /// Who receives the changes. The lock is held while it runs, so that
    /// clearing it waits for a delivery in progress.
    target: Mutex<Option<HostChangeTarget>>,
    /// A target set from within a delivery, which takes over when the
    /// delivery ends.
    replacement: Mutex<Option<Option<HostChangeTarget>>>,
}

impl Shared {
    fn lock_target(&self) -> MutexGuard<'_, Option<HostChangeTarget>> {
        // A poisoned lock only means a panic in a target: the slot itself
        // is still whole.
        self.target.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_replacement(&self) -> MutexGuard<'_, Option<Option<HostChangeTarget>>> {
        self.replacement.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether this thread is in a delivery of this notifier.
    fn is_delivering_here(&self) -> bool {
        DELIVERING.with(|delivering| std::ptr::eq(delivering.get(), self))
    }

    /// Hand the pending changes to the target. Without one they wait for
    /// the next delivery.
    fn deliver(&self) {
        let changes = HostChanges(self.pending.swap(0, Ordering::AcqRel));
        if changes.is_empty() {
            return;
        }
        let mut slot = self.lock_target();
        match slot.as_mut() {
            Some(target) => {
                let outer = DELIVERING.with(|delivering| delivering.replace(self));
                // A panic must not unwind into the caller, which may be C code.
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| target(changes)));
                DELIVERING.with(|delivering| delivering.set(outer));
            }
            None => {
                self.pending.fetch_or(changes.0, Ordering::AcqRel);
            }
        }
        // A target set from within the delivery (a host that closes the
        // plugin when it hears of a change) takes over now.
        if let Some(replacement) = self.lock_replacement().take() {
            let old = std::mem::replace(&mut *slot, replacement);
            drop(slot);
            drop(old);
        }
    }
}

/// Takes changes for the host from any thread and delivers them on the
/// main thread.
///
/// [`notify()`](Self::notify) is realtime-safe: an atomic operation and a
/// wake-up of the main thread, no lock and no allocation. Changes noted
/// before a delivery come together in one call of the target.
///
/// On macOS the delivery runs on the main dispatch queue. Elsewhere there
/// is no main-thread delivery yet: changes wait for
/// [`deliver()`](Self::deliver).
pub struct HostNotifier {
    shared: Arc<Shared>,
    #[cfg(target_os = "macos")]
    source: dispatch::Source,
}

impl HostNotifier {
    /// A notifier without a target: changes are collected until one is set.
    pub fn new() -> Self {
        let shared = Arc::new(Shared {
            pending: AtomicU32::new(0),
            target: Mutex::new(None),
            replacement: Mutex::new(None),
        });
        Self {
            #[cfg(target_os = "macos")]
            source: dispatch::Source::new(Arc::clone(&shared)),
            shared,
        }
    }

    /// Set who receives the changes on the main thread, or no one.
    ///
    /// Clearing the target waits for a delivery in progress on another
    /// thread, so that whatever the target refers to may go away
    /// afterwards. From within the target (a host that closes the plugin
    /// when it hears of a change) the new target takes over once the
    /// delivery ends. Changes noted while there is no target wait for one.
    pub fn set_target(&self, target: Option<HostChangeTarget>) {
        if self.shared.is_delivering_here() {
            *self.shared.lock_replacement() = Some(target);
            return;
        }
        let has_target = target.is_some();
        let old = std::mem::replace(&mut *self.shared.lock_target(), target);
        // The old target is dropped outside the lock.
        drop(old);
        // Changes that waited for a target go out now.
        #[cfg(target_os = "macos")]
        if has_target && self.shared.pending.load(Ordering::Acquire) != 0 {
            self.source.wake();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = has_target;
    }

    /// Note changes for the host. Realtime-safe, from any thread.
    #[inline]
    pub fn notify(&self, changes: HostChanges) {
        if changes.is_empty() {
            return;
        }
        self.shared.pending.fetch_or(changes.0, Ordering::AcqRel);
        #[cfg(target_os = "macos")]
        self.source.wake();
    }

    /// Deliver the pending changes on this thread now. The main thread
    /// does this by itself; tests and callers without a main-thread
    /// delivery use it.
    pub fn deliver(&self) {
        self.shared.deliver();
    }
}

impl Default for HostNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for HostNotifier {
    fn drop(&mut self) {
        // A delivery in progress ends before the target goes. The source
        // stops when it drops after this; its cancel handler releases the
        // shared part on the main queue later. A process whose main queue
        // never runs (a validator) keeps that small part until it exits.
        self.set_target(None);
    }
}

/// The main dispatch queue and a source that runs a delivery on it.
#[cfg(target_os = "macos")]
mod dispatch {
    use std::ffi::c_void;
    use std::sync::Arc;

    use super::Shared;

    #[allow(non_upper_case_globals)]
    unsafe extern "C" {
        // The main queue: `dispatch_get_main_queue()` is its address.
        static _dispatch_main_q: c_void;
        // `DISPATCH_SOURCE_TYPE_DATA_OR` is the address of this.
        static _dispatch_source_type_data_or: c_void;

        fn dispatch_source_create(
            source_type: *const c_void,
            handle: usize,
            mask: usize,
            queue: *const c_void,
        ) -> *mut c_void;
        fn dispatch_set_context(object: *mut c_void, context: *mut c_void);
        fn dispatch_source_set_event_handler_f(source: *mut c_void, handler: extern "C" fn(*mut c_void));
        fn dispatch_source_set_cancel_handler_f(source: *mut c_void, handler: extern "C" fn(*mut c_void));
        fn dispatch_source_merge_data(source: *mut c_void, value: usize);
        fn dispatch_resume(object: *mut c_void);
        fn dispatch_source_cancel(source: *mut c_void);
        fn dispatch_release(object: *mut c_void);
    }

    /// A data source on the main queue. Merging data into it wakes the
    /// main queue once for all merges since its last run.
    pub(super) struct Source(*mut c_void);

    // SAFETY: Dispatch objects are thread-safe; merging data and cancelling
    // are allowed from any thread.
    unsafe impl Send for Source {}
    // SAFETY: See above.
    unsafe impl Sync for Source {}

    impl Source {
        /// A source that delivers the changes of `shared` on the main queue.
        /// The source keeps a reference to `shared` until it is cancelled.
        pub(super) fn new(shared: Arc<Shared>) -> Self {
            // SAFETY: The source type and the queue are the ones libdispatch
            // exports. The context is an `Arc` the cancel handler releases.
            unsafe {
                let source = dispatch_source_create(
                    &_dispatch_source_type_data_or,
                    0,
                    0,
                    &_dispatch_main_q,
                );
                assert!(!source.is_null(), "dispatch_source_create failed");
                dispatch_set_context(source, Arc::into_raw(shared) as *mut c_void);
                dispatch_source_set_event_handler_f(source, deliver);
                dispatch_source_set_cancel_handler_f(source, release_context);
                dispatch_resume(source);
                Self(source)
            }
        }

        /// Have the main queue run a delivery.
        #[inline]
        pub(super) fn wake(&self) {
            // SAFETY: The source lives until drop. Merging is allowed from
            // any thread and does not allocate.
            unsafe { dispatch_source_merge_data(self.0, 1) };
        }
    }

    impl Drop for Source {
        fn drop(&mut self) {
            // SAFETY: The source is valid. After the cancel no delivery
            // starts; the cancel handler releases the context once a
            // delivery in progress has ended.
            unsafe {
                dispatch_source_cancel(self.0);
                dispatch_release(self.0);
            }
        }
    }

    /// The event handler: deliver the pending changes.
    extern "C" fn deliver(context: *mut c_void) {
        // SAFETY: The context is the `Arc<Shared>` of `Source::new`, which
        // lives until the cancel handler.
        let shared = unsafe { &*(context as *const Shared) };
        shared.deliver();
    }

    /// The cancel handler: release the context.
    extern "C" fn release_context(context: *mut c_void) {
        // SAFETY: The context is the `Arc<Shared>` of `Source::new`, released
        // once here.
        drop(unsafe { Arc::from_raw(context as *const Shared) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A notifier whose target records what it receives.
    fn recording() -> (HostNotifier, Arc<Mutex<Vec<HostChanges>>>) {
        let notifier = HostNotifier::new();
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        notifier.set_target(Some(Box::new(move |changes| sink.lock().unwrap().push(changes))));
        (notifier, received)
    }

    #[test]
    fn test_changes_before_a_delivery_come_together() {
        let (notifier, received) = recording();
        notifier.notify(HostChanges::LATENCY);
        notifier.notify(HostChanges::PARAMETER_VALUES);
        notifier.notify(HostChanges::LATENCY);
        notifier.deliver();
        notifier.deliver();
        assert_eq!(*received.lock().unwrap(), [HostChanges::LATENCY | HostChanges::PARAMETER_VALUES]);
    }

    #[test]
    fn test_no_change_delivers_nothing() {
        let (notifier, received) = recording();
        notifier.notify(HostChanges::NONE);
        notifier.deliver();
        assert!(received.lock().unwrap().is_empty());
    }

    #[test]
    fn test_cleared_target_receives_nothing() {
        let (notifier, received) = recording();
        notifier.set_target(None);
        notifier.notify(HostChanges::TAIL);
        notifier.deliver();
        assert!(received.lock().unwrap().is_empty());
    }

    #[test]
    fn test_changes_wait_for_a_target() {
        let notifier = HostNotifier::new();
        notifier.notify(HostChanges::TAIL);
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        notifier.set_target(Some(Box::new(move |changes| sink.lock().unwrap().push(changes))));
        notifier.deliver();
        assert_eq!(*received.lock().unwrap(), [HostChanges::TAIL]);
    }

    #[test]
    fn test_target_cleared_from_within_a_delivery_goes_after_it() {
        // A host that closes the plugin when it hears of a change clears
        // the target from within the delivery: no deadlock and nothing
        // more reaches the old target.
        let notifier = Arc::new(HostNotifier::new());
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        let weak = Arc::downgrade(&notifier);
        notifier.set_target(Some(Box::new(move |changes| {
            sink.lock().unwrap().push(changes);
            if let Some(notifier) = weak.upgrade() {
                notifier.set_target(None);
            }
        })));
        notifier.notify(HostChanges::LATENCY);
        notifier.deliver();
        notifier.notify(HostChanges::LATENCY);
        notifier.deliver();
        assert_eq!(*received.lock().unwrap(), [HostChanges::LATENCY]);
    }

    #[test]
    fn test_changes_delivered_without_a_target_wait_for_one() {
        let notifier = HostNotifier::new();
        notifier.notify(HostChanges::LATENCY);
        // The main thread delivers before the target is set.
        notifier.deliver();
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        notifier.set_target(Some(Box::new(move |changes| sink.lock().unwrap().push(changes))));
        notifier.deliver();
        assert_eq!(*received.lock().unwrap(), [HostChanges::LATENCY]);
    }

    #[test]
    fn test_contains() {
        let both = HostChanges::LATENCY | HostChanges::TAIL;
        assert!(both.contains(HostChanges::LATENCY));
        assert!(!both.contains(HostChanges::PARAMETER_VALUES));
        assert!(HostChanges::NONE.is_empty());
    }
}
