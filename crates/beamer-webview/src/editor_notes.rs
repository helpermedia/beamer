//! Notes from the web UI to the processor.
//!
//! A page plays notes with `__BEAMER__.noteOn()` and `noteOff()`: drum pads
//! that sound when clicked, an on-screen keyboard. The editor sync puts each
//! note into the plugin instance's [`EditorNotes`] on the main thread. The
//! format wrapper's render takes them out with the [`EditorNoteReceiver`]
//! and adds them to the MIDI input of the block, at offset 0, so that
//! `process_midi()` and `process()` get them as they get the host's notes.
//!
//! The way between the two is a ring of a fixed size: the render takes
//! notes without a lock, never allocates and never frees memory. Only main
//! thread code adds notes, behind a lock that the render never takes.
//!
//! # No hanging notes
//!
//! The receiver keeps which notes of the editor it handed on. A note-off for
//! a note that is not on is dropped; a note-on for a note that is on ends it
//! first. The notes stay on through a reset of the processor and for the
//! next receiver, so that the note-off of a note still reaches the plugin:
//! one for a voice that is gone already does no harm, a lost one hangs a
//! note. [`release_all()`](EditorNotes::release_all) ends every note of the
//! editor that is on: the editor sync calls it when the page goes away (the
//! editor closes, the page loads again, its process ends), so a note the
//! page left on does not hang.
//!
//! # Room and age
//!
//! The ring has room for [`CAPACITY`] requests. A note-on goes in only
//! while fewer than [`NOTE_ON_ROOM`] wait, so that the note-off of each
//! fits, even while the host does not call the plugin. The last slot is
//! kept for a release of all notes. A note-off that does not fit (for a
//! note that was on before the host stopped calling) is owed: a bit per
//! channel and pitch records it, with MIDI's default release velocity. It
//! goes in once there is room, checked with each note and each frame of
//! the editor.
//!
//! A host that does not call the plugin for a while (a track it does not
//! process) leaves the notes in the ring. A note-on that waited longer than
//! [`MAX_AGE`] is dropped when the render takes it: a click should sound now
//! or not at all, not as part of a burst when the host calls again. Its
//! note-off is dropped with it, since that note is not on.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use beamer_core::midi::MAX_MIDI_EVENTS;
use beamer_core::{editor_note_id, scale, MidiBuffer, MidiChannel, MidiEvent, MidiNote};

/// A note-on goes in only while fewer requests than this wait.
pub const NOTE_ON_ROOM: usize = 128;

/// How many requests the ring holds: the note-ons, a note-off for each and
/// a release of all notes.
pub const CAPACITY: usize = 2 * NOTE_ON_ROOM + 1;

/// A note-on that waited longer than this is dropped.
pub const MAX_AGE: Duration = Duration::from_millis(500);

/// The release velocity of a note-off that does not name one: MIDI's
/// default, 64 of 127.
fn default_release_velocity() -> f32 {
    scale::velocity_from_7bit(64)
}

/// What the page asked for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Request {
    /// Start a note.
    On { channel: MidiChannel, pitch: MidiNote, velocity: f32 },
    /// End a note.
    Off { channel: MidiChannel, pitch: MidiNote, velocity: f32 },
    /// End every note of the editor that is on.
    ReleaseAll,
}

/// A request in the ring, with the time it was put in.
#[derive(Debug, Clone, Copy)]
struct Entry {
    request: Request,
    /// Nanoseconds from the ring's epoch.
    time: u64,
}

impl Entry {
    const EMPTY: Self = Self { request: Request::ReleaseAll, time: 0 };
}

/// What the main thread's side and the receiver share.
///
/// The positions count requests from the start and only grow; a position
/// modulo the capacity is the slot. The main thread writes the slots from
/// the write position up to the read position plus the capacity, then moves
/// the write position on. The receiver reads the slots from the read
/// position up to the write position, then moves the read position on.
/// Neither side waits for the other.
struct Shared {
    slots: Box<[UnsafeCell<Entry>]>,
    /// How many requests the main thread put in.
    write: AtomicU64,
    /// How many requests the receiver took.
    read: AtomicU64,
    /// The time the entries count from.
    epoch: Instant,
    /// Whether a receiver exists: there is only one reader.
    receiver: AtomicBool,
    /// The notes of the editor that are on: a bit per pitch of each
    /// channel. Only the one receiver uses it.
    on: UnsafeCell<[u128; 16]>,
    /// The main thread's side.
    producer: Mutex<Producer>,
}

