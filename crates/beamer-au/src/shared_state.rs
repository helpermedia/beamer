//! What host threads and the render call exchange without a lock.
//!
//! While audio runs, the processor belongs to the audio thread. A host thread
//! that wants something from it does not wait for it and does not take it:
//! it reads what the audio thread published, or leaves a request that the
//! next render call picks up before it processes.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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
}

impl SharedState {
    /// Latency in samples of the most recent block, 0 while unprepared.
    pub(crate) fn latency_samples(&self) -> u32 {
        self.latency_samples.load(Ordering::Relaxed)
    }

    /// Tail in samples of the most recent block, 0 while unprepared.
    pub(crate) fn tail_samples(&self) -> u32 {
        self.tail_samples.load(Ordering::Relaxed)
    }

    /// Publish what the processor reports. Called by whoever holds it.
    pub(crate) fn publish_timing(&self, latency_samples: u32, tail_samples: u32) {
        self.latency_samples.store(latency_samples, Ordering::Relaxed);
        self.tail_samples.store(tail_samples, Ordering::Relaxed);
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
