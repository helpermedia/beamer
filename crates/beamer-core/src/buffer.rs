//! Audio buffer abstractions for plugin processing.
//!
//! This module provides [`Buffer`] for main audio I/O and [`AuxiliaryBuffers`]
//! for sidechain and auxiliary bus access.
//!
//! # Architecture
//!
//! Audio processing in Beamer uses two separate buffer types:
//!
//! - **[`Buffer`]**: Main stereo/surround I/O - used by all plugins
//! - **[`AuxiliaryBuffers`]**: Sidechain and aux buses - used by multi-bus plugins
//!
//! This separation solves Rust's lifetime variance constraints with nested mutable
//! references while providing a clean, ergonomic API.
//!
//! # Real-Time Safety
//!
//! All buffer types use fixed-size stack storage with no heap allocations.
//! This guarantees real-time safety in audio processing callbacks.
//!
//! # Generic Sample Type
//!
//! All buffer types are generic over `S: Sample`, defaulting to `f32`. This enables
//! zero-cost generic processing for both 32-bit and 64-bit audio.
//!
//! # Example: Simple Gain Plugin
//!
//! ```ignore
//! fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers) {
//!     let gain = self.params.gain();
//!     for (input, output) in buffer.zip_channels() {
//!         for (i, o) in input.iter().zip(output.iter_mut()) {
//!             *o = *i * gain;
//!         }
//!     }
//! }
//! ```
//!
//! # Example: Sidechain Compressor
//!
//! ```ignore
//! fn process(&mut self, buffer: &mut Buffer, aux: &mut AuxiliaryBuffers) {
//!     // Analyze sidechain input
//!     let key_level = aux.sidechain()
//!         .map(|sc| sc.rms(0))  // RMS of first channel
//!         .unwrap_or(0.0);
//!
//!     // Apply compression based on sidechain
//!     let reduction = self.compute_gain_reduction(key_level);
//!     for output in buffer.outputs_mut() {
//!         for sample in output {
//!             *sample *= reduction;
//!         }
//!     }
//! }
//! ```

use crate::sample::Sample;
use crate::types::{MAX_AUX_BUSES, MAX_CHANNELS};

// =============================================================================
// Buffer - Main Audio I/O
// =============================================================================

