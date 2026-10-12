//! Transport information for Audio Unit.
//!
//! The AU wrappers ask the host for its musical context and transport state on
//! every render call (through the host blocks of an AUv3, or the host callbacks
//! of an AUv2) and pass the answers to `beamer_au_render` as a
//! [`BeamerAuTransport`]. This module converts that to the framework's
//! [`Transport`], which enables tempo-synced effects, sequencers and other
//! plugins that need timing information.

use beamer_core::Transport;

/// Host transport and musical context for one render call (matches
/// BeamerAuTransport in header).
///
/// A host does not have to provide everything. `flags` tells which groups of
/// fields are valid, the other fields of a missing group are zero.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BeamerAuTransport {
    /// Combination of the `BeamerAuTransport` flag constants
    pub flags: u32,
    /// Time signature denominator (e.g., 4 in 3/4)
    pub time_sig_denominator: i32,
    /// Tempo in beats per minute
    pub tempo: f64,
    /// Position on the timeline in beats (quarter notes)
    pub beat_position: f64,
    /// Time signature numerator (e.g., 3 in 3/4)
    pub time_sig_numerator: f64,
    /// Position of the start of the current measure in beats
    pub measure_downbeat_position: f64,
    /// Position on the timeline in samples
    pub sample_position: f64,
    /// Start of the cycle (loop) range in beats
    pub cycle_start_beat: f64,
    /// End of the cycle (loop) range in beats
    pub cycle_end_beat: f64,
}

impl BeamerAuTransport {
    /// `tempo` and `beat_position` are valid.
    pub const HAS_BEAT_AND_TEMPO: u32 = 1 << 0;
    /// The time signature and `measure_downbeat_position` are valid.
    pub const HAS_TIME_SIGNATURE: u32 = 1 << 1;
    /// The state flags below, `sample_position` and the cycle range are valid.
    pub const HAS_STATE: u32 = 1 << 2;
    /// The transport is moving.
    pub const PLAYING: u32 = 1 << 3;
    /// The host is recording.
    pub const RECORDING: u32 = 1 << 4;
    /// Cycle (loop) mode is on.
    pub const CYCLING: u32 = 1 << 5;

    fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }
}

