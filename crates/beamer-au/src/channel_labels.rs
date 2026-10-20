//! Audio Unit channel layouts of the channel sets.
//!
//! An Audio Unit host describes the channels of a bus with an
//! `AudioChannelLayout`: a layout tag, or one channel label per channel.
//! The wrappers expand a host's layout into labels. A set matches labels
//! that name its speakers in any order; the bus then gets a
//! [`ChannelOrder`], so that the plugin sees the channels in the order of
//! its set whatever order the host has them in.

use beamer_core::{ChannelSet, MAX_BUSES, MAX_CHANNELS};

// Channel labels (`AudioChannelLabel` in CoreAudioBaseTypes.h).
const LEFT: u32 = 1;
const RIGHT: u32 = 2;
const CENTER: u32 = 3;
const LFE: u32 = 4;
// The surround pair of quad and 5.x, the side pair of 7.x.
const LEFT_SURROUND: u32 = 5;
const RIGHT_SURROUND: u32 = 6;
const VERTICAL_HEIGHT_LEFT: u32 = 13;
const VERTICAL_HEIGHT_RIGHT: u32 = 15;
const REAR_SURROUND_LEFT: u32 = 33;
const REAR_SURROUND_RIGHT: u32 = 34;
const MONO: u32 = 42;
const LEFT_TOP_REAR: u32 = 52;
const RIGHT_TOP_REAR: u32 = 54;
// The first of the discrete and of the ambisonic (ACN, SN3D) labels.
const DISCRETE_0: u32 = 1 << 16;
const HOA_ACN_0: u32 = 2 << 16;

// Layout tags (`AudioChannelLayoutTag` in CoreAudioBaseTypes.h): the kind
// in the high 16 bits, the channel count in the low 16.
const TAG_MONO: u32 = (100 << 16) | 1;
const TAG_STEREO: u32 = (101 << 16) | 2;
const TAG_MPEG_3_0_A: u32 = (113 << 16) | 3;
const TAG_QUADRAPHONIC: u32 = (108 << 16) | 4;
const TAG_MPEG_5_0_A: u32 = (117 << 16) | 5;
const TAG_MPEG_5_1_A: u32 = (121 << 16) | 6;
const TAG_AUDIO_UNIT_7_0: u32 = (140 << 16) | 7;
const TAG_AUDIO_UNIT_7_1: u32 = (128 << 16) | 8;
const TAG_ATMOS_7_1_4: u32 = (192 << 16) | 12;
const TAG_HOA_ACN_SN3D: u32 = 190 << 16;
const TAG_DISCRETE_IN_ORDER: u32 = 147 << 16;

/// The sets a host may mean without the plugin listing them.
const NAMED: [ChannelSet; 12] = [
    ChannelSet::Stereo,
    ChannelSet::Lcr,
    ChannelSet::Quad,
    ChannelSet::Surround50,
    ChannelSet::Surround51,
    ChannelSet::Surround70,
    ChannelSet::Surround71,
    ChannelSet::Surround714,
    ChannelSet::Ambisonic(1),
    ChannelSet::Ambisonic(2),
    ChannelSet::Ambisonic(3),
    ChannelSet::Ambisonic(4),
];

/// The channel labels of a set, in the order the set names its channels.
/// Discrete channels and sets wider than [`MAX_CHANNELS`] have labels of
/// discrete channels.
pub(crate) fn labels_of(set: ChannelSet) -> Labels {
    let named: &[u32] = match set {
        ChannelSet::Mono => &[MONO],
        ChannelSet::Stereo => &[LEFT, RIGHT],
        ChannelSet::Lcr => &[LEFT, RIGHT, CENTER],
        ChannelSet::Quad => &[LEFT, RIGHT, LEFT_SURROUND, RIGHT_SURROUND],
        ChannelSet::Surround50 => &[LEFT, RIGHT, CENTER, LEFT_SURROUND, RIGHT_SURROUND],
        ChannelSet::Surround51 => &[LEFT, RIGHT, CENTER, LFE, LEFT_SURROUND, RIGHT_SURROUND],
        ChannelSet::Surround70 => &[
            LEFT,
            RIGHT,
            CENTER,
            REAR_SURROUND_LEFT,
            REAR_SURROUND_RIGHT,
            LEFT_SURROUND,
            RIGHT_SURROUND,
        ],
        ChannelSet::Surround71 => &[
            LEFT,
            RIGHT,
            CENTER,
            LFE,
            REAR_SURROUND_LEFT,
            REAR_SURROUND_RIGHT,
            LEFT_SURROUND,
            RIGHT_SURROUND,
        ],
        ChannelSet::Surround714 => &[
            LEFT,
            RIGHT,
            CENTER,
            LFE,
            REAR_SURROUND_LEFT,
            REAR_SURROUND_RIGHT,
            LEFT_SURROUND,
            RIGHT_SURROUND,
            VERTICAL_HEIGHT_LEFT,
            VERTICAL_HEIGHT_RIGHT,
            LEFT_TOP_REAR,
            RIGHT_TOP_REAR,
        ],
        ChannelSet::Ambisonic(_) | ChannelSet::Discrete(_) => &[],
    };
    let mut labels = Labels::default();
    if !named.is_empty() {
        labels.extend(named.iter().copied());
    } else {
        let first = match set {
            ChannelSet::Ambisonic(_) if set.channel_count() as usize <= MAX_CHANNELS => HOA_ACN_0,
            _ => DISCRETE_0,
        };
        labels.extend((0..set.channel_count().min(MAX_CHANNELS as u32)).map(|channel| first + channel));
    }
    labels
}

