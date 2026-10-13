//! Handing values over to the audio thread.
//!
//! A [`Handover`] holds a value that host and UI threads replace as a whole
//! and that the audio thread reads without waiting: a sample that was
//! loaded, a wavetable, a curve the user drew. It is the counterpart of a
//! parameter for data that is not a number.
//!
//! The threads that set a value may allocate and may wait for each other.
//! The audio thread owns a [`HandoverReader`], which the plugin creates in
//! `prepare()`. Reading through it takes no lock, never allocates and never
//! frees memory: a value the reader gives up is released later, by a thread
//! that sets a value or by the thread that drops the reader.
//!
//! Dropping a reader does free memory and may drop values. That is fine
//! where it normally happens: a reader lives in the processor, and the
//! framework drops a processor on a host thread. Do not create or drop a
//! reader inside `process()`.
//!
//! ```ignore
//! #[derive(Parameters)]
//! struct SamplerParameters {
//!     #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
//!     gain: FloatParameter,
//!     // Not a parameter and not saved: the decoded sample.
//!     sample: Handover<Vec<f32>>,
//! }
//!
//! // On a host or UI thread, when the user picked a file:
//! parameters.sample.set(decode(&path));
//!
//! // In prepare():
//! SamplerProcessor { sample: self.parameters.sample.reader(), /* ... */ }
//!
//! // In process(), once per block:
//! let sample = self.sample.latest();
//! ```
//!
//! For a value that is saved with the plugin state, see
//! [`State`](crate::State).

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

/// Set in `Link::middle` when the middle slot holds a value the reader has
/// not taken over yet.
const FRESH: u8 = 0b100;

/// The bits of `Link::middle` that hold a slot index.
const INDEX: u8 = 0b011;

/// What one reader and the setters share: three slots that are passed
/// around without copying their content.
///
/// At every moment one slot belongs to the reader (its front slot), one to
/// the setters (the back slot of this link) and one is in between (the
/// middle slot). A setter fills its back slot and exchanges it with the
/// middle slot. The reader exchanges its front slot with the middle slot
/// when that holds something new. Each exchange is one atomic swap of an
/// index, so neither side ever waits.
struct Link<T> {
    slots: [UnsafeCell<Option<Arc<T>>>; 3],
    /// The index of the middle slot, with `FRESH` when a setter put it
    /// there since the reader last took one.
    middle: AtomicU8,
}

// SAFETY: A slot is only accessed by the side that owns it at that moment:
// the front slot by the one reader, the back slot by a setter that holds the
// lock of the `Handover`. Ownership of a slot changes hands through the
// atomic swaps of `middle`, which order the accesses. The content is an
// `Arc<T>`, which may go to another thread when `T` is `Send + Sync`.
unsafe impl<T: Send + Sync> Send for Link<T> {}
// SAFETY: See above.
unsafe impl<T: Send + Sync> Sync for Link<T> {}

/// The setter side of one link.
struct SetterEnd<T> {
    link: Weak<Link<T>>,
    /// The index of the slot the setters own.
    back: u8,
}

/// What the threads that set a value share, behind the lock.
struct Host<T> {
    latest: Arc<T>,
    links: Vec<SetterEnd<T>>,
}

/// A value that host and UI threads replace and the audio thread reads
/// without waiting.
///
/// See the [module documentation](self).
pub struct Handover<T> {
    host: Mutex<Host<T>>,
    generation: AtomicU64,
}

impl<T: Send + Sync> Handover<T> {
    /// Create a hand-over that holds `value`.
    pub fn new(value: T) -> Self {
        Self::new_shared(Arc::new(value))
    }

