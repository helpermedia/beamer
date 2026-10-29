//! Assembling MIDI 1.0 control change sequences into events.
//!
//! MIDI 1.0 sets an RPN or NRPN with several control changes and selects a
//! bank with CC 0 and 32 before a program change. MIDI 2.0 has one message
//! for each; a host that translates MIDI 1.0 to 2.0 for a plugin turns
//! the sequences into those messages. The wrappers run the
//! [`Midi1Translator`] over a block's MIDI input, so a plugin gets the same
//! events in either protocol: a [`ParameterNumber`] instead of the
//! selection and data entry CCs, also a [`ProgramChange`] that carries the
//! bank.

use crate::midi::{
    cc, combine_14bit_raw, rpn, scale, ControlChange, MidiBuffer, MidiChannel, MidiEventKind, MidiEventsMut,
    ParameterChange, ParameterNumber, ParameterNumberKind, ProgramChange, RELATIVE_14BIT_STEP,
};

/// What one channel has selected so far.
#[derive(Debug, Clone, Copy, Default)]
struct ChannelState {
    /// The kind of the selected parameter, if one is selected.
    kind: Option<ParameterNumberKind>,
    /// The parameter's MSB (CC 101 or 99), 0 until sent.
    parameter_msb: u8,
    /// The parameter's LSB (CC 100 or 98), 0 until sent.
    parameter_lsb: u8,
    /// The last data entry MSB (CC 6) for the selected parameter.
    data_msb: u8,
    /// The bank selected by CC 0 and 32, if any.
    bank_msb: Option<u8>,
    bank_lsb: Option<u8>,
}

impl ChannelState {
    /// The selected parameter, if one is selected. The null RPN selects
    /// none.
    fn parameter(&self) -> Option<(ParameterNumberKind, u16)> {
        let kind = self.kind?;
        let parameter = combine_14bit_raw(self.parameter_msb, self.parameter_lsb);
        if kind == ParameterNumberKind::Rpn && parameter == rpn::NULL {
            return None;
        }
        Some((kind, parameter))
    }

    /// Select the null RPN, which selects none.
    fn select_null(&mut self) {
        self.kind = Some(ParameterNumberKind::Rpn);
        self.parameter_msb = 0x7F;
        self.parameter_lsb = 0x7F;
        self.data_msb = 0;
    }

    /// Select a parameter half. A half of the other kind starts a new
    /// selection.
    fn select(&mut self, kind: ParameterNumberKind, msb: Option<u8>, lsb: Option<u8>) {
        if self.kind != Some(kind) {
            self.parameter_msb = 0;
            self.parameter_lsb = 0;
        }
        self.kind = Some(kind);
        if let Some(msb) = msb {
            self.parameter_msb = msb;
        }
        if let Some(lsb) = lsb {
            self.parameter_lsb = lsb;
        }
        self.data_msb = 0;
    }

    /// The bank of a program change that carries none, from bank select.
    fn bank(&self) -> Option<u16> {
        if self.bank_msb.is_none() && self.bank_lsb.is_none() {
            return None;
        }
        Some(combine_14bit_raw(self.bank_msb.unwrap_or(0), self.bank_lsb.unwrap_or(0)))
    }
}

/// Turns MIDI 1.0 control change sequences into the events MIDI 2.0 has for
/// them, in a block's MIDI input.
///
/// | Input | Output |
/// |---|---|
/// | CC 101 and 100 (RPN), 99 and 98 (NRPN) | removed, they select the parameter |
/// | CC 6 with a selection | [`ParameterNumber`], absolute: the MSB with LSB 0 |
/// | CC 38 with a selection | [`ParameterNumber`], absolute: the last MSB with this LSB |
/// | CC 96, 97 with a selection | [`ParameterNumber`], relative: one 14-bit step up or down |
/// | CC 6, 38, 96, 97 without a selection | kept |
/// | the null RPN (127, 127) | removed, ends the selection |
/// | CC 0 and 32 | kept, remembered as the bank |
/// | a program change without a bank | the remembered bank, if any |
/// | an RPN change of the null parameter | removed, ends the selection |
///
/// A control change is read as 7 bits ([`scale::to_7bit`]), so a MIDI 2.0
/// control change of these numbers (a host that translates MIDI 1.0 to 2.0
/// may pass the sequences on as such) is assembled the same way. Bank
/// select stays in effect until the next one. The state is kept per
/// channel across blocks; [`reset`](Self::reset) clears it.
#[derive(Debug, Clone)]
pub struct Midi1Translator {
    channels: [ChannelState; 16],
}