/// The layout tag a host knows a set by. Its channels may be in another
/// order than the set's (7.x has the side pair first).
pub(crate) fn tag_of(set: ChannelSet) -> u32 {
    match set {
        ChannelSet::Mono => TAG_MONO,
        ChannelSet::Stereo => TAG_STEREO,
        ChannelSet::Lcr => TAG_MPEG_3_0_A,
        ChannelSet::Quad => TAG_QUADRAPHONIC,
        ChannelSet::Surround50 => TAG_MPEG_5_0_A,
        ChannelSet::Surround51 => TAG_MPEG_5_1_A,
        ChannelSet::Surround70 => TAG_AUDIO_UNIT_7_0,
        ChannelSet::Surround71 => TAG_AUDIO_UNIT_7_1,
        ChannelSet::Surround714 => TAG_ATMOS_7_1_4,
        ChannelSet::Ambisonic(_) if set.channel_count() as usize <= MAX_CHANNELS => {
            TAG_HOA_ACN_SN3D | set.channel_count()
        }
        ChannelSet::Ambisonic(_) | ChannelSet::Discrete(_) => {
            TAG_DISCRETE_IN_ORDER | set.channel_count().min(0xffff)
        }
    }
}

/// The sets a host may mean without the plugin listing them, for a bus
/// that takes any set.
pub(crate) fn named_sets() -> impl Iterator<Item = ChannelSet> {
    std::iter::once(ChannelSet::Mono).chain(NAMED)
}

/// Up to [`MAX_CHANNELS`] channel labels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Labels {
    labels: [u32; MAX_CHANNELS],
    len: usize,
}

impl Labels {
    fn extend(&mut self, labels: impl Iterator<Item = u32>) {
        for label in labels {
            if self.len < MAX_CHANNELS {
                self.labels[self.len] = label;
                self.len += 1;
            }
        }
    }

    pub(crate) fn as_slice(&self) -> &[u32] {
        &self.labels[..self.len]
    }
}

/// Which host channel each channel of a bus's set is: the host's buffer
/// index for each channel in the order of the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelOrder {
    host: [u8; MAX_CHANNELS],
}

impl Default for ChannelOrder {
    /// The host's channels in the set's order.
    fn default() -> Self {
        IDENTITY
    }
}

impl ChannelOrder {
    /// The host's buffer index of channel `channel` of the set.
    #[inline]
    pub(crate) fn host_channel(&self, channel: usize) -> usize {
        self.host.get(channel).map_or(channel, |&host| host as usize)
    }

    /// Whether the host has the channels in the set's order.
    #[cfg(test)]
    pub(crate) fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// The order in which `host` has the channels of `set`, `None` when the
    /// labels are not those of the set.
    fn of(set: ChannelSet, host: &[u32]) -> Option<Self> {
        let own = labels_of(set);
        if own.as_slice().len() != host.len() {
            return None;
        }
        let mut order = Self::default();
        let mut taken = [false; MAX_CHANNELS];
        for (channel, label) in own.as_slice().iter().enumerate() {
            let index = host.iter().enumerate().position(|(index, host)| host == label && !taken[index])?;
            taken[index] = true;
            order.host[channel] = index as u8;
        }
        Some(order)
    }
}

/// The channel order of each bus, the set's order where none is set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChannelOrders {
    inputs: [ChannelOrder; MAX_BUSES],
    outputs: [ChannelOrder; MAX_BUSES],
}

