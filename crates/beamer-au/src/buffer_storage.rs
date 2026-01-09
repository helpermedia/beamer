//! Pre-allocated buffer storage for real-time safe audio processing.
//!
//! This module provides `ProcessBufferStorage`, which pre-allocates capacity
//! for channel pointers during `allocateRenderResources`. The storage is then
//! reused for each render call without allocations.
//!
//! # Pattern
//!
//! This follows the same pattern as `beamer-vst3`:
//! 1. Allocate storage once during setup (non-real-time)
//! 2. Clear storage at start of each render (O(1), no deallocation)
//! 3. Push pointers from AudioBufferList (never exceeds capacity)
//! 4. Build slices from pointers
//!
//! # Memory Optimization Strategy
//!
//! The allocation strategy is **config-based, not worst-case**:
//!
//! - **Channel counts**: Allocates exact number of channels from bus config, not MAX_CHANNELS
//! - **Bus counts**: Allocates only for buses that exist, not MAX_BUSES
//! - **Lazy aux allocation**: No heap allocation for aux buses if plugin doesn't use them
//! - **Asymmetric support**: Mono input can have stereo output (allocates 1 + 2, not 2 + 2)
//!
//! Examples:
//! - Mono plugin (1in/1out): Allocates 2 pointers (16 bytes on 64-bit)
//! - Stereo plugin (2in/2out): Allocates 4 pointers (32 bytes on 64-bit)
//! - Stereo with sidechain (2+2in/2out): Allocates 6 pointers (48 bytes on 64-bit)
//! - Worst-case (32ch x 16 buses): Would be MAX_CHANNELS * MAX_BUSES = 512 pointers (4KB)
//!
//! This means simple plugins use **32x less memory** than worst-case allocation.
//!
//! # Real-Time Safety
//!
//! - `clear()` is O(1) - only sets Vec lengths to 0
//! - `push()` never allocates - capacity is pre-reserved
//! - No heap operations during audio processing
//! - All allocations happen in `allocate_from_config()` (non-real-time)

use crate::buffers::AudioBufferList;
use crate::bus_config::{CachedBusConfig, MAX_BUSES, MAX_CHANNELS};
use beamer_core::Sample;
use std::slice;

// =============================================================================
// Bus Limit Validation
// =============================================================================

/// Validate that bus configuration doesn't exceed system limits.
///
/// This function checks that:
/// - Bus counts are within MAX_BUSES (from beamer_core)
/// - Channel counts per bus are within MAX_CHANNELS (from beamer_core)
///
/// # Arguments
///
/// * `bus_config` - The cached bus configuration to validate
///
/// # Returns
///
/// Ok(()) if valid, Err with detailed message if limits are exceeded.
///
/// # Example
///
/// ```ignore
/// let config = extract_bus_config_from_au(au)?;
/// validate_bus_limits_from_config(&config)?;
/// let storage = ProcessBufferStorage::allocate_from_config(&config);
/// ```
pub fn validate_bus_limits_from_config(bus_config: &CachedBusConfig) -> Result<(), String> {
    // Validate input bus count
    if bus_config.input_bus_count > MAX_BUSES {
        return Err(format!(
            "Plugin declares {} input buses, but MAX_BUSES is {}",
            bus_config.input_bus_count, MAX_BUSES
        ));
    }

    // Validate output bus count
    if bus_config.output_bus_count > MAX_BUSES {
        return Err(format!(
            "Plugin declares {} output buses, but MAX_BUSES is {}",
            bus_config.output_bus_count, MAX_BUSES
        ));
    }

    // Validate channel counts for each input bus
    for (i, bus) in bus_config.input_buses.iter().enumerate() {
        if bus.channel_count > MAX_CHANNELS {
            return Err(format!(
                "Input bus {} declares {} channels, but MAX_CHANNELS is {}",
                i, bus.channel_count, MAX_CHANNELS
            ));
        }
    }

    // Validate channel counts for each output bus
    for (i, bus) in bus_config.output_buses.iter().enumerate() {
        if bus.channel_count > MAX_CHANNELS {
            return Err(format!(
                "Output bus {} declares {} channels, but MAX_CHANNELS is {}",
                i, bus.channel_count, MAX_CHANNELS
            ));
        }
    }

    Ok(())
}

