//! Channel layouts: the speaker layout of a bus and the combinations of bus
//! layouts a plugin supports.
//!
//! A plugin lists the layouts it supports in [`Model::LAYOUTS`], the default
//! first. The layouts give the number of buses and the channels of each bus;
//! the bus info ([`Model::input_bus_info`]) gives their names, types and
//! whether they are on by default.
//!
//! ```ignore
//! impl Model for CompressorModel {
//!     // Stereo in and out with a stereo sidechain.
//!     const LAYOUTS: &'static [Layout] = &[Layout {
//!         inputs: &[BusLayout::STEREO, BusLayout::STEREO],
//!         outputs: &[BusLayout::STEREO],
//!     }];
//!     // ...
//! }
//! ```
//!
//! [`Model::LAYOUTS`]: crate::Model::LAYOUTS
//! [`Model::input_bus_info`]: crate::Model::input_bus_info

use crate::active_buses::ActiveBuses;
use crate::types::{MAX_BUSES, MAX_CHANNELS};

/// The speaker layout of one bus.
///
/// The variants name the channels in the order a plugin sees them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelSet {
    /// One channel.
    Mono,
    /// L R.
    Stereo,
    /// L R C.
    Lcr,
    /// L R Ls Rs.
    Quad,
    /// L R C Ls Rs.
    Surround50,
    /// L R C LFE Ls Rs.
    Surround51,
    /// L R C Lrs Rrs Lss Rss: the rear surround pair before the side pair.
    Surround70,
    /// L R C LFE Lrs Rrs Lss Rss.
    Surround71,
    /// 7.1 with four height speakers: L R C LFE Lrs Rrs Lss Rss Ltf Rtf Ltr
    /// Rtr.
    Surround714,
    /// Ambisonics of an order from 1 (ACN order, SN3D), (order + 1)^2
    /// channels. Orders 1 to 4 fit in [`MAX_CHANNELS`].
    Ambisonic(u8),
    /// Channels without speaker positions.
    Discrete(u32),
}

impl ChannelSet {
    /// Number of channels in this set.
    pub const fn channel_count(&self) -> u32 {
        match *self {
            ChannelSet::Mono => 1,
            ChannelSet::Stereo => 2,
            ChannelSet::Lcr => 3,
            ChannelSet::Quad => 4,
            ChannelSet::Surround50 => 5,
            ChannelSet::Surround51 => 6,
            ChannelSet::Surround70 => 7,
            ChannelSet::Surround71 => 8,
            ChannelSet::Surround714 => 12,
            ChannelSet::Ambisonic(order) => (order as u32 + 1) * (order as u32 + 1),
            ChannelSet::Discrete(count) => count,
        }
    }

    /// The set a host most likely means with `count` channels: the
    /// speaker layout of that size (mono, stereo, LCR, quad, 5.0, 5.1,
    /// 7.0, 7.1 or 7.1.4), discrete channels for other counts. `None` for
    /// 0 channels.
    pub const fn for_count(count: u32) -> Option<ChannelSet> {
        Some(match count {
            0 => return None,
            1 => ChannelSet::Mono,
            2 => ChannelSet::Stereo,
            3 => ChannelSet::Lcr,
            4 => ChannelSet::Quad,
            5 => ChannelSet::Surround50,
            6 => ChannelSet::Surround51,
            7 => ChannelSet::Surround70,
            8 => ChannelSet::Surround71,
            12 => ChannelSet::Surround714,
            _ => ChannelSet::Discrete(count),
        })
    }
}

/// What a listed layout allows for one bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusLayout {
    /// Exactly this set.
    Set(ChannelSet),
    /// Any set, of any count from 1 to [`MAX_CHANNELS`].
    Any,
    /// As many channels as the main input, any set of that count. For an
    /// output bus of a layout with a main input.
    SameAsMainInput,
}

impl BusLayout {
    /// A mono bus.
    pub const MONO: BusLayout = BusLayout::Set(ChannelSet::Mono);
    /// A stereo bus.
    pub const STEREO: BusLayout = BusLayout::Set(ChannelSet::Stereo);