impl Default for Midi1Translator {
    fn default() -> Self {
        Self::new()
    }
}

impl Midi1Translator {
    /// A translator with nothing selected.
    pub const fn new() -> Self {
        Self {
            channels: [ChannelState {
                kind: None,
                parameter_msb: 0,
                parameter_lsb: 0,
                data_msb: 0,
                bank_msb: None,
                bank_lsb: None,
            }; 16],
        }
    }

    /// Forget every selection and bank.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Translate the events of a buffer in place, in their order.
    pub fn translate(&mut self, buffer: &mut MidiBuffer) {
        buffer.map_kinds(|kind| self.translate_kind(kind));
    }

    /// What takes the place of one event, or `None` to remove it.
    fn translate_kind(&mut self, kind: MidiEventKind<'static>) -> Option<MidiEventKind<'static>> {
        match kind {
            MidiEventKind::ControlChange(change) => self.control_change(change),
            MidiEventKind::ProgramChange(program) => {
                let state = &self.channels[usize::from(program.channel & 0x0F)];
                Some(MidiEventKind::ProgramChange(ProgramChange {
                    bank: program.bank.or_else(|| state.bank()),
                    ..program
                }))
            }
            // A MIDI 2.0 change of the null RPN ends the selection, as the
            // CCs do.
            MidiEventKind::ParameterNumber(change)
                if change.is_rpn() && change.parameter == rpn::NULL =>
            {
                self.channels[usize::from(change.channel & 0x0F)].select_null();
                None
            }
            other => Some(other),
        }
    }

    fn control_change(&mut self, change: ControlChange) -> Option<MidiEventKind<'static>> {
        let channel: MidiChannel = change.channel & 0x0F;
        let state = &mut self.channels[usize::from(channel)];
        let value = scale::to_7bit(change.value);
        let parameter_number = |state: &ChannelState, change: ParameterChange| {
            state.parameter().map(|(kind, parameter)| {
                MidiEventKind::ParameterNumber(ParameterNumber {
                    channel,
                    kind,
                    parameter,
                    change,
                })
            })
        };
        match change.controller {
            cc::RPN_MSB => state.select(ParameterNumberKind::Rpn, Some(value), None),
            cc::RPN_LSB => state.select(ParameterNumberKind::Rpn, None, Some(value)),
            cc::NRPN_MSB => state.select(ParameterNumberKind::Nrpn, Some(value), None),
            cc::NRPN_LSB => state.select(ParameterNumberKind::Nrpn, None, Some(value)),
            cc::DATA_ENTRY_MSB if state.parameter().is_some() => {
                state.data_msb = value;
                let data = combine_14bit_raw(value, 0);
                return parameter_number(state, ParameterChange::Absolute(scale::from_14bit(data)));
            }
            cc::DATA_ENTRY_LSB if state.parameter().is_some() => {
                let data = combine_14bit_raw(state.data_msb, value);
                return parameter_number(state, ParameterChange::Absolute(scale::from_14bit(data)));
            }
            cc::DATA_INCREMENT if state.parameter().is_some() => {
                return parameter_number(state, ParameterChange::Relative(RELATIVE_14BIT_STEP));
            }
            cc::DATA_DECREMENT if state.parameter().is_some() => {
                return parameter_number(state, ParameterChange::Relative(-RELATIVE_14BIT_STEP));
            }
            cc::BANK_SELECT_MSB => {
                state.bank_msb = Some(value);
                return Some(MidiEventKind::ControlChange(change));
            }
            cc::BANK_SELECT_LSB => {
                state.bank_lsb = Some(value);
                return Some(MidiEventKind::ControlChange(change));
            }
            _ => return Some(MidiEventKind::ControlChange(change)),
        }
        // A selection CC.
        None
    }
}

