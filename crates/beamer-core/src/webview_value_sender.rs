//! The newest value from the audio thread to the web UI.
//!
//! A [`WebViewValueSender`] sends a value that the web UI shows as it is
//! now: a level meter, a gain reduction, a spectrum. The audio thread sends
//! as often as it likes; at each tick of the GUI (about 60 times a second)
//! the newest value is delivered and the ones before it are gone. A value
//! is delivered once. A page that loads gets it again.
//!
//! The plugin creates the sender in `prepare()` from the queue of its model
//! and keeps it in the processor. Sending takes no lock, never allocates and
//! never frees memory:
//!
//! ```ignore
//! // In prepare(), before `self` moves into the processor:
//! MeterProcessor { levels: self.events.value_sender("levels", [0.0f32; 2]), model: self }
//!
//! // In process(), once per block. No GUI, nothing to measure:
//! if self.model.events.is_gui_open() {
//!     self.levels.send(&self.peaks());
//! }
//! ```
//!
//! ```js
//! __BEAMER__.on("levels", ([left, right]) => meter.show(left, right));
//! ```
//!
//! A value sender keeps what happened between two ticks only when the
//! plugin does: a meter holds its peaks (with a decay) instead of sending
//! the peak of each block. For every value in order, see
//! [`WebViewStreamSender`](crate::WebViewStreamSender).
//!
//! Dropping a sender frees memory. That is fine where it normally happens:
//! a sender lives in the processor, which the framework drops on a host
//! thread. Do not create or drop a sender inside `process()`.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Weak};

use serde::Serialize;

use crate::webview_events::{push_event, SenderEnd};

/// Set in `Link::middle` when the middle slot holds a value the sender put
/// there since the queue last took one.
const FRESH: u8 = 0b100;

/// The bits of `Link::middle` that hold a slot index.
const INDEX: u8 = 0b011;

/// What a sender and the queue share: three slots that change hands without
/// copying their content.
///
/// At every moment one slot belongs to the sender (its back slot), one to
/// the queue (its front slot) and one is in between (the middle slot). The
/// sender fills its back slot and exchanges it with the middle slot. The
/// queue exchanges its front slot with the middle slot when that holds
/// something new. Each exchange is one atomic swap of an index, so neither
/// side ever waits.
struct Link<T> {
    slots: [UnsafeCell<T>; 3],
    /// The index of the middle slot, with `FRESH` when the sender put it
    /// there since the queue last took one.
    middle: AtomicU8,
}

// SAFETY: A slot is only accessed by the side that owns it at that moment:
// the back slot by the one sender (`send` takes `&mut self`), the front slot
// by the queue's end under the lock of the queue's senders. Ownership of a
// slot changes hands through the atomic swaps of `middle`, which order the
// accesses. A value moves between threads as a whole and is never accessed
// from two threads at once, so `T: Send` is enough.
unsafe impl<T: Send> Send for Link<T> {}
// SAFETY: See above.
unsafe impl<T: Send> Sync for Link<T> {}

/// The audio thread's end of a value for the web UI.
///
/// Created by
/// [`WebViewEventQueue::value_sender()`](crate::WebViewEventQueue::value_sender)
/// in `prepare()` and owned by the processor. See the
/// [module documentation](self).
pub struct WebViewValueSender<T> {
    link: Arc<Link<T>>,
    /// The index of the slot this sender owns.
    back: u8,
}

impl<T: Clone + Send> WebViewValueSender<T> {
    /// Send a value. It replaces one that was sent since the last tick.
    ///
    /// Realtime-safe: copies `value` into memory the sender holds with
    /// `clone_from`, so a `Vec` of the same length as the initial value
    /// does not allocate. A longer one does; for such values fill the
    /// sender in place with [`send_with()`](Self::send_with).
    ///
    /// Sends also while no GUI is open, so that a GUI that opens shows the
    /// value of now. To skip the measurement itself, ask
    /// [`is_gui_open()`](crate::WebViewEventQueue::is_gui_open) first.
    #[inline]
    pub fn send(&mut self, value: &T) {
        self.slot().clone_from(value);
        self.publish();
    }

    /// Send a value that `fill` writes in place.
    ///
    /// What `fill` finds is an older value, not the one sent last: it has
    /// to write the whole value. Realtime-safe as long as `fill` is.
    #[inline]
    pub fn send_with(&mut self, fill: impl FnOnce(&mut T)) {
        fill(self.slot());
        self.publish();
    }

    /// The slot this sender owns.
    #[inline]
    fn slot(&mut self) -> &mut T {
        // SAFETY: The back slot belongs to this sender. The queue does not
        // touch it while it is the back slot. `&mut self` makes this the
        // only access from the sender's side.
        unsafe { &mut *self.link.slots[usize::from(self.back)].get() }
    }

    /// Hand the filled slot to the queue and take the slot it is not using.
    #[inline]
    fn publish(&mut self) {
        let previous = self.link.middle.swap(self.back | FRESH, Ordering::AcqRel);
        self.back = previous & INDEX;
    }
}

