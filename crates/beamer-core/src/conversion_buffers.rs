//! Pre-allocated buffers for f64↔f32 sample format conversion.
//!
//! When a plugin's processor only supports f32 but the host provides f64 buffers,
//! format wrappers need to convert samples before/after processing. This module
//! provides `ConversionBuffers` which pre-allocates the necessary buffers during
//! setup to avoid heap allocations during audio processing.
//!
//! # Real-Time Safety
//!
//! - All buffers are allocated once during `allocate()` or `allocate_from_buses()`
//! - No heap allocations occur during audio processing
//! - Buffer access is O(1)
//!
//! # Usage
//!
//! ```ignore
//! // During setup (non-real-time):
//! let buffers = ConversionBuffers::allocate_from_buses(&input_buses, &output_buses, max_frames);
//!
//! // During process (real-time):
//! // 1. Convert f64 inputs to f32
//! for (ch, f64_slice) in input_f64.iter().enumerate() {
//!     if let Some(f32_buf) = buffers.main_input_mut(ch, num_samples) {
//!         for (i, &sample) in f64_slice.iter().enumerate() {
//!             f32_buf[i] = sample as f32;
//!         }
//!     }
//! }
//!
//! // 2. Process with f32 buffers
//! // 3. Convert f32 outputs back to f64
//! ```

use crate::BusInfo;

/// Pre-allocated buffers for f64↔f32 conversion.
///
/// Avoids heap allocation during audio processing when the processor
/// only supports f32 but the host provides f64 buffers.
///
/// Fields are public for direct iteration access in format wrappers.
pub struct ConversionBuffers {
    /// Main input bus conversion buffers: [channel][samples]
    pub main_input_f32: Vec<Vec<f32>>,
    /// Main output bus conversion buffers: [channel][samples]
    pub main_output_f32: Vec<Vec<f32>>,
    /// Auxiliary input buses: [bus_index][channel_index][samples]
    pub aux_input_f32: Vec<Vec<Vec<f32>>>,
    /// Auxiliary output buses: [bus_index][channel_index][samples]
    pub aux_output_f32: Vec<Vec<Vec<f32>>>,
}

impl ConversionBuffers {
    /// Create empty conversion buffers (no capacity reserved).
    ///
    /// Use this when you don't know the configuration yet.
    /// Call `allocate()` or `allocate_from_buses()` later.
    pub fn new() -> Self {
        Self {
            main_input_f32: Vec::new(),
            main_output_f32: Vec::new(),
            aux_input_f32: Vec::new(),
            aux_output_f32: Vec::new(),
        }
    }

    /// Pre-allocate buffers from bus information.
    ///
    /// This is the preferred allocation method when you have access to `BusInfo`.
    /// Extracts channel counts from main bus (index 0) and auxiliary buses (index 1+).
    ///
    /// # Arguments
    ///
    /// * `input_buses` - Slice of input bus information
    /// * `output_buses` - Slice of output bus information
    /// * `max_frames` - Maximum number of samples per buffer
    pub fn allocate_from_buses(
        input_buses: &[BusInfo],
        output_buses: &[BusInfo],
        max_frames: usize,
    ) -> Self {
        // Main bus (bus 0) channels
        let main_in_channels = input_buses
            .first()
            .map(|b| b.channel_count as usize)
            .unwrap_or(0);
        let main_out_channels = output_buses
            .first()
            .map(|b| b.channel_count as usize)
            .unwrap_or(0);

        let main_input_f32: Vec<Vec<f32>> = (0..main_in_channels)
            .map(|_| vec![0.0f32; max_frames])
            .collect();

        let main_output_f32: Vec<Vec<f32>> = (0..main_out_channels)
            .map(|_| vec![0.0f32; max_frames])
            .collect();

        // Auxiliary buses (bus 1+)
        let aux_input_f32: Vec<Vec<Vec<f32>>> = input_buses
            .iter()
            .skip(1)
            .map(|info| {
                (0..info.channel_count)
                    .map(|_| vec![0.0f32; max_frames])
                    .collect()
            })
            .collect();

        let aux_output_f32: Vec<Vec<Vec<f32>>> = output_buses
            .iter()
            .skip(1)
            .map(|info| {
                (0..info.channel_count)
                    .map(|_| vec![0.0f32; max_frames])
                    .collect()
            })
            .collect();

        Self {
            main_input_f32,
            main_output_f32,
            aux_input_f32,
            aux_output_f32,
        }
    }

