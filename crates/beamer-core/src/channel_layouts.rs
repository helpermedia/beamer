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
    /// 5.0 with two more surround channels.
    Surround70,
    /// 5.1 with two more surround channels.
    Surround71,
    /// 7.1 with four height speakers.
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
