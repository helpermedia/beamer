//! Problems of the audio thread, reported later on a host thread.
//!
//! The audio thread must not log: formatting a message allocates, and a
//! logger may take a lock or write to the system. A format wrapper notes
//! what went wrong in a [`ProcessWarnings`] with one atomic operation, and
//! a host thread logs it later, for example when the plugin is prepared
//! again or released.

use std::sync::atomic::{AtomicU32, Ordering};

/// Something the audio thread had to drop or delay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ProcessWarning {
    /// More MIDI input in one block than the buffer holds.
    MidiInputOverflow = 1,
    /// More MIDI output in one block than the buffer holds.
    MidiOutputOverflow = 2,
    /// More parameter events in one block than the buffer holds.
    ParameterEventOverflow = 4,
    /// An event list that did not end, possibly a damaged one.
    EventListTooLong = 8,
    /// More SysEx output in one block than the pool holds.
    SysExOutputOverflow = 16,
    /// SysEx output that did not fit was sent with the next block.
    SysExOutputDelayed = 32,
}

impl ProcessWarning {
    const ALL: [ProcessWarning; 6] = [
        ProcessWarning::MidiInputOverflow,
        ProcessWarning::MidiOutputOverflow,
        ProcessWarning::ParameterEventOverflow,
        ProcessWarning::EventListTooLong,
        ProcessWarning::SysExOutputOverflow,
        ProcessWarning::SysExOutputDelayed,
    ];

    /// What the warning tells a developer.
    pub fn message(self) -> &'static str {
        match self {
            ProcessWarning::MidiInputOverflow => {
                "MIDI input events were dropped: more than one block holds"
            }
            ProcessWarning::MidiOutputOverflow => {
                "MIDI output events were dropped: more than one block holds"
            }
            ProcessWarning::ParameterEventOverflow => {
                "parameter events were dropped: more than one block holds"
            }
            ProcessWarning::EventListTooLong => {
                "an event list exceeded the maximum length and was cut, possibly a damaged one"
            }
            ProcessWarning::SysExOutputOverflow => {
                "SysEx output messages were dropped: more than the SysEx pool holds"
            }
            ProcessWarning::SysExOutputDelayed => {
                "SysEx output messages that did not fit into a block were sent with the next one"
            }
        }
    }
}

/// The warnings the audio thread noted since a host thread last looked.
#[derive(Debug, Default)]
pub struct ProcessWarnings {
    /// Bits of `ProcessWarning`.
    bits: AtomicU32,
}

impl ProcessWarnings {
    /// Note a warning. For the audio thread: one atomic operation, no
    /// allocation.
    #[inline]
    pub fn note(&self, warning: ProcessWarning) {
        self.bits.fetch_or(warning as u32, Ordering::Relaxed);
    }

    /// Take the warnings noted since the last call.
    pub fn take(&self) -> impl Iterator<Item = ProcessWarning> {
        let bits = self.bits.swap(0, Ordering::Relaxed);
        ProcessWarning::ALL
            .into_iter()
            .filter(move |warning| bits & (*warning as u32) != 0)
    }

    /// Log the warnings noted since the last call. For host threads.
    pub fn log(&self) {
        for warning in self.take() {
            log::warn!("{}", warning.message());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_warnings_are_taken_once() {
        let warnings = ProcessWarnings::default();
        assert_eq!(warnings.take().count(), 0);

        warnings.note(ProcessWarning::MidiOutputOverflow);
        warnings.note(ProcessWarning::SysExOutputDelayed);
        warnings.note(ProcessWarning::MidiOutputOverflow);
        assert_eq!(
            warnings.take().collect::<Vec<_>>(),
            [ProcessWarning::MidiOutputOverflow, ProcessWarning::SysExOutputDelayed]
        );
        assert_eq!(warnings.take().count(), 0);
    }
}