    /// Pre-allocate buffers with explicit channel counts.
    ///
    /// Use this when you have channel counts directly rather than `BusInfo`.
    ///
    /// # Arguments
    ///
    /// * `main_input_channels` - Number of main input channels
    /// * `main_output_channels` - Number of main output channels
    /// * `aux_input_channels` - Channel count for each auxiliary input bus
    /// * `aux_output_channels` - Channel count for each auxiliary output bus
    /// * `max_frames` - Maximum number of samples per buffer
    pub fn allocate(
        main_input_channels: usize,
        main_output_channels: usize,
        aux_input_channels: &[usize],
        aux_output_channels: &[usize],
        max_frames: usize,
    ) -> Self {
        let main_input_f32 = (0..main_input_channels)
            .map(|_| vec![0.0f32; max_frames])
            .collect();

        let main_output_f32 = (0..main_output_channels)
            .map(|_| vec![0.0f32; max_frames])
            .collect();

        let aux_input_f32 = aux_input_channels
            .iter()
            .map(|&channels| (0..channels).map(|_| vec![0.0f32; max_frames]).collect())
            .collect();

        let aux_output_f32 = aux_output_channels
            .iter()
            .map(|&channels| (0..channels).map(|_| vec![0.0f32; max_frames]).collect())
            .collect();

        Self {
            main_input_f32,
            main_output_f32,
            aux_input_f32,
            aux_output_f32,
        }
    }

    // =========================================================================
    // Main bus accessors
    // =========================================================================

    /// Get mutable reference to a main input channel buffer.
    ///
    /// Returns `None` if the channel index is out of bounds.
    #[inline]
    pub fn main_input_mut(&mut self, channel: usize) -> Option<&mut [f32]> {
        self.main_input_f32.get_mut(channel).map(|v| v.as_mut_slice())
    }

    /// Get reference to a main input channel buffer.
    #[inline]
    pub fn main_input(&self, channel: usize) -> Option<&[f32]> {
        self.main_input_f32.get(channel).map(|v| v.as_slice())
    }

    /// Get mutable reference to a main output channel buffer.
    #[inline]
    pub fn main_output_mut(&mut self, channel: usize) -> Option<&mut [f32]> {
        self.main_output_f32.get_mut(channel).map(|v| v.as_mut_slice())
    }

    /// Get reference to a main output channel buffer.
    #[inline]
    pub fn main_output(&self, channel: usize) -> Option<&[f32]> {
        self.main_output_f32.get(channel).map(|v| v.as_slice())
    }

    /// Get number of main input channels.
    #[inline]
    pub fn main_input_channel_count(&self) -> usize {
        self.main_input_f32.len()
    }

    /// Get number of main output channels.
    #[inline]
    pub fn main_output_channel_count(&self) -> usize {
        self.main_output_f32.len()
    }

    // =========================================================================
    // Auxiliary bus accessors
    // =========================================================================

    /// Get mutable slice of an auxiliary input channel buffer.
    ///
    /// # Arguments
    ///
    /// * `bus` - Auxiliary bus index (0 = first aux bus, not main bus)
    /// * `channel` - Channel index within the bus
    /// * `len` - Number of samples to access
    #[inline]
    pub fn aux_input_mut(&mut self, bus: usize, channel: usize, len: usize) -> Option<&mut [f32]> {
        self.aux_input_f32
            .get_mut(bus)
            .and_then(|b| b.get_mut(channel))
            .map(|v| {
                let actual_len = len.min(v.len());
                &mut v[..actual_len]
            })
    }

    /// Get slice of an auxiliary input channel buffer.
    #[inline]
    pub fn aux_input(&self, bus: usize, channel: usize, len: usize) -> Option<&[f32]> {
        self.aux_input_f32
            .get(bus)
            .and_then(|b| b.get(channel))
            .map(|v| &v[..len.min(v.len())])
    }

    /// Get mutable slice of an auxiliary output channel buffer.
    #[inline]
    pub fn aux_output_mut(&mut self, bus: usize, channel: usize, len: usize) -> Option<&mut [f32]> {
        self.aux_output_f32
            .get_mut(bus)
            .and_then(|b| b.get_mut(channel))
            .map(|v| {
                let actual_len = len.min(v.len());
                &mut v[..actual_len]
            })
    }

