//! Audio buffer abstractions for plugin processing.
//!
//! This module provides [`Buffer`] for main audio I/O and [`AuxiliaryBuffers`]
//! for sidechain and auxiliary bus access.
//!
//! # Architecture
//!
//! Audio processing in BEAMR uses two separate buffer types:
//!
//! - **[`Buffer`]**: Main stereo/surround I/O - used by all plugins
//! - **[`AuxiliaryBuffers`]**: Sidechain and aux buses - used by multi-bus plugins
//!
//! This separation solves Rust's lifetime variance constraints with nested mutable
//! references while providing a clean, ergonomic API.
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

// =============================================================================
// Buffer - Main Audio I/O
// =============================================================================

/// Main audio buffer for plugin processing.
///
/// Contains input and output channel slices for the primary audio bus.
/// This is what most plugins interact with - simple stereo or surround I/O.
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
pub struct Buffer<'a> {
    /// Input channel slices (immutable audio from host)
    inputs: Vec<&'a [f32]>,
    /// Output channel slices (mutable audio to host)
    outputs: Vec<&'a mut [f32]>,
    /// Number of samples in this processing block
    num_samples: usize,
}

impl<'a> Buffer<'a> {
    /// Create a new buffer from channel slices.
    ///
    /// This is called by the VST3 wrapper, not by plugin code.
    #[inline]
    pub fn new(
        inputs: Vec<&'a [f32]>,
        outputs: Vec<&'a mut [f32]>,
        num_samples: usize,
    ) -> Self {
        Self { inputs, outputs, num_samples }
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
        self.inputs.len()
    }

    /// Number of output channels.
    #[inline]
    pub fn num_output_channels(&self) -> usize {
        self.outputs.len()
    }

    /// Returns true if this is a stereo buffer (2 in, 2 out).
    #[inline]
    pub fn is_stereo(&self) -> bool {
        self.inputs.len() == 2 && self.outputs.len() == 2
    }

    /// Returns true if this is a mono buffer (1 in, 1 out).
    #[inline]
    pub fn is_mono(&self) -> bool {
        self.inputs.len() == 1 && self.outputs.len() == 1
    }

    // =========================================================================
    // Channel Access
    // =========================================================================

    /// Get an input channel by index.
    ///
    /// Returns an empty slice if the channel doesn't exist.
    #[inline]
    pub fn input(&self, channel: usize) -> &[f32] {
        self.inputs
            .get(channel)
            .map(|ch| &ch[..self.num_samples])
            .unwrap_or(&[])
    }

    /// Get a mutable output channel by index.
    ///
    /// # Panics
    ///
    /// Panics if the channel index is out of bounds.
    #[inline]
    pub fn output(&mut self, channel: usize) -> &mut [f32] {
        &mut self.outputs[channel][..self.num_samples]
    }

    /// Try to get a mutable output channel by index.
    ///
    /// Returns `None` if the channel doesn't exist.
    #[inline]
    pub fn output_checked(&mut self, channel: usize) -> Option<&mut [f32]> {
        self.outputs
            .get_mut(channel)
            .map(|ch| &mut ch[..self.num_samples])
    }

    // =========================================================================
    // Iterators
    // =========================================================================