// =============================================================================
// ProcessBufferStorage
// =============================================================================

/// Pre-allocated storage for AU audio processing.
///
/// Stores channel pointers collected from `AudioBufferList` during render.
/// The Vecs have pre-allocated capacity matching the **actual** bus configuration,
/// ensuring no allocations occur during audio callbacks while minimizing memory usage.
///
/// # Memory Layout
///
/// The storage is optimized based on the actual plugin configuration:
/// - `main_inputs`: Capacity = actual input channel count (e.g., 1 for mono, 2 for stereo)
/// - `main_outputs`: Capacity = actual output channel count (e.g., 1 for mono, 2 for stereo)
/// - `aux_inputs`: Only allocated if plugin declares aux input buses
/// - `aux_outputs`: Only allocated if plugin declares aux output buses
///
/// This means a simple stereo plugin uses only 32 bytes (4 pointers × 8 bytes),
/// not the worst-case 4KB (MAX_CHANNELS × MAX_BUSES × pointer size).
///
/// # Type Parameter
///
/// `S` is the sample type (`f32` or `f64`).
#[derive(Clone)]
pub struct ProcessBufferStorage<S: Sample> {
    /// Main input channel pointers (capacity = actual channel count)
    pub main_inputs: Vec<*const S>,
    /// Main output channel pointers (capacity = actual channel count)
    pub main_outputs: Vec<*mut S>,
    /// Auxiliary input buses (only allocated if plugin uses them)
    pub aux_inputs: Vec<Vec<*const S>>,
    /// Auxiliary output buses (only allocated if plugin uses them)
    pub aux_outputs: Vec<Vec<*mut S>>,
}

impl<S: Sample> ProcessBufferStorage<S> {
    /// Create storage from cached bus configuration (recommended).
    ///
    /// This is the preferred way to allocate storage as it automatically
    /// extracts the correct channel counts from the bus configuration.
    /// Should be called during `allocateRenderResources` (non-real-time).
    ///
    /// # Memory Optimization
    ///
    /// This method implements smart allocation strategies:
    /// - Allocates only for channels actually present in the config
    /// - No pre-allocation for aux buses if plugin doesn't use them
    /// - Uses actual channel counts, not MAX_CHANNELS worst-case
    /// - Zero heap allocation for simple mono/stereo plugins without aux buses
    ///
    /// # Arguments
    ///
    /// * `bus_config` - Cached bus configuration from AU
    ///
    /// # Example
    ///
    /// ```ignore
    /// let config = extract_bus_config_from_au(au)?;
    /// validate_bus_limits_from_config(&config)?;
    /// let storage = ProcessBufferStorage::allocate_from_config(&config);
    /// ```
    pub fn allocate_from_config(bus_config: &CachedBusConfig) -> Self {
        // Extract main bus channel counts (bus 0)
        let main_in_channels = bus_config
            .input_bus_info(0)
            .map(|b| b.channel_count)
            .unwrap_or(0);
        let main_out_channels = bus_config
            .output_bus_info(0)
            .map(|b| b.channel_count)
            .unwrap_or(0);

        // Count auxiliary buses (all buses except main bus 0)
        let aux_in_buses = bus_config.input_bus_count.saturating_sub(1);
        let aux_out_buses = bus_config.output_bus_count.saturating_sub(1);

        // Optimization: Only allocate aux bus storage if actually needed.
        // For simple plugins (mono/stereo with no aux), this avoids any
        // heap allocation for the outer Vec containers.
        let aux_inputs = if aux_in_buses > 0 {
            let mut vec = Vec::with_capacity(aux_in_buses);
            for i in 1..=aux_in_buses {
                let channels = bus_config
                    .input_bus_info(i)
                    .map(|b| b.channel_count)
                    .unwrap_or(0);
                vec.push(Vec::with_capacity(channels));
            }
            vec
        } else {
            Vec::new() // Zero-capacity allocation - no heap memory
        };

        let aux_outputs = if aux_out_buses > 0 {
            let mut vec = Vec::with_capacity(aux_out_buses);
            for i in 1..=aux_out_buses {
                let channels = bus_config
                    .output_bus_info(i)
                    .map(|b| b.channel_count)
                    .unwrap_or(0);
                vec.push(Vec::with_capacity(channels));
            }
            vec
        } else {
            Vec::new() // Zero-capacity allocation - no heap memory
        };

        Self {
            main_inputs: Vec::with_capacity(main_in_channels),
            main_outputs: Vec::with_capacity(main_out_channels),
            aux_inputs,
            aux_outputs,
        }
    }