    /// Get slice of an auxiliary output channel buffer.
    #[inline]
    pub fn aux_output(&self, bus: usize, channel: usize, len: usize) -> Option<&[f32]> {
        self.aux_output_f32
            .get(bus)
            .and_then(|b| b.get(channel))
            .map(|v| &v[..len.min(v.len())])
    }

    /// Get number of auxiliary input buses.
    #[inline]
    pub fn aux_input_bus_count(&self) -> usize {
        self.aux_input_f32.len()
    }

    /// Get number of auxiliary output buses.
    #[inline]
    pub fn aux_output_bus_count(&self) -> usize {
        self.aux_output_f32.len()
    }

    /// Get number of channels in an auxiliary input bus.
    #[inline]
    pub fn aux_input_channel_count(&self, bus: usize) -> usize {
        self.aux_input_f32.get(bus).map(|b| b.len()).unwrap_or(0)
    }

    /// Get number of channels in an auxiliary output bus.
    #[inline]
    pub fn aux_output_channel_count(&self, bus: usize) -> usize {
        self.aux_output_f32.get(bus).map(|b| b.len()).unwrap_or(0)
    }
}

impl Default for ConversionBuffers {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_empty() {
        let buffers = ConversionBuffers::new();
        assert_eq!(buffers.main_input_channel_count(), 0);
        assert_eq!(buffers.main_output_channel_count(), 0);
        assert_eq!(buffers.aux_input_bus_count(), 0);
        assert_eq!(buffers.aux_output_bus_count(), 0);
    }

    #[test]
    fn test_allocate_stereo() {
        let buffers = ConversionBuffers::allocate(2, 2, &[], &[], 512);

        assert_eq!(buffers.main_input_channel_count(), 2);
        assert_eq!(buffers.main_output_channel_count(), 2);
        assert_eq!(buffers.aux_input_bus_count(), 0);
        assert_eq!(buffers.aux_output_bus_count(), 0);

        // Check buffer sizes
        assert_eq!(buffers.main_input_f32[0].len(), 512);
        assert_eq!(buffers.main_output_f32[1].len(), 512);
    }

    #[test]
    fn test_allocate_with_aux() {
        let buffers = ConversionBuffers::allocate(2, 2, &[2, 1], &[2], 256);

        assert_eq!(buffers.main_input_channel_count(), 2);
        assert_eq!(buffers.main_output_channel_count(), 2);
        assert_eq!(buffers.aux_input_bus_count(), 2);
        assert_eq!(buffers.aux_output_bus_count(), 1);

        assert_eq!(buffers.aux_input_channel_count(0), 2);
        assert_eq!(buffers.aux_input_channel_count(1), 1);
        assert_eq!(buffers.aux_output_channel_count(0), 2);
    }

    #[test]
    fn test_allocate_from_buses() {
        let input_buses = vec![
            BusInfo::stereo("Main In"),
            BusInfo::aux("Sidechain", 2),
        ];
        let output_buses = vec![BusInfo::stereo("Main Out")];

        let buffers = ConversionBuffers::allocate_from_buses(&input_buses, &output_buses, 1024);

        assert_eq!(buffers.main_input_channel_count(), 2);
        assert_eq!(buffers.main_output_channel_count(), 2);
        assert_eq!(buffers.aux_input_bus_count(), 1);
        assert_eq!(buffers.aux_input_channel_count(0), 2);
        assert_eq!(buffers.aux_output_bus_count(), 0);
    }

    #[test]
    fn test_accessors() {
        let mut buffers = ConversionBuffers::allocate(2, 2, &[2], &[], 128);

        // Main input
        if let Some(buf) = buffers.main_input_mut(0) {
            buf[0] = 0.5;
        }
        assert_eq!(buffers.main_input(0).unwrap()[0], 0.5);

        // Main output
        if let Some(buf) = buffers.main_output_mut(1) {
            buf[10] = -0.5;
        }
        assert_eq!(buffers.main_output(1).unwrap()[10], -0.5);

        // Aux input
        if let Some(buf) = buffers.aux_input_mut(0, 0, 64) {
            buf[0] = 0.25;
        }
        assert_eq!(buffers.aux_input(0, 0, 64).unwrap()[0], 0.25);

        // Out of bounds returns None
        assert!(buffers.main_input(5).is_none());
        assert!(buffers.aux_input(5, 0, 64).is_none());
    }
}