    /// Whether a bus of this layout can have the set `set`. `main_input` is
    /// the set of the main input, which [`SameAsMainInput`](Self::SameAsMainInput)
    /// follows.
    pub fn allows(&self, set: ChannelSet, main_input: Option<ChannelSet>) -> bool {
        let count = set.channel_count();
        match *self {
            BusLayout::Set(own) => own == set,
            BusLayout::Any => count > 0 && count as usize <= MAX_CHANNELS,
            BusLayout::SameAsMainInput => {
                main_input.is_some_and(|main| main.channel_count() == count)
            }
        }
    }

    /// The set a bus of this layout starts with, before a host asks for
    /// another: the set itself, stereo for [`Any`](Self::Any) and the set
    /// of the main input `main_input` for
    /// [`SameAsMainInput`](Self::SameAsMainInput) (stereo without one).
    pub const fn first_set(&self, main_input: Option<ChannelSet>) -> ChannelSet {
        match *self {
            BusLayout::Set(set) => set,
            BusLayout::Any => ChannelSet::Stereo,
            BusLayout::SameAsMainInput => match main_input {
                Some(set) => set,
                None => ChannelSet::Stereo,
            },
        }
    }
}

/// A combination of bus layouts a plugin supports: one entry per bus, in
/// order, the main bus first.
///
/// Every layout of a plugin has the same number of input buses and of
/// output buses: that is the plugin's bus count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The input buses.
    pub inputs: &'static [BusLayout],
    /// The output buses.
    pub outputs: &'static [BusLayout],
}

impl Layout {
    /// Stereo in, stereo out.
    pub const STEREO: Layout = Layout {
        inputs: &[BusLayout::STEREO],
        outputs: &[BusLayout::STEREO],
    };
    /// Mono in, mono out.
    pub const MONO: Layout = Layout {
        inputs: &[BusLayout::MONO],
        outputs: &[BusLayout::MONO],
    };
    /// Mono in, stereo out.
    pub const MONO_TO_STEREO: Layout = Layout {
        inputs: &[BusLayout::MONO],
        outputs: &[BusLayout::STEREO],
    };
    /// Any count in, as many out (a gain, a utility).
    pub const MATCHED: Layout = Layout {
        inputs: &[BusLayout::Any],
        outputs: &[BusLayout::SameAsMainInput],
    };
    /// No input, stereo out (an instrument, a generator).
    pub const INSTRUMENT_STEREO: Layout = Layout {
        inputs: &[],
        outputs: &[BusLayout::STEREO],
    };
    /// No input, mono out.
    pub const INSTRUMENT_MONO: Layout = Layout {
        inputs: &[],
        outputs: &[BusLayout::MONO],
    };

    // A layout without buses, the default of an empty list.
    const NONE: Layout = Layout { inputs: &[], outputs: &[] };

    /// The default of `layouts`: the first entry, a layout without buses
    /// for an empty list (which [`validate_layouts`] refuses).
    pub const fn default_of(layouts: &'static [Layout]) -> &'static Layout {
        match layouts.first() {
            Some(layout) => layout,
            None => &Layout::NONE,
        }
    }

    /// The set input bus `bus` starts with (see [`BusLayout::first_set`]),
    /// `None` past the input buses.
    pub const fn first_input_set(&self, bus: usize) -> Option<ChannelSet> {
        if bus >= self.inputs.len() {
            return None;
        }
        Some(self.inputs[bus].first_set(None))
    }

    /// The set output bus `bus` starts with (see [`BusLayout::first_set`]),
    /// `None` past the output buses.
    pub const fn first_output_set(&self, bus: usize) -> Option<ChannelSet> {
        if bus >= self.outputs.len() {
            return None;
        }
        Some(self.outputs[bus].first_set(self.first_input_set(0)))
    }
}

impl Layout {
    /// Whether this layout allows `buses`: the same number of buses in each
    /// direction and a set this layout allows on every bus that is on. A
    /// bus that is off matches any entry. An output that follows the main
    /// input follows the set this layout starts with when the main input is
    /// off.
    pub fn allows(&self, buses: &ActiveBuses) -> bool {
        if buses.input_bus_count() != self.inputs.len() || buses.output_bus_count() != self.outputs.len() {
            return false;
        }
        let main_input = self.main_input_of(buses);
        let inputs = self.inputs.iter().enumerate().all(|(bus, layout)| {
            buses.input_set(bus).is_none_or(|set| layout.allows(set, None))
        });
        let outputs = self.outputs.iter().enumerate().all(|(bus, layout)| {
            buses.output_set(bus).is_none_or(|set| layout.allows(set, main_input))
        });
        inputs && outputs
    }