    /// Create new storage with pre-allocated capacity (manual).
    ///
    /// This is a lower-level method for manual capacity specification.
    /// Prefer `allocate_from_config()` when possible as it's less error-prone.
    ///
    /// # Arguments
    ///
    /// * `main_in_channels` - Number of main input channels
    /// * `main_out_channels` - Number of main output channels
    /// * `aux_in_buses` - Number of auxiliary input buses
    /// * `aux_out_buses` - Number of auxiliary output buses
    /// * `aux_channels` - Channels per aux bus (assumes uniform)
    pub fn allocate(
        main_in_channels: usize,
        main_out_channels: usize,
        aux_in_buses: usize,
        aux_out_buses: usize,
        aux_channels: usize,
    ) -> Self {
        let mut aux_inputs = Vec::with_capacity(aux_in_buses);
        for _ in 0..aux_in_buses {
            aux_inputs.push(Vec::with_capacity(aux_channels));
        }

        let mut aux_outputs = Vec::with_capacity(aux_out_buses);
        for _ in 0..aux_out_buses {
            aux_outputs.push(Vec::with_capacity(aux_channels));
        }

        Self {
            main_inputs: Vec::with_capacity(main_in_channels),
            main_outputs: Vec::with_capacity(main_out_channels),
            aux_inputs,
            aux_outputs,
        }
    }

    /// Clear all pointer storage without deallocating.
    ///
    /// This is O(1) - it only sets Vec lengths to 0 while preserving capacity.
    /// Call this at the start of each render call.
    #[inline]
    pub fn clear(&mut self) {
        self.main_inputs.clear();
        self.main_outputs.clear();
        for bus in &mut self.aux_inputs {
            bus.clear();
        }
        for bus in &mut self.aux_outputs {
            bus.clear();
        }
    }

    /// Collect input pointers from an AudioBufferList.
    ///
    /// # Safety
    ///
    /// - `buffer_list` must be a valid pointer
    /// - Pointers are only valid for the current render call
    /// - num_samples must not exceed actual buffer sizes
    #[inline]
    pub unsafe fn collect_inputs(
        &mut self,
        buffer_list: *const AudioBufferList,
        num_samples: usize,
    ) {
        if buffer_list.is_null() {
            return;
        }

        let list = &*buffer_list;
        let max_channels = self.main_inputs.capacity();

        for i in 0..list.number_buffers.min(max_channels as u32) {
            let buffer = list.buffer_at(i);
            if !buffer.data.is_null() && buffer.number_channels == 1 {
                // Non-interleaved: one channel per buffer
                let data_ptr = buffer.data as *const S;
                // Validate we have enough data
                let available_samples = buffer.data_byte_size as usize / std::mem::size_of::<S>();
                if available_samples >= num_samples {
                    self.main_inputs.push(data_ptr);
                }
            }
            // Skip interleaved buffers (number_channels > 1) - handled separately
        }
    }

    /// Collect output pointers from an AudioBufferList.
    ///
    /// # Safety
    ///
    /// - `buffer_list` must be a valid pointer
    /// - Pointers are only valid for the current render call
    /// - num_samples must not exceed actual buffer sizes
    #[inline]
    pub unsafe fn collect_outputs(
        &mut self,
        buffer_list: *mut AudioBufferList,
        num_samples: usize,
    ) {
        if buffer_list.is_null() {
            return;
        }

        let list = &mut *buffer_list;
        let max_channels = self.main_outputs.capacity();

        for i in 0..list.number_buffers.min(max_channels as u32) {
            let buffer = list.buffer_at_mut(i);
            if !buffer.data.is_null() && buffer.number_channels == 1 {
                // Non-interleaved: one channel per buffer
                let data_ptr = buffer.data as *mut S;
                // Validate we have enough data
                let available_samples = buffer.data_byte_size as usize / std::mem::size_of::<S>();
                if available_samples >= num_samples {
                    self.main_outputs.push(data_ptr);
                }
            }
            // Skip interleaved buffers (number_channels > 1) - handled separately
        }
    }

