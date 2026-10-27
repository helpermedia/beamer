//! What host threads and the render call exchange without a lock.
//!
//! While audio runs, the processor belongs to the audio thread. A host thread
//! that wants something from it does not wait for it and does not take it:
//! it reads what the audio thread published, or leaves a request that the
//! next render call picks up before it processes.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use beamer_core::{HostChanges, HostNotifier, ProcessWarning, ProcessWarnings};

/// State shared by an instance handle and its render blocks.
#[derive(Default)]
pub(crate) struct SharedState {
    /// Latency in samples, as the processor reported it last.
    latency_samples: AtomicU32,
    /// Tail in samples, as the processor reported it last.
    tail_samples: AtomicU32,
    /// The host asked for a reset of the DSP state.
    reset_requested: AtomicBool,
    /// A state was loaded: the smoothers have to jump to the new values.
    smoothing_reset_requested: AtomicBool,
    /// What went wrong in render calls since a host thread last looked.
    /// The audio thread does not log: a host thread logs them when the
    /// plugin is prepared again or released.
    render_warnings: ProcessWarnings,
    /// The diagnostics of the instance, in a diagnostics build.
    #[cfg(feature = "diagnostics")]
    diagnostics: Option<beamer_core::diagnostics::Diagnostics>,
    /// The host's bypass as last noted: 0 not yet, 1 off, 2 on.
    #[cfg(feature = "diagnostics")]
    host_bypass: std::sync::atomic::AtomicU8,
    /// Tells the shim on the main thread that the latency or the tail
    /// changed, so that it notifies the host.
    host_notifier: HostNotifier,
}

impl SharedState {
    /// State for an instance with diagnostics.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn with_diagnostics(diagnostics: beamer_core::diagnostics::Diagnostics) -> Self {
        Self { diagnostics: Some(diagnostics), ..Self::default() }
    }

    /// The diagnostics of the instance, if it has them.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn diagnostics(&self) -> Option<&beamer_core::diagnostics::Diagnostics> {
        self.diagnostics.as_ref()
    }

    /// Note the host's bypass. Only a change is logged: a host may set the
    /// same value again.
    #[cfg(feature = "diagnostics")]
    pub(crate) fn note_host_bypass(&self, bypassed: bool) {
        let now = if bypassed { 2 } else { 1 };
        let before = self.host_bypass.swap(now, Ordering::Relaxed);
        if let (Some(diagnostics), true) = (self.diagnostics(), before != now) {
            diagnostics.note(format_args!("host bypass {}", if bypassed { "on" } else { "off" }));
        }
    }

    /// Latency in samples of the most recent block, 0 while unprepared.
    pub(crate) fn latency_samples(&self) -> u32 {
        self.latency_samples.load(Ordering::Relaxed)
    }

    /// Tail in samples of the most recent block, 0 while unprepared.
    pub(crate) fn tail_samples(&self) -> u32 {
        self.tail_samples.load(Ordering::Relaxed)
    }

    /// Publish what the processor reports. Called by whoever holds it,
    /// when it is prepared or released: the host asks for the values then
    /// anyway, so they are no change to tell it about.
    pub(crate) fn publish_timing(&self, latency_samples: u32, tail_samples: u32) {
        self.latency_samples.store(latency_samples, Ordering::Relaxed);
        self.tail_samples.store(tail_samples, Ordering::Relaxed);
    }

    /// Publish what the processor reports around a render call and tell
    /// the host about a latency or tail other than the one before. For the
    /// audio thread: atomic operations, no lock, no allocation.
    pub(crate) fn publish_block_timing(&self, latency_samples: u32, tail_samples: u32) {
        let mut changes = HostChanges::NONE;
        if self.latency_samples.swap(latency_samples, Ordering::Relaxed) != latency_samples {
            changes |= HostChanges::LATENCY;
        }
        if self.tail_samples.swap(tail_samples, Ordering::Relaxed) != tail_samples {
            changes |= HostChanges::TAIL;
        }
        self.host_notifier.notify(changes);
    }

    /// The notifier that tells the shim about changes for the host.
    pub(crate) fn host_notifier(&self) -> &HostNotifier {
        &self.host_notifier
    }

    /// Ask the audio thread to reset the DSP state before its next block.
    pub(crate) fn request_reset(&self) {
        self.reset_requested.store(true, Ordering::Release);
    }

    /// Ask the audio thread to reset the smoothers before its next block.
    ///
    /// Call this after the parameter values are written, so that the reset
    /// sees all of them.
    pub(crate) fn request_smoothing_reset(&self) {
        self.smoothing_reset_requested.store(true, Ordering::Release);
    }

    /// Take the reset request, if there is one.
    pub(crate) fn take_reset_request(&self) -> bool {
        self.reset_requested.swap(false, Ordering::AcqRel)
    }

    /// Take the smoother reset request, if there is one.
    pub(crate) fn take_smoothing_reset_request(&self) -> bool {
        self.smoothing_reset_requested.swap(false, Ordering::AcqRel)
    }

    /// Drop a reset request that a new processor makes pointless.
    pub(crate) fn clear_reset_request(&self) {
        self.reset_requested.store(false, Ordering::Release);
    }

    /// Note a warning of a render call. For the audio thread: one atomic
    /// operation, no allocation.
    pub(crate) fn note_render_warning(&self, warning: ProcessWarning) {
        self.render_warnings.note(warning);
    }

    /// Log the warnings of the render calls since the last call. For host
    /// threads.
    pub(crate) fn log_render_warnings(&self) {
        self.render_warnings.log();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_requests_are_taken_once() {
        let shared = SharedState::default();
        assert!(!shared.take_reset_request());
        assert!(!shared.take_smoothing_reset_request());

        shared.request_reset();
        shared.request_smoothing_reset();
        assert!(shared.take_reset_request());
        assert!(!shared.take_reset_request());
        assert!(shared.take_smoothing_reset_request());
        assert!(!shared.take_smoothing_reset_request());
    }

    #[test]
    fn test_only_a_changed_block_timing_is_told() {
        use std::sync::{Arc, Mutex};

        let shared = SharedState::default();
        let received = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&received);
        shared
            .host_notifier()
            .set_target(Some(Box::new(move |changes| sink.lock().unwrap().push(changes))));

        // Prepared: no change to tell.
        shared.publish_timing(64, 1000);
        shared.publish_block_timing(64, 1000);
        shared.host_notifier().deliver();
        assert!(received.lock().unwrap().is_empty());

        // The latency, then both.
        shared.publish_block_timing(128, 1000);
        shared.host_notifier().deliver();
        shared.publish_block_timing(0, 0);
        shared.host_notifier().deliver();
        assert_eq!(
            *received.lock().unwrap(),
            [HostChanges::LATENCY, HostChanges::LATENCY | HostChanges::TAIL]
        );
    }

    #[test]
    fn test_cleared_reset_request_is_gone() {
        let shared = SharedState::default();
        shared.request_reset();
        shared.clear_reset_request();
        assert!(!shared.take_reset_request());
    }

    #[test]
    fn test_timing_is_published() {
        let shared = SharedState::default();
        assert_eq!((shared.latency_samples(), shared.tail_samples()), (0, 0));
        shared.publish_timing(64, 48_000);
        assert_eq!((shared.latency_samples(), shared.tail_samples()), (64, 48_000));
    }
}