    /// `buses` as this layout has them: each bus on or off as in `buses`,
    /// with its set where this layout allows it and the set this layout
    /// starts with otherwise. A bus that follows the main input gets its
    /// set when the counts differ.
    ///
    /// Buses past the bus counts of this layout are left as they are.
    pub fn apply(&self, buses: &ActiveBuses) -> ActiveBuses {
        let mut applied = *buses;
        for (bus, layout) in self.inputs.iter().enumerate() {
            if let Some(set) = buses.input_set(bus) {
                let kept = layout.allows(set, None).then_some(set);
                let _ = applied.set_input(bus, kept.or(Some(layout.first_set(None))));
            }
        }
        // Outputs follow the main input as it is now.
        let main_input = self.main_input_of(&applied);
        for (bus, layout) in self.outputs.iter().enumerate() {
            if let Some(set) = buses.output_set(bus) {
                let kept = layout.allows(set, main_input).then_some(set);
                let _ = applied.set_output(bus, kept.or(Some(layout.first_set(main_input))));
            }
        }
        applied
    }

    /// The set of the main input that outputs following it follow: the one
    /// in `buses`, or the one this layout starts with when it is off.
    fn main_input_of(&self, buses: &ActiveBuses) -> Option<ChannelSet> {
        buses.input_set(0).or(self.first_input_set(0))
    }

    /// How close this layout is to `buses`, for [`nearest`]: whether it
    /// allows the main output, whether its main output has the same count,
    /// the same for the main input, then the number of aux buses it allows.
    fn closeness(&self, buses: &ActiveBuses) -> Closeness {
        // An output that follows the main input follows the one this layout
        // would give.
        let main_input = self.main_input_of(&self.apply(buses));
        let input = |bus: usize| {
            buses.input_set(bus).is_none_or(|set| self.inputs[bus].allows(set, None))
        };
        let output = |bus: usize| {
            buses.output_set(bus).is_none_or(|set| self.outputs[bus].allows(set, main_input))
        };
        // A main bus that is off, or that this layout does not have, counts
        // as allowed.
        let same_count = |requested: Option<ChannelSet>, first: Option<ChannelSet>| match (requested, first) {
            (Some(requested), Some(first)) => requested.channel_count() == first.channel_count(),
            _ => true,
        };
        let aux_inputs = (1..self.inputs.len()).filter(|&bus| input(bus)).count();
        let aux_outputs = (1..self.outputs.len()).filter(|&bus| output(bus)).count();
        let main_output = self.outputs.is_empty() || output(0);
        let main_input_allowed = self.inputs.is_empty() || input(0);
        Closeness {
            main_output,
            main_output_count: main_output || same_count(buses.output_set(0), self.first_output_set(0)),
            main_input: main_input_allowed,
            main_input_count: main_input_allowed || same_count(buses.input_set(0), self.first_input_set(0)),
            aux_buses: aux_inputs + aux_outputs,
        }
    }
}

/// How close a layout is to a request, compared field by field in order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Closeness {
    // Whether the layout allows the main output.
    main_output: bool,
    // Whether its main output has the requested channel count.
    main_output_count: bool,
    // Whether it allows the main input.
    main_input: bool,
    // Whether its main input has the requested channel count.
    main_input_count: bool,
    // How many aux buses it allows.
    aux_buses: usize,
}

