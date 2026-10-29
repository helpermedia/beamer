//! Every value from the audio thread to the web UI, in order.
//!
//! A [`WebViewStreamSender`] sends values that the web UI needs all of: a
//! scrolling level history, a scope, the notes that were played. The audio
//! thread pushes values into a ring of a fixed size. At each tick of the
//! editor (about 60 times a second) every value pushed since the last tick
//! is delivered as one array, oldest first.
//!
//! The plugin creates the sender in `prepare()` from the queue of its model
//! and keeps it in the processor. Pushing takes no lock, never allocates and
//! never frees memory:
//!
//! ```ignore
//! // In prepare(), room for 1024 values between two ticks:
//! ScopeProcessor { history: self.events.stream_sender("history", 1024), model: self }
//!
//! // In process(), once per block:
//! self.history.push(block_peak);
//! ```
//!
//! ```js
//! __BEAMER__.on("history", (peaks) => strip.append(peaks));
//! ```
//!
//! A push is dropped while no editor is open: nothing would show it. When the
//! ring is full, the newest values are dropped and the push says so. Size
//! the ring for what the plugin pushes in two ticks, so that a tick that
//! comes late loses nothing. For a value of which only the newest counts,
//! such as a meter, see [`WebViewValueSender`](crate::WebViewValueSender).
//!
//! Dropping a sender frees memory. That is fine where it normally happens:
//! a sender lives in the processor, which the framework drops on a host
//! thread. Do not create or drop a sender inside `process()`.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use serde::Serialize;

use crate::webview_events::{push_event, SenderEnd, WebViewEventQueue};

/// What a sender and the queue share: a ring of slots with a write and a
/// read position.
///
/// The positions count values from the start and only grow; a position
/// modulo the capacity is the slot. The sender writes the slots from the
/// write position up to the read position plus the capacity, then moves
/// the write position on. The queue
/// reads the slots from the read position up to the write position, then
/// moves the read position on. Neither side ever waits. A 64-bit position
/// does not wrap: at 192000 values a second that takes millions of years.
struct Ring<T> {
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    /// How many values the sender wrote.
    write: AtomicU64,
    /// How many values the queue read or skipped.
    read: AtomicU64,
}

// SAFETY: A slot is only accessed by the side that owns it at that moment:
// the slots from the read position up to the write position by the queue's
// end under the lock of the queue's senders, the others by the one sender
// (`push` takes `&mut self`). The sender hands slots over with a Release
// store of the write position that the queue loads with Acquire; the queue
// hands them back the same way through the read position. A value is
// copied between threads and never accessed from two threads at once, so
// `T: Send` is enough.
unsafe impl<T: Send> Send for Ring<T> {}
// SAFETY: See above.
unsafe impl<T: Send> Sync for Ring<T> {}

impl<T> Ring<T> {
    fn capacity(&self) -> u64 {
        self.slots.len() as u64
    }

    /// The slot of a position.
    fn slot(&self, position: u64) -> *mut MaybeUninit<T> {
        self.slots[(position % self.capacity()) as usize].get()
    }
}

/// The audio thread's end of a stream of values for the web UI.
///
/// Created by
/// [`WebViewEventQueue::stream_sender()`](crate::WebViewEventQueue::stream_sender)
/// in `prepare()` and owned by the processor. See the
/// [module documentation](self).
pub struct WebViewStreamSender<T> {
    ring: Arc<Ring<T>>,
    /// The queue, to tell whether an editor is open.
    queue: WebViewEventQueue,
    /// The write position. Only this sender changes it.
    write: u64,
}

impl<T: Copy + Send> WebViewStreamSender<T> {
    /// Push a value. Returns whether it was kept: a value is dropped while
    /// no editor is open or when the ring is full.
    ///
    /// Realtime-safe: no lock, no allocation and no release of memory.
    #[inline]
    pub fn push(&mut self, value: T) -> bool {
        self.push_slice(std::slice::from_ref(&value)) == 1
    }

    /// Push values, oldest first. Returns how many were kept: none while no
    /// editor is open; when the ring is full the rest of `values` is dropped.
    ///
    /// Realtime-safe: no lock, no allocation and no release of memory.
    #[inline]
    pub fn push_slice(&mut self, values: &[T]) -> usize {
        if !self.queue.is_editor_open() {
            return 0;
        }

        let read = self.ring.read.load(Ordering::Acquire);
        let free = self.ring.capacity() - (self.write - read);
        let count = values.len().min(free as usize);
        for (offset, value) in values[..count].iter().enumerate() {
            // SAFETY: The slots from the write position up to the read
            // position plus the capacity belong to this sender: the queue
            // gave them back with the store of the read position loaded
            // above. `&mut self` makes this the only access from the
            // sender's side.
            unsafe { (*self.ring.slot(self.write + offset as u64)).write(*value) };
        }
        self.write += count as u64;
        // Hand the written slots to the queue.
        self.ring.write.store(self.write, Ordering::Release);
        count
    }
}

