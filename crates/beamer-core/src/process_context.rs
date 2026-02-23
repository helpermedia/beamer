//! Transport and process context for audio plugins.
//!
//! This module provides [`Transport`] for DAW timing/playback state and
//! [`ProcessContext`] which bundles transport with sample rate and buffer size.
//!
//! # Example: Tempo-Synced Effect
//!
//! ```ignore
//! fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
//!     // Calculate LFO rate synced to tempo
//!     let lfo_hz = if let Some(tempo) = context.transport.tempo {
//!         tempo / 60.0 / 4.0  // 1 cycle per 4 beats
//!     } else {
//!         2.0  // Fallback to 2 Hz
//!     };
//!
//!     let samples_per_cycle = context.sample_rate / lfo_hz;
//!     // ...
//! }
//! ```
//!
//! # Example: Accessing MIDI CC Values
//!
//! ```ignore
//! fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
//!     if let Some(cc) = context.midi_cc() {
//!         let pitch_bend = cc.pitch_bend();  // -1.0 to 1.0
//!         let mod_wheel = cc.mod_wheel();    // 0.0 to 1.0
//!         let volume = cc.cc(7);             // 0.0 to 1.0
//!     }
//! }
//! ```

use crate::midi_cc_state::MidiCcState;

// =============================================================================
// FrameRate Enum
// =============================================================================

/// SMPTE frame rate for video synchronization.
///
/// Used with [`Transport::frame_rate`] for film/video sync applications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum FrameRate {
    /// 24 fps (film)
    #[default]
    Fps24 = 0,
    /// 25 fps (PAL video)
    Fps25 = 1,
    /// 29.97 fps non-drop (NTSC video)
    Fps2997 = 2,
    /// 30 fps
    Fps30 = 3,
    /// 29.97 fps drop-frame (NTSC broadcast)
    Fps2997Drop = 4,
    /// 30 fps drop-frame
    Fps30Drop = 5,
    /// 50 fps
    Fps50 = 10,
    /// 59.94 fps
    Fps5994 = 11,
    /// 60 fps
    Fps60 = 12,
    /// 59.94 fps drop-frame
    Fps5994Drop = 13,
    /// 60 fps drop-frame
    Fps60Drop = 14,
}

impl FrameRate {
    /// Returns the frames per second as an f64.
    ///
    /// Drop-frame rates return their actual (non-integer) values.
    #[inline]
    pub fn fps(&self) -> f64 {
        match self {
            Self::Fps24 => 24.0,
            Self::Fps25 => 25.0,
            Self::Fps2997 | Self::Fps2997Drop => 30000.0 / 1001.0, // 29.97...
            Self::Fps30 | Self::Fps30Drop => 30.0,
            Self::Fps50 => 50.0,
            Self::Fps5994 | Self::Fps5994Drop => 60000.0 / 1001.0, // 59.94...
            Self::Fps60 | Self::Fps60Drop => 60.0,
        }
    }

    /// Returns true if this is a drop-frame format.
    #[inline]
    pub fn is_drop_frame(&self) -> bool {
        matches!(
            self,
            Self::Fps2997Drop | Self::Fps30Drop | Self::Fps5994Drop | Self::Fps60Drop
        )
    }

    /// Creates a FrameRate from raw frames-per-second and drop-frame flag.
    ///
    /// This is the canonical conversion from VST3's FrameRate struct.
    /// Returns `None` for unsupported frame rates.
    ///
    /// # Arguments
    /// * `fps` - Frames per second (24, 25, 29, 30, 50, 59, 60)
    /// * `is_drop` - True if drop-frame timecode (only affects 29.97, 30, 59.94, 60)
    #[inline]
    pub fn from_raw(fps: u32, is_drop: bool) -> Option<Self> {
        match fps {
            24 => Some(Self::Fps24),
            25 => Some(Self::Fps25),
            29 if is_drop => Some(Self::Fps2997Drop),
            29 => Some(Self::Fps2997),
            30 if is_drop => Some(Self::Fps30Drop),
            30 => Some(Self::Fps30),
            50 => Some(Self::Fps50),
            59 if is_drop => Some(Self::Fps5994Drop),
            59 => Some(Self::Fps5994),
            60 if is_drop => Some(Self::Fps60Drop),
            60 => Some(Self::Fps60),
            _ => None,
        }
    }
}

// =============================================================================
// Transport Struct
// =============================================================================