/// The first of `layouts` that allows `buses` (see [`Layout::allows`]).
pub fn matching(layouts: &'static [Layout], buses: &ActiveBuses) -> Option<&'static Layout> {
    layouts.iter().find(|layout| layout.allows(buses))
}

/// The buses of the layout of `layouts` nearest to `buses`, for a host
/// that asks for buses no layout allows: `buses` as that layout has them
/// (see [`Layout::apply`]).
///
/// The nearest layout allows the main output of `buses` (by set, else by
/// channel count), then its main input (the same way), then the most aux
/// buses; of layouts equally near, the first. When no layout allows a main
/// bus or the aux buses, that is the default layout. A layout that allows
/// `buses` gives them unchanged, as does a request with other bus counts
/// than the layouts.
pub fn nearest(layouts: &'static [Layout], buses: &ActiveBuses) -> ActiveBuses {
    nearest_with(layouts, buses, |_| true)
}

/// [`nearest`] among the layouts whose buses `accepts` takes: the nearest
/// layout for which it returns true for `buses` as that layout has them.
/// When it takes none, [`nearest`] of all layouts.
///
/// A format wrapper passes the plugin's
/// [`supports_buses`](crate::Model::supports_buses), so that it does not
/// suggest buses the plugin refuses.
pub fn nearest_with(
    layouts: &'static [Layout],
    buses: &ActiveBuses,
    mut accepts: impl FnMut(&ActiveBuses) -> bool,
) -> ActiveBuses {
    let mut ranked: Vec<(&Layout, Closeness)> = layouts
        .iter()
        .filter(|layout| {
            layout.inputs.len() == buses.input_bus_count() && layout.outputs.len() == buses.output_bus_count()
        })
        .map(|layout| (layout, layout.closeness(buses)))
        .collect();
    if ranked.is_empty() {
        return *buses;
    }
    // Nearest first; the sort is stable, so equally near layouts keep
    // their order.
    ranked.sort_by(|(_, a), (_, b)| b.cmp(a));
    ranked
        .iter()
        .map(|(layout, _)| layout.apply(buses))
        .find(|applied| accepts(applied))
        .unwrap_or_else(|| ranked[0].0.apply(buses))
}

/// Checks the layouts a plugin lists: at least one, all with the same
/// number of input buses and of output buses, within [`MAX_BUSES`], every
/// set within 1 to [`MAX_CHANNELS`] channels, an ambisonic order of at
/// least 1 and [`BusLayout::SameAsMainInput`] only on an output of a
/// layout with a main input.
///
/// `#[beamer::export]` runs it when the plugin is compiled, the format
/// wrappers again before they prepare the plugin.
pub const fn validate_layouts(layouts: &[Layout]) -> Result<(), &'static str> {
    let Some(first) = layouts.first() else {
        return Err("LAYOUTS lists no layout");
    };
    if first.inputs.len() > MAX_BUSES || first.outputs.len() > MAX_BUSES {
        return Err("a layout in LAYOUTS has more buses than MAX_BUSES");
    }
    let mut index = 0;
    while index < layouts.len() {
        let layout = &layouts[index];
        if layout.inputs.len() != first.inputs.len() || layout.outputs.len() != first.outputs.len() {
            return Err("the layouts in LAYOUTS differ in their number of buses");
        }
        if let Err(message) = validate_buses(layout.inputs, false) {
            return Err(message);
        }
        if let Err(message) = validate_buses(layout.outputs, !layout.inputs.is_empty()) {
            return Err(message);
        }
        index += 1;
    }
    Ok(())
}