/// The queue's end of a stream sender.
struct StreamEnd<T> {
    ring: Weak<Ring<T>>,
    /// The name of the events, as JSON.
    name_json: String,
    /// Whether values that cannot be JSON were reported already.
    reported: bool,
}

impl<T: Copy + Serialize + Send + 'static> SenderEnd for StreamEnd<T> {
    #[cfg(debug_assertions)]
    fn name_json(&self) -> &str {
        &self.name_json
    }

    fn is_gone(&self) -> bool {
        self.ring.strong_count() == 0
    }

    fn editor_opened(&mut self, first: bool) {
        // What was pushed before is not for this editor: a push can pass the
        // check that an editor is open just before the last one closed. With
        // an editor open already, the values are that one's.
        if !first {
            return;
        }
        if let Some(ring) = self.ring.upgrade() {
            let write = ring.write.load(Ordering::Acquire);
            ring.read.store(write, Ordering::Release);
        }
    }

    fn page_loaded(&mut self) {
        // A stream starts from the page's first tick: what was pushed
        // before it reached no page.
    }

    fn deliver(&mut self, script: &mut String) {
        let Some(ring) = self.ring.upgrade() else {
            return;
        };
        let read = ring.read.load(Ordering::Relaxed);
        let write = ring.write.load(Ordering::Acquire);
        if read == write {
            return;
        }

        let values: Vec<T> = (read..write)
            .map(|position| {
                // SAFETY: The slots from the read position up to the write
                // position belong to the queue's end: the sender wrote them
                // before the store of the write position loaded above. The
                // lock of the queue's senders makes this the only access
                // from the queue's side.
                unsafe { (*ring.slot(position)).assume_init() }
            })
            .collect();
        // Hand the slots back to the sender.
        ring.read.store(write, Ordering::Release);

        match serde_json::to_string(&values) {
            Ok(json) => push_event(script, &self.name_json, &json),
            Err(error) => {
                if !std::mem::replace(&mut self.reported, true) {
                    log::error!("Failed to serialize the values of {}: {error}", self.name_json);
                }
            }
        }
    }
}

/// Create a sender with room for `capacity` values and the queue's end of
/// it.
pub(crate) fn stream_link<T>(
    name_json: String,
    capacity: usize,
    queue: WebViewEventQueue,
) -> (WebViewStreamSender<T>, Box<dyn SenderEnd>)
where
    T: Copy + Serialize + Send + 'static,
{
    assert!(capacity > 0, "a WebView stream sender needs room for at least one value");
    let ring = Arc::new(Ring {
        slots: (0..capacity).map(|_| UnsafeCell::new(MaybeUninit::uninit())).collect(),
        write: AtomicU64::new(0),
        read: AtomicU64::new(0),
    });
    let end = StreamEnd {
        ring: Arc::downgrade(&ring),
        name_json,
        reported: false,
    };
    (WebViewStreamSender { ring, queue, write: 0 }, Box::new(end))
}

#[cfg(test)]
mod tests {
    use crate::WebViewEventQueue;

    /// A queue as it is while an editor is open and its page loaded.
    fn open_queue() -> WebViewEventQueue {
        let events = WebViewEventQueue::new();
        events.claim();
        events.editor_opened();
        events.page_loaded();
        events
    }