    /// Iterate over all input channels.
    #[inline]
    pub fn inputs(&self) -> impl Iterator<Item = &[f32]> + '_ {
        let n = self.num_samples;
        self.inputs.iter().map(move |ch| &ch[..n])
    }

    /// Iterate over all output channels mutably.
    #[inline]
    pub fn outputs_mut(&mut self) -> impl Iterator<Item = &mut [f32]> + use<'_, 'a> {
        let n = self.num_samples;
        self.outputs.iter_mut().map(move |ch| &mut ch[..n])
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
    pub fn zip_channels(&mut self) -> impl Iterator<Item = (&[f32], &mut [f32])> + use<'_, 'a> {
        let n = self.num_samples;
        let num_pairs = self.inputs.len().min(self.outputs.len());
        self.inputs[..num_pairs]
            .iter()
            .zip(self.outputs[..num_pairs].iter_mut())
            .map(move |(i, o)| (&i[..n], &mut o[..n]))
    }

    // =========================================================================
    // Bulk Operations
    // =========================================================================

    /// Copy all input channels to output channels.
    ///
    /// Useful for bypass or passthrough. Only copies channels that exist
    /// in both input and output.
    pub fn copy_to_output(&mut self) {
        let num_channels = self.inputs.len().min(self.outputs.len());
        for ch in 0..num_channels {
            let input = &self.inputs[ch][..self.num_samples];
            let output = &mut self.outputs[ch][..self.num_samples];
            output.copy_from_slice(input);
        }
    }

    /// Clear all output channels to silence.
    pub fn clear_outputs(&mut self) {
        for output in self.outputs.iter_mut() {
            output[..self.num_samples].fill(0.0);
        }
    }

    /// Apply a gain factor to all output channels.
    pub fn apply_output_gain(&mut self, gain: f32) {
        for output in self.outputs.iter_mut() {
            for sample in &mut output[..self.num_samples] {
                *sample *= gain;
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
/// # Bus Indexing
///
/// Auxiliary buses are indexed starting from 0:
/// - Bus 0: Sidechain (most common aux use case)
/// - Bus 1+: Additional auxiliary I/O
///
/// # Example: Sidechain Access
///
/// ```ignore
/// if let Some(sidechain) = aux.sidechain() {
///     let key_signal = sidechain.channel(0);
///     // Use for compression keying, ducking, etc.
/// }
/// ```
pub struct AuxiliaryBuffers<'a> {
    /// Auxiliary input buses (e.g., sidechain inputs)
    inputs: Vec<Vec<&'a [f32]>>,
    /// Auxiliary output buses (e.g., aux sends)
    outputs: Vec<Vec<&'a mut [f32]>>,
    /// Number of samples in this processing block
    num_samples: usize,
}

impl<'a> AuxiliaryBuffers<'a> {
    /// Create new auxiliary buffers.
    ///
    /// This is called by the VST3 wrapper, not by plugin code.
    #[inline]
    pub fn new(
        inputs: Vec<Vec<&'a [f32]>>,
        outputs: Vec<Vec<&'a mut [f32]>>,
        num_samples: usize,
    ) -> Self {
        Self { inputs, outputs, num_samples }
    }

    /// Create empty auxiliary buffers (no aux buses).
    #[inline]
    pub fn empty() -> Self {
        Self {
            inputs: Vec::new(),
            outputs: Vec::new(),
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
        self.inputs.len()
    }

    /// Number of auxiliary output buses.
    #[inline]
    pub fn num_output_buses(&self) -> usize {
        self.outputs.len()
    }

    /// Returns true if there are no auxiliary buses.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty() && self.outputs.is_empty()
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
    pub fn sidechain(&self) -> Option<AuxInput<'_>> {
        self.input(0)
    }

    /// Get an auxiliary input bus by index.
    ///
    /// Returns `None` if the bus doesn't exist or has no channels.
    #[inline]
    pub fn input(&self, bus: usize) -> Option<AuxInput<'_>> {
        self.inputs.get(bus).and_then(|channels| {
            if channels.is_empty() {
                None
            } else {
                Some(AuxInput {
                    channels,
                    num_samples: self.num_samples
                })
            }
        })
    }

    /// Get a mutable auxiliary output bus by index.
    ///
    /// Returns `None` if the bus doesn't exist or has no channels.
    #[inline]
    pub fn output(&mut self, bus: usize) -> Option<AuxOutput<'_, 'a>> {
        let num_samples = self.num_samples;
        self.outputs.get_mut(bus).and_then(move |channels| {
            if channels.is_empty() {
                None
            } else {
                Some(AuxOutput {
                    channels: channels.as_mut_slice(),
                    num_samples,
                })
            }
        })
    }

    // =========================================================================
    // Iterators
    // =========================================================================

    /// Iterate over all auxiliary input buses.
    #[inline]
    pub fn iter_inputs(&self) -> impl Iterator<Item = AuxInput<'_>> + '_ {
        let num_samples = self.num_samples;
        self.inputs
            .iter()
            .filter(|ch| !ch.is_empty())
            .map(move |channels| AuxInput { channels, num_samples })
    }

    /// Iterate over all auxiliary output buses mutably.
    #[inline]
    pub fn iter_outputs(&mut self) -> impl Iterator<Item = AuxOutput<'_, 'a>> + '_ {
        let num_samples = self.num_samples;
        self.outputs
            .iter_mut()
            .filter(|ch| !ch.is_empty())
            .map(move |channels| AuxOutput { channels, num_samples })
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
pub struct AuxInput<'a> {
    channels: &'a [&'a [f32]],
    num_samples: usize,
}

impl<'a> AuxInput<'a> {
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
    pub fn channel(&self, index: usize) -> &[f32] {
        self.channels
            .get(index)
            .map(|ch| &ch[..self.num_samples])
            .unwrap_or(&[])
    }

    /// Iterate over all channel slices.
    #[inline]
    pub fn iter_channels(&self) -> impl Iterator<Item = &[f32]> + '_ {
        let n = self.num_samples;
        self.channels.iter().map(move |ch| &ch[..n])
    }

    // =========================================================================
    // Analysis Utilities
    // =========================================================================

    /// Calculate the RMS (root mean square) level of a channel.
    ///
    /// Returns 0.0 if the channel doesn't exist or is empty.
    pub fn rms(&self, channel: usize) -> f32 {
        let ch = self.channel(channel);
        if ch.is_empty() {
            return 0.0;
        }
        let sum: f32 = ch.iter().map(|&s| s * s).sum();
        (sum / ch.len() as f32).sqrt()
    }

    /// Calculate the peak level of a channel.
    ///
    /// Returns 0.0 if the channel doesn't exist or is empty.
    pub fn peak(&self, channel: usize) -> f32 {
        self.channel(channel)
            .iter()
            .map(|s| s.abs())
            .fold(0.0f32, |a, b| a.max(b))
    }

    /// Calculate the average absolute level of a channel.
    ///
    /// Returns 0.0 if the channel doesn't exist or is empty.
    pub fn average(&self, channel: usize) -> f32 {
        let ch = self.channel(channel);
        if ch.is_empty() {
            return 0.0;
        }
        ch.iter().map(|s| s.abs()).sum::<f32>() / ch.len() as f32
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
pub struct AuxOutput<'borrow, 'data> {
    channels: &'borrow mut [&'data mut [f32]],
    num_samples: usize,
}

impl<'borrow, 'data> AuxOutput<'borrow, 'data> {
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
    pub fn channel(&mut self, index: usize) -> &mut [f32] {
        &mut self.channels[index][..self.num_samples]
    }

    /// Try to get a mutable channel by index.
    ///
    /// Returns `None` if the channel doesn't exist.
    #[inline]
    pub fn channel_checked(&mut self, index: usize) -> Option<&mut [f32]> {
        self.channels
            .get_mut(index)
            .map(|ch| &mut ch[..self.num_samples])
    }

    /// Iterate over all channel slices mutably.
    #[inline]
    pub fn iter_channels(&mut self) -> impl Iterator<Item = &mut [f32]> + use<'_, 'data> {
        let n = self.num_samples;
        self.channels.iter_mut().map(move |ch| &mut ch[..n])
    }

    /// Clear all channels to silence.
    pub fn clear(&mut self) {
        for ch in self.channels.iter_mut() {
            ch[..self.num_samples].fill(0.0);
        }
    }

    /// Fill all channels with a constant value.
    pub fn fill(&mut self, value: f32) {
        for ch in self.channels.iter_mut() {
            ch[..self.num_samples].fill(value);
        }
    }
}

// =============================================================================
// Deprecated Compatibility Types
// =============================================================================

/// DEPRECATED: Use [`Buffer`] and [`AuxiliaryBuffers`] instead.
///
/// This type alias exists for migration purposes only.
#[deprecated(since = "0.2.0", note = "Use Buffer and AuxiliaryBuffers instead")]
pub type AudioBuffer<'a> = Buffer<'a>;

/// DEPRECATED: Use [`AuxInput`] or [`AuxOutput`] instead.
///
/// This type alias exists for migration purposes only.
#[deprecated(since = "0.2.0", note = "Use AuxInput or AuxOutput instead")]
pub type Bus<'a> = AuxInput<'a>;