/// Host transport and timing information.
///
/// Contains tempo, time signature, playback position and transport state.
/// All timing fields are `Option<T>` because not all hosts provide all data.
/// Playback state fields (`is_playing`, etc.) are always valid.
///
/// # Field Availability
///
/// Different DAWs provide different subsets of transport information:
/// - **Tempo/time signature**: Most DAWs provide these
/// - **Musical position**: Common but not universal
/// - **SMPTE/timecode**: Only in video-oriented DAWs
/// - **System time**: Rarely provided
///
/// Always check `Option` fields before use and provide sensible fallbacks.
///
/// # Example
///
/// ```ignore
/// // Safe tempo access with fallback
/// let tempo = context.transport.tempo.unwrap_or(120.0);
///
/// // Check if we have valid musical position
/// if let Some(beats) = context.transport.project_time_beats {
///     // Sync effect to beat position
/// }
///
/// // Transport state is always valid
/// if context.transport.is_playing {
///     // Process audio
/// } else {
///     // Maybe bypass or fade out
/// }
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Transport {
    // =========================================================================
    // Tempo and Time Signature
    // =========================================================================
    /// Current tempo in BPM (beats per minute).
    ///
    /// Typically 20-300, but can be any positive value.
    pub tempo: Option<f64>,

    /// Time signature numerator (e.g., 4 in 4/4, 3 in 3/4, 6 in 6/8).
    pub time_sig_numerator: Option<i32>,

    /// Time signature denominator (e.g., 4 in 4/4, 4 in 3/4, 8 in 6/8).
    pub time_sig_denominator: Option<i32>,

    // =========================================================================
    // Position
    // =========================================================================
    /// Project time in samples from the start of the timeline.
    ///
    /// This is the primary sample-accurate position. Always increments
    /// during playback, may jump on loop or locate.
    pub project_time_samples: Option<i64>,

    /// Project time in quarter notes (musical time).
    ///
    /// Takes tempo changes into account. 1.0 = one quarter note.
    pub project_time_beats: Option<f64>,

    /// Position of the last bar start in quarter notes.
    ///
    /// Useful for bar-synchronized effects (e.g., 4-bar delay).
    pub bar_position_beats: Option<f64>,

    // =========================================================================
    // Loop/Cycle
    // =========================================================================
    /// Loop/cycle start position in quarter notes.
    pub cycle_start_beats: Option<f64>,

    /// Loop/cycle end position in quarter notes.
    pub cycle_end_beats: Option<f64>,

    // =========================================================================
    // Transport State (always valid)
    // =========================================================================
    /// True if transport is currently playing.
    ///
    /// This is always valid (not an Option) because VST3 always provides it.
    pub is_playing: bool,

    /// True if recording is active.
    pub is_recording: bool,

    /// True if loop/cycle mode is enabled.
    pub is_cycle_active: bool,

    // =========================================================================
    // Advanced Timing
    // =========================================================================
    /// System time in nanoseconds.
    ///
    /// Can be used to sync to wall-clock time. Rarely provided by hosts.
    pub system_time_ns: Option<i64>,

    /// Continuous time in samples (doesn't reset on loop).
    ///
    /// Unlike `project_time_samples`, this never jumps during cycle playback -
    /// it always increments monotonically.
    pub continuous_time_samples: Option<i64>,

    /// Samples until next MIDI beat clock (24 ppqn).
    ///
    /// Used for generating MIDI clock messages or syncing to external gear.
    pub samples_to_next_clock: Option<i32>,

    // =========================================================================
    // SMPTE/Timecode
    // =========================================================================
    /// SMPTE offset in subframes (1/80th of a frame).
    ///
    /// For video synchronization. Divide by 80 to get frame offset.
    pub smpte_offset_subframes: Option<i32>,

    /// SMPTE frame rate.
    pub frame_rate: Option<FrameRate>,
}

impl Transport {
    /// Returns the time signature as a tuple (numerator, denominator).
    ///
    /// Returns `None` if either component is unavailable.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some((num, denom)) = transport.time_signature() {
    ///     println!("Playing in {}/{} time", num, denom);
    /// }
    /// ```
    #[inline]
    pub fn time_signature(&self) -> Option<(i32, i32)> {
        match (self.time_sig_numerator, self.time_sig_denominator) {
            (Some(num), Some(denom)) => Some((num, denom)),
            _ => None,
        }
    }

    /// Returns the loop/cycle range in quarter notes as (start, end).
    ///
    /// Returns `None` if either endpoint is unavailable.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some((start, end)) = transport.cycle_range() {
    ///     let loop_length_beats = end - start;
    /// }
    /// ```
    #[inline]
    pub fn cycle_range(&self) -> Option<(f64, f64)> {
        match (self.cycle_start_beats, self.cycle_end_beats) {
            (Some(start), Some(end)) => Some((start, end)),
            _ => None,
        }
    }

    /// Returns true if loop is active and has valid range.
    #[inline]
    pub fn is_looping(&self) -> bool {
        self.is_cycle_active && self.cycle_range().is_some()
    }

    /// Returns true if any timing info is available.
    #[inline]
    pub fn has_timing_info(&self) -> bool {
        self.tempo.is_some()
            || self.project_time_samples.is_some()
            || self.project_time_beats.is_some()
    }

    /// Returns true if time signature info is complete.
    #[inline]
    pub fn has_time_signature(&self) -> bool {
        self.time_sig_numerator.is_some() && self.time_sig_denominator.is_some()
    }