impl ChannelOrders {
    /// The order of input bus `bus`.
    #[inline]
    pub(crate) fn input(&self, bus: usize) -> &ChannelOrder {
        self.inputs.get(bus).unwrap_or(&IDENTITY)
    }

    /// The order of output bus `bus`.
    #[inline]
    pub(crate) fn output(&self, bus: usize) -> &ChannelOrder {
        self.outputs.get(bus).unwrap_or(&IDENTITY)
    }

    /// Sets the order of input bus `bus` (ignored past [`MAX_BUSES`]).
    pub(crate) fn set_input(&mut self, bus: usize, order: ChannelOrder) {
        if let Some(own) = self.inputs.get_mut(bus) {
            *own = order;
        }
    }

    /// Sets the order of output bus `bus` (ignored past [`MAX_BUSES`]).
    pub(crate) fn set_output(&mut self, bus: usize, order: ChannelOrder) {
        if let Some(own) = self.outputs.get_mut(bus) {
            *own = order;
        }
    }
}

/// The set's own order.
static IDENTITY: ChannelOrder = ChannelOrder { host: identity() };

/// Each channel at its own index.
const fn identity() -> [u8; MAX_CHANNELS] {
    let mut host = [0; MAX_CHANNELS];
    let mut channel = 0;
    while channel < MAX_CHANNELS {
        host[channel] = channel as u8;
        channel += 1;
    }
    host
}