    /// Build input slices from collected pointers.
    ///
    /// # Safety
    ///
    /// - Pointers must still be valid (within same render call)
    /// - num_samples must match what was used in collect_*
    #[inline]
    pub unsafe fn input_slices(&self, num_samples: usize) -> Vec<&[S]> {
        self.main_inputs
            .iter()
            .map(|&ptr| slice::from_raw_parts(ptr, num_samples))
            .collect()
    }

    /// Build output slices from collected pointers.
    ///
    /// # Safety
    ///
    /// - Pointers must still be valid (within same render call)
    /// - num_samples must match what was used in collect_*
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn output_slices(&self, num_samples: usize) -> Vec<&mut [S]> {
        self.main_outputs
            .iter()
            .map(|&ptr| slice::from_raw_parts_mut(ptr, num_samples))
            .collect()
    }

    /// Get the number of input channels collected.
    #[inline]
    pub fn input_channel_count(&self) -> usize {
        self.main_inputs.len()
    }

    /// Get the number of output channels collected.
    #[inline]
    pub fn output_channel_count(&self) -> usize {
        self.main_outputs.len()
    }

    /// Collect auxiliary input pointers from multiple AudioBufferLists.
    ///
    /// This method collects channel pointers from auxiliary input buses
    /// (buses 1 and above) for processing sidechain inputs, etc.
    ///
    /// # Safety
    ///
    /// - `buffer_lists` must be a valid slice of valid pointers
    /// - Each pointer is only valid for the current render call
    /// - num_samples must not exceed actual buffer sizes
    ///
    /// # Arguments
    ///
    /// * `buffer_lists` - Slice of AudioBufferList pointers (one per aux bus)
    /// * `num_samples` - Number of samples in each buffer
    #[inline]
    pub unsafe fn collect_aux_inputs(
        &mut self,
        buffer_lists: &[*const AudioBufferList],
        num_samples: usize,
    ) {
        for (aux_idx, &buffer_list) in buffer_lists.iter().enumerate() {
            if buffer_list.is_null() || aux_idx >= self.aux_inputs.len() {
                continue;
            }

            let list = &*buffer_list;
            let max_channels = self.aux_inputs[aux_idx].capacity();

            for i in 0..list.number_buffers.min(max_channels as u32) {
                let buffer = list.buffer_at(i);
                if !buffer.data.is_null() && buffer.number_channels == 1 {
                    // Non-interleaved: one channel per buffer
                    let data_ptr = buffer.data as *const S;
                    // Validate we have enough data
                    let available_samples =
                        buffer.data_byte_size as usize / std::mem::size_of::<S>();
                    if available_samples >= num_samples {
                        self.aux_inputs[aux_idx].push(data_ptr);
                    }
                }
                // Skip interleaved buffers (number_channels > 1) - handled separately
            }
        }
    }

    /// Collect auxiliary output pointers from multiple AudioBufferLists.
    ///
    /// # Safety
    ///
    /// - `buffer_lists` must be a valid slice of valid pointers
    /// - Each pointer is only valid for the current render call
    /// - num_samples must not exceed actual buffer sizes
    ///
    /// # Arguments
    ///
    /// * `buffer_lists` - Slice of AudioBufferList pointers (one per aux bus)
    /// * `num_samples` - Number of samples in each buffer
    #[inline]
    pub unsafe fn collect_aux_outputs(
        &mut self,
        buffer_lists: &[*mut AudioBufferList],
        num_samples: usize,
    ) {
        for (aux_idx, &buffer_list) in buffer_lists.iter().enumerate() {
            if buffer_list.is_null() || aux_idx >= self.aux_outputs.len() {
                continue;
            }

            let list = &mut *buffer_list;
            let max_channels = self.aux_outputs[aux_idx].capacity();

            for i in 0..list.number_buffers.min(max_channels as u32) {
                let buffer = list.buffer_at_mut(i);
                if !buffer.data.is_null() && buffer.number_channels == 1 {
                    // Non-interleaved: one channel per buffer
                    let data_ptr = buffer.data as *mut S;
                    // Validate we have enough data
                    let available_samples =
                        buffer.data_byte_size as usize / std::mem::size_of::<S>();
                    if available_samples >= num_samples {
                        self.aux_outputs[aux_idx].push(data_ptr);
                    }
                }
                // Skip interleaved buffers (number_channels > 1) - handled separately
            }
        }
    }

