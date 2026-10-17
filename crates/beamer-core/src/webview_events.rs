//! Events from Rust to the web UI.
//!
//! A [`WebViewEvents`] is a queue of named events for the plugin's editor.
//! Rust code puts events in with [`emit()`](WebViewEvents::emit), and
//! JavaScript receives them with `__BEAMER__.on("name", callback)`.
//!
//! The queue lives in the model, which every thread of a plugin
//! instance shares, as a field without an attribute. `#[beamer::model]`
//! finds it by its type, also in a nested group, and the format wrappers
//! take it from there:
//!
//! ```ignore
//! #[beamer::model]
//! struct SamplerModel {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     gain: FloatParameter,
//!
//!     events: WebViewEvents,
//! }
//!
//! // On a host or UI thread, or on a thread the plugin started:
//! model.events.emit("sampleLoaded", &"kick.wav");
//! ```
//!
//! ```js
//! __BEAMER__.on("sampleLoaded", (name) => showSample(name));
//! ```
//!
//! # What an event is
//!
//! An event is a notification for an editor that is open. It is delivered
//! on the editor's next sync tick, about 16 ms later. While no editor is
//! open, events are dropped: an editor that opens asks for what it shows,
//! with `invoke()` or through the parameters and `State` fields. Events are
//! therefore not the place for anything that has to arrive.
//!
//! # Threads
//!
//! `emit()` turns the data into JSON and takes a lock, so it is for host
//! and UI threads and for threads the plugin starts. It must not be called
//! on the audio thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;

/// How many events wait for the editor at most.
///
/// An editor takes the events 60 times a second. When it stalls, or when a
/// plugin emits faster than that for a while, the oldest events are dropped.
const MAX_PENDING: usize = 1024;

/// A queue of events for the web UI.
///
/// See the [module documentation](self). Cloning gives another handle to
/// the same queue.
#[derive(Clone, Default)]
pub struct WebViewEvents {
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    /// The events that wait for the editor, each as the arguments of the
    /// JavaScript call that delivers it.
    pending: Mutex<VecDeque<String>>,
    /// How many editors are open.
    editors: AtomicUsize,
    /// Whether a format wrapper took this queue.
    claimed: AtomicBool,
    /// Whether an event for a queue that no wrapper took was reported.
    reported: AtomicBool,
}

impl WebViewEvents {
    /// Create an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Send a named event to the web UI.
    ///
    /// `data` can be anything that serializes to JSON. JavaScript receives
    /// it as the argument of the callbacks registered with
    /// `__BEAMER__.on(name, callback)`.
    ///
    /// The event is dropped while no editor is open. Not for the audio
    /// thread: this allocates and takes a lock.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn emit(&self, name: &str, data: &impl Serialize) {
        crate::audio_thread::assert_not_in_process(
            "WebViewEvents::emit()",
            "Data from the audio thread needs a lock-free channel to the editor.",
        );
        if !self.shared.claimed.load(Ordering::Acquire) {
            self.report_unclaimed(name);
            return;
        }
        if self.shared.editors.load(Ordering::Acquire) == 0 {
            return;
        }

        let name_json = match serde_json::to_string(name) {
            Ok(json) => json,
            Err(error) => {
                log::error!("Failed to serialize the name of event {name:?}: {error}");
                return;
            }
        };
        let data_json = match serde_json::to_string(data) {
            Ok(json) => json,
            Err(error) => {
                log::error!("Failed to serialize the data of event {name:?}: {error}");
                return;
            }
        };

