//! Transport information extraction for Audio Unit.
//!
//! This module provides functions to extract transport information from the
//! AU host's musical context. This enables tempo-synced effects, sequencers,
//! and other plugins that need timing information.
//!
//! # AU Musical Context
//!
//! Audio Units provide transport info via `AUHostMusicalContextBlock`, a block
//! that the host provides to query tempo, time signature, beat position, etc.
//!
//! Unlike VST3's ProcessContext (which provides all info in a struct), AU
//! requires calling a block to retrieve the information.

use beamer_core::Transport;
use std::ffi::c_void;

use crate::objc_block;

/// Extract transport information from AU's host musical context.
///
/// Queries the AU host for tempo, time signature, position, and playback state.
/// Returns `Transport::default()` if the context block is unavailable or if
/// the query fails.
///
/// # Arguments
///
/// * `musical_context_block` - Pointer to the AU host's musical context block
/// * `sample_position` - Current sample position from AudioTimeStamp
/// * `is_playing` - Whether transport is currently playing (from action flags)
///
/// # Safety
///
/// Must be called from within render callback with valid pointers.
/// The `musical_context_block` must point to a valid `AUHostMusicalContextBlock`
/// or be null.
///
/// # Returns
///
/// A `Transport` struct populated with available information, or default values
/// if the host doesn't provide transport info.
///
/// # Example
///
/// ```ignore
/// // In render callback:
/// let transport = unsafe {
///     extract_transport_from_au(
///         host_musical_context_block,
///         timestamp.sample_time as i64,
///         is_playing,
///     )
/// };
/// ```
pub unsafe fn extract_transport_from_au(
    musical_context_block: *const c_void,
    sample_position: i64,
    is_playing: bool,
) -> Transport {
    // If no musical context block provided, return default transport
    if musical_context_block.is_null() {
        return Transport {
            project_time_samples: Some(sample_position),
            is_playing,
            ..Default::default()
        };
    }

    // AUHostMusicalContextBlock signature (from Apple's Audio Unit v3 API):
    // OSStatus (^)(
    //     double *outCurrentTempo,
    //     double *outTimeSignatureNumerator,
    //     NSInteger *outTimeSignatureDenominator,
    //     double *outCurrentBeatPosition,
    //     NSInteger *outSampleOffsetToNextBeat,
    //     double *outCurrentMeasureDownbeatPosition
    // )

    let mut tempo: f64 = 0.0;
    let mut time_sig_numerator: f64 = 0.0;
    let mut time_sig_denominator: isize = 0;
    let mut beat_position: f64 = 0.0;
    let mut sample_offset_to_next_beat: isize = 0;
    let mut measure_downbeat_position: f64 = 0.0;

    // Define the function signature that matches Apple's AUHostMusicalContextBlock.
    // The first parameter is the block pointer itself (Objective-C block convention).
    type MusicalContextBlockFn = unsafe extern "C" fn(
        *const c_void, // Block pointer itself (implicit in Objective-C blocks)
        *mut f64,      // outCurrentTempo
        *mut f64,      // outTimeSignatureNumerator
        *mut isize,    // outTimeSignatureDenominator (NSInteger)
        *mut f64,      // outCurrentBeatPosition
        *mut isize,    // outSampleOffsetToNextBeat (NSInteger)
        *mut f64,      // outCurrentMeasureDownbeatPosition
    ) -> i32; // OSStatus (0 = success)

    // SAFETY: This transmute is required because Rust doesn't have native Objective-C block support.
    //
    // Why this transmute is needed:
    // - AU hosts provide callbacks as Objective-C blocks (*const c_void)
    // - Objective-C blocks have a C function pointer at a known offset in their structure
    // - We must call this function pointer with the correct signature
    //
    // Invariants that must hold:
    // 1. `musical_context_block` must be a valid AUHostMusicalContextBlock provided by the AU host
    // 2. The block must remain valid for the duration of this render callback
    // 3. The function signature must exactly match Apple's documented AUHostMusicalContextBlock:
    //    - First arg: block pointer itself (Objective-C convention)
    //    - Remaining args: output parameters as documented in AU API
    // 4. The block must be called from the same thread that received it (AU render thread)
    //
    // What could go wrong:
    // - If the block pointer is invalid or corrupted → undefined behavior (likely crash)
    // - If the signature doesn't match → argument misalignment, undefined behavior
    // - If called from wrong thread → potential race conditions (violates AU threading model)
    // - If the block is used after the render callback returns → use-after-free
    //
    // Why this is safe in practice:
    // - AU hosts guarantee the block pointer is valid during the render callback
    // - Our signature matches Apple's documented API exactly
    // - We only call from within the render callback, never store the pointer
    // - The block is provided by the host, not created by us
    //
    // Alternative approach:
    // - Use the `block2` crate for proper Objective-C block handling (adds dependency)
    // - This would eliminate the transmute but requires understanding block2's API
    let invoke = objc_block::invoke_ptr(musical_context_block);
    let block_fn: MusicalContextBlockFn = std::mem::transmute(invoke);

    let result = block_fn(
        musical_context_block,
        &mut tempo,
        &mut time_sig_numerator,
        &mut time_sig_denominator,
        &mut beat_position,
        &mut sample_offset_to_next_beat,
        &mut measure_downbeat_position,
    );

    // If the call succeeded (result == 0), populate Transport
    if result == 0 {
        Transport {
            // Tempo and time signature
            tempo: if tempo > 0.0 { Some(tempo) } else { None },
            time_sig_numerator: if time_sig_numerator > 0.0 {
                Some(time_sig_numerator as i32)
            } else {
                None
            },
            time_sig_denominator: if time_sig_denominator > 0 {
                Some(time_sig_denominator as i32)
            } else {
                None
            },

            // Position information
            project_time_samples: Some(sample_position),
            project_time_beats: if beat_position >= 0.0 {
                Some(beat_position)
            } else {
                None
            },
            bar_position_beats: if measure_downbeat_position >= 0.0 {
                Some(beat_position - measure_downbeat_position)
            } else {
                None
            },

            // Playback state
            is_playing,

            // Fields not provided by AU (remain None)
            is_recording: false,    // AU doesn't provide recording state
            is_cycle_active: false, // AU doesn't provide loop info directly
            cycle_start_beats: None,
            cycle_end_beats: None,
            system_time_ns: None,
            continuous_time_samples: None,
            samples_to_next_clock: if sample_offset_to_next_beat > 0 {
                Some(sample_offset_to_next_beat as i32)
            } else {
                None
            },
            smpte_offset_subframes: None, // AU doesn't provide SMPTE
            frame_rate: None,
        }
    } else {
        // Query failed, return minimal transport with just sample position
        Transport {
            project_time_samples: Some(sample_position),
            is_playing,
            ..Default::default()
        }
    }
}