    /// Build auxiliary input slices from collected pointers.
    ///
    /// Returns an iterator over buses, where each bus is an iterator over channel slices.
    ///
    /// # Safety
    ///
    /// - Pointers must still be valid (within same render call)
    /// - num_samples must match what was used in collect_aux_inputs
    #[inline]
    pub unsafe fn aux_input_slices(&self, num_samples: usize) -> Vec<Vec<&[S]>> {
        self.aux_inputs
            .iter()
            .map(|bus| {
                bus.iter()
                    .map(|&ptr| slice::from_raw_parts(ptr, num_samples))
                    .collect()
            })
            .collect()
    }

    /// Build auxiliary output slices from collected pointers.
    ///
    /// Returns an iterator over buses, where each bus is an iterator over channel slices.
    ///
    /// # Safety
    ///
    /// - Pointers must still be valid (within same render call)
    /// - num_samples must match what was used in collect_aux_outputs
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn aux_output_slices(&self, num_samples: usize) -> Vec<Vec<&mut [S]>> {
        self.aux_outputs
            .iter()
            .map(|bus| {
                bus.iter()
                    .map(|&ptr| slice::from_raw_parts_mut(ptr, num_samples))
                    .collect()
            })
            .collect()
    }

    /// Get the number of auxiliary input buses.
    #[inline]
    pub fn aux_input_bus_count(&self) -> usize {
        self.aux_inputs.len()
    }

    /// Get the number of auxiliary output buses.
    #[inline]
    pub fn aux_output_bus_count(&self) -> usize {
        self.aux_outputs.len()
    }
}

// SAFETY: The raw pointers are only used within a single render call
// where AU guarantees single-threaded access.
unsafe impl<S: Sample> Send for ProcessBufferStorage<S> {}
unsafe impl<S: Sample> Sync for ProcessBufferStorage<S> {}

/// Validate buffer alignment before creating slices.
///
/// Returns an error message if the buffer is not properly aligned.
#[inline]
pub fn validate_buffer_alignment(data_ptr: *const u8, byte_size: u32) -> Result<(), &'static str> {
    // Check f32 alignment (4 bytes)
    if data_ptr.align_offset(std::mem::align_of::<f32>()) != 0 {
        return Err("Buffer not aligned for f32");
    }

    // Check size is multiple of f32
    if !(byte_size as usize).is_multiple_of(std::mem::size_of::<f32>()) {
        return Err("Buffer size not multiple of f32");
    }

    Ok(())
}