    #[test]
    fn test_values_arrive_in_order_as_one_array() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 8);
        assert_eq!(events.take_script(), None);

        assert!(history.push(1u8));
        assert_eq!(history.push_slice(&[2, 3]), 2);
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"history\",[1,2,3]);")
        );
        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_a_full_ring_drops_the_newest_values() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 3);

        assert_eq!(history.push_slice(&[1u8, 2]), 2);
        assert_eq!(history.push_slice(&[3, 4, 5]), 1);
        assert!(!history.push(6));
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"history\",[1,2,3]);")
        );

        // The tick made room again.
        assert!(history.push(7));
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"history\",[7]);")
        );
    }

    #[test]
    fn test_the_ring_wraps_around() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 3);

        for tick in 0..10u32 {
            let values = [tick * 2, tick * 2 + 1];
            assert_eq!(history.push_slice(&values), 2);
            assert_eq!(
                events.take_script(),
                Some(format!(
                    "window.__BEAMER__._onEvent(\"history\",[{},{}]);",
                    values[0], values[1]
                ))
            );
        }
    }

    #[test]
    fn test_values_are_dropped_without_an_editor() {
        let events = WebViewEventQueue::new();
        events.claim();
        let mut history = events.stream_sender("history", 8);

        assert!(!history.push(1u8));
        assert_eq!(history.push_slice(&[2, 3]), 0);

        events.editor_opened();
        events.page_loaded();
        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_values_from_before_an_editor_opened_are_skipped() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 8);

        // Pushed after the last tick of an editor that closes now.
        assert!(history.push(1u8));
        events.editor_closed();

        events.editor_opened();
        events.page_loaded();
        assert_eq!(events.take_script(), None);
        assert!(history.push(2));
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"history\",[2]);")
        );
    }

    #[test]
    fn test_a_second_editor_does_not_skip_the_values_of_the_first() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 8);

        assert!(history.push(1u8));
        events.editor_opened();
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"history\",[1]);")
        );
    }

    #[test]
    fn test_not_a_number_is_null() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 8);

        history.push_slice(&[0.5f32, f32::NAN, f32::INFINITY]);
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"history\",[0.5,null,null]);")
        );
    }

    #[test]
    fn test_a_dropped_sender_delivers_nothing_more() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 8);

        history.push(1u8);
        drop(history);
        assert_eq!(events.take_script(), None);
        assert_eq!(events.sender_count(), 0);
    }

    #[test]
    fn test_preparing_again_does_not_pile_up_senders() {
        let events = open_queue();
        for prepare in 0..5u8 {
            let mut history = events.stream_sender("history", 8);
            history.push(prepare);
        }
        // Each sender was dropped before the next one was created.
        assert_eq!(events.sender_count(), 1);
    }

    #[test]
    fn test_more_values_than_room_on_an_empty_ring() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 3);

        assert_eq!(history.push_slice(&[1u8, 2, 3, 4, 5]), 3);
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"history\",[1,2,3]);")
        );
        assert_eq!(history.push_slice(&[6, 7, 8, 9]), 3);
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"history\",[6,7,8]);")
        );
    }

    #[test]
    fn test_sender_dropped_on_another_thread_while_ticks_run() {
        let events = open_queue();
        for _ in 0..if cfg!(miri) { 5 } else { 200 } {
            let mut history = events.stream_sender("history", 4);
            let audio = std::thread::spawn(move || {
                for step in 1..=10u64 {
                    history.push(step);
                }
                drop(history);
            });
            while !audio.is_finished() {
                events.take_script();
            }
            audio.join().expect("the audio thread panicked");
            events.take_script();
            assert_eq!(events.sender_count(), 0);
        }
    }

    #[test]
    fn test_value_and_stream_senders_deliver_in_the_order_they_were_created() {
        let events = open_queue();
        let mut history = events.stream_sender("history", 8);
        let mut level = events.value_sender("level", 0u8);
        events.take_script();

        level.send(&2);
        history.push(1u8);
        assert_eq!(
            events.take_script().as_deref(),
            Some(
                "window.__BEAMER__._onEvent(\"history\",[1]);\
                 window.__BEAMER__._onValue(\"level\",2);"
            )
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "A second WebView sender is named \"history\"")]
    fn test_two_stream_senders_with_one_name_panic() {
        let events = open_queue();
        let _first = events.stream_sender::<u8>("history", 8);
        let _second = events.stream_sender::<u8>("history", 8);
    }

    #[test]
    #[should_panic(expected = "room for at least one value")]
    fn test_a_ring_without_room_panics() {
        let events = open_queue();
        let _history = events.stream_sender::<u8>("history", 0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "WebViewEventQueue::stream_sender() was called in process()")]
    fn test_creating_a_sender_in_process_panics() {
        let events = open_queue();
        let _scope = crate::ProcessScope::enter();
        let _history = events.stream_sender::<u8>("history", 8);
    }

    /// How many values the sender pushes in the concurrent test.
    const PUSHES: u64 = if cfg!(miri) { 200 } else { 20_000 };

    #[test]
    fn test_every_value_arrives_while_the_sender_and_ticks_run_at_once() {
        // A small ring, so that the sender often finds it full and the
        // positions wrap around many times.
        let events = open_queue();
        let mut history = events.stream_sender("history", 16);

        let audio = std::thread::spawn(move || {
            let mut next = 0u64;
            while next < PUSHES {
                // Push what fits, then try again with the rest.
                let values: Vec<u64> = (next..(next + 5).min(PUSHES)).collect();
                next += history.push_slice(&values) as u64;
            }
            history
        });

        let mut expected = 0u64;
        let mut check = |script: String| {
            let json = script
                .strip_prefix("window.__BEAMER__._onEvent(\"history\",")
                .and_then(|rest| rest.strip_suffix(");"))
                .expect("one event");
            let values: Vec<u64> = serde_json::from_str(json).expect("an array");
            assert!(!values.is_empty());
            for value in values {
                assert_eq!(value, expected);
                expected += 1;
            }
        };
        while !audio.is_finished() {
            if let Some(script) = events.take_script() {
                check(script);
            }
        }
        let _history = audio.join().expect("the audio thread panicked");
        if let Some(script) = events.take_script() {
            check(script);
        }
        assert_eq!(expected, PUSHES);
    }
}