/// The place of a control change in an RPN or NRPN sequence, for
/// [`order_parameter_number_controllers`].
fn sequence_role(kind: &MidiEventKind) -> Option<u8> {
    match kind {
        MidiEventKind::ControlChange(change) => match change.controller {
            cc::RPN_MSB | cc::NRPN_MSB => Some(0),
            cc::RPN_LSB | cc::NRPN_LSB => Some(1),
            cc::DATA_ENTRY_MSB => Some(2),
            cc::DATA_ENTRY_LSB => Some(3),
            cc::DATA_INCREMENT | cc::DATA_DECREMENT => Some(4),
            _ => None,
        },
        _ => None,
    }
}

/// Put the control changes of an RPN or NRPN sequence that share a sample
/// offset into the order MIDI sends them: selection MSB, selection LSB,
/// data entry MSB, data entry LSB, then increment and decrement. The other
/// events keep their places.
///
/// For events whose order at one offset is not MIDI's: VST3 hands control
/// changes as parameter changes, one queue per controller, so the CCs of a
/// sequence at one offset come in the order of the queues. The events must
/// be sorted by offset.
pub fn order_parameter_number_controllers(mut events: MidiEventsMut<'_>) {
    let len = events.len();
    let mut start = 0;
    while start < len {
        let offset = events.as_slice()[start].sample_offset;
        let mut end = start + 1;
        while end < len && events.as_slice()[end].sample_offset == offset {
            end += 1;
        }
        // Sort the sequence CCs among their own places: an insertion sort
        // that only moves an event past others of the sequence.
        for i in start..end {
            let Some(role) = sequence_role(&events.as_slice()[i].event) else {
                continue;
            };
            let mut j = i;
            loop {
                let previous = (start..j)
                    .rev()
                    .find(|&k| sequence_role(&events.as_slice()[k].event).is_some());
                match previous {
                    Some(k) if sequence_role(&events.as_slice()[k].event) > Some(role) => {
                        events.swap(k, j);
                        j = k;
                    }
                    _ => break,
                }
            }
        }
        start = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::{MidiEvent, RELATIVE_14BIT_STEP};

    /// Translate MIDI 1.0 messages, each `[status, data1, data2]` at its
    /// index as offset; return the events.
    fn translate(translator: &mut Midi1Translator, messages: &[[u8; 3]]) -> Vec<MidiEventKind<'static>> {
        let mut buffer = MidiBuffer::new_boxed(0);
        for (offset, message) in messages.iter().enumerate() {
            let event = MidiEvent::from_midi1_bytes(
                offset as u32,
                0,
                message[0] & 0xF0,
                message[0] & 0x0F,
                message[1],
                message[2],
            )
            .unwrap();
            assert!(buffer.push(event));
        }
        translator.translate(&mut buffer);
        buffer
            .as_slice()
            .iter()
            .map(|event| event.to_static().unwrap().event)
            .collect()
    }

    fn absolute(channel: u8, kind: ParameterNumberKind, parameter: u16, data: u16) -> MidiEventKind<'static> {
        MidiEventKind::ParameterNumber(ParameterNumber {
            channel,
            kind,
            parameter,
            change: ParameterChange::Absolute(scale::from_14bit(data)),
        })
    }

    #[test]
    fn test_rpn_sends_a_change_at_each_data_entry() {
        let mut translator = Midi1Translator::new();
        let events = translate(
            &mut translator,
            &[[0xB0, 101, 0], [0xB0, 100, 0], [0xB0, 6, 12], [0xB0, 38, 50]],
        );
        assert_eq!(
            events,
            [
                absolute(0, ParameterNumberKind::Rpn, 0, 12 << 7),
                absolute(0, ParameterNumberKind::Rpn, 0, (12 << 7) | 50),
            ]
        );
        let MidiEventKind::ParameterNumber(change) = events[1] else { unreachable!() };
        assert_eq!(change.pitch_bend_sensitivity(), Some((12, 50)));
    }

    #[test]
    fn test_a_new_msb_clears_the_lsb_and_a_lone_lsb_keeps_the_msb() {
        let mut translator = Midi1Translator::new();
        let events = translate(
            &mut translator,
            &[[0xB0, 101, 0], [0xB0, 100, 1], [0xB0, 6, 12], [0xB0, 38, 7], [0xB0, 6, 13], [0xB0, 38, 9]],
        );
        let values: Vec<u16> = events
            .iter()
            .map(|kind| match kind {
                MidiEventKind::ParameterNumber(change) => change.value_14bit().unwrap(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(values, [12 << 7, (12 << 7) | 7, 13 << 7, (13 << 7) | 9]);
    }

    #[test]
    fn test_nrpn_and_a_selection_of_one_half() {
        let mut translator = Midi1Translator::new();
        let events = translate(
            &mut translator,
            &[[0xB1, 99, 1], [0xB1, 98, 2], [0xB1, 6, 100], [0xB2, 101, 0], [0xB2, 6, 2]],
        );
        assert_eq!(
            events,
            [
                absolute(1, ParameterNumberKind::Nrpn, (1 << 7) | 2, 100 << 7),
                absolute(2, ParameterNumberKind::Rpn, 0, 2 << 7),
            ]
        );
    }

    #[test]
    fn test_a_selection_of_the_other_kind_starts_anew() {
        let mut translator = Midi1Translator::new();
        let events = translate(&mut translator, &[[0xB0, 99, 5], [0xB0, 100, 3], [0xB0, 6, 1]]);
        // The NRPN MSB does not carry over to the RPN.
        assert_eq!(events, [absolute(0, ParameterNumberKind::Rpn, 3, 1 << 7)]);
    }

    #[test]
    fn test_the_null_rpn_ends_the_selection() {
        let mut translator = Midi1Translator::new();
        let events = translate(
            &mut translator,
            &[[0xB0, 101, 0], [0xB0, 100, 0], [0xB0, 101, 127], [0xB0, 100, 127], [0xB0, 6, 33]],
        );
        assert_eq!(
            events,
            [MidiEventKind::ControlChange(ControlChange {
                channel: 0,
                controller: 6,
                value: scale::from_7bit(33),
            })]
        );
    }

    #[test]
    fn test_the_null_rpn_ends_a_selection_that_shares_one_half() {
        // After `101 = 127` the number is null already; `100 = 127` must not
        // start a new selection of RPN 127.
        let mut translator = Midi1Translator::new();
        let events = translate(
            &mut translator,
            &[[0xB0, 101, 0], [0xB0, 100, 127], [0xB0, 6, 5], [0xB0, 101, 127], [0xB0, 100, 127], [0xB0, 6, 33]],
        );
        assert_eq!(events[0], absolute(0, ParameterNumberKind::Rpn, 127, 5 << 7));
        assert!(matches!(events[1], MidiEventKind::ControlChange(ControlChange { controller: 6, .. })));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_a_null_rpn_change_ends_the_selection() {
        let mut translator = Midi1Translator::new();
        translate(&mut translator, &[[0xB0, 101, 0], [0xB0, 100, 0]]);
        let mut buffer = MidiBuffer::new_boxed(0);
        buffer.push(MidiEvent::parameter_number(0, ParameterNumber::rpn(0, rpn::NULL, 0.0)));
        buffer.push(MidiEvent::control_change(1, 0, 6, scale::from_7bit(3)));
        translator.translate(&mut buffer);
        assert_eq!(buffer.len(), 1);
        assert!(matches!(buffer.as_slice()[0].event, MidiEventKind::ControlChange(_)));
    }

    #[test]
    fn test_increment_and_decrement_step() {
        let mut translator = Midi1Translator::new();
        let events = translate(
            &mut translator,
            &[[0xB0, 96, 0], [0xB0, 101, 0], [0xB0, 100, 1], [0xB0, 96, 0], [0xB0, 97, 0]],
        );
        let relative = |step: f64| {
            MidiEventKind::ParameterNumber(ParameterNumber {
                channel: 0,
                kind: ParameterNumberKind::Rpn,
                parameter: 1,
                change: ParameterChange::Relative(step),
            })
        };
        assert_eq!(
            events,
            [
                MidiEventKind::ControlChange(ControlChange {
                    channel: 0,
                    controller: 96,
                    value: 0.0,
                }),
                relative(RELATIVE_14BIT_STEP),
                relative(-RELATIVE_14BIT_STEP),
            ]
        );
    }

    #[test]
    fn test_bank_select_stays_and_reaches_the_program_change() {
        let mut translator = Midi1Translator::new();
        let events = translate(
            &mut translator,
            &[[0xC0, 4, 0], [0xB0, 0, 1], [0xB0, 32, 2], [0xC0, 5, 0], [0xC0, 6, 0], [0xC1, 7, 0]],
        );
        let program = |channel: u8, program: u8, bank: Option<u16>| {
            MidiEventKind::ProgramChange(ProgramChange { channel, program, bank })
        };
        assert_eq!(events[0], program(0, 4, None));
        assert!(matches!(events[1], MidiEventKind::ControlChange(ControlChange { controller: 0, .. })));
        assert!(matches!(events[2], MidiEventKind::ControlChange(ControlChange { controller: 32, .. })));
        assert_eq!(events[3], program(0, 5, Some((1 << 7) | 2)));
        assert_eq!(events[4], program(0, 6, Some((1 << 7) | 2)));
        // Another channel has its own bank.
        assert_eq!(events[5], program(1, 7, None));
    }

    #[test]
    fn test_a_program_change_with_a_bank_keeps_it() {
        let mut translator = Midi1Translator::new();
        translate(&mut translator, &[[0xB0, 0, 1]]);
        let mut buffer = MidiBuffer::new_boxed(0);
        buffer.push(MidiEvent::program_change(0, 0, 3, Some(9)));
        translator.translate(&mut buffer);
        assert!(matches!(
            buffer.as_slice()[0].event,
            MidiEventKind::ProgramChange(ProgramChange { bank: Some(9), .. })
        ));
    }

    #[test]
    fn test_a_32_bit_control_change_is_read_as_7_bits() {
        // A host that translates MIDI 1.0 to 2.0 may pass a sequence on as
        // MIDI 2.0 control changes, with their values scaled up.
        let mut translator = Midi1Translator::new();
        let mut buffer = MidiBuffer::new_boxed(0);
        for (controller, value) in [(101, 0u8), (100, 0), (6, 12)] {
            let value = scale::unipolar_from_u32(scale::upscale_7_to_32(value));
            buffer.push(MidiEvent::control_change(0, 0, controller, value));
        }
        translator.translate(&mut buffer);
        assert_eq!(buffer.as_slice()[0].event, absolute(0, ParameterNumberKind::Rpn, 0, 12 << 7));
    }

    #[test]
    fn test_state_lasts_until_reset() {
        let mut translator = Midi1Translator::new();
        translate(&mut translator, &[[0xB0, 101, 0], [0xB0, 100, 0], [0xB0, 0, 1]]);
        assert_eq!(
            translate(&mut translator, &[[0xB0, 6, 2]]),
            [absolute(0, ParameterNumberKind::Rpn, 0, 2 << 7)]
        );
        translator.reset();
        let events = translate(&mut translator, &[[0xB0, 6, 2], [0xC0, 1, 0]]);
        assert!(matches!(events[0], MidiEventKind::ControlChange(_)));
        assert!(matches!(events[1], MidiEventKind::ProgramChange(ProgramChange { bank: None, .. })));
    }

    #[test]
    fn test_sequence_ccs_at_one_offset_are_put_in_midi_order() {
        let mut buffer = MidiBuffer::new_boxed(0);
        // The order of VST3 parameter queues: by controller number.
        for (offset, controller, value) in [(0, 6, 12u8), (0, 7, 100), (0, 100, 0), (0, 101, 0), (1, 6, 3)] {
            buffer.push(MidiEvent::control_change(offset, 0, controller, scale::from_7bit(value)));
        }
        order_parameter_number_controllers(buffer.events_mut());
        let controllers: Vec<u8> = buffer
            .iter()
            .map(|event| match event.event {
                MidiEventKind::ControlChange(change) => change.controller,
                _ => unreachable!(),
            })
            .collect();
        // CC 7 keeps its place; the sequence takes the other places in order.
        assert_eq!(controllers, [101, 7, 100, 6, 6]);
        let mut translator = Midi1Translator::new();
        translator.translate(&mut buffer);
        // The selection CCs are gone; CC 7 stays first.
        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer.as_slice()[1].event, absolute(0, ParameterNumberKind::Rpn, 0, 12 << 7));
        assert_eq!(buffer.as_slice()[2].event, absolute(0, ParameterNumberKind::Rpn, 0, 3 << 7));
    }
}