/// The queue's end of a value sender.
struct ValueEnd<T> {
    link: Weak<Link<T>>,
    /// The name of the events, as JSON.
    name_json: String,
    /// The index of the slot the queue owns.
    front: u8,
    /// Whether the value in the front slot goes out at the next tick even
    /// when the sender sent nothing new: a page loaded.
    resend: bool,
    /// Whether a value that cannot be JSON was reported already.
    reported: bool,
}

impl<T: Serialize + Send + 'static> SenderEnd for ValueEnd<T> {
    #[cfg(debug_assertions)]
    fn name_json(&self) -> &str {
        &self.name_json
    }

    fn is_gone(&self) -> bool {
        self.link.strong_count() == 0
    }

    fn gui_opened(&mut self, _first: bool) {
        // The editor's page has not loaded yet: the value goes out again
        // when it has.
    }

    fn page_loaded(&mut self) {
        self.resend = true;
    }

    fn deliver(&mut self, script: &mut String) {
        let Some(link) = self.link.upgrade() else {
            return;
        };
        if link.middle.load(Ordering::Relaxed) & FRESH != 0 {
            // Give the slot with the old value back and take the fresh one.
            let previous = link.middle.swap(self.front, Ordering::AcqRel);
            self.front = previous & INDEX;
        } else if !self.resend {
            return;
        }
        self.resend = false;

        // SAFETY: The front slot belongs to the queue's end. The sender does
        // not touch it while it is the front slot. The lock of the queue's
        // senders makes this the only access from the queue's side.
        let value = unsafe { &*link.slots[usize::from(self.front)].get() };
        match serde_json::to_string(value) {
            Ok(json) => push_event(script, &self.name_json, &json),
            Err(error) => {
                if !std::mem::replace(&mut self.reported, true) {
                    log::error!("Failed to serialize a value of {}: {error}", self.name_json);
                }
            }
        }
    }
}

/// Create a sender and the queue's end of it, with `initial` in all three
/// slots. The queue's front slot is the first one, the sender's back slot
/// the last one.
pub(crate) fn value_link<T>(name_json: String, initial: T) -> (WebViewValueSender<T>, Box<dyn SenderEnd>)
where
    T: Clone + Serialize + Send + 'static,
{
    let link = Arc::new(Link {
        slots: [
            UnsafeCell::new(initial.clone()),
            UnsafeCell::new(initial.clone()),
            UnsafeCell::new(initial),
        ],
        middle: AtomicU8::new(1),
    });
    let end = ValueEnd {
        link: Arc::downgrade(&link),
        name_json,
        front: 0,
        // A page that is loaded already shows the initial value.
        resend: true,
        reported: false,
    };
    (WebViewValueSender { link, back: 2 }, Box::new(end))
}

#[cfg(test)]
mod tests {
    use crate::WebViewEventQueue;

    /// A queue as it is while a GUI is open and its page loaded.
    fn open_queue() -> WebViewEventQueue {
        let events = WebViewEventQueue::new();
        events.claim();
        events.editor_opened();
        events.page_loaded();
        events
    }

