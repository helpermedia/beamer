//! A check in debug builds that calls meant for host threads stay out of
//! `process()`.
//!
//! Some calls of the framework take a lock, allocate or free memory:
//! [`Handover::get`](crate::Handover::get) and the rest of the setting side
//! of a [`Handover`](crate::Handover) or [`State`](crate::State), and
//! [`WebViewEventQueue::emit`](crate::WebViewEventQueue::emit). On the audio thread
//! that can make the audio drop out. A call from `process()` still compiles
//! and mostly works, so the mistake is easy to miss.
//!
//! The format wrappers mark the time a plugin processes with a
//! [`ProcessScope`], and those calls check in debug builds that they do not
//! run inside one. A call that does is logged (see `log_level` in
//! `Config.toml`) and panics, which in a host ends the plugin: a VST3 host
//! aborts, an Audio Unit stops rendering. In release builds all of this
//! compiles to nothing.

use std::marker::PhantomData;

#[cfg(debug_assertions)]
thread_local! {
    /// Whether this thread is inside `process()` of a plugin now.
    static IN_PROCESS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The time a plugin processes on this thread, from [`enter`](Self::enter)
/// until the scope is dropped.
///
/// For format wrappers, at the start of their process call. Scopes can be
/// nested: dropping one restores the state before it.
#[must_use = "the scope ends when it is dropped"]
pub struct ProcessScope {
    /// Whether the thread was inside a scope already.
    #[cfg(debug_assertions)]
    previous: bool,
    /// The scope belongs to its thread.
    _not_send: PhantomData<*const ()>,
}

impl ProcessScope {
    /// Mark this thread as processing until the scope is dropped.
    ///
    /// Takes no lock and does not allocate.
    #[inline]
    pub fn enter() -> Self {
        Self {
            #[cfg(debug_assertions)]
            previous: IN_PROCESS.with(|flag| flag.replace(true)),
            _not_send: PhantomData,
        }
    }
}

impl Drop for ProcessScope {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        IN_PROCESS.with(|flag| flag.set(self.previous));
    }
}

/// Whether this thread is inside `process()` of a plugin now. Always
/// `false` in release builds.
#[inline]
pub fn in_process() -> bool {
    #[cfg(debug_assertions)]
    {
        IN_PROCESS.with(|flag| flag.get())
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// What to use on the audio thread instead of a `State` or `Handover` call.
pub(crate) const READ_THROUGH_A_READER: &str =
    "On the audio thread, read the value through a reader made in prepare().";

/// Log and panic in debug builds when `call`, a call for host threads, runs
/// inside `process()`. `instead` says what to use there.
#[inline]
#[cfg_attr(debug_assertions, track_caller)]
pub(crate) fn assert_not_in_process(call: &str, instead: &str) {
    #[cfg(debug_assertions)]
    if in_process() {
        let message = format!(
            "{call} was called in process(). It takes a lock and may allocate or free \
             memory, so it is for host and UI threads only. {instead}"
        );
        // The panic message goes to standard error, which a host seldom
        // shows, so it goes to the plugin's log first.
        log::error!("{message} ({})", std::panic::Location::caller());
        panic!("{message}");
    }
    #[cfg(not(debug_assertions))]
    let _ = (call, instead);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_marks_the_thread_until_it_is_dropped() {
        assert!(!in_process());
        {
            let _scope = ProcessScope::enter();
            assert_eq!(in_process(), cfg!(debug_assertions));
            {
                let _inner = ProcessScope::enter();
                assert_eq!(in_process(), cfg!(debug_assertions));
            }
            // The inner scope restored the outer one.
            assert_eq!(in_process(), cfg!(debug_assertions));
        }
        assert!(!in_process());
    }

    #[test]
    fn test_scope_is_per_thread() {
        let _scope = ProcessScope::enter();
        let other = std::thread::spawn(in_process).join().expect("the thread runs");
        assert!(!other);
    }

    #[test]
    fn test_host_calls_are_fine_outside_a_scope() {
        assert_not_in_process("A call", READ_THROUGH_A_READER);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "Handover::get() was called in process()")]
    fn test_host_call_in_a_scope_panics() {
        let _scope = ProcessScope::enter();
        assert_not_in_process("Handover::get()", READ_THROUGH_A_READER);
    }
}