/// Build the `Transport` for a render call.
///
/// `host` is what the wrapper got from the host, `None` if it passed nothing.
/// `render_sample_time` is the sample time of the render timestamp: a running
/// count that also serves as the position when the host does not report one.
pub fn transport_from_host(host: Option<&BeamerAuTransport>, render_sample_time: i64) -> Transport {
    let mut transport = Transport {
        project_time_samples: Some(render_sample_time),
        continuous_time_samples: Some(render_sample_time),
        ..Default::default()
    };

    let Some(host) = host else {
        return transport;
    };

    if host.has(BeamerAuTransport::HAS_BEAT_AND_TEMPO) {
        if host.tempo.is_finite() && host.tempo > 0.0 {
            transport.tempo = Some(host.tempo);
        }
        if host.beat_position.is_finite() {
            transport.project_time_beats = Some(host.beat_position);
        }
    }

    if host.has(BeamerAuTransport::HAS_TIME_SIGNATURE) {
        let numerator = host.time_sig_numerator.round();
        if numerator >= 1.0 && numerator <= i32::MAX as f64 && host.time_sig_denominator > 0 {
            transport.time_sig_numerator = Some(numerator as i32);
            transport.time_sig_denominator = Some(host.time_sig_denominator);
        }
        if host.measure_downbeat_position.is_finite() {
            transport.bar_position_beats = Some(host.measure_downbeat_position);
        }
    }

    if host.has(BeamerAuTransport::HAS_STATE) {
        transport.is_playing = host.has(BeamerAuTransport::PLAYING);
        transport.is_recording = host.has(BeamerAuTransport::RECORDING);
        transport.is_cycle_active = host.has(BeamerAuTransport::CYCLING);

        if host.sample_position.is_finite() {
            transport.project_time_samples = Some(host.sample_position.round() as i64);
        }
        let cycle_is_valid = host.cycle_start_beat.is_finite() && host.cycle_end_beat.is_finite();
        if transport.is_cycle_active && cycle_is_valid {
            transport.cycle_start_beats = Some(host.cycle_start_beat);
            transport.cycle_end_beats = Some(host.cycle_end_beat);
        }
    }

    transport
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_host_transport() -> BeamerAuTransport {
        BeamerAuTransport {
            flags: BeamerAuTransport::HAS_BEAT_AND_TEMPO
                | BeamerAuTransport::HAS_TIME_SIGNATURE
                | BeamerAuTransport::HAS_STATE
                | BeamerAuTransport::PLAYING
                | BeamerAuTransport::CYCLING,
            time_sig_denominator: 8,
            tempo: 97.5,
            beat_position: 17.5,
            time_sig_numerator: 6.0,
            measure_downbeat_position: 15.0,
            sample_position: 5000.0,
            cycle_start_beat: 8.0,
            cycle_end_beat: 24.0,
        }
    }

    #[test]
    fn test_layout_matches_header() {
        assert_eq!(std::mem::size_of::<BeamerAuTransport>(), 64);
        assert_eq!(std::mem::offset_of!(BeamerAuTransport, tempo), 8);
        assert_eq!(std::mem::offset_of!(BeamerAuTransport, cycle_end_beat), 56);
    }

    #[test]
    fn test_without_host_transport() {
        let transport = transport_from_host(None, 44100);
        assert_eq!(transport.project_time_samples, Some(44100));
        assert_eq!(transport.continuous_time_samples, Some(44100));
        assert_eq!(transport.tempo, None);
        assert!(!transport.is_playing);
    }

    #[test]
    fn test_full_host_transport() {
        let transport = transport_from_host(Some(&full_host_transport()), 44100);

        assert_eq!(transport.tempo, Some(97.5));
        assert_eq!(transport.time_signature(), Some((6, 8)));
        assert_eq!(transport.project_time_beats, Some(17.5));
        assert_eq!(transport.bar_position_beats, Some(15.0));

        // The host position replaces the render clock, which stays available
        // as the continuous time.
        assert_eq!(transport.project_time_samples, Some(5000));
        assert_eq!(transport.continuous_time_samples, Some(44100));

        assert!(transport.is_playing);
        assert!(!transport.is_recording);
        assert!(transport.is_cycle_active);
        assert_eq!(transport.cycle_start_beats, Some(8.0));
        assert_eq!(transport.cycle_end_beats, Some(24.0));
    }

    #[test]
    fn test_only_valid_groups_are_used() {
        // Values without their flag are ignored.
        let host = BeamerAuTransport {
            flags: BeamerAuTransport::HAS_BEAT_AND_TEMPO,
            ..full_host_transport()
        };
        let transport = transport_from_host(Some(&host), 100);

        assert_eq!(transport.tempo, Some(97.5));
        assert_eq!(transport.project_time_beats, Some(17.5));
        assert_eq!(transport.time_signature(), None);
        assert_eq!(transport.bar_position_beats, None);
        assert_eq!(transport.project_time_samples, Some(100));
        assert!(!transport.is_playing);
        assert_eq!(transport.cycle_start_beats, None);
    }

    #[test]
    fn test_invalid_values_are_dropped() {
        let host = BeamerAuTransport {
            tempo: 0.0,
            time_sig_numerator: 0.0,
            beat_position: f64::NAN,
            ..full_host_transport()
        };
        let transport = transport_from_host(Some(&host), 0);

        assert_eq!(transport.tempo, None);
        assert_eq!(transport.project_time_beats, None);
        assert_eq!(transport.time_signature(), None);
        // The rest of the time signature group is still used.
        assert_eq!(transport.bar_position_beats, Some(15.0));
    }

    #[test]
    fn test_time_signature_values() {
        let with = |numerator: f64, denominator: i32| {
            let host = BeamerAuTransport {
                time_sig_numerator: numerator,
                time_sig_denominator: denominator,
                ..full_host_transport()
            };
            transport_from_host(Some(&host), 0).time_signature()
        };

        // The numerator is a floating point value in the AU APIs.
        assert_eq!(with(3.9999, 4), Some((4, 4)));
        assert_eq!(with(7.0, 8), Some((7, 8)));
        assert_eq!(with(0.4, 4), None);
        assert_eq!(with(4.0, 0), None);
        assert_eq!(with(4.0, -4), None);
    }

    #[test]
    fn test_negative_positions_are_kept() {
        // Hosts report negative positions during a count-in.
        let host = BeamerAuTransport {
            beat_position: -2.5,
            measure_downbeat_position: -4.0,
            sample_position: -1000.0,
            ..full_host_transport()
        };
        let transport = transport_from_host(Some(&host), 0);

        assert_eq!(transport.project_time_beats, Some(-2.5));
        assert_eq!(transport.bar_position_beats, Some(-4.0));
        assert_eq!(transport.project_time_samples, Some(-1000));
    }

    #[test]
    fn test_state_without_musical_context() {
        let host = BeamerAuTransport {
            flags: BeamerAuTransport::HAS_STATE | BeamerAuTransport::RECORDING,
            ..full_host_transport()
        };
        let transport = transport_from_host(Some(&host), 100);

        assert!(!transport.is_playing);
        assert!(transport.is_recording);
        assert_eq!(transport.project_time_samples, Some(5000));
        assert_eq!(transport.tempo, None);
        assert_eq!(transport.project_time_beats, None);
    }

    #[test]
    fn test_cycle_range_only_while_cycling() {
        let mut host = full_host_transport();
        host.flags &= !BeamerAuTransport::CYCLING;
        let transport = transport_from_host(Some(&host), 0);

        assert!(!transport.is_cycle_active);
        assert_eq!(transport.cycle_start_beats, None);
        assert_eq!(transport.cycle_end_beats, None);

        // An unusable range is left out, the state itself is kept.
        let host = BeamerAuTransport {
            cycle_end_beat: f64::NAN,
            ..full_host_transport()
        };
        let transport = transport_from_host(Some(&host), 0);
        assert!(transport.is_cycle_active);
        assert_eq!(transport.cycle_start_beats, None);
        assert_eq!(transport.cycle_end_beats, None);
    }
}