    #[test]
    fn test_a_sent_value_is_delivered_once() {
        let events = open_queue();
        let mut levels = events.value_sender("levels", [0.0f32; 2]);
        // The initial value reaches the GUI that is open.
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"levels\",[0.0,0.0]);")
        );

        levels.send(&[0.5, 0.25]);
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"levels\",[0.5,0.25]);")
        );
        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_the_last_of_several_values_is_delivered() {
        let events = open_queue();
        let mut level = events.value_sender("level", 0u32);
        events.take_script();

        for value in 1..=5 {
            level.send(&value);
        }
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"level\",5);")
        );
        assert_eq!(events.take_script(), None);
    }

    #[test]
    fn test_a_page_that_loads_gets_the_current_value() {
        let events = WebViewEventQueue::new();
        events.claim();
        let mut level = events.value_sender("level", 0u32);

        // Sent while no GUI is open: kept for the one that opens.
        level.send(&7);
        events.editor_opened();
        // The first tick may come before the page loaded and reach nothing.
        assert!(events.take_script().is_some());
        events.page_loaded();
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"level\",7);")
        );
        assert_eq!(events.take_script(), None);

        // The plugin sends nothing more. A GUI that opens next still gets
        // the value.
        events.editor_closed();
        events.editor_opened();
        events.page_loaded();
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"level\",7);")
        );
    }

    #[test]
    fn test_send_with_fills_in_place() {
        let events = open_queue();
        let mut spectrum = events.value_sender("spectrum", vec![0.0f32; 3]);
        events.take_script();

        spectrum.send_with(|bins| bins.copy_from_slice(&[1.0, 2.0, 3.0]));
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"spectrum\",[1.0,2.0,3.0]);")
        );
    }

    #[test]
    fn test_send_with_finds_an_older_value() {
        let events = open_queue();
        let mut level = events.value_sender("level", 0u32);
        events.take_script();

        level.send(&1);
        events.take_script();
        level.send(&2);
        // The slot the sender fills now held the value of two sends ago.
        level.send_with(|value| {
            assert_ne!(*value, 2);
            *value = 3;
        });
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"level\",3);")
        );
    }

    #[test]
    fn test_not_a_number_is_null() {
        let events = open_queue();
        let mut levels = events.value_sender("levels", [0.0f32; 2]);
        events.take_script();

        levels.send(&[f32::NAN, f32::NEG_INFINITY]);
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"levels\",[null,null]);")
        );
    }

    #[test]
    fn test_a_value_that_cannot_be_json_is_dropped() {
        use std::collections::BTreeMap;

        let events = open_queue();
        // JSON has no keys that are sequences.
        let mut data = BTreeMap::new();
        data.insert(vec![1u8], 1u8);
        let mut bad = events.value_sender("bad", data.clone());

        assert_eq!(events.take_script(), None);
        bad.send(&BTreeMap::new());
        assert_eq!(
            events.take_script().as_deref(),
            Some("window.__BEAMER__._onEvent(\"bad\",{});")
        );

        // Another value that cannot be JSON is dropped as well. The values
        // after it go out again.
        bad.send(&data);
        assert_eq!(events.take_script(), None);
        bad.send(&BTreeMap::new());
        assert!(events.take_script().is_some());
    }

    #[test]
    fn test_a_dropped_sender_delivers_nothing_more() {
        let events = open_queue();
        let mut level = events.value_sender("level", 0u32);
        events.take_script();

        level.send(&1);
        drop(level);
        assert_eq!(events.take_script(), None);
        assert_eq!(events.sender_count(), 0);
    }

    #[test]
    fn test_preparing_again_does_not_pile_up_senders() {
        let events = open_queue();
        for prepare in 0..5u32 {
            let mut level = events.value_sender("level", prepare);
            level.send(&prepare);
        }
        // Each sender was dropped before the next one was created.
        assert_eq!(events.sender_count(), 1);
    }

    #[test]
    fn test_senders_deliver_in_the_order_they_were_created() {
        let events = open_queue();
        let mut first = events.value_sender("first", 0u8);
        let mut second = events.value_sender("second", 0u8);
        events.take_script();

        second.send(&2);
        first.send(&1);
        events.emit("emitted", &0);
        assert_eq!(
            events.take_script().as_deref(),
            Some(
                "window.__BEAMER__._onEvent(\"emitted\",0);\
                 window.__BEAMER__._onEvent(\"first\",1);\
                 window.__BEAMER__._onEvent(\"second\",2);"
            )
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "A second WebView sender is named \"level\"")]
    fn test_two_senders_with_one_name_panic() {
        let events = open_queue();
        let _first = events.value_sender("level", 0u8);
        let _second = events.value_sender("level", 0u8);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "WebViewEventQueue::value_sender() was called in process()")]
    fn test_creating_a_sender_in_process_panics() {
        let events = open_queue();
        let _scope = crate::ProcessScope::enter();
        let _level = events.value_sender("level", 0u8);
    }

    /// How many values the sender sends in the concurrent tests.
    const SENDS: u64 = if cfg!(miri) { 200 } else { 20_000 };

    #[test]
    fn test_sender_and_ticks_at_the_same_time() {
        // Each value is a pair that only a read of a slot in the wrong hands
        // could break.
        let events = open_queue();
        let mut pair = events.value_sender("pair", (0u64, !0u64));

        let audio = std::thread::spawn(move || {
            for step in 1..=SENDS {
                pair.send(&(step, !step));
            }
            pair
        });

        let mut last = 0;
        let mut check = |script: String| {
            let json = script
                .strip_prefix("window.__BEAMER__._onEvent(\"pair\",")
                .and_then(|rest| rest.strip_suffix(");"))
                .expect("one event");
            let (value, inverse): (u64, u64) = serde_json::from_str(json).expect("a pair");
            assert_eq!(inverse, !value);
            // The newest value never goes back in time.
            assert!(value >= last);
            last = value;
        };
        while !audio.is_finished() {
            if let Some(script) = events.take_script() {
                check(script);
            }
        }
        let _pair = audio.join().expect("the audio thread panicked");
        if let Some(script) = events.take_script() {
            check(script);
        }
        // Whatever happened in between, the GUI ends on the last value.
        assert_eq!(last, SENDS);
    }

    #[test]
    fn test_sender_dropped_on_another_thread_while_ticks_run() {
        let events = open_queue();
        for _ in 0..if cfg!(miri) { 5 } else { 200 } {
            let mut level = events.value_sender("level", 0u64);
            let audio = std::thread::spawn(move || {
                for step in 1..=10 {
                    level.send(&step);
                }
                drop(level);
            });
            while !audio.is_finished() {
                events.take_script();
            }
            audio.join().expect("the audio thread panicked");
            events.take_script();
            assert_eq!(events.sender_count(), 0);
        }
    }
}
