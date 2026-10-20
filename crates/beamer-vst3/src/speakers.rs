//! Speaker arrangements of the channel sets.
//!
//! A VST3 host describes the channels of a bus with a speaker arrangement:
//! a bit per speaker, the channels in the order of their bits. Each
//! [`ChannelSet`] has one, in the order the set names its channels.
//! Discrete channels have no speaker positions; they take the first bits.

use beamer_core::{ChannelSet, MAX_CHANNELS};
use vst3::Steinberg::Vst::{SpeakerArr, SpeakerArrangement};

/// The speaker arrangement of a set.
pub(crate) fn arrangement_for(set: ChannelSet) -> SpeakerArrangement {
    match set {
        ChannelSet::Mono => SpeakerArr::kMono,
        ChannelSet::Stereo => SpeakerArr::kStereo,
        ChannelSet::Lcr => SpeakerArr::k30Cine,
        ChannelSet::Quad => SpeakerArr::k40Music,
        ChannelSet::Surround50 => SpeakerArr::k50,
        ChannelSet::Surround51 => SpeakerArr::k51,
        ChannelSet::Surround70 => SpeakerArr::k70Music,
        ChannelSet::Surround71 => SpeakerArr::k71Music,
        ChannelSet::Surround714 => SpeakerArr::k71_4,
        ChannelSet::Ambisonic(1) => SpeakerArr::kAmbi1stOrderACN,
        ChannelSet::Ambisonic(2) => SpeakerArr::kAmbi2cdOrderACN,
        ChannelSet::Ambisonic(3) => SpeakerArr::kAmbi3rdOrderACN,
        ChannelSet::Ambisonic(4) => SpeakerArr::kAmbi4thOrderACN,
        // Wider than MAX_CHANNELS (and their arrangements in the SDK share
        // bits with other speakers): as many discrete channels.
        set @ ChannelSet::Ambisonic(_) | set @ ChannelSet::Discrete(_) => discrete(set.channel_count()),
    }
}

/// The arrangement of `count` discrete channels: the first `count` bits.
fn discrete(count: u32) -> SpeakerArrangement {
    if count >= 64 {
        SpeakerArrangement::MAX
    } else {
        (1 << count) - 1
    }
}

/// The set a host means with `arrangement`.
///
/// Of the sets in `listed` (the ones the plugin lists for the bus), the
/// first with this arrangement. Otherwise the set with this arrangement;
/// any one channel is mono, other arrangements are discrete channels.
/// `None` for an arrangement without speakers or with more than
/// [`MAX_CHANNELS`].
pub(crate) fn set_for(
    arrangement: SpeakerArrangement,
    listed: impl IntoIterator<Item = ChannelSet>,
) -> Option<ChannelSet> {
    let count = arrangement.count_ones();
    if count == 0 || count as usize > MAX_CHANNELS {
        return None;
    }
    if let Some(set) = listed.into_iter().find(|&set| arrangement_for(set) == arrangement) {
        return Some(set);
    }
    let named = [
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
    if count == 1 {
        return Some(ChannelSet::Mono);
    }
    Some(
        named
            .into_iter()
            .find(|&set| arrangement_for(set) == arrangement)
            .unwrap_or(ChannelSet::Discrete(count)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::Steinberg::Vst::{kSpeakerC, kSpeakerL, kSpeakerSl, kSpeakerSr};

    #[test]
    fn test_arrangements_have_the_channel_count_of_their_set() {
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
            ChannelSet::Ambisonic(1),
            ChannelSet::Ambisonic(2),
            ChannelSet::Ambisonic(3),
            ChannelSet::Ambisonic(4),
            ChannelSet::Discrete(3),
            ChannelSet::Discrete(MAX_CHANNELS as u32),
        ] {
            assert_eq!(arrangement_for(set).count_ones(), set.channel_count(), "{set:?}");
            // Each set comes back as itself when the plugin lists it.
            assert_eq!(set_for(arrangement_for(set), [set]), Some(set), "{set:?}");
        }
    }

    #[test]
    fn test_named_arrangements_are_named_sets() {
        assert_eq!(set_for(SpeakerArr::kStereo, []), Some(ChannelSet::Stereo));
        assert_eq!(set_for(SpeakerArr::k51, []), Some(ChannelSet::Surround51));
        assert_eq!(set_for(SpeakerArr::k71_4, []), Some(ChannelSet::Surround714));
        assert_eq!(set_for(SpeakerArr::kAmbi2cdOrderACN, []), Some(ChannelSet::Ambisonic(2)));
        // 7.1 with the order of the 7.1 the sets name.
        assert_eq!(
            arrangement_for(ChannelSet::Surround71),
            SpeakerArr::k51 | kSpeakerSl | kSpeakerSr
        );
    }

    #[test]
    fn test_other_arrangements() {
        // Any one speaker is mono.
        assert_eq!(set_for(kSpeakerL, []), Some(ChannelSet::Mono));
        assert_eq!(set_for(kSpeakerC, []), Some(ChannelSet::Mono));
        // Arrangements the sets do not name are discrete channels.
        assert_eq!(set_for(SpeakerArr::k70Cine, []), Some(ChannelSet::Discrete(7)));
        // The arrangement of discrete channels is a named one for some
        // counts: the listed set decides.
        assert_eq!(arrangement_for(ChannelSet::Discrete(6)), SpeakerArr::k51);
        assert_eq!(set_for(SpeakerArr::k51, [ChannelSet::Discrete(6)]), Some(ChannelSet::Discrete(6)));
        assert_eq!(set_for(SpeakerArr::k51, [ChannelSet::Quad]), Some(ChannelSet::Surround51));
        // No speakers, or too many.
        assert_eq!(set_for(SpeakerArr::kEmpty, []), None);
        assert_eq!(set_for(discrete(MAX_CHANNELS as u32 + 1), []), None);
        assert_eq!(arrangement_for(ChannelSet::Ambisonic(5)).count_ones(), 36);
    }
}