/// Main audio buffer for plugin processing.
///
/// Contains input and output channel slices for the primary audio bus.
/// This is what most plugins interact with - simple stereo or surround I/O.
///
/// # Type Parameter
///
/// `S` is the sample type, defaulting to `f32`. Use `Buffer<f64>` for
/// 64-bit double precision processing.
///
/// # Lifetime
///
/// The `'a` lifetime ties the buffer to the host's audio data. Buffers are
/// only valid within a single `process()` call.
///
/// # Channel Layout
///
/// Channels are indexed starting from 0:
/// - Stereo: 0 = Left, 1 = Right
/// - Surround: 0 = Left, 1 = Right, 2 = Center, etc.
///
/// # Real-Time Safety
///
/// This struct uses fixed-size stack storage. No heap allocations occur
/// during construction or use.
pub struct Buffer<'a, S: Sample = f32> {
    /// Input channel slices (immutable audio from host)
    /// Option<&[S]> is Copy, so [None; N] works
    inputs: [Option<&'a [S]>; MAX_CHANNELS],
    /// Output channel slices (mutable audio to host)
    outputs: [Option<&'a mut [S]>; MAX_CHANNELS],
    /// Number of active input channels
    num_input_channels: usize,
    /// Number of active output channels
    num_output_channels: usize,
    /// Number of samples in this processing block
    num_samples: usize,
}

impl<'a, S: Sample> Buffer<'a, S> {
    /// Create a new buffer from channel slices.
    ///
    /// This is called by the VST3 wrapper, not by plugin code.
    /// Channels beyond [`MAX_CHANNELS`] are silently ignored.
    #[inline]
    pub fn new(
        inputs: impl IntoIterator<Item = &'a [S]>,
        outputs: impl IntoIterator<Item = &'a mut [S]>,
        num_samples: usize,
    ) -> Self {
        let mut input_arr: [Option<&'a [S]>; MAX_CHANNELS] = [None; MAX_CHANNELS];
        let mut num_input_channels = 0;
        for (i, slice) in inputs.into_iter().take(MAX_CHANNELS).enumerate() {
            input_arr[i] = Some(slice);
            num_input_channels = i + 1;
        }

        // Can't use [None; N] for &mut because it's not Copy
        let mut output_arr: [Option<&'a mut [S]>; MAX_CHANNELS] = std::array::from_fn(|_| None);
        let mut num_output_channels = 0;
        for (i, slice) in outputs.into_iter().take(MAX_CHANNELS).enumerate() {
            output_arr[i] = Some(slice);
            num_output_channels = i + 1;
        }

        Self {
            inputs: input_arr,
            outputs: output_arr,
            num_input_channels,
            num_output_channels,
            num_samples,
        }
    }

    // =========================================================================
    // Buffer Info
    // =========================================================================

    /// Number of samples in this processing block.
    #[inline]
    pub fn num_samples(&self) -> usize {
        self.num_samples
    }

    /// Number of input channels.
    #[inline]
    pub fn num_input_channels(&self) -> usize {
        self.num_input_channels
    }

    /// Number of output channels.
    #[inline]
    pub fn num_output_channels(&self) -> usize {
        self.num_output_channels
    }

    /// Returns true if this is a stereo buffer (2 in, 2 out).
    #[inline]
    pub fn is_stereo(&self) -> bool {
        self.num_input_channels == 2 && self.num_output_channels == 2
    }

    /// Returns true if this is a mono buffer (1 in, 1 out).
    #[inline]
    pub fn is_mono(&self) -> bool {
        self.num_input_channels == 1 && self.num_output_channels == 1
    }

    // =========================================================================
    // Channel Access
    // =========================================================================

    /// Get an input channel by index.
    ///
    /// Returns an empty slice if the channel doesn't exist.
    #[inline]
    pub fn input(&self, channel: usize) -> &[S] {
        self.inputs
            .get(channel)
            .and_then(|opt| opt.as_ref())
            .map(|ch| &ch[..self.num_samples])
            .unwrap_or(&[])
    }

    /// Get a mutable output channel by index.
    ///
    /// # Panics
    ///
    /// Panics if the channel index is out of bounds.
    #[inline]
    pub fn output(&mut self, channel: usize) -> &mut [S] {
        let n = self.num_samples;
        self.outputs[channel]
            .as_mut()
            .map(|ch| &mut ch[..n])
            .expect("output channel out of bounds")
    }

    /// Try to get a mutable output channel by index.
    ///
    /// Returns `None` if the channel doesn't exist.
    #[inline]
    pub fn output_checked(&mut self, channel: usize) -> Option<&mut [S]> {
        let n = self.num_samples;
        self.outputs
            .get_mut(channel)
            .and_then(|opt| opt.as_mut())
            .map(|ch| &mut ch[..n])
    }

    // =========================================================================
    // Iterators
    // =========================================================================

    /// Iterate over all input channels.
    #[inline]
    pub fn inputs(&self) -> impl Iterator<Item = &[S]> + '_ {
        let n = self.num_samples;
        self.inputs[..self.num_input_channels]
            .iter()
            .filter_map(move |opt| opt.as_ref().map(|ch| &ch[..n]))
    }

    /// Iterate over all output channels mutably.
    #[inline]
    pub fn outputs_mut(&mut self) -> impl Iterator<Item = &mut [S]> + use<'_, 'a, S> {
        let n = self.num_samples;
        self.outputs[..self.num_output_channels]
            .iter_mut()
            .filter_map(move |opt| opt.as_mut().map(|ch| &mut ch[..n]))
    }

    /// Iterate over paired (input, output) channels.
    ///
    /// This is the most common pattern for in-place processing.
    /// Only yields channels that exist in both input and output.
    ///
    /// # Example
    ///
    /// ```ignore
    /// for (input, output) in buffer.zip_channels() {
    ///     for (i, o) in input.iter().zip(output.iter_mut()) {
    ///         *o = *i * gain;
    ///     }
    /// }
    /// ```
    #[inline]
    pub fn zip_channels(&mut self) -> impl Iterator<Item = (&[S], &mut [S])> + use<'_, 'a, S> {
        let n = self.num_samples;
        let num_pairs = self.num_input_channels.min(self.num_output_channels);
        self.inputs[..num_pairs]
            .iter()
            .zip(self.outputs[..num_pairs].iter_mut())
            .filter_map(move |(i_opt, o_opt)| {
                match (i_opt.as_ref(), o_opt.as_mut()) {
                    (Some(i), Some(o)) => Some((&i[..n], &mut o[..n])),
                    _ => None,
                }
            })
    }

    // =========================================================================
    // Bulk Operations
    // =========================================================================

    /// Copy all input channels to output channels.
    ///
    /// Useful for bypass or passthrough. Only copies channels that exist
    /// in both input and output.
    pub fn copy_to_output(&mut self) {
        let num_channels = self.num_input_channels.min(self.num_output_channels);
        let n = self.num_samples;
        for ch in 0..num_channels {
            if let (Some(input), Some(output)) = (self.inputs[ch].as_ref(), self.outputs[ch].as_mut()) {
                output[..n].copy_from_slice(&input[..n]);
            }
        }
    }

    /// Clear all output channels to silence.
    pub fn clear_outputs(&mut self) {
        let n = self.num_samples;
        for opt in self.outputs[..self.num_output_channels].iter_mut() {
            if let Some(output) = opt.as_mut() {
                output[..n].fill(S::ZERO);
            }
        }
    }

    /// Apply a gain factor to all output channels.
    pub fn apply_output_gain(&mut self, gain: S) {
        let n = self.num_samples;
        for opt in self.outputs[..self.num_output_channels].iter_mut() {
            if let Some(output) = opt.as_mut() {
                for sample in &mut output[..n] {
                    *sample = *sample * gain;
                }
            }
        }
    }
}

// =============================================================================
// AuxiliaryBuffers - Sidechain and Aux Buses
// =============================================================================

/// Auxiliary audio buffers for sidechain and multi-bus processing.
///
/// Contains all non-main audio buses. Most plugins don't need this -
/// they only use the main [`Buffer`].
///
/// # Type Parameter
///
/// `S` is the sample type, defaulting to `f32`. Use `AuxiliaryBuffers<f64>` for
/// 64-bit double precision processing.
///
/// # Bus Indexing
///
/// Auxiliary buses are indexed starting from 0:
/// - Bus 0: Sidechain (most common aux use case)
/// - Bus 1+: Additional auxiliary I/O
///
/// # Real-Time Safety
///
/// This struct uses fixed-size stack storage. No heap allocations occur
/// during construction or use.
///
/// # Example: Sidechain Access
///
/// ```ignore
/// if let Some(sidechain) = aux.sidechain() {
///     let key_signal = sidechain.channel(0);
///     // Use for compression keying, ducking, etc.
/// }
/// ```
pub struct AuxiliaryBuffers<'a, S: Sample = f32> {
    /// Auxiliary input buses (e.g., sidechain inputs)
    /// Outer array: buses, Inner array: channels per bus
    inputs: [[Option<&'a [S]>; MAX_CHANNELS]; MAX_AUX_BUSES],
    /// Number of channels per input bus
    input_channel_counts: [usize; MAX_AUX_BUSES],
    /// Number of active input buses
    num_input_buses: usize,

    /// Auxiliary output buses (e.g., aux sends)
    outputs: [[Option<&'a mut [S]>; MAX_CHANNELS]; MAX_AUX_BUSES],
    /// Number of channels per output bus
    output_channel_counts: [usize; MAX_AUX_BUSES],
    /// Number of active output buses
    num_output_buses: usize,

    /// Number of samples in this processing block
    num_samples: usize,
}

impl<'a, S: Sample> AuxiliaryBuffers<'a, S> {
    /// Create new auxiliary buffers.
    ///
    /// This is called by the VST3 wrapper, not by plugin code.
    /// Buses/channels beyond the limits are silently ignored.
    #[inline]
    pub fn new(
        inputs: impl IntoIterator<Item = impl IntoIterator<Item = &'a [S]>>,
        outputs: impl IntoIterator<Item = impl IntoIterator<Item = &'a mut [S]>>,
        num_samples: usize,
    ) -> Self {
        // Initialize input buses
        let mut input_arr: [[Option<&'a [S]>; MAX_CHANNELS]; MAX_AUX_BUSES] =
            [[None; MAX_CHANNELS]; MAX_AUX_BUSES];
        let mut input_channel_counts = [0usize; MAX_AUX_BUSES];
        let mut num_input_buses = 0;

        for (bus_idx, bus) in inputs.into_iter().take(MAX_AUX_BUSES).enumerate() {
            let mut ch_count = 0;
            for (ch_idx, slice) in bus.into_iter().take(MAX_CHANNELS).enumerate() {
                input_arr[bus_idx][ch_idx] = Some(slice);
                ch_count = ch_idx + 1;
            }
            input_channel_counts[bus_idx] = ch_count;
            if ch_count > 0 {
                num_input_buses = bus_idx + 1;
            }
        }

        // Initialize output buses - need from_fn because &mut is not Copy
        let mut output_arr: [[Option<&'a mut [S]>; MAX_CHANNELS]; MAX_AUX_BUSES] =
            std::array::from_fn(|_| std::array::from_fn(|_| None));
        let mut output_channel_counts = [0usize; MAX_AUX_BUSES];
        let mut num_output_buses = 0;

        for (bus_idx, bus) in outputs.into_iter().take(MAX_AUX_BUSES).enumerate() {
            let mut ch_count = 0;
            for (ch_idx, slice) in bus.into_iter().take(MAX_CHANNELS).enumerate() {
                output_arr[bus_idx][ch_idx] = Some(slice);
                ch_count = ch_idx + 1;
            }
            output_channel_counts[bus_idx] = ch_count;
            if ch_count > 0 {
                num_output_buses = bus_idx + 1;
            }
        }

        Self {
            inputs: input_arr,
            input_channel_counts,
            num_input_buses,
            outputs: output_arr,
            output_channel_counts,
            num_output_buses,
            num_samples,
        }
    }

    /// Create empty auxiliary buffers (no aux buses).
    #[inline]
    pub fn empty() -> Self {
        Self {
            inputs: [[None; MAX_CHANNELS]; MAX_AUX_BUSES],
            input_channel_counts: [0; MAX_AUX_BUSES],
            num_input_buses: 0,
            outputs: std::array::from_fn(|_| std::array::from_fn(|_| None)),
            output_channel_counts: [0; MAX_AUX_BUSES],
            num_output_buses: 0,
            num_samples: 0,
        }
    }

    // =========================================================================
    // Info
    // =========================================================================

    /// Number of samples in this processing block.
    #[inline]
    pub fn num_samples(&self) -> usize {
        self.num_samples
    }

    /// Number of auxiliary input buses.
    #[inline]
    pub fn num_input_buses(&self) -> usize {
        self.num_input_buses
    }

    /// Number of auxiliary output buses.
    #[inline]
    pub fn num_output_buses(&self) -> usize {
        self.num_output_buses
    }

    /// Returns true if there are no auxiliary buses.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.num_input_buses == 0 && self.num_output_buses == 0
    }

    // =========================================================================
    // Bus Access
    // =========================================================================

    /// Get the sidechain input bus (auxiliary input bus 0).
    ///
    /// This is the most common aux use case. Returns `None` if no
    /// sidechain is connected.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let key_level = aux.sidechain()
    ///     .map(|sc| sc.rms(0))
    ///     .unwrap_or(0.0);
    /// ```
    #[inline]
    pub fn sidechain(&self) -> Option<AuxInput<'_, S>> {
        self.input(0)
    }

    /// Get an auxiliary input bus by index.
    ///
    /// Returns `None` if the bus doesn't exist or has no channels.
    #[inline]
    pub fn input(&self, bus: usize) -> Option<AuxInput<'_, S>> {
        if bus >= MAX_AUX_BUSES {
            return None;
        }
        let num_channels = self.input_channel_counts[bus];
        if num_channels == 0 {
            return None;
        }
        Some(AuxInput {
            channels: &self.inputs[bus][..num_channels],
            num_samples: self.num_samples,
        })
    }

    /// Get a mutable auxiliary output bus by index.
    ///
    /// Returns `None` if the bus doesn't exist or has no channels.
    #[inline]
    pub fn output(&mut self, bus: usize) -> Option<AuxOutput<'_, 'a, S>> {
        if bus >= MAX_AUX_BUSES {
            return None;
        }
        let num_channels = self.output_channel_counts[bus];
        if num_channels == 0 {
            return None;
        }
        let num_samples = self.num_samples;
        Some(AuxOutput {
            channels: &mut self.outputs[bus][..num_channels],
            num_samples,
        })
    }

    // =========================================================================
    // Iterators
    // =========================================================================

    /// Iterate over all auxiliary input buses.
    #[inline]
    pub fn iter_inputs(&self) -> impl Iterator<Item = AuxInput<'_, S>> + '_ {
        let num_samples = self.num_samples;
        self.inputs[..self.num_input_buses]
            .iter()
            .zip(self.input_channel_counts[..self.num_input_buses].iter())
            .filter(|(_, &count)| count > 0)
            .map(move |(channels, &count)| AuxInput {
                channels: &channels[..count],
                num_samples,
            })
    }

    /// Iterate over all auxiliary output buses mutably.
    #[inline]
    pub fn iter_outputs(&mut self) -> impl Iterator<Item = AuxOutput<'_, 'a, S>> + '_ {
        let num_samples = self.num_samples;
        let num_buses = self.num_output_buses;
        self.outputs[..num_buses]
            .iter_mut()
            .zip(self.output_channel_counts[..num_buses].iter())
            .filter(|(_, &count)| count > 0)
            .map(move |(channels, &count)| AuxOutput {
                channels: &mut channels[..count],
                num_samples,
            })
    }
}

// =============================================================================
// AuxInput - Immutable Auxiliary Input Bus
// =============================================================================

/// Immutable view of an auxiliary input bus.
///
/// Provides access to input channels for a single auxiliary bus
/// (typically sidechain). Created via [`AuxiliaryBuffers::sidechain()`]
/// or [`AuxiliaryBuffers::input()`].
///
/// # Type Parameter
///
/// `S` is the sample type, defaulting to `f32`.
///
/// # Example
///
/// ```ignore
/// if let Some(sidechain) = aux.sidechain() {
///     // Calculate RMS of sidechain for keying
///     let rms = sidechain.rms(0);
///
///     // Or iterate over all channels
///     for ch in sidechain.iter_channels() {
///         // Process channel...
///     }
/// }
/// ```
pub struct AuxInput<'a, S: Sample = f32> {
    channels: &'a [Option<&'a [S]>],
    num_samples: usize,
}

impl<'a, S: Sample> AuxInput<'a, S> {
    /// Number of samples in each channel.
    #[inline]
    pub fn num_samples(&self) -> usize {
        self.num_samples
    }

    /// Number of channels in this bus.
    #[inline]
    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    /// Get a channel by index.
    ///
    /// Returns an empty slice if the channel doesn't exist.
    #[inline]
    pub fn channel(&self, index: usize) -> &[S] {
        self.channels
            .get(index)
            .and_then(|opt| opt.as_ref())
            .map(|ch| &ch[..self.num_samples])
            .unwrap_or(&[])
    }

    /// Iterate over all channel slices.
    #[inline]
    pub fn iter_channels(&self) -> impl Iterator<Item = &[S]> + '_ {
        let n = self.num_samples;
        self.channels
            .iter()
            .filter_map(move |opt| opt.as_ref().map(|ch| &ch[..n]))
    }

    // =========================================================================
    // Analysis Utilities
    // =========================================================================

    /// Calculate the RMS (root mean square) level of a channel.
    ///
    /// Returns zero if the channel doesn't exist or is empty.
    pub fn rms(&self, channel: usize) -> S {
        let ch = self.channel(channel);
        if ch.is_empty() {
            return S::ZERO;
        }
        let sum: S = ch.iter().fold(S::ZERO, |acc, &s| acc + s * s);
        let len = S::from_f32(ch.len() as f32);
        (sum / len).sqrt()
    }

    /// Calculate the peak level of a channel.
    ///
    /// Returns zero if the channel doesn't exist or is empty.
    pub fn peak(&self, channel: usize) -> S {
        self.channel(channel)
            .iter()
            .map(|&s| s.abs())
            .fold(S::ZERO, |a, b| a.max(b))
    }

    /// Calculate the average absolute level of a channel.
    ///
    /// Returns zero if the channel doesn't exist or is empty.
    pub fn average(&self, channel: usize) -> S {
        let ch = self.channel(channel);
        if ch.is_empty() {
            return S::ZERO;
        }
        let sum: S = ch.iter().map(|&s| s.abs()).fold(S::ZERO, |a, b| a + b);
        let len = S::from_f32(ch.len() as f32);
        sum / len
    }
}

// =============================================================================
// AuxOutput - Mutable Auxiliary Output Bus
// =============================================================================

/// Mutable view of an auxiliary output bus.
///
/// Provides access to output channels for a single auxiliary bus
/// (e.g., aux sends, multi-out). Created via [`AuxiliaryBuffers::output()`].
///
/// # Type Parameter
///
/// `S` is the sample type, defaulting to `f32`.
///
/// # Lifetime Parameters
///
/// - `'borrow` - The borrow lifetime (from `&mut self` on `AuxiliaryBuffers`)
/// - `'data` - The underlying audio data lifetime
///
/// This separation is required because `&'a mut [&'a mut T]` is invariant
/// in Rust, which prevents returning borrowed data from methods.
///
/// # Example
///
/// ```ignore
/// if let Some(mut aux_out) = aux.output(0) {
///     // Write to aux output
///     for sample in aux_out.channel(0) {
///         *sample = processed_signal;
///     }
/// }
/// ```
pub struct AuxOutput<'borrow, 'data, S: Sample = f32> {
    channels: &'borrow mut [Option<&'data mut [S]>],
    num_samples: usize,
}

impl<'borrow, 'data, S: Sample> AuxOutput<'borrow, 'data, S> {
    /// Number of samples in each channel.
    #[inline]
    pub fn num_samples(&self) -> usize {
        self.num_samples
    }

    /// Number of channels in this bus.
    #[inline]
    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    /// Get a mutable channel by index.
    ///
    /// # Panics
    ///
    /// Panics if the channel index is out of bounds.
    #[inline]
    pub fn channel(&mut self, index: usize) -> &mut [S] {
        let n = self.num_samples;
        self.channels[index]
            .as_mut()
            .map(|ch| &mut ch[..n])
            .expect("aux output channel out of bounds")
    }

    /// Try to get a mutable channel by index.
    ///
    /// Returns `None` if the channel doesn't exist.
    #[inline]
    pub fn channel_checked(&mut self, index: usize) -> Option<&mut [S]> {
        let n = self.num_samples;
        self.channels
            .get_mut(index)
            .and_then(|opt| opt.as_mut())
            .map(|ch| &mut ch[..n])
    }

    /// Iterate over all channel slices mutably.
    #[inline]
    pub fn iter_channels(&mut self) -> impl Iterator<Item = &mut [S]> + use<'_, 'data, S> {
        let n = self.num_samples;
        self.channels
            .iter_mut()
            .filter_map(move |opt| opt.as_mut().map(|ch| &mut ch[..n]))
    }

    /// Clear all channels to silence.
    pub fn clear(&mut self) {
        let n = self.num_samples;
        for opt in self.channels.iter_mut() {
            if let Some(ch) = opt.as_mut() {
                ch[..n].fill(S::ZERO);
            }
        }
    }

    /// Fill all channels with a constant value.
    pub fn fill(&mut self, value: S) {
        let n = self.num_samples;
        for opt in self.channels.iter_mut() {
            if let Some(ch) = opt.as_mut() {
                ch[..n].fill(value);
            }
        }
    }
}