/// Extract transport from AU with simplified signature.
///
/// This is a convenience function for when you only have basic timing info
/// and no musical context block.
///
/// # Arguments
///
/// * `sample_position` - Current sample position
/// * `is_playing` - Whether transport is playing
#[allow(dead_code)]
pub fn create_basic_transport(sample_position: i64, is_playing: bool) -> Transport {
    Transport {
        project_time_samples: Some(sample_position),
        is_playing,
        ..Default::default()
    }
}

/// Check if AU provides musical context.
///
/// Some hosts (like Logic Pro, GarageBand) provide rich musical context,
/// while others may not.
///
/// # Safety
///
/// The `musical_context_block` pointer must be valid or null.
#[allow(dead_code)]
pub unsafe fn has_musical_context(musical_context_block: *const c_void) -> bool {
    !musical_context_block.is_null()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_transport() {
        let transport = create_basic_transport(44100, true);
        assert_eq!(transport.project_time_samples, Some(44100));
        assert!(transport.is_playing);
        assert_eq!(transport.tempo, None);
    }

    #[test]
    fn test_null_context() {
        // SAFETY: Passing a null pointer is explicitly safe per the function documentation.
        // The function checks for null and returns a default Transport with the provided sample position.
        let transport = unsafe { extract_transport_from_au(std::ptr::null(), 0, false) };
        assert_eq!(transport.project_time_samples, Some(0));
        assert!(!transport.is_playing);
    }

    #[test]
    fn test_has_musical_context() {
        // SAFETY: Passing a null pointer is valid and expected - the function checks for null.
        // This test verifies the null-checking behavior of has_musical_context.
        unsafe {
            assert!(!has_musical_context(std::ptr::null()));
            // Can't test non-null case without actual AU host
        }
    }
}