// SAFETY: The map of the notes that are on is only accessed by the one
// receiver; a new one claims it after the last one released it (Release
// store in `Drop`, Acquire in `receiver()`). A slot is only accessed by the
// side that owns it at that moment: the slots from the read position up to
// the write position by the one receiver (`take_into` takes `&mut self`,
// `receiver()` makes at most one), the others by the producer under its
// lock. The producer hands slots over with a Release store of the write
// position that the receiver loads with Acquire; the receiver hands them
// back the same way through the read position. An entry is plain data,
// copied between threads.
unsafe impl Sync for Shared {}

impl Shared {
    /// The slot of a position.
    fn slot(&self, position: u64) -> *mut Entry {
        self.slots[(position % CAPACITY as u64) as usize].get()
    }

    /// Nanoseconds since the epoch. Reads the clock: no lock, no
    /// allocation.
    fn now(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }
}

/// The main thread's side of the ring.
struct Producer {
    /// The write position. Only the producer changes it.
    write: u64,
    /// The note-offs that did not fit: a bit per pitch of each channel.
    /// A second note-off for a note whose note-off is owed adds nothing.
    /// They go in with MIDI's default release velocity, in any order: each
    /// ends another note.
    owed: [u128; 16],
}

impl Producer {
    /// How many requests wait in the ring.
    fn used(&self, shared: &Shared) -> usize {
        (self.write - shared.read.load(Ordering::Acquire)) as usize
    }

    /// Put a request into the ring. The caller checked that it has room.
    fn push(&mut self, shared: &Shared, entry: Entry) {
        // SAFETY: The slots from the write position up to the read position
        // plus the capacity belong to the producer (see `Shared`): the
        // caller saw room for this one. The lock of the producer makes this
        // the only access from its side.
        unsafe { *shared.slot(self.write) = entry };
        self.write += 1;
        // Hand the slot to the receiver.
        shared.write.store(self.write, Ordering::Release);
    }

    /// Whether a note-off is owed.
    fn owes(&self) -> bool {
        self.owed.iter().any(|&notes| notes != 0)
    }

    /// Move the owed note-offs into the ring, as far as they fit.
    fn flush(&mut self, shared: &Shared) {
        for channel in 0..16u8 {
            while self.owed[channel as usize] != 0 {
                if self.used(shared) >= CAPACITY - 1 {
                    return;
                }
                let pitch = self.owed[channel as usize].trailing_zeros() as u8;
                self.owed[channel as usize] &= !(1u128 << pitch);
                let velocity = default_release_velocity();
                let entry = Entry { request: Request::Off { channel, pitch, velocity }, time: shared.now() };
                self.push(shared, entry);
            }
        }
    }
}

/// The notes of a plugin instance's editor on their way to the processor.
///
/// The format wrapper creates it with the instance, for a plugin with an
/// editor and MIDI input. Each editor sync gets a clone. Cloning gives
/// another handle to the same notes. See the
/// [module documentation](self).
#[derive(Clone)]
pub struct EditorNotes {
    shared: Arc<Shared>,
}

impl Default for EditorNotes {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EditorNotes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorNotes").finish_non_exhaustive()
    }
}