    /// Converts SMPTE subframes to (frames, subframes) tuple.
    ///
    /// Subframes are 0-79 within each frame.
    /// Uses Euclidean division to correctly handle negative offsets.
    #[inline]
    pub fn smpte_frames(&self) -> Option<(i32, i32)> {
        self.smpte_offset_subframes
            .map(|sf| (sf.div_euclid(80), sf.rem_euclid(80)))
    }
}

// =============================================================================
// ProcessContext Struct
// =============================================================================

/// Complete processing context for a single `process()` call.
///
/// Contains sample rate, buffer size, transport/timing information and
/// optional MIDI CC state for direct access to controller values.
/// Passed as the third parameter to [`Processor::process()`].
///
/// # Lifetime
///
/// ProcessContext is valid only within a single `process()` call.
/// Do not store references to it across calls.
///
/// # Example
///
/// ```ignore
/// impl Processor for MyDelayPlugin {
///     fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
///         // Calculate tempo-synced delay time
///         let delay_samples = if let Some(tempo) = context.transport.tempo {
///             // Quarter note delay
///             let quarter_note_sec = 60.0 / tempo;
///             (quarter_note_sec * context.sample_rate) as usize
///         } else {
///             // Fallback: 500ms
///             (0.5 * context.sample_rate) as usize
///         };
///
///         // Access MIDI CC values directly
///         if let Some(cc) = context.midi_cc() {
///             let mod_depth = cc.mod_wheel();
///         }
///
///         // Use context.num_samples for buffer size
///         for i in 0..context.num_samples {
///             // Process...
///         }
///     }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ProcessContext<'a> {
    /// Current sample rate in Hz.
    ///
    /// Same value passed to [`Processor::setup()`], provided here
    /// for convenience during processing.
    pub sample_rate: f64,

    /// Number of samples in this processing block.
    ///
    /// Same as [`Buffer::num_samples()`], provided here for convenience.
    pub num_samples: usize,

    /// Host transport and timing information.
    pub transport: Transport,

    /// MIDI CC state for direct access to controller values.
    ///
    /// Only present if the plugin returned `Some(MidiCcConfig)` from
    /// `midi_cc_config()`. Use [`ProcessContext::midi_cc()`] to access.
    midi_cc_state: Option<&'a MidiCcState>,
}

impl<'a> ProcessContext<'a> {
    /// Creates a new ProcessContext.
    ///
    /// This is called by the VST3 wrapper, not by plugin code.
    #[inline]
    pub fn new(sample_rate: f64, num_samples: usize, transport: Transport) -> Self {
        Self {
            sample_rate,
            num_samples,
            transport,
            midi_cc_state: None,
        }
    }

    /// Creates a new ProcessContext with MIDI CC state.
    ///
    /// This is called by the VST3 wrapper when the plugin has MIDI CC config.
    #[inline]
    pub fn with_midi_cc(
        sample_rate: f64,
        num_samples: usize,
        transport: Transport,
        midi_cc_state: &'a MidiCcState,
    ) -> Self {
        Self {
            sample_rate,
            num_samples,
            transport,
            midi_cc_state: Some(midi_cc_state),
        }
    }

    /// Creates a context with default (empty) transport.
    ///
    /// Used when the host doesn't provide ProcessContext.
    #[inline]
    pub fn with_empty_transport(sample_rate: f64, num_samples: usize) -> Self {
        Self {
            sample_rate,
            num_samples,
            transport: Transport::default(),
            midi_cc_state: None,
        }
    }

    /// Returns MIDI CC state for direct access to controller values.
    ///
    /// Only returns `Some` if the plugin returned `Some(MidiCcConfig)` from
    /// `midi_cc_config()`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &ProcessContext) {
    ///     if let Some(cc) = context.midi_cc() {
    ///         let pitch_bend = cc.pitch_bend();  // -1.0 to 1.0
    ///         let mod_wheel = cc.mod_wheel();    // 0.0 to 1.0
    ///         let volume = cc.cc(7);             // 0.0 to 1.0
    ///     }
    /// }
    /// ```
    #[inline]
    pub fn midi_cc(&self) -> Option<&MidiCcState> {
        self.midi_cc_state
    }

    /// Calculates the duration of this buffer in seconds.
    #[inline]
    pub fn buffer_duration(&self) -> f64 {
        self.num_samples as f64 / self.sample_rate
    }

    /// Calculates samples per beat at the current tempo.
    ///
    /// Returns `None` if tempo is unavailable.
    ///
    /// # Example
    ///
    /// ```ignore
    /// if let Some(spb) = context.samples_per_beat() {
    ///     let delay_samples = spb * 0.25; // 16th note delay
    /// }
    /// ```
    #[inline]
    pub fn samples_per_beat(&self) -> Option<f64> {
        self.transport
            .tempo
            .map(|tempo| self.sample_rate * 60.0 / tempo)
    }
}

impl Default for ProcessContext<'_> {
    fn default() -> Self {
        Self {
            sample_rate: 44100.0,
            num_samples: 0,
            transport: Transport::default(),
            midi_cc_state: None,
        }
    }
}