        let mut pending = self.pending();
        if pending.len() >= MAX_PENDING {
            pending.pop_front();
        }
        pending.push_back(format!("{name_json},{data_json}"));
    }

    /// Take this queue for a plugin instance.
    ///
    /// For format wrappers, which call it when they create the instance.
    /// It is what tells a queue the framework delivers from one that no
    /// model of an instance holds.
    pub fn claim(&self) {
        self.shared.claimed.store(true, Ordering::Release);
    }

    /// An editor was opened: events are kept for it from now on.
    ///
    /// For format wrappers.
    pub fn editor_opened(&self) {
        // Hold the lock, so that an `emit()` that runs now lands either
        // before the queue is emptied or after the editor counts as open.
        let mut pending = self.pending();
        if self.shared.editors.fetch_add(1, Ordering::AcqRel) == 0 {
            pending.clear();
        }
    }

    /// An editor was closed. Without an editor, events are dropped.
    ///
    /// For format wrappers.
    pub fn editor_closed(&self) {
        let mut pending = self.pending();
        let editors = &self.shared.editors;
        let open = editors
            .try_update(Ordering::AcqRel, Ordering::Acquire, |open| Some(open.saturating_sub(1)))
            .unwrap_or(0);
        if open <= 1 {
            pending.clear();
        }
    }

    /// Take the waiting events as one script for the WebView.
    ///
    /// For format wrappers, which run the script on the main thread from
    /// the editor's sync timer. Returns `None` when nothing waits.
    pub fn take_script(&self) -> Option<String> {
        let mut pending = self.pending();
        if pending.is_empty() {
            return None;
        }

        let mut script = String::new();
        for event in pending.drain(..) {
            script.push_str("window.__BEAMER__._onEvent(");
            script.push_str(&event);
            script.push_str(");");
        }
        Some(script)
    }

    /// Lock the waiting events.
    fn pending(&self) -> MutexGuard<'_, VecDeque<String>> {
        // A poisoned lock only means a panic elsewhere: the queue holds
        // finished strings and stays usable.
        self.shared
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Say once that an event went to a queue that nothing delivers.
    fn report_unclaimed(&self, name: &str) {
        if self.shared.reported.swap(true, Ordering::AcqRel) {
            return;
        }

        let message = format!(
            "WebView event {name:?} was dropped: no plugin instance delivers this queue. \
             Is it the only `WebViewEvents` field of the model and its nested groups?"
        );
        // A plugin has no logger unless it sets `log_level` in its
        // Config.toml, so the message also goes to standard error.
        log::warn!("{message}");
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A queue as it is while an editor is open.
    fn open_queue() -> WebViewEvents {
        let events = WebViewEvents::new();
        events.claim();
        events.editor_opened();
        events
    }

    #[test]
    fn test_events_arrive_in_order_as_one_script() {
        let events = open_queue();
        events.emit("loaded", &"kick.wav");
        events.emit("progress", &0.5);

        assert_eq!(
            events.take_script().as_deref(),
            Some(
                "window.__BEAMER__._onEvent(\"loaded\",\"kick.wav\");\
                 window.__BEAMER__._onEvent(\"progress\",0.5);"
            )
        );
        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_name_and_data_are_json() {
        #[derive(Serialize)]
        struct Learned {
            controller: u8,
            label: &'static str,
        }

        let events = open_queue();
        events.emit(
            "midi \"learn\"",
            &Learned {
                controller: 74,
                label: "cut\noff",
            },
        );

        assert_eq!(
            events.take_script().as_deref(),
            Some(
                "window.__BEAMER__._onEvent(\"midi \\\"learn\\\"\",\
                 {\"controller\":74,\"label\":\"cut\\noff\"});"
            )
        );
    }

    #[test]
    fn test_events_are_dropped_without_an_editor() {
        let events = WebViewEvents::new();
        events.claim();

        events.emit("early", &1);
        assert_eq!(events.take_script(), None);

        events.editor_opened();
        events.emit("kept", &2);
        events.editor_closed();
        assert_eq!(events.take_script(), None);

        events.emit("late", &3);
        events.editor_opened();
        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_queue_stays_open_while_one_of_two_editors_is() {
        let events = open_queue();
        events.editor_opened();
        events.editor_closed();

        events.emit("still", &true);
        assert!(events.take_script().is_some());

        // Closing more often than opening leaves the queue closed.
        events.editor_closed();
        events.editor_closed();
        events.emit("gone", &true);
        assert_eq!(events.take_script(), None);

        events.editor_opened();
        events.emit("again", &true);
        assert!(events.take_script().is_some());
    }

    #[test]
    fn test_oldest_events_are_dropped_when_the_queue_is_full() {
        let events = open_queue();
        for number in 0..MAX_PENDING + 3 {
            events.emit("n", &number);
        }

        let script = events.take_script().expect("events wait");
        assert_eq!(script.matches("_onEvent(").count(), MAX_PENDING);
        assert!(script.starts_with("window.__BEAMER__._onEvent(\"n\",3);"));
        assert!(script.ends_with(&format!("_onEvent(\"n\",{});", MAX_PENDING + 2)));
    }

    #[test]
    fn test_clone_is_the_same_queue() {
        let events = WebViewEvents::new();
        let wrapper_end = events.clone();
        wrapper_end.claim();
        wrapper_end.editor_opened();

        events.emit("shared", &1);
        assert!(wrapper_end.take_script().is_some());
    }

    #[test]
    fn test_queue_that_nothing_delivers_reports_once() {
        let events = WebViewEvents::new();
        assert!(!events.shared.reported.load(Ordering::Acquire));

        events.emit("lost", &1);
        events.emit("lost", &2);
        assert!(events.shared.reported.load(Ordering::Acquire));

        // Nothing was kept for a wrapper that takes the queue later.
        events.claim();
        events.editor_opened();
        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_data_that_cannot_be_json_is_dropped() {
        use std::collections::BTreeMap;

        let events = open_queue();
        // JSON has no keys that are sequences.
        let mut data = BTreeMap::new();
        data.insert(vec![1u8], 1u8);
        events.emit("bad", &data);

        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_threads_emit_while_the_editor_takes() {
        const THREADS: usize = 4;
        const EVENTS: usize = 200;

        let events = open_queue();
        let mut delivered = 0;

        std::thread::scope(|scope| {
            let emitters: Vec<_> = (0..THREADS)
                .map(|thread| {
                    let events = events.clone();
                    scope.spawn(move || {
                        for number in 0..EVENTS {
                            events.emit("tick", &(thread, number));
                        }
                    })
                })
                .collect();

            while emitters.iter().any(|emitter| !emitter.is_finished()) {
                if let Some(script) = events.take_script() {
                    delivered += script.matches("_onEvent(").count();
                }
            }
        });

        if let Some(script) = events.take_script() {
            delivered += script.matches("_onEvent(").count();
        }
        assert_eq!(delivered, THREADS * EVENTS);
    }
}