/// Validate buffer alignment for f64.
#[inline]
pub fn validate_buffer_alignment_f64(
    data_ptr: *const u8,
    byte_size: u32,
) -> Result<(), &'static str> {
    // Check f64 alignment (8 bytes)
    if data_ptr.align_offset(std::mem::align_of::<f64>()) != 0 {
        return Err("Buffer not aligned for f64");
    }

    // Check size is multiple of f64
    if !(byte_size as usize).is_multiple_of(std::mem::size_of::<f64>()) {
        return Err("Buffer size not multiple of f64");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus_config::{BusInfo, BusType, CachedBusConfig};

    #[test]
    fn test_validate_bus_limits_success() {
        let config = CachedBusConfig::new(
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
        );

        assert!(validate_bus_limits_from_config(&config).is_ok());
    }

    #[test]
    fn test_validate_bus_limits_too_many_channels() {
        let config = CachedBusConfig::new(
            vec![BusInfo {
                channel_count: MAX_CHANNELS + 1,
                bus_type: BusType::Main,
            }],
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
        );

        assert!(validate_bus_limits_from_config(&config).is_err());
    }

    #[test]
    fn test_allocate_from_config_stereo() {
        let config = CachedBusConfig::default(); // 2in/2out
        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        assert_eq!(storage.main_inputs.capacity(), 2);
        assert_eq!(storage.main_outputs.capacity(), 2);
        assert_eq!(storage.aux_inputs.len(), 0);
        assert_eq!(storage.aux_outputs.len(), 0);
    }

    #[test]
    fn test_allocate_from_config_with_aux() {
        let config = CachedBusConfig::new(
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Auxiliary,
                },
            ],
            vec![BusInfo {
                channel_count: 6,
                bus_type: BusType::Main,
            }],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        assert_eq!(storage.main_inputs.capacity(), 2);
        assert_eq!(storage.main_outputs.capacity(), 6);
        assert_eq!(storage.aux_inputs.len(), 1);
        assert_eq!(storage.aux_inputs[0].capacity(), 2);
        assert_eq!(storage.aux_outputs.len(), 0);
    }

    #[test]
    fn test_allocate_and_clear() {
        let mut storage: ProcessBufferStorage<f32> = ProcessBufferStorage::allocate(2, 2, 1, 0, 2);

        // Verify capacities
        assert_eq!(storage.main_inputs.capacity(), 2);
        assert_eq!(storage.main_outputs.capacity(), 2);
        assert_eq!(storage.aux_inputs.len(), 1);
        assert_eq!(storage.aux_inputs[0].capacity(), 2);

        // Simulate pushing pointers
        let dummy: f32 = 0.0;
        storage.main_inputs.push(&dummy as *const f32);
        storage.main_inputs.push(&dummy as *const f32);

        assert_eq!(storage.main_inputs.len(), 2);

        // Clear should reset length but not capacity
        storage.clear();
        assert_eq!(storage.main_inputs.len(), 0);
        assert_eq!(storage.main_inputs.capacity(), 2);
    }

    #[test]
    fn test_alignment_validation() {
        // Aligned pointer
        let aligned: [f32; 4] = [0.0; 4];
        let ptr = aligned.as_ptr() as *const u8;
        assert!(validate_buffer_alignment(ptr, 16).is_ok());

        // Wrong size
        assert!(validate_buffer_alignment(ptr, 15).is_err());
    }

    #[test]
    fn test_allocate_from_config_mono() {
        // Mono plugin: 1 in, 1 out, no aux buses
        let config = CachedBusConfig::new(
            vec![BusInfo {
                channel_count: 1,
                bus_type: BusType::Main,
            }],
            vec![BusInfo {
                channel_count: 1,
                bus_type: BusType::Main,
            }],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        // Verify exact allocation - no wasted space
        assert_eq!(
            storage.main_inputs.capacity(),
            1,
            "Mono input should allocate 1 channel"
        );
        assert_eq!(
            storage.main_outputs.capacity(),
            1,
            "Mono output should allocate 1 channel"
        );
        assert_eq!(
            storage.aux_inputs.len(),
            0,
            "No aux buses should allocate no aux input vecs"
        );
        assert_eq!(
            storage.aux_outputs.len(),
            0,
            "No aux buses should allocate no aux output vecs"
        );
        assert_eq!(
            storage.aux_inputs.capacity(),
            0,
            "Aux inputs outer vec should have 0 capacity"
        );
        assert_eq!(
            storage.aux_outputs.capacity(),
            0,
            "Aux outputs outer vec should have 0 capacity"
        );
    }

    #[test]
    fn test_allocate_from_config_asymmetric() {
        // Asymmetric plugin: 1 in, 2 out (e.g., mono-to-stereo effect)
        let config = CachedBusConfig::new(
            vec![BusInfo {
                channel_count: 1,
                bus_type: BusType::Main,
            }],
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        assert_eq!(storage.main_inputs.capacity(), 1);
        assert_eq!(storage.main_outputs.capacity(), 2);
        assert_eq!(storage.aux_inputs.len(), 0);
        assert_eq!(storage.aux_outputs.len(), 0);
    }

    #[test]
    fn test_allocate_from_config_multiple_aux_different_sizes() {
        // Complex plugin: different channel counts per aux bus
        let config = CachedBusConfig::new(
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 1, // Mono sidechain
                    bus_type: BusType::Auxiliary,
                },
                BusInfo {
                    channel_count: 4, // Quad input
                    bus_type: BusType::Auxiliary,
                },
            ],
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 6, // 5.1 aux output
                    bus_type: BusType::Auxiliary,
                },
            ],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        assert_eq!(storage.main_inputs.capacity(), 2);
        assert_eq!(storage.main_outputs.capacity(), 2);
        assert_eq!(storage.aux_inputs.len(), 2, "Should have 2 aux input buses");
        assert_eq!(storage.aux_outputs.len(), 1, "Should have 1 aux output bus");

        // Verify each aux bus has correct channel capacity
        assert_eq!(
            storage.aux_inputs[0].capacity(),
            1,
            "First aux input is mono"
        );
        assert_eq!(
            storage.aux_inputs[1].capacity(),
            4,
            "Second aux input is quad"
        );
        assert_eq!(
            storage.aux_outputs[0].capacity(),
            6,
            "First aux output is 5.1"
        );
    }

    #[test]
    fn test_memory_efficiency_comparison() {
        // This test documents the memory savings of config-based allocation
        // vs worst-case allocation

        // Mono plugin with config-based allocation
        let mono_config = CachedBusConfig::new(
            vec![BusInfo {
                channel_count: 1,
                bus_type: BusType::Main,
            }],
            vec![BusInfo {
                channel_count: 1,
                bus_type: BusType::Main,
            }],
        );
        let mono_storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&mono_config);

        // Calculate actual memory used (capacity * size_of::<*const f32>)
        let mono_memory = (mono_storage.main_inputs.capacity()
            + mono_storage.main_outputs.capacity())
            * std::mem::size_of::<*const f32>();

        // Worst-case allocation would be MAX_CHANNELS for all
        let worst_case_memory = (MAX_CHANNELS + MAX_CHANNELS) * std::mem::size_of::<*const f32>();

        // Mono should use much less memory
        assert!(
            mono_memory < worst_case_memory,
            "Config-based allocation ({} bytes) should use less than worst-case ({} bytes)",
            mono_memory,
            worst_case_memory
        );

        // Specifically, mono uses 2 channels worth, worst-case uses 64 channels worth
        assert_eq!(mono_memory, 2 * std::mem::size_of::<*const f32>());
        assert_eq!(worst_case_memory, 64 * std::mem::size_of::<*const f32>());
    }

    #[test]
    fn test_zero_allocation_for_simple_plugins() {
        // This test verifies that simple mono/stereo plugins without aux buses
        // truly get zero heap allocation for aux bus containers

        let stereo_config = CachedBusConfig::default(); // 2in/2out, no aux
        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&stereo_config);

        // The aux_inputs and aux_outputs should be completely empty Vec::new()
        assert_eq!(storage.aux_inputs.len(), 0);
        assert_eq!(storage.aux_outputs.len(), 0);
        assert_eq!(storage.aux_inputs.capacity(), 0);
        assert_eq!(storage.aux_outputs.capacity(), 0);

        // Only main buses should have allocated capacity
        assert!(storage.main_inputs.capacity() > 0);
        assert!(storage.main_outputs.capacity() > 0);
    }

    #[test]
    fn test_aux_bus_lazy_allocation() {
        // Test that aux buses are only allocated when they exist in the config

        // Config with only output aux bus (no input aux)
        let config = CachedBusConfig::new(
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Auxiliary,
                },
            ],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        // No input aux buses - should be empty
        assert_eq!(storage.aux_inputs.len(), 0);
        assert_eq!(storage.aux_inputs.capacity(), 0);

        // One output aux bus - should be allocated
        assert_eq!(storage.aux_outputs.len(), 1);
        assert!(storage.aux_outputs.capacity() >= 1);
        assert_eq!(storage.aux_outputs[0].capacity(), 2);
    }

    #[test]
    fn test_clear_maintains_capacity() {
        // Verify that clear() is O(1) and maintains capacity (real-time safety)

        let config = CachedBusConfig::new(
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Auxiliary,
                },
            ],
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
        );

        let mut storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        // Record initial capacities
        let main_in_cap = storage.main_inputs.capacity();
        let main_out_cap = storage.main_outputs.capacity();
        let aux_in_count = storage.aux_inputs.len();
        let aux_in_cap = if aux_in_count > 0 {
            storage.aux_inputs[0].capacity()
        } else {
            0
        };

        // Simulate some usage
        let dummy: f32 = 0.0;
        storage.main_inputs.push(&dummy as *const f32);
        storage.main_inputs.push(&dummy as *const f32);
        if aux_in_count > 0 {
            storage.aux_inputs[0].push(&dummy as *const f32);
        }

        // Clear and verify capacities are unchanged
        storage.clear();

        assert_eq!(
            storage.main_inputs.capacity(),
            main_in_cap,
            "clear() must not change capacity"
        );
        assert_eq!(
            storage.main_outputs.capacity(),
            main_out_cap,
            "clear() must not change capacity"
        );
        assert_eq!(
            storage.aux_inputs.len(),
            aux_in_count,
            "clear() must not change aux bus count"
        );
        if aux_in_count > 0 {
            assert_eq!(
                storage.aux_inputs[0].capacity(),
                aux_in_cap,
                "clear() must not change aux channel capacity"
            );
        }

        // Verify lengths are reset to 0
        assert_eq!(storage.main_inputs.len(), 0);
        assert_eq!(storage.main_outputs.len(), 0);
        if aux_in_count > 0 {
            assert_eq!(storage.aux_inputs[0].len(), 0);
        }
    }

    #[test]
    fn test_aux_bus_collection_methods() {
        // Test that aux bus collection methods work correctly
        let config = CachedBusConfig::new(
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 2, // Stereo sidechain
                    bus_type: BusType::Auxiliary,
                },
            ],
            vec![BusInfo {
                channel_count: 2,
                bus_type: BusType::Main,
            }],
        );

        let mut storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        // Verify aux input bus count
        assert_eq!(storage.aux_input_bus_count(), 1);
        assert_eq!(storage.aux_output_bus_count(), 0);

        // Verify aux input bus capacity
        assert_eq!(storage.aux_inputs.len(), 1);
        assert_eq!(storage.aux_inputs[0].capacity(), 2);

        // Clear and verify it resets aux buses too
        storage.clear();
        assert_eq!(storage.aux_inputs[0].len(), 0);
        assert_eq!(storage.aux_inputs[0].capacity(), 2);
    }

    #[test]
    fn test_aux_input_output_slices() {
        // Test building aux input/output slices from collected pointers
        let config = CachedBusConfig::new(
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Auxiliary,
                },
            ],
            vec![
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Main,
                },
                BusInfo {
                    channel_count: 2,
                    bus_type: BusType::Auxiliary,
                },
            ],
        );

        let mut storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config);

        // Create dummy buffers
        let aux_input_buffer: [f32; 8] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let aux_output_buffer: [f32; 8] = [0.0; 8];

        // Simulate collecting aux input pointers
        storage.aux_inputs[0].push(aux_input_buffer.as_ptr());
        // SAFETY: aux_input_buffer is a valid array with at least 8 elements, so add(4) is valid.
        // The pointer remains valid for the entire test scope.
        storage.aux_inputs[0].push(unsafe { aux_input_buffer.as_ptr().add(4) });

        // Simulate collecting aux output pointers
        storage.aux_outputs[0].push(aux_output_buffer.as_ptr() as *mut f32);
        // SAFETY: aux_output_buffer is a valid array with at least 8 elements, so add(4) is valid.
        // The pointer remains valid for the entire test scope.
        storage.aux_outputs[0].push(unsafe { aux_output_buffer.as_ptr().add(4) as *mut f32 });

        // Build slices
        // SAFETY: Pointers are still valid (within test scope), and num_samples (4) matches
        // what was pushed: 2 pointers × 4 samples each covers the 8-element test arrays.
        let aux_input_slices = unsafe { storage.aux_input_slices(4) };
        // SAFETY: Same justification as aux_input_slices - pointers valid, num_samples matches.
        let aux_output_slices = unsafe { storage.aux_output_slices(4) };

        // Verify aux input slices
        assert_eq!(aux_input_slices.len(), 1); // One aux bus
        assert_eq!(aux_input_slices[0].len(), 2); // Two channels
        assert_eq!(aux_input_slices[0][0], &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(aux_input_slices[0][1], &[5.0, 6.0, 7.0, 8.0]);

        // Verify aux output slices
        assert_eq!(aux_output_slices.len(), 1); // One aux bus
        assert_eq!(aux_output_slices[0].len(), 2); // Two channels
    }
}