impl EditorNotes {
    /// Create the notes of an editor, with room for [`CAPACITY`] requests.
    /// Allocates: not for the audio thread.
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared {
                slots: (0..CAPACITY).map(|_| UnsafeCell::new(Entry::EMPTY)).collect(),
                write: AtomicU64::new(0),
                read: AtomicU64::new(0),
                epoch: Instant::now(),
                receiver: AtomicBool::new(false),
                on: UnsafeCell::new([0; 16]),
                producer: Mutex::new(Producer { write: 0, owed: [0; 16] }),
            }),
        }
    }

    /// The receiver for the render, or `None` while another one exists:
    /// the notes have one reader.
    ///
    /// For format wrappers, which create it with what renders (a processor,
    /// a render block). It takes the notes that wait and knows the notes the
    /// last receiver left on.
    pub fn receiver(&self) -> Option<EditorNoteReceiver> {
        self.shared
            .receiver
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(EditorNoteReceiver {
            read: self.shared.read.load(Ordering::Acquire),
            shared: Arc::clone(&self.shared),
        })
    }

    /// Start a note. `velocity` is clamped to above 0 up to 1: a note-on
    /// with velocity 0 has the smallest velocity, as in MIDI 2.0. Returns
    /// whether the note went in: not while the ring holds
    /// [`NOTE_ON_ROOM`] requests or a note-off is owed, nor for a
    /// velocity that is not a number or a channel or pitch out of range.
    pub fn note_on(&self, channel: MidiChannel, pitch: MidiNote, velocity: f32) -> bool {
        if velocity.is_nan() || channel > 15 || pitch > 127 {
            return false;
        }
        let velocity = velocity.clamp(scale::velocity_from_u16(1), 1.0);
        let shared = &*self.shared;
        let mut producer = self.producer();
        producer.flush(shared);
        // A note-on does not pass an owed note-off: it could be the note-off
        // of the same note.
        if producer.owes() || producer.used(shared) >= NOTE_ON_ROOM {
            return false;
        }
        let entry = Entry { request: Request::On { channel, pitch, velocity }, time: shared.now() };
        producer.push(shared, entry);
        true
    }

    /// End a note. `velocity` is the release velocity, MIDI's default (64
    /// of 127) when `None`, clamped to 0 to 1. A note-off that does not fit
    /// is owed: it goes in with the default release velocity once there is
    /// room, checked with each call and each [`flush()`](Self::flush).
    pub fn note_off(&self, channel: MidiChannel, pitch: MidiNote, velocity: Option<f32>) {
        if channel > 15 || pitch > 127 {
            return;
        }
        let velocity = velocity
            .filter(|velocity| !velocity.is_nan())
            .map_or_else(default_release_velocity, |velocity| velocity.clamp(0.0, 1.0));
        let shared = &*self.shared;
        let mut producer = self.producer();
        producer.flush(shared);
        if !producer.owes() && producer.used(shared) < CAPACITY - 1 {
            let entry = Entry { request: Request::Off { channel, pitch, velocity }, time: shared.now() };
            producer.push(shared, entry);
            return;
        }
        producer.owed[channel as usize] |= 1u128 << pitch;
    }

    /// End every note of the editor that is on, after the requests that
    /// wait. The owed note-offs are not needed any more.
    pub fn release_all(&self) {
        let shared = &*self.shared;
        let mut producer = self.producer();
        producer.owed = [0; 16];
        // Only a release takes the last slot. With the ring full, the last
        // request in it is therefore a release, with nothing after it.
        if producer.used(shared) < CAPACITY {
            producer.push(shared, Entry { request: Request::ReleaseAll, time: shared.now() });
        }
    }

    /// Move the owed note-offs into the ring, as far as they fit. The
    /// editor sync calls it at every frame.
    pub fn flush(&self) {
        let mut producer = self.producer();
        if producer.owes() {
            producer.flush(&self.shared);
        }
    }

    /// Lock the main thread's side.
    fn producer(&self) -> MutexGuard<'_, Producer> {
        // A poisoned lock only means a panic elsewhere: the positions are
        // only changed after a slot was written.
        self.shared.producer.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The render's end of the notes of the editor.
///
/// Created by [`EditorNotes::receiver()`] and kept by what renders. See the
/// [module documentation](self).
pub struct EditorNoteReceiver {
    shared: Arc<Shared>,
    /// The read position. Only this receiver changes it.
    read: u64,
}

impl std::fmt::Debug for EditorNoteReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorNoteReceiver").field("read", &self.read).finish_non_exhaustive()
    }
}