/// The set a host means with the channel labels `labels`, with the order in
/// which it has its channels.
///
/// Of the sets in `listed` (the ones the plugin lists for the bus), the
/// first whose labels these are in any order, else listed discrete channels
/// of this count (they take any labels, in the host's order); otherwise the
/// named set they are. Other labels are mono for one channel, discrete
/// channels in the host's order for more. `None` for no labels or more than
/// [`MAX_CHANNELS`].
pub(crate) fn set_for_labels(
    labels: &[u32],
    listed: impl IntoIterator<Item = ChannelSet> + Clone,
) -> Option<(ChannelSet, ChannelOrder)> {
    let count = labels.len();
    if count == 0 || count > MAX_CHANNELS {
        return None;
    }
    let discrete = ChannelSet::Discrete(count as u32);
    let other = if count == 1 { ChannelSet::Mono } else { discrete };
    listed
        .clone()
        .into_iter()
        .find_map(|set| ChannelOrder::of(set, labels).map(|order| (set, order)))
        .or_else(|| listed.into_iter().any(|set| set == discrete).then(|| (discrete, ChannelOrder::default())))
        .or_else(|| NAMED.into_iter().find_map(|set| ChannelOrder::of(set, labels).map(|order| (set, order))))
        .or(Some((other, ChannelOrder::default())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_labels_have_the_channel_count_of_their_set() {
        for set in named_sets().chain([ChannelSet::Discrete(5), ChannelSet::Discrete(MAX_CHANNELS as u32)]) {
            assert_eq!(labels_of(set).as_slice().len() as u32, set.channel_count(), "{set:?}");
            // Each set's own labels name it, in its order.
            let (found, order) = set_for_labels(labels_of(set).as_slice(), [set]).unwrap();
            assert_eq!(found, set);
            assert!(order.is_identity(), "{set:?}");
            // So does its tag's channel count.
            assert_eq!(tag_of(set) & 0xffff, set.channel_count(), "{set:?}");
        }
    }

    #[test]
    fn test_apples_7_1_has_the_side_pair_first() {
        // AudioUnit_7_1: L R C LFE Ls Rs Rls Rrs.
        let host = [1, 2, 3, 4, 5, 6, 33, 34];
        let (set, order) = set_for_labels(&host, []).unwrap();
        assert_eq!(set, ChannelSet::Surround71);
        // The set's rear pair (channels 4, 5) is the host's 6, 7; its side
        // pair (6, 7) the host's 4, 5.
        let host_channels: Vec<usize> = (0..8).map(|channel| order.host_channel(channel)).collect();
        assert_eq!(host_channels, [0, 1, 2, 3, 6, 7, 4, 5]);

        // WAVE_7_1 is the set's order.
        let (_, order) = set_for_labels(&[1, 2, 3, 4, 33, 34, 5, 6], []).unwrap();
        assert!(order.is_identity());

        // Atmos_7_1_4.
        let host = [1, 2, 3, 4, 5, 6, 33, 34, 13, 15, 52, 54];
        assert_eq!(set_for_labels(&host, []).unwrap().0, ChannelSet::Surround714);
    }

    #[test]
    fn test_other_labels() {
        // Any one channel is mono, or listed discrete channels.
        assert_eq!(set_for_labels(&[CENTER], []).unwrap().0, ChannelSet::Mono);
        assert_eq!(set_for_labels(&[CENTER], [ChannelSet::Mono]).unwrap().0, ChannelSet::Mono);
        let discrete_one = set_for_labels(&[DISCRETE_0], [ChannelSet::Stereo, ChannelSet::Discrete(1)]);
        assert_eq!(discrete_one.unwrap().0, ChannelSet::Discrete(1));
        assert_eq!(set_for_labels(&[CENTER], [ChannelSet::Discrete(1)]).unwrap().0, ChannelSet::Discrete(1));
        // The labels decide between listed mono and discrete.
        let both = [ChannelSet::Discrete(1), ChannelSet::Mono];
        assert_eq!(set_for_labels(&[MONO], both).unwrap().0, ChannelSet::Mono);
        assert_eq!(set_for_labels(&[DISCRETE_0], both).unwrap().0, ChannelSet::Discrete(1));
        // Ambisonics (HOA_ACN_SN3D | 4).
        let ambisonic = [HOA_ACN_0, HOA_ACN_0 + 1, HOA_ACN_0 + 2, HOA_ACN_0 + 3];
        assert_eq!(set_for_labels(&ambisonic, []).unwrap().0, ChannelSet::Ambisonic(1));
        // Discrete or unknown labels: discrete channels in the host's order.
        let (set, order) = set_for_labels(&[0, 0, 0], []).unwrap();
        assert_eq!(set, ChannelSet::Discrete(3));
        assert!(order.is_identity());
        // Discrete channels the plugin lists.
        let discrete = labels_of(ChannelSet::Discrete(4));
        assert_eq!(set_for_labels(discrete.as_slice(), [ChannelSet::Discrete(4)]).unwrap().0, ChannelSet::Discrete(4));
        // Listed discrete channels take any labels of their count.
        let (set, order) = set_for_labels(&[LEFT, RIGHT], [ChannelSet::Discrete(2)]).unwrap();
        assert_eq!(set, ChannelSet::Discrete(2));
        assert!(order.is_identity());
        // A listed named set comes first.
        let found = set_for_labels(&[LEFT, RIGHT], [ChannelSet::Stereo, ChannelSet::Discrete(2)]);
        assert_eq!(found.unwrap().0, ChannelSet::Stereo);
        // A label twice is no named set.
        assert_eq!(set_for_labels(&[LEFT, LEFT], []).unwrap().0, ChannelSet::Discrete(2));
        assert_eq!(set_for_labels(&[], []), None);
    }

    #[test]
    fn test_each_tag_names_its_set() {
        // The labels the system gives each tag (AudioFormatGetProperty with
        // kAudioFormatProperty_ChannelLayoutForTag, macOS 26).
        let expansions: [(ChannelSet, &[u32]); 11] = [
            (ChannelSet::Mono, &[42]),
            (ChannelSet::Stereo, &[1, 2]),
            (ChannelSet::Lcr, &[1, 2, 3]),
            (ChannelSet::Quad, &[1, 2, 5, 6]),
            (ChannelSet::Surround50, &[1, 2, 3, 5, 6]),
            (ChannelSet::Surround51, &[1, 2, 3, 4, 5, 6]),
            (ChannelSet::Surround70, &[1, 2, 5, 6, 3, 33, 34]),
            (ChannelSet::Surround71, &[1, 2, 3, 4, 5, 6, 33, 34]),
            (ChannelSet::Surround714, &[1, 2, 3, 4, 5, 6, 33, 34, 13, 15, 52, 54]),
            (ChannelSet::Ambisonic(1), &[HOA_ACN_0, HOA_ACN_0 + 1, HOA_ACN_0 + 2, HOA_ACN_0 + 3]),
            (ChannelSet::Discrete(3), &[DISCRETE_0, DISCRETE_0 + 1, DISCRETE_0 + 2]),
        ];
        // A host that keeps the tag a set reports gets that set, in the
        // tag's order.
        for (set, labels) in expansions {
            assert_eq!(set_for_labels(labels, [set]).map(|(found, _)| found), Some(set), "{set:?}");
        }
    }

    #[test]
    fn test_tags() {
        assert_eq!(tag_of(ChannelSet::Surround51), 0x0079_0006);
        assert_eq!(tag_of(ChannelSet::Ambisonic(2)), 0x00be_0009);
        assert_eq!(tag_of(ChannelSet::Discrete(3)), 0x0093_0003);
        assert_eq!(tag_of(ChannelSet::Surround714), 0x00c0_000c);
    }
}
