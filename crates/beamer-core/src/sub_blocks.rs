//! Sample-accurate parameter changes for the format wrappers.
//!
//! A host block runs as sub-blocks that start at the sample offsets of its
//! parameter changes: the wrapper applies the changes due at the start of a
//! sub-block and then processes it. The MIDI of each sub-block reaches the
//! plugin with offsets relative to the start of that sub-block, as it does
//! for a whole block. The format-specific parts (how a change is applied and
//! how the buffers of a sample range are built) stay in the wrapper.
//!
//! Everything here runs on the audio thread and does not allocate.

use crate::midi::MidiEvent;

/// One sub-block of a host block.
pub struct SubBlock<'a, C> {
    /// First sample of the sub-block within the host block.
    pub start: usize,
    /// Its length in samples, at least 1.
    pub len: usize,
    /// The parameter changes to apply before processing it.
    pub changes: &'a [C],
    /// Its MIDI events, with offsets relative to `start`.
    pub midi: &'a mut [MidiEvent],
}

/// What no sub-block reached: everything when `num_samples` is 0, or what
/// comes after a sub-block that stopped the run, and the changes at or after
/// the end of the block. The MIDI keeps the offsets of the host block.
pub struct Unreached<'a, C> {
    /// Changes still to apply.
    pub changes: &'a [C],
    /// MIDI events still to process.
    pub midi: &'a mut [MidiEvent],
}

/// Run a host block of `num_samples` samples as sub-blocks that start at the
/// offsets of `changes`.
///
/// `changes` are sorted by `change_offset`, and `midi` by sample offset
/// (see [`sort_by_offset`]). Changes at the same offset start one sub-block
/// together; a change at or before the start of the block is applied
/// before the first one. There is no smallest sub-block size: changes one
/// sample apart make a sub-block of one sample. MIDI at or after the end of
/// the block, which a host should not send, goes to the last sub-block at
/// its last sample, so that a `process()` call follows it.
///
/// `run` processes one sub-block and returns `false` to stop, for example
/// when processing failed. What no sub-block reached is returned, so that
/// the caller can still apply those changes and process that MIDI.
pub fn run_sub_blocks<'a, C>(
    num_samples: usize,
    changes: &'a [C],
    change_offset: impl Fn(&C) -> u32,
    midi: &'a mut [MidiEvent],
    mut run: impl FnMut(SubBlock<'_, C>) -> bool,
) -> Unreached<'a, C> {
    let mut changes = changes;
    let mut midi = midi;
    let mut start = 0;

    while start < num_samples {
        // The changes due at the start of this sub-block.
        let due = changes
            .iter()
            .take_while(|change| change_offset(change) as usize <= start)
            .count();
        let (due_changes, later_changes) = changes.split_at(due);

        // The sub-block ends at the next change or at the end of the block.
        // The count above stopped at the first change after `start`, so the
        // sub-block has at least one sample and the loop advances, also for
        // a list that is not sorted.
        let end = later_changes
            .first()
            .map_or(num_samples, |change| (change_offset(change) as usize).min(num_samples));

        // The last sub-block takes the rest of the MIDI.
        let in_block = if end == num_samples {
            midi.len()
        } else {
            midi.iter()
                .take_while(|event| (event.sample_offset as usize) < end)
                .count()
        };
        let last_offset = (end - start - 1) as u32;
        let (block_midi, later_midi) = std::mem::take(&mut midi).split_at_mut(in_block);
        for event in block_midi.iter_mut() {
            event.sample_offset = event.sample_offset.saturating_sub(start as u32).min(last_offset);
        }

        let go_on = run(SubBlock {
            start,
            len: end - start,
            changes: due_changes,
            midi: block_midi,
        });

        changes = later_changes;
        midi = later_midi;
        start = end;
        if !go_on {
            break;
        }
    }

    Unreached { changes, midi }
}

/// Move MIDI events from the offsets of a sub-block to the offsets of its
/// host block, for MIDI the plugin sent while processing the sub-block.
pub fn offset_from_sub_block(events: &mut [MidiEvent], start: usize) {
    for event in events {
        event.sample_offset = event.sample_offset.saturating_add(start as u32);
    }
}