/// Checks the buses of one direction of a layout. `same_as_main_input` is
/// whether they may follow the main input.
const fn validate_buses(buses: &[BusLayout], same_as_main_input: bool) -> Result<(), &'static str> {
    let mut bus = 0;
    while bus < buses.len() {
        match buses[bus] {
            BusLayout::Set(ChannelSet::Ambisonic(0)) => {
                return Err("an ambisonic set in LAYOUTS has order 0, the lowest is 1");
            }
            BusLayout::Set(set) => {
                let count = set.channel_count();
                if count == 0 || count as usize > MAX_CHANNELS {
                    return Err("a set in LAYOUTS has no channels or more than MAX_CHANNELS");
                }
            }
            BusLayout::Any => {}
            BusLayout::SameAsMainInput => {
                if !same_as_main_input {
                    return Err("SameAsMainInput in LAYOUTS needs an output bus and a main input");
                }
            }
        }
        bus += 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_counts() {
        assert_eq!(ChannelSet::Mono.channel_count(), 1);
        assert_eq!(ChannelSet::Surround51.channel_count(), 6);
        assert_eq!(ChannelSet::Surround714.channel_count(), 12);
        assert_eq!(ChannelSet::Ambisonic(1).channel_count(), 4);
        assert_eq!(ChannelSet::Ambisonic(3).channel_count(), 16);
        assert_eq!(ChannelSet::Discrete(5).channel_count(), 5);
    }

    #[test]
    fn test_for_count_names_the_speaker_layouts() {
        assert_eq!(ChannelSet::for_count(0), None);
        for set in [
            ChannelSet::Mono,
            ChannelSet::Stereo,
            ChannelSet::Lcr,
            ChannelSet::Quad,
            ChannelSet::Surround50,
            ChannelSet::Surround51,
            ChannelSet::Surround70,
            ChannelSet::Surround71,
            ChannelSet::Surround714,
        ] {
            assert_eq!(ChannelSet::for_count(set.channel_count()), Some(set));
        }
        assert_eq!(ChannelSet::for_count(9), Some(ChannelSet::Discrete(9)));
    }

    #[test]
    fn test_first_sets() {
        assert_eq!(Layout::MONO_TO_STEREO.first_input_set(0), Some(ChannelSet::Mono));
        assert_eq!(Layout::MONO_TO_STEREO.first_output_set(0), Some(ChannelSet::Stereo));
        assert_eq!(Layout::MONO_TO_STEREO.first_output_set(1), None);
        assert_eq!(Layout::INSTRUMENT_MONO.first_input_set(0), None);

        // Wildcards start as stereo, or as the main input.
        assert_eq!(Layout::MATCHED.first_input_set(0), Some(ChannelSet::Stereo));
        assert_eq!(Layout::MATCHED.first_output_set(0), Some(ChannelSet::Stereo));
        let surround = Layout {
            inputs: &[BusLayout::Set(ChannelSet::Surround51)],
            outputs: &[BusLayout::SameAsMainInput],
        };
        assert_eq!(surround.first_output_set(0), Some(ChannelSet::Surround51));
    }

    #[test]
    fn test_default_of() {
        assert_eq!(Layout::default_of(&[Layout::MONO, Layout::STEREO]), &Layout::MONO);
        assert_eq!(Layout::default_of(&[]).inputs.len(), 0);
        assert_eq!(Layout::default_of(&[]).outputs.len(), 0);
    }

    /// Buses on with `inputs` and `outputs`, `None` for a bus that is off.
    fn buses(inputs: &[Option<ChannelSet>], outputs: &[Option<ChannelSet>]) -> ActiveBuses {
        let mut buses = ActiveBuses::all_off(inputs.len(), outputs.len());
        for (bus, &set) in inputs.iter().enumerate() {
            assert!(buses.set_input(bus, set));
        }
        for (bus, &set) in outputs.iter().enumerate() {
            assert!(buses.set_output(bus, set));
        }
        buses
    }

    use ChannelSet::{Mono, Quad, Stereo, Surround51};

    /// Mono or stereo in and out, mono to stereo, with a sidechain of any
    /// width.
    const EFFECT: &[Layout] = &[
        Layout {
            inputs: &[BusLayout::STEREO, BusLayout::Any],
            outputs: &[BusLayout::STEREO],
        },
        Layout {
            inputs: &[BusLayout::MONO, BusLayout::Any],
            outputs: &[BusLayout::MONO],
        },
        Layout {
            inputs: &[BusLayout::MONO, BusLayout::Any],
            outputs: &[BusLayout::STEREO],
        },
    ];

    #[test]
    fn test_bus_layout_allows() {
        assert!(BusLayout::MONO.allows(Mono, None));
        assert!(!BusLayout::MONO.allows(Stereo, None));
        // A set of the same count is another set.
        assert!(!BusLayout::Set(Quad).allows(ChannelSet::Ambisonic(1), None));
        assert!(BusLayout::Any.allows(Surround51, None));
        assert!(BusLayout::Any.allows(ChannelSet::Discrete(MAX_CHANNELS as u32), None));
        assert!(!BusLayout::Any.allows(ChannelSet::Discrete(MAX_CHANNELS as u32 + 1), None));
        assert!(!BusLayout::Any.allows(ChannelSet::Discrete(0), None));
        // Any set of the main input's count.
        assert!(BusLayout::SameAsMainInput.allows(Quad, Some(ChannelSet::Ambisonic(1))));
        assert!(!BusLayout::SameAsMainInput.allows(Stereo, Some(Mono)));
        assert!(!BusLayout::SameAsMainInput.allows(Stereo, None));
    }

    #[test]
    fn test_layout_allows() {
        // Every bus as listed, the sidechain of any width.
        assert!(EFFECT[0].allows(&buses(&[Some(Stereo), Some(Surround51)], &[Some(Stereo)])));
        // A bus that is off matches anything.
        assert!(EFFECT[0].allows(&buses(&[Some(Stereo), None], &[Some(Stereo)])));
        assert!(EFFECT[0].allows(&buses(&[Some(Stereo), None], &[None])));
        assert!(!EFFECT[0].allows(&buses(&[Some(Mono), None], &[Some(Stereo)])));
        // Another bus count.
        assert!(!EFFECT[0].allows(&buses(&[Some(Stereo)], &[Some(Stereo)])));

        assert!(Layout::MATCHED.allows(&buses(&[Some(Surround51)], &[Some(ChannelSet::Discrete(6))])));
        assert!(!Layout::MATCHED.allows(&buses(&[Some(Surround51)], &[Some(Stereo)])));
    }

    #[test]
    fn test_matching_takes_the_first_layout_that_allows() {
        let mono_to_stereo = buses(&[Some(Mono), None], &[Some(Stereo)]);
        assert_eq!(matching(EFFECT, &mono_to_stereo), Some(&EFFECT[2]));
        let mono = buses(&[Some(Mono), Some(Mono)], &[Some(Mono)]);
        assert_eq!(matching(EFFECT, &mono), Some(&EFFECT[1]));
        assert_eq!(matching(EFFECT, &buses(&[Some(Stereo), None], &[Some(Mono)])), None);
    }

    #[test]
    fn test_apply_keeps_what_the_layout_allows() {
        // Stereo in is not mono: it gets the layout's set; the sidechain
        // keeps its width, a bus that is off stays off.
        let applied = EFFECT[2].apply(&buses(&[Some(Stereo), Some(Quad)], &[Some(Stereo)]));
        assert_eq!(applied, buses(&[Some(Mono), Some(Quad)], &[Some(Stereo)]));
        let applied = EFFECT[2].apply(&buses(&[Some(Stereo), None], &[None]));
        assert_eq!(applied, buses(&[Some(Mono), None], &[None]));

        // An output that follows the main input takes its set when the
        // counts differ.
        let applied = Layout::MATCHED.apply(&buses(&[Some(Surround51)], &[Some(Stereo)]));
        assert_eq!(applied, buses(&[Some(Surround51)], &[Some(Surround51)]));
        let applied = Layout::MATCHED.apply(&buses(&[Some(Quad)], &[Some(ChannelSet::Ambisonic(1))]));
        assert_eq!(applied, buses(&[Some(Quad)], &[Some(ChannelSet::Ambisonic(1))]));
    }

    #[test]
    fn test_layout_allows_what_it_applies() {
        let requests = [
            buses(&[None], &[Some(Stereo)]),
            buses(&[None], &[Some(Mono)]),
            buses(&[Some(Surround51)], &[Some(Stereo)]),
            buses(&[Some(ChannelSet::Discrete(MAX_CHANNELS as u32 + 8))], &[None]),
        ];
        let layouts = [Layout::MATCHED, Layout::STEREO, Layout::MONO_TO_STEREO];
        for layout in layouts {
            for request in requests {
                assert!(layout.allows(&layout.apply(&request)), "{layout:?} {request:?}");
            }
        }
        // A main input that is off matches anything, also with an output
        // that follows it: the output follows the set it starts with.
        assert!(Layout::MATCHED.allows(&buses(&[None], &[Some(Stereo)])));
        assert!(!Layout::MATCHED.allows(&buses(&[None], &[Some(Mono)])));
    }

    #[test]
    fn test_nearest() {
        // A layout that allows the buses gives them unchanged.
        let mono = buses(&[Some(Mono), Some(Stereo)], &[Some(Mono)]);
        assert_eq!(nearest(EFFECT, &mono), mono);

        // Stereo in, mono out: the main output decides first (mono to
        // mono, the first layout with a mono output).
        let nearest_layout = nearest(EFFECT, &buses(&[Some(Stereo), None], &[Some(Mono)]));
        assert_eq!(nearest_layout, buses(&[Some(Mono), None], &[Some(Mono)]));

        // 5.1 in, stereo out: stereo out, then the first such layout.
        let nearest_layout = nearest(EFFECT, &buses(&[Some(Surround51), None], &[Some(Stereo)]));
        assert_eq!(nearest_layout, buses(&[Some(Stereo), None], &[Some(Stereo)]));

        // Mono in, stereo out: both main buses decide for the third.
        let request = buses(&[Some(Mono), None], &[Some(Stereo)]);
        assert_eq!(nearest(EFFECT, &request), request);

        // Nothing allowed: the default layout.
        let nearest_layout = nearest(EFFECT, &buses(&[Some(Quad), None], &[Some(Quad)]));
        assert_eq!(nearest_layout, buses(&[Some(Stereo), None], &[Some(Stereo)]));

        // The aux buses decide between layouts with the same main buses.
        const AUX: &[Layout] = &[
            Layout {
                inputs: &[BusLayout::STEREO, BusLayout::MONO],
                outputs: &[BusLayout::STEREO],
            },
            Layout {
                inputs: &[BusLayout::STEREO, BusLayout::STEREO],
                outputs: &[BusLayout::STEREO],
            },
        ];
        let request = buses(&[Some(Mono), Some(Stereo)], &[Some(Stereo)]);
        assert_eq!(nearest(AUX, &request), buses(&[Some(Stereo), Some(Stereo)], &[Some(Stereo)]));

        // Of two layouts that do not allow the main output, the one with its
        // channel count.
        const QUAD_OR_STEREO: &[Layout] = &[
            Layout::STEREO,
            Layout {
                inputs: &[BusLayout::Set(Quad)],
                outputs: &[BusLayout::Set(Quad)],
            },
        ];
        let request = buses(&[Some(Stereo)], &[Some(ChannelSet::Ambisonic(1))]);
        assert_eq!(nearest(QUAD_OR_STEREO, &request), buses(&[Some(Quad)], &[Some(Quad)]));

        // The output follows the input as the nearest layout has it.
        const STEREO_FOLLOWED: &[Layout] = &[Layout {
            inputs: &[BusLayout::STEREO],
            outputs: &[BusLayout::SameAsMainInput],
        }];
        let request = buses(&[Some(Mono)], &[Some(Mono)]);
        assert_eq!(nearest(STEREO_FOLLOWED, &request), buses(&[Some(Stereo)], &[Some(Stereo)]));

        // A layout whose output would follow an input it does not keep is
        // no nearer for that output.
        const FOLLOWED_OR_MONO: &[Layout] = &[
            Layout {
                inputs: &[BusLayout::STEREO],
                outputs: &[BusLayout::SameAsMainInput],
            },
            Layout::MONO_TO_STEREO,
        ];
        assert_eq!(nearest(FOLLOWED_OR_MONO, &request), buses(&[Some(Mono)], &[Some(Stereo)]));

        // Of the layouts the predicate takes.
        let request = buses(&[Some(Stereo), None], &[Some(Mono)]);
        let mono_out = |buses: &ActiveBuses| buses.output_set(0) == Some(Mono);
        let not_mono_out = |buses: &ActiveBuses| buses.output_set(0) != Some(Mono);
        assert_eq!(nearest_with(EFFECT, &request, mono_out), buses(&[Some(Mono), None], &[Some(Mono)]));
        assert_eq!(nearest_with(EFFECT, &request, not_mono_out), buses(&[Some(Stereo), None], &[Some(Stereo)]));
        // It takes none: the nearest of all.
        assert_eq!(nearest_with(EFFECT, &request, |_| false), buses(&[Some(Mono), None], &[Some(Mono)]));

        // Other bus counts than the layouts: unchanged.
        let request = buses(&[Some(Quad)], &[Some(Quad)]);
        assert_eq!(nearest(EFFECT, &request), request);
    }

    #[test]
    fn test_validate_accepts_the_constants() {
        assert_eq!(
            validate_layouts(&[
                Layout::STEREO,
                Layout::MONO,
                Layout::MONO_TO_STEREO,
                Layout::MATCHED,
            ]),
            Ok(())
        );
        assert_eq!(validate_layouts(&[Layout::INSTRUMENT_STEREO, Layout::INSTRUMENT_MONO]), Ok(()));
    }

    #[test]
    fn test_validate_refuses() {
        assert!(validate_layouts(&[]).is_err());
        // Different bus counts.
        assert!(validate_layouts(&[Layout::STEREO, Layout::INSTRUMENT_STEREO]).is_err());
        let sidechain = Layout {
            inputs: &[BusLayout::STEREO, BusLayout::MONO],
            outputs: &[BusLayout::STEREO],
        };
        assert!(validate_layouts(&[sidechain, Layout::STEREO]).is_err());
        assert_eq!(validate_layouts(&[sidechain]), Ok(()));

        let one = |inputs: &'static [BusLayout], outputs: &'static [BusLayout]| {
            validate_layouts(&[Layout { inputs, outputs }])
        };
        assert!(one(&[BusLayout::Set(ChannelSet::Discrete(0))], &[]).is_err());
        assert!(one(&[BusLayout::Set(ChannelSet::Discrete(MAX_CHANNELS as u32 + 1))], &[]).is_err());
        assert!(one(&[BusLayout::Set(ChannelSet::Discrete(MAX_CHANNELS as u32))], &[]).is_ok());
        assert!(one(&[BusLayout::Set(ChannelSet::Ambisonic(0))], &[]).is_err());
        assert!(one(&[BusLayout::Set(ChannelSet::Ambisonic(4))], &[]).is_ok());
        // 36 channels.
        assert!(one(&[BusLayout::Set(ChannelSet::Ambisonic(5))], &[]).is_err());
        // SameAsMainInput on an input, or without a main input.
        assert!(one(&[BusLayout::SameAsMainInput], &[BusLayout::STEREO]).is_err());
        assert!(one(&[], &[BusLayout::SameAsMainInput]).is_err());
        assert!(one(&[BusLayout::MONO], &[BusLayout::STEREO, BusLayout::SameAsMainInput]).is_ok());
        assert!(one(&[BusLayout::STEREO; MAX_BUSES + 1], &[]).is_err());
        assert!(one(&[BusLayout::STEREO; MAX_BUSES], &[]).is_ok());
    }
}
