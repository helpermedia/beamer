//! Handle for sending events from Rust to the WebView.
//!
//! **Not yet wired up.** The type is defined here for Phase 2D (real-time
//! visualization). It will be instantiated and provided to plugins when
//! the format wrappers gain Rust-to-JS event emission support.

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

/// Function pointer type for evaluating JavaScript in the WebView.
///
/// Called with the context pointer and a JSON-encoded JavaScript expression.
/// Must be called on the main thread. The implementation dispatches via
/// `dispatch_async` if called from a background thread.
pub type EvalJsFn = unsafe extern "C-unwind" fn(context: *mut c_void, script: *const u8, len: usize);

/// Handle for sending events from Rust to the WebView.
///
/// The handle is `Send + Sync` and can be used from non-realtime threads.
/// Calls are dispatched to the main thread internally.
///
/// **Not audio-thread safe.** This struct allocates (JSON serialization).
/// For sending visualization data from the audio thread, see Phase 2D's
/// lock-free ring buffer approach.
#[derive(Clone)]
pub struct WebViewHandle {
    eval_fn: EvalJsFn,
    context: Arc<AtomicPtr<c_void>>,
}

// SAFETY: The context pointer is only dereferenced on the main thread
// inside the eval_fn callback. The Arc<AtomicPtr> ensures thread-safe
// access to the pointer itself.
unsafe impl Send for WebViewHandle {}
// SAFETY: Same reasoning as Send - context is only dereferenced on the
// main thread inside eval_fn and Arc<AtomicPtr> is inherently Sync.
unsafe impl Sync for WebViewHandle {}

impl WebViewHandle {
    /// Create a new WebView handle.
    ///
    /// # Safety
    ///
    /// - `eval_fn` must be a valid function pointer that remains valid for
    ///   the lifetime of the handle
    /// - `context` must remain valid until `invalidate()` is called
    pub unsafe fn new(eval_fn: EvalJsFn, context: *mut c_void) -> Self {
        Self {
            eval_fn,
            context: Arc::new(AtomicPtr::new(context)),
        }
    }

    /// Emit a named event to JavaScript.
    ///
    /// The event is delivered asynchronously. If the WebView is not
    /// attached (context is null), the call is silently dropped.
    pub fn emit(&self, name: &str, data: &impl serde::Serialize) {
        let ctx = self.context.load(Ordering::Acquire);
        if ctx.is_null() {
            return;
        }

        let data_json = match serde_json::to_string(data) {
            Ok(json) => json,
            Err(e) => {
                log::error!("Failed to serialize event data: {e}");
                return;
            }
        };

        let script = format!(
            "window.__BEAMER__._onEvent({},{})",
            serde_json::to_string(name).unwrap_or_default(),
            data_json,
        );

        // SAFETY: eval_fn is a valid function pointer (guaranteed by new()),
        // and ctx was checked non-null above. The callee dispatches to the
        // main thread if needed.
        unsafe {
            (self.eval_fn)(ctx, script.as_ptr(), script.len());
        }
    }

    /// Invalidate the handle, preventing further calls.
    ///
    /// Called when the WebView is detached. After this, `emit()` becomes
    /// a no-op.
    pub fn invalidate(&self) {
        self.context.store(std::ptr::null_mut(), Ordering::Release);
    }
}