impl EditorNoteReceiver {
    /// Add the notes that wait to `buffer`, at offset 0 and in order.
    ///
    /// With `play` false (an offline render) note-ons are dropped; note-offs
    /// still end the notes that are on. What does not fit into `buffer`
    /// waits for the next call.
    ///
    /// Realtime-safe: no lock, no allocation and no release of memory. It
    /// reads the clock while notes wait.
    pub fn take_into(&mut self, buffer: &mut MidiBuffer, play: bool) {
        let write = self.shared.write.load(Ordering::Acquire);
        if self.read == write {
            return;
        }
        let now = self.shared.now();
        let max_age = MAX_AGE.as_nanos() as u64;

        while self.read < write {
            // SAFETY: The slots from the read position up to the write
            // position belong to the receiver: the producer wrote them
            // before the store of the write position loaded above. `&mut
            // self` and the one receiver make this the only access from this
            // side.
            let entry = unsafe { *self.shared.slot(self.read) };
            let room = MAX_MIDI_EVENTS - buffer.len();
            let taken = match entry.request {
                Request::On { channel, pitch, velocity } => {
                    if !play || now.saturating_sub(entry.time) > max_age {
                        true
                    } else {
                        let on = self.is_on(channel, pitch);
                        if room < 1 + on as usize {
                            false
                        } else {
                            // A note that is on ends before it starts again.
                            if on {
                                buffer.push(note_off(channel, pitch, default_release_velocity()));
                            }
                            buffer.push(MidiEvent::note_on(
                                0,
                                channel,
                                pitch,
                                velocity,
                                editor_note_id(channel, pitch),
                                0.0,
                                0,
                            ));
                            self.set_on(channel, pitch, true);
                            true
                        }
                    }
                }
                Request::Off { channel, pitch, velocity } => {
                    if !self.is_on(channel, pitch) {
                        true
                    } else if room < 1 {
                        false
                    } else {
                        buffer.push(note_off(channel, pitch, velocity));
                        self.set_on(channel, pitch, false);
                        true
                    }
                }
                Request::ReleaseAll => self.release_into(buffer),
            };
            if !taken {
                break;
            }
            self.read += 1;
        }
        // Hand the slots back to the producer.
        self.shared.read.store(self.read, Ordering::Release);
    }

    /// End the notes that are on, as far as `buffer` has room. Returns
    /// whether all ended.
    fn release_into(&mut self, buffer: &mut MidiBuffer) -> bool {
        for channel in 0..16u8 {
            while self.on()[channel as usize] != 0 {
                if buffer.len() == MAX_MIDI_EVENTS {
                    return false;
                }
                let pitch = self.on()[channel as usize].trailing_zeros() as u8;
                buffer.push(note_off(channel, pitch, default_release_velocity()));
                self.set_on(channel, pitch, false);
            }
        }
        true
    }

    /// The map of the notes that are on.
    fn on(&mut self) -> &mut [u128; 16] {
        // SAFETY: Only the one receiver uses the map (see `Shared`);
        // `&mut self` makes this the only access from it.
        unsafe { &mut *self.shared.on.get() }
    }

    fn is_on(&mut self, channel: MidiChannel, pitch: MidiNote) -> bool {
        self.on()[channel as usize & 0x0F] & (1u128 << (pitch & 0x7F)) != 0
    }

    fn set_on(&mut self, channel: MidiChannel, pitch: MidiNote, on: bool) {
        let bit = 1u128 << (pitch & 0x7F);
        let notes = &mut self.on()[channel as usize & 0x0F];
        if on {
            *notes |= bit;
        } else {
            *notes &= !bit;
        }
    }
}

impl Drop for EditorNoteReceiver {
    fn drop(&mut self) {
        // The next receiver may read.
        self.shared.receiver.store(false, Ordering::Release);
    }
}

/// A note-off of the editor at offset 0.
fn note_off(channel: MidiChannel, pitch: MidiNote, velocity: f32) -> MidiEvent<'static> {
    MidiEvent::note_off(0, channel, pitch, velocity, editor_note_id(channel, pitch), 0.0)
}

#[cfg(test)]
mod tests {
    use beamer_core::MidiEventKind;

    use super::*;

    /// The events of `buffer` as (on, channel, pitch, velocity, note id).
    fn events(buffer: &MidiBuffer) -> Vec<(bool, u8, u8, f32, i32)> {
        buffer
            .iter()
            .map(|event| {
                assert_eq!(event.sample_offset, 0);
                match event.event {
                    MidiEventKind::NoteOn(on) => (true, on.channel, on.pitch, on.velocity, on.note_id),
                    MidiEventKind::NoteOff(off) => (false, off.channel, off.pitch, off.velocity, off.note_id),
                    other => panic!("not a note: {other:?}"),
                }
            })
            .collect()
    }

    /// Take what waits into a fresh buffer.
    fn take(receiver: &mut EditorNoteReceiver) -> Vec<(bool, u8, u8, f32, i32)> {
        let mut buffer = MidiBuffer::new_boxed(0);
        receiver.take_into(&mut buffer, true);
        events(&buffer)
    }

    fn release() -> f32 {
        default_release_velocity()
    }