/// Sort by a sample offset on the audio thread: stable and without
/// allocating.
///
/// The standard library's stable sort allocates a scratch buffer, and an
/// unstable sort would reorder events at the same offset, of which the last
/// has to win. Event lists arrive sorted or nearly sorted, where an
/// insertion sort takes one pass. A list in reverse order takes quadratic
/// time, bounded by the capacity of the buffer it sorts.
pub fn sort_by_offset<T>(items: &mut [T], offset: impl Fn(&T) -> u32) {
    for i in 1..items.len() {
        let mut j = i;
        while j > 0 && offset(&items[j - 1]) > offset(&items[j]) {
            items.swap(j - 1, j);
            j -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A change: its offset and a tag to recognize it.
    type Change = (u32, char);

    fn note(offset: u32, pitch: u8) -> MidiEvent {
        MidiEvent::note_on(offset, 0, pitch, 1.0, -1, 0.0, 0)
    }

    fn pitch(event: &MidiEvent) -> u8 {
        match &event.event {
            crate::midi::MidiEventKind::NoteOn(note) => note.pitch,
            _ => panic!("a note-on"),
        }
    }

    /// Each sub-block as (start, len, change tags, MIDI as (offset, pitch)).
    type Run = Vec<(usize, usize, String, Vec<(u32, u8)>)>;

    fn run(num_samples: usize, changes: &[Change], midi: &mut [MidiEvent]) -> (Run, Vec<char>, Vec<u32>) {
        let mut blocks = Vec::new();
        let unreached = run_sub_blocks(num_samples, changes, |change| change.0, midi, |block| {
            let tags = block.changes.iter().map(|change| change.1).collect();
            let events = block.midi.iter().map(|event| (event.sample_offset, pitch(event))).collect();
            blocks.push((block.start, block.len, tags, events));
            true
        });
        let changes = unreached.changes.iter().map(|change| change.1).collect();
        let midi = unreached.midi.iter().map(|event| event.sample_offset).collect();
        (blocks, changes, midi)
    }

    #[test]
    fn test_without_changes_the_block_is_one_sub_block() {
        let (blocks, _, _) = run(64, &[], &mut []);
        assert_eq!(blocks, [(0, 64, String::new(), vec![])]);
    }

    #[test]
    fn test_changes_split_the_block_at_their_offsets() {
        let changes = [(0, 'a'), (32, 'b'), (32, 'c'), (33, 'd')];
        let (blocks, unreached, _) = run(64, &changes, &mut []);
        assert_eq!(
            blocks,
            [
                (0, 32, "a".into(), vec![]),
                (32, 1, "bc".into(), vec![]),
                (33, 31, "d".into(), vec![]),
            ]
        );
        assert!(unreached.is_empty());
    }

    #[test]
    fn test_midi_is_relative_to_its_sub_block() {
        let mut midi = [note(0, 60), note(31, 61), note(32, 62), note(63, 63)];
        let (blocks, _, _) = run(64, &[(32, 'a')], &mut midi);
        assert_eq!(
            blocks,
            [
                (0, 32, String::new(), vec![(0, 60), (31, 61)]),
                (32, 32, "a".into(), vec![(0, 62), (31, 63)]),
            ]
        );
    }

    #[test]
    fn test_what_no_sub_block_reached_is_returned() {
        // A change at or after the end of the block. The MIDI there goes
        // to the last sub-block at its last sample.
        let mut midi = [note(10, 60), note(64, 61), note(70, 62)];
        let (blocks, unreached, unreached_midi) = run(64, &[(32, 'a'), (64, 'b')], &mut midi);
        assert_eq!(
            blocks,
            [
                (0, 32, String::new(), vec![(10, 60)]),
                (32, 32, "a".into(), vec![(31, 61), (31, 62)]),
            ]
        );
        assert_eq!(unreached, ['b']);
        assert!(unreached_midi.is_empty());

        // A block without samples runs no sub-block.
        let mut midi = [note(0, 60)];
        let (blocks, unreached, unreached_midi) = run(0, &[(0, 'a')], &mut midi);
        assert!(blocks.is_empty());
        assert_eq!(unreached, ['a']);
        assert_eq!(unreached_midi, [0]);
    }

    #[test]
    fn test_a_stopped_run_returns_the_rest_with_block_offsets() {
        let mut midi = [note(5, 60), note(40, 61)];
        let changes = [(16, 'a'), (32, 'b')];
        let mut starts = Vec::new();
        let unreached = run_sub_blocks(64, &changes, |change| change.0, &mut midi, |block| {
            starts.push(block.start);
            block.start < 16
        });
        // The sub-block at 16 ran and stopped the run.
        assert_eq!(starts, [0, 16]);
        assert_eq!(unreached.changes, [(32, 'b')]);
        assert_eq!(unreached.midi.len(), 1);
        assert_eq!(unreached.midi[0].sample_offset, 40);
    }

    #[test]
    fn test_an_unsorted_list_still_ends() {
        // A change out of order is applied with the change before it.
        let changes = [(32, 'a'), (16, 'b')];
        let (blocks, unreached, _) = run(64, &changes, &mut []);
        assert_eq!(blocks, [(0, 32, String::new(), vec![]), (32, 32, "ab".into(), vec![])]);
        assert!(unreached.is_empty());
    }

    #[test]
    fn test_output_moves_to_block_offsets() {
        let mut events = [note(0, 60), note(3, 61)];
        offset_from_sub_block(&mut events, 32);
        assert_eq!(events.map(|event| event.sample_offset), [32, 35]);
    }

    #[test]
    fn test_sort_keeps_the_order_at_one_offset() {
        // Pairs of (offset, arrival), out of order between offsets.
        let mut items = [(8, 0), (0, 1), (8, 2), (4, 3), (0, 4), (8, 5), (4, 6)];
        sort_by_offset(&mut items, |item| item.0);
        assert_eq!(
            items,
            [(0, 1), (0, 4), (4, 3), (4, 6), (8, 0), (8, 2), (8, 5)]
        );
    }
}