    /// Create a hand-over that holds a value which is shared already.
    pub fn new_shared(value: Arc<T>) -> Self {
        Self {
            host: Mutex::new(Host {
                latest: value,
                links: Vec::new(),
            }),
            generation: AtomicU64::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Host<T>> {
        // A poisoned lock only means a panic in another setter. What it
        // guards is replaced as a whole and stays usable.
        self.host
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Get the current value.
    ///
    /// For host and UI threads: this takes a lock that other host threads
    /// may hold. The audio thread reads through a [`HandoverReader`].
    pub fn get(&self) -> Arc<T> {
        Arc::clone(&self.lock().latest)
    }

    /// Replace the value.
    ///
    /// Takes the value itself or an `Arc` of it. For host and UI threads:
    /// this takes a lock, allocates and may free values that are no longer
    /// used. Every reader gets the new value with its next
    /// [`changed()`](HandoverReader::changed) or
    /// [`latest()`](HandoverReader::latest).
    pub fn set(&self, value: impl Into<Arc<T>>) {
        let value: Arc<T> = value.into();
        // What the readers and this hand-over no longer use, and the links
        // this call holds on to. Both are dropped after the lock is
        // released: a value may do anything when it is dropped, and a link
        // whose reader went away meanwhile takes its values with it.
        let mut unused: Vec<Arc<T>> = Vec::new();
        let mut links: Vec<Arc<Link<T>>> = Vec::new();

        let mut host = self.lock();
        unused.push(std::mem::replace(&mut host.latest, Arc::clone(&value)));
        host.links.retain_mut(|end| {
            let Some(link) = end.link.upgrade() else {
                // The reader is gone.
                return false;
            };

            // SAFETY: The back slot belongs to the setters, and this is the
            // only setter: it holds the lock.
            let slot = unsafe { &mut *link.slots[usize::from(end.back)].get() };
            unused.extend(slot.replace(Arc::clone(&value)));

            // Hand the filled slot to the reader and take the slot it is
            // not using.
            let previous = link.middle.swap(end.back | FRESH, Ordering::AcqRel);
            end.back = previous & INDEX;

            // SAFETY: As above, for the new back slot.
            let slot = unsafe { &mut *link.slots[usize::from(end.back)].get() };
            unused.extend(slot.take());
            links.push(link);
            true
        });
        self.generation.fetch_add(1, Ordering::Release);
        drop(host);
        drop(links);
        drop(unused);
    }

    /// Create a reader for the audio thread.
    ///
    /// Call this in `prepare()` and keep the reader in the processor. It
    /// starts with the current value.
    pub fn reader(&self) -> HandoverReader<T> {
        let mut host = self.lock();
        // Forget the readers that are gone, so that preparing again and
        // again without a `set` in between does not pile them up.
        host.links.retain(|end| end.link.strong_count() > 0);
        let link = Arc::new(Link {
            slots: [
                UnsafeCell::new(Some(Arc::clone(&host.latest))),
                UnsafeCell::new(None),
                UnsafeCell::new(None),
            ],
            middle: AtomicU8::new(1),
        });
        host.links.push(SetterEnd {
            link: Arc::downgrade(&link),
            back: 2,
        });
        HandoverReader { link, front: 0 }
    }

    /// How often the value was set.
    ///
    /// Takes no lock. A UI can compare this with the number it saw last to
    /// find out whether it has to read the value again.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
}

impl<T: Default + Send + Sync> Default for Handover<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// The audio thread's end of a [`Handover`].
///
/// Created by [`Handover::reader()`] in `prepare()` and owned by the
/// processor. Its methods are realtime-safe: no lock, no allocation and no
/// release of memory.
pub struct HandoverReader<T> {
    link: Arc<Link<T>>,
    /// The index of the slot this reader owns.
    front: u8,
}

impl<T: Send + Sync> HandoverReader<T> {
    /// The value this reader holds: the one it took over last.
    ///
    /// A value that was set since then is not visible here before
    /// [`changed()`](Self::changed) or [`latest()`](Self::latest) took it
    /// over. To follow the hand-over, call one of those once per block.
    #[inline]
    pub fn current(&self) -> &T {
        // SAFETY: The front slot belongs to this reader. No setter touches
        // it while it is the front slot.
        match unsafe { &*self.link.slots[usize::from(self.front)].get() } {
            Some(value) => value,
            // The front slot is filled when the reader is created, and
            // every slot the reader takes over was filled by a setter.
            None => unreachable!("the front slot of a hand-over is never empty"),
        }
    }

    /// Take over a value that was set since the last call.
    ///
    /// Returns the new value once, or `None` when nothing was set. If the
    /// value was set several times, the reader gets the last one.
    #[inline]
    pub fn changed(&mut self) -> Option<&T> {
        if self.link.middle.load(Ordering::Relaxed) & FRESH == 0 {
            return None;
        }
        // Give the slot with the old value back and take the fresh one.
        // The old value stays alive in its slot until a setter collects it.
        let previous = self.link.middle.swap(self.front, Ordering::AcqRel);
        self.front = previous & INDEX;
        Some(self.current())
    }

    /// Take over a value that was set since the last call, if any, and
    /// return the current one.
    ///
    /// This is the call for the start of a block when the processor only
    /// needs the value and not the moment it changed.
    #[inline]
    pub fn latest(&mut self) -> &T {
        self.changed();
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::thread::ThreadId;

    #[test]
    fn test_set_and_get() {
        let handover = Handover::new(1);
        assert_eq!(*handover.get(), 1);
        assert_eq!(handover.generation(), 0);

        handover.set(2);
        assert_eq!(*handover.get(), 2);
        assert_eq!(handover.generation(), 1);
    }

    #[test]
    fn test_reader_starts_with_the_current_value() {
        let handover = Handover::new(String::from("first"));
        handover.set(String::from("second"));

        let mut reader = handover.reader();
        assert_eq!(reader.current(), "second");
        assert_eq!(reader.changed(), None);
    }

    #[test]
    fn test_reader_sees_each_change_once() {
        let handover = Handover::new(0);
        let mut reader = handover.reader();

        handover.set(1);
        // Not taken over yet.
        assert_eq!(*reader.current(), 0);
        assert_eq!(reader.changed(), Some(&1));
        assert_eq!(reader.changed(), None);
        assert_eq!(*reader.current(), 1);

        handover.set(2);
        assert_eq!(*reader.latest(), 2);
        assert_eq!(reader.changed(), None);
    }

    #[test]
    fn test_reader_gets_the_last_of_several_values() {
        let handover = Handover::new(0);
        let mut reader = handover.reader();

        for value in 1..=5 {
            handover.set(value);
        }
        assert_eq!(reader.changed(), Some(&5));
        assert_eq!(reader.changed(), None);
    }

    #[test]
    fn test_readers_are_independent() {
        let handover = Handover::new(0);
        let mut first = handover.reader();
        let mut second = handover.reader();

        handover.set(1);
        assert_eq!(first.changed(), Some(&1));
        handover.set(2);
        assert_eq!(second.changed(), Some(&2));
        assert_eq!(second.changed(), None);
        assert_eq!(first.changed(), Some(&2));
    }

    #[test]
    fn test_new_reader_after_the_old_one_is_gone() {
        let handover = Handover::new(0);
        let mut reader = handover.reader();
        handover.set(1);
        assert_eq!(*reader.latest(), 1);

        // The processor is dropped and the plugin is prepared again.
        drop(reader);
        handover.set(2);
        let mut reader = handover.reader();
        assert_eq!(*reader.current(), 2);
        handover.set(3);
        assert_eq!(reader.changed(), Some(&3));

        // The link of the first reader was forgotten.
        assert_eq!(handover.lock().links.len(), 1);
    }

    /// A value that counts its drops and remembers the thread of the last.
    struct Tracked {
        drops: Arc<AtomicUsize>,
        last_thread: Arc<Mutex<Option<ThreadId>>>,
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            *self.last_thread.lock().unwrap() = Some(std::thread::current().id());
        }
    }

    #[test]
    fn test_reader_never_drops_a_value() {
        let drops = Arc::new(AtomicUsize::new(0));
        let last_thread = Arc::new(Mutex::new(None));
        let tracked = || Tracked {
            drops: Arc::clone(&drops),
            last_thread: Arc::clone(&last_thread),
        };

        let handover = Handover::new(tracked());
        let mut reader = handover.reader();
        handover.set(tracked());

        // The audio thread takes the new value over and moves on. The
        // first value is given up, and nothing is dropped.
        let reader = std::thread::spawn(move || {
            assert!(reader.changed().is_some());
            assert!(reader.changed().is_none());
            reader
        })
        .join()
        .expect("the audio thread panicked");
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        // The next setter collects the first value, on its own thread.
        handover.set(tracked());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            *last_thread.lock().unwrap(),
            Some(std::thread::current().id())
        );

        // The rest goes with the reader and the hand-over.
        drop(reader);
        drop(handover);
        assert_eq!(drops.load(Ordering::SeqCst), 3);
    }

    /// How many values a setter sets in the concurrent tests.
    const SETS: u64 = if cfg!(miri) { 200 } else { 20_000 };

    #[test]
    fn test_setters_and_a_reader_at_the_same_time() {
        use std::sync::atomic::AtomicBool;

        // Each value is a pair that only a read of a slot in the wrong
        // hands could break.
        let handover = Arc::new(Handover::new((0u64, 0u64)));
        let mut reader = handover.reader();
        let setters_done = Arc::new(AtomicBool::new(false));

        let setters: Vec<_> = (0..2u64)
            .map(|setter| {
                let handover = Arc::clone(&handover);
                std::thread::spawn(move || {
                    for step in 1..=SETS {
                        let value = step * 2 + setter;
                        handover.set((value, !value));
                    }
                })
            })
            .collect();

        // The reader runs for as long as the setters do.
        let audio = {
            let setters_done = Arc::clone(&setters_done);
            std::thread::spawn(move || {
                let mut changes = 0u64;
                while !setters_done.load(Ordering::Acquire) {
                    if let Some(&(value, check)) = reader.changed() {
                        assert_eq!(check, !value);
                        changes += 1;
                    }
                    let &(value, check) = reader.current();
                    assert_eq!(check, !value);
                }
                (reader, changes)
            })
        };

        for setter in setters {
            setter.join().expect("a setter panicked");
        }
        setters_done.store(true, Ordering::Release);
        let (mut reader, changes) = audio.join().expect("the reader panicked");
        assert!(changes > 0, "the reader never saw a change");

        // Whatever happened in between, the reader ends on the last value.
        let last = *handover.get();
        assert_eq!(*reader.latest(), last);
        assert_eq!(handover.generation(), 2 * SETS);
    }

    #[test]
    fn test_readers_come_and_go_while_values_are_set() {
        let handover = Arc::new(Handover::new(vec![0u64; 4]));

        let setter = {
            let handover = Arc::clone(&handover);
            std::thread::spawn(move || {
                for step in 1..=SETS {
                    handover.set(vec![step; 4]);
                }
            })
        };

        // Like a host that prepares the plugin again and again: readers
        // are created, used and dropped on this thread while the other one
        // sets values.
        let mut readers = 0u64;
        while !setter.is_finished() {
            let mut first = handover.reader();
            let mut second = handover.reader();
            let value = first.latest();
            assert!(value.iter().all(|step| *step == value[0]));
            let value = second.latest();
            assert!(value.iter().all(|step| *step == value[0]));
            readers += 2;
        }
        setter.join().expect("the setter panicked");
        assert!(readers > 0);

        // Only readers that still exist are remembered.
        let reader = handover.reader();
        assert_eq!(handover.lock().links.len(), 1);
        assert_eq!(reader.current(), &vec![SETS; 4]);
    }

    #[test]
    fn test_readers_that_are_gone_are_forgotten_without_a_set() {
        let handover = Handover::new(0);
        for _ in 0..10 {
            drop(handover.reader());
        }
        let _reader = handover.reader();
        assert_eq!(handover.lock().links.len(), 1);
    }

    #[test]
    fn test_set_takes_a_shared_value() {
        let handover = Handover::new(String::from("first"));
        let shared = Arc::new(String::from("second"));

        handover.set(Arc::clone(&shared));
        assert!(Arc::ptr_eq(&handover.get(), &shared));
    }

    /// A value whose drop panics, once.
    struct PanicsOnDrop(Arc<AtomicUsize>);

    impl Drop for PanicsOnDrop {
        fn drop(&mut self) {
            if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("a value that panics when it is dropped");
            }
        }
    }

    #[test]
    fn test_value_that_panics_in_drop_does_not_break_the_handover() {
        let drops = Arc::new(AtomicUsize::new(0));
        let handover = Handover::new(PanicsOnDrop(Arc::clone(&drops)));

        // The first value is dropped by this set, after the lock is
        // released: the panic reaches the caller and the lock is intact.
        let set = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handover.set(PanicsOnDrop(Arc::clone(&drops)));
        }));
        assert!(set.is_err());
        assert!(!handover.host.is_poisoned());

        handover.set(PanicsOnDrop(Arc::clone(&drops)));
        assert_eq!(handover.generation(), 2);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }
}