    #[test]
    fn test_notes_arrive_in_order_with_editor_note_ids() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        assert!(take(&mut receiver).is_empty());

        assert!(notes.note_on(0, 36, 1.0));
        assert!(notes.note_on(9, 38, 0.5));
        notes.note_off(0, 36, Some(0.25));
        notes.note_off(9, 38, None);
        assert_eq!(
            take(&mut receiver),
            [
                (true, 0, 36, 1.0, editor_note_id(0, 36)),
                (true, 9, 38, 0.5, editor_note_id(9, 38)),
                (false, 0, 36, 0.25, editor_note_id(0, 36)),
                (false, 9, 38, release(), editor_note_id(9, 38)),
            ]
        );
        assert!(take(&mut receiver).is_empty());
    }

    #[test]
    fn test_velocities_are_clamped_and_zero_is_no_note_off() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        assert!(notes.note_on(0, 60, 0.0));
        assert!(notes.note_on(0, 61, 2.0));
        assert!(!notes.note_on(0, 62, f32::NAN));
        notes.note_off(0, 61, Some(-1.0));
        let taken = take(&mut receiver);
        assert_eq!(taken.len(), 3);
        assert_eq!(taken[0], (true, 0, 60, scale::velocity_from_u16(1), editor_note_id(0, 60)));
        assert_eq!(taken[1].3, 1.0);
        assert_eq!(taken[2], (false, 0, 61, 0.0, editor_note_id(0, 61)));
    }

    #[test]
    fn test_notes_out_of_range_are_refused() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        assert!(!notes.note_on(16, 60, 1.0));
        assert!(!notes.note_on(0, 128, 1.0));
        notes.note_off(16, 60, None);
        assert!(take(&mut receiver).is_empty());
    }

    #[test]
    fn test_a_note_off_without_its_note_on_is_dropped() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        notes.note_off(0, 60, None);
        assert!(notes.note_on(0, 61, 1.0));
        notes.note_off(0, 61, None);
        notes.note_off(0, 61, None);
        assert_eq!(
            take(&mut receiver),
            [(true, 0, 61, 1.0, editor_note_id(0, 61)), (false, 0, 61, release(), editor_note_id(0, 61))]
        );
    }

    #[test]
    fn test_a_note_that_is_on_ends_before_it_starts_again() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        assert!(notes.note_on(0, 60, 1.0));
        assert!(notes.note_on(0, 60, 0.5));
        assert_eq!(
            take(&mut receiver),
            [
                (true, 0, 60, 1.0, editor_note_id(0, 60)),
                (false, 0, 60, release(), editor_note_id(0, 60)),
                (true, 0, 60, 0.5, editor_note_id(0, 60)),
            ]
        );
    }

    #[test]
    fn test_a_release_ends_every_note_that_is_on() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        assert!(notes.note_on(0, 36, 1.0));
        assert!(notes.note_on(15, 127, 1.0));
        assert!(notes.note_on(0, 40, 1.0));
        notes.note_off(0, 40, None);
        take(&mut receiver);

        notes.release_all();
        assert_eq!(
            take(&mut receiver),
            [(false, 0, 36, release(), editor_note_id(0, 36)), (false, 15, 127, release(), editor_note_id(15, 127))]
        );
        // Nothing is on any more.
        notes.release_all();
        notes.note_off(0, 36, None);
        assert!(take(&mut receiver).is_empty());
    }

    #[test]
    fn test_note_ons_leave_room_for_note_offs() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        // The host does not call: the note-ons fill their half.
        for pitch in 0..NOTE_ON_ROOM as u8 {
            assert!(notes.note_on(0, pitch, 1.0));
        }
        assert!(!notes.note_on(0, 127, 1.0));
        // Every note-on still gets its note-off.
        for pitch in 0..NOTE_ON_ROOM as u8 {
            notes.note_off(0, pitch, None);
        }
        assert!(!notes.producer().owes());
        let taken = take(&mut receiver);
        assert_eq!(taken.len(), 2 * NOTE_ON_ROOM);
        assert!(taken[NOTE_ON_ROOM..].iter().all(|event| !event.0));
    }

    /// Notes on before the host stops calling: two channels of 127.
    fn notes_on_before_a_stall(notes: &EditorNotes, receiver: &mut EditorNoteReceiver) {
        for pitch in 0..127u8 {
            assert!(notes.note_on(0, pitch, 1.0));
            assert!(notes.note_on(1, pitch, 1.0));
            take(receiver);
        }
    }

    /// Their note-offs while the host does not call, then as many more as
    /// fill the ring up to the release's slot.
    fn note_offs_during_the_stall(notes: &EditorNotes) {
        for pitch in 0..127u8 {
            notes.note_off(0, pitch, None);
            notes.note_off(1, pitch, None);
        }
        for _ in 254..CAPACITY - 1 {
            notes.note_off(0, 0, None);
        }
    }

    #[test]
    fn test_note_offs_that_do_not_fit_wait_for_room() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        notes_on_before_a_stall(&notes, &mut receiver);
        note_offs_during_the_stall(&notes);
        // This one is owed, once.
        notes.note_off(2, 60, None);
        notes.note_off(2, 60, None);
        assert_eq!(notes.producer().owed[2], 1u128 << 60);
        assert_eq!(notes.producer().owed.iter().map(|notes| notes.count_ones()).sum::<u32>(), 1);
        // A note-on does not pass the owed note-off.
        assert!(!notes.note_on(2, 60, 1.0));

        let mut buffer = MidiBuffer::new_boxed(0);
        receiver.take_into(&mut buffer, true);
        assert_eq!(buffer.len(), 254);
        assert!(events(&buffer).iter().all(|event| !event.0));
        // The note-off goes in, then a note-on again.
        notes.flush();
        assert!(!notes.producer().owes());
        assert!(notes.note_on(2, 60, 1.0));
        assert_eq!(take(&mut receiver), [(true, 2, 60, 1.0, editor_note_id(2, 60))]);
    }

    #[test]
    fn test_an_owed_note_off_ends_its_note_with_the_default_velocity() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        notes_on_before_a_stall(&notes, &mut receiver);
        assert!(notes.note_on(3, 10, 1.0));
        take(&mut receiver);
        note_offs_during_the_stall(&notes);
        notes.note_off(3, 10, Some(0.25));
        assert_eq!(take(&mut receiver).len(), 254);

        notes.flush();
        assert_eq!(take(&mut receiver), [(false, 3, 10, release(), editor_note_id(3, 10))]);
    }

    #[test]
    fn test_owed_note_offs_go_in_as_far_as_they_fit() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        notes_on_before_a_stall(&notes, &mut receiver);
        for pitch in 0..3u8 {
            assert!(notes.note_on(3, pitch, 1.0));
            take(&mut receiver);
        }
        note_offs_during_the_stall(&notes);
        for pitch in 0..3u8 {
            notes.note_off(3, pitch, None);
        }
        assert_eq!(notes.producer().owed[3], 0b111);

        // Room for two: the render takes two of the 256 requests (its
        // buffer has room for two events); the last slot is the release's.
        let mut buffer = MidiBuffer::new_boxed(0);
        for _ in 0..MAX_MIDI_EVENTS - 2 {
            buffer.push(MidiEvent::control_change(0, 0, 1, 0.5));
        }
        receiver.take_into(&mut buffer, true);
        assert_eq!(buffer.len(), MAX_MIDI_EVENTS);
        notes.flush();
        assert_eq!(notes.producer().owed[3], 0b100);
        take(&mut receiver);
        notes.flush();
        assert!(!notes.producer().owes());
        let taken = take(&mut receiver);
        assert_eq!(taken, [(false, 3, 2, release(), editor_note_id(3, 2))]);
    }

    #[test]
    fn test_a_release_takes_the_last_slot() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        notes_on_before_a_stall(&notes, &mut receiver);
        note_offs_during_the_stall(&notes);
        notes.note_off(2, 60, None);
        // The release drops the owed note-off; a second one finds the
        // ring full with a release last.
        notes.release_all();
        notes.release_all();
        assert!(!notes.producer().owes());

        let start = receiver.read;
        let mut buffer = MidiBuffer::new_boxed(0);
        receiver.take_into(&mut buffer, true);
        assert_eq!(buffer.len(), 254);
        assert_eq!(receiver.read - start, CAPACITY as u64);
    }

    #[test]
    fn test_old_note_ons_and_their_note_offs_are_dropped() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        assert!(notes.note_on(0, 36, 1.0));
        notes.note_off(0, 36, None);
        std::thread::sleep(MAX_AGE + Duration::from_millis(50));
        assert!(notes.note_on(0, 38, 1.0));
        assert_eq!(take(&mut receiver), [(true, 0, 38, 1.0, editor_note_id(0, 38))]);
    }

    #[test]
    fn test_an_offline_render_plays_no_notes_but_ends_them() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        assert!(notes.note_on(0, 36, 1.0));
        take(&mut receiver);

        assert!(notes.note_on(0, 38, 1.0));
        notes.note_off(0, 36, None);
        let mut buffer = MidiBuffer::new_boxed(0);
        receiver.take_into(&mut buffer, false);
        assert_eq!(events(&buffer), [(false, 0, 36, release(), editor_note_id(0, 36))]);
    }

    #[test]
    fn test_what_does_not_fit_the_buffer_waits() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        assert!(notes.note_on(0, 36, 1.0));
        assert!(notes.note_on(0, 36, 1.0));
        let mut buffer = MidiBuffer::new_boxed(0);
        for _ in 0..MAX_MIDI_EVENTS - 2 {
            buffer.push(MidiEvent::control_change(0, 0, 1, 0.5));
        }
        // The first note-on fits; the second needs room for the end of the
        // first as well.
        receiver.take_into(&mut buffer, true);
        assert_eq!(buffer.len(), MAX_MIDI_EVENTS - 1);
        let mut buffer = MidiBuffer::new_boxed(0);
        receiver.take_into(&mut buffer, true);
        assert_eq!(events(&buffer).len(), 2);
    }

    #[test]
    fn test_a_release_that_does_not_fit_goes_on_with_the_next_buffer() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        for pitch in 0..3u8 {
            assert!(notes.note_on(0, pitch, 1.0));
        }
        take(&mut receiver);
        notes.release_all();
        let mut buffer = MidiBuffer::new_boxed(0);
        for _ in 0..MAX_MIDI_EVENTS - 2 {
            buffer.push(MidiEvent::control_change(0, 0, 1, 0.5));
        }
        receiver.take_into(&mut buffer, true);
        assert_eq!(buffer.len(), MAX_MIDI_EVENTS);
        assert_eq!(take(&mut receiver).len(), 1);
        assert!(take(&mut receiver).is_empty());
    }

    #[test]
    fn test_there_is_one_receiver_at_a_time() {
        let notes = EditorNotes::new();
        let mut first = notes.receiver().unwrap();
        assert!(notes.receiver().is_none());
        assert!(notes.note_on(0, 36, 1.0));
        take(&mut first);
        drop(first);

        // A new receiver takes the notes that wait and ends the note the
        // last one left on.
        let mut second = notes.receiver().unwrap();
        assert!(notes.note_on(0, 38, 1.0));
        notes.note_off(0, 36, None);
        assert_eq!(
            take(&mut second),
            [(true, 0, 38, 1.0, editor_note_id(0, 38)), (false, 0, 36, release(), editor_note_id(0, 36))]
        );
    }

    #[test]
    fn test_notes_cross_threads_in_order() {
        let notes = EditorNotes::new();
        let mut receiver = notes.receiver().unwrap();
        let player = {
            let notes = notes.clone();
            std::thread::spawn(move || {
                for round in 0..2000u32 {
                    let pitch = (round % 128) as u8;
                    while !notes.note_on(0, pitch, 1.0) {
                        std::thread::yield_now();
                    }
                    notes.note_off(0, pitch, None);
                }
            })
        };
        // Until the player is done and nothing waits. No note-off is owed
        // here: each follows a note-on that found the ring at most half
        // full. A loaded machine may stall this thread long enough for
        // note-ons to age out: then their note-offs drop too and the pairs
        // that arrive still alternate.
        let mut seen = Vec::new();
        loop {
            let done = player.is_finished();
            let mut buffer = MidiBuffer::new_boxed(0);
            receiver.take_into(&mut buffer, true);
            seen.extend(events(&buffer).into_iter().map(|event| (event.0, event.2)));
            if done && buffer.is_empty() {
                break;
            }
            std::thread::yield_now();
        }
        player.join().unwrap();
        assert!(!seen.is_empty());
        for pair in seen.chunks(2) {
            assert_eq!(pair.len(), 2);
            assert!(pair[0].0 && !pair[1].0 && pair[0].1 == pair[1].1, "{pair:?}");
        }
    }
}
