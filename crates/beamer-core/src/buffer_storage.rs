//! Pre-allocated buffer storage for real-time safe audio processing.
//!
//! This module provides [`ProcessBufferStorage`], which pre-allocates capacity
//! for channel pointers during plugin setup. The storage is then reused for each
//! render call without allocations.
//!
//! # Pattern
//!
//! This storage follows a consistent pattern across all plugin formats:
//! 1. Allocate storage once during setup (non-real-time)
//! 2. Clear storage at start of each render (O(1), no deallocation)
//! 3. Push pointers from host buffers (never exceeds capacity)
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

use crate::bus_config::CachedBusConfig;
use crate::sample::Sample;
use std::slice;

/// Pre-allocated storage for audio processing channel pointers.
///
/// Stores channel pointers collected from host audio buffers during render.
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
/// This means a simple stereo plugin uses only 32 bytes (4 pointers x 8 bytes),
/// not the worst-case 4KB (MAX_CHANNELS x MAX_BUSES x pointer size).
///
/// # Type Parameter
///
/// `S` is the sample type (`f32` or `f64`).
#[derive(Clone)]
pub struct ProcessBufferStorage<S: Sample> {
    /// Main input channel pointers (capacity = actual channel count).
    /// Format wrappers may access this directly for performance-critical inline collection.
    pub main_inputs: Vec<*const S>,
    /// Main output channel pointers (capacity = actual channel count).
    /// Format wrappers may access this directly for performance-critical inline collection.
    pub main_outputs: Vec<*mut S>,
    /// Auxiliary input buses (only allocated if plugin uses them).
    /// Format wrappers may access this directly for performance-critical inline collection.
    pub aux_inputs: Vec<Vec<*const S>>,
    /// Auxiliary output buses (only allocated if plugin uses them).
    /// Format wrappers may access this directly for performance-critical inline collection.
    pub aux_outputs: Vec<Vec<*mut S>>,
    /// Internal output buffers for instruments (when host provides null pointers).
    /// Only allocated for plugins with no input buses (instruments/generators).
    /// When a host provides null output pointers, the format wrapper can use these
    /// buffers and update the host's buffer list to point to them.
    pub internal_output_buffers: Option<Vec<Vec<S>>>,
    /// Max frames for internal buffers (set during allocation).
    /// Used to validate that num_samples doesn't exceed buffer capacity.
    pub max_frames: usize,
}

impl<S: Sample> ProcessBufferStorage<S> {
    /// Create empty storage (no capacity reserved).
    pub fn new() -> Self {
        Self {
            main_inputs: Vec::new(),
            main_outputs: Vec::new(),
            aux_inputs: Vec::new(),
            aux_outputs: Vec::new(),
            internal_output_buffers: None,
            max_frames: 0,
        }
    }

    /// Create storage from cached bus configuration (recommended).
    ///
    /// This is the preferred way to allocate storage as it automatically
    /// extracts the correct channel counts from the bus configuration.
    /// Should be called during plugin setup (non-real-time).
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
    /// * `bus_config` - Cached bus configuration
    /// * `max_frames` - Maximum frames per render call (for internal buffer allocation)
    ///
    /// # Example
    ///
    /// ```ignore
    /// let config = extract_bus_config();
    /// config.validate()?;
    /// let storage = ProcessBufferStorage::allocate_from_config(&config, 4096);
    /// ```
    pub fn allocate_from_config(bus_config: &CachedBusConfig, max_frames: usize) -> Self {
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

        // For instruments (no input buses), allocate internal output buffers.
        // Some hosts (Logic Pro, Reaper) may provide null output buffer pointers,
        // expecting the plugin to use its own buffers.
        let internal_output_buffers = if main_in_channels == 0 && main_out_channels > 0 {
            let mut buffers = Vec::with_capacity(main_out_channels);
            for _ in 0..main_out_channels {
                buffers.push(vec![S::ZERO; max_frames]);
            }
            Some(buffers)
        } else {
            None
        };

        Self {
            main_inputs: Vec::with_capacity(main_in_channels),
            main_outputs: Vec::with_capacity(main_out_channels),
            aux_inputs,
            aux_outputs,
            internal_output_buffers,
            max_frames,
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
            internal_output_buffers: None, // Manual allocation doesn't set up internal buffers
            max_frames: 0,
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

    /// Get the maximum frames for internal buffers.
    #[inline]
    pub fn max_frames(&self) -> usize {
        self.max_frames
    }

    /// Check if internal output buffers are available.
    #[inline]
    pub fn has_internal_output_buffers(&self) -> bool {
        self.internal_output_buffers.is_some()
    }

    /// Push a main input pointer.
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the duration of the current render call.
    #[inline]
    pub unsafe fn push_main_input(&mut self, ptr: *const S) {
        self.main_inputs.push(ptr);
    }

    /// Push a main output pointer.
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the duration of the current render call.
    #[inline]
    pub unsafe fn push_main_output(&mut self, ptr: *mut S) {
        self.main_outputs.push(ptr);
    }

    /// Push an auxiliary input pointer for a specific bus.
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the duration of the current render call.
    #[inline]
    pub unsafe fn push_aux_input(&mut self, bus_index: usize, ptr: *const S) {
        if bus_index < self.aux_inputs.len() {
            self.aux_inputs[bus_index].push(ptr);
        }
    }

    /// Push an auxiliary output pointer for a specific bus.
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the duration of the current render call.
    #[inline]
    pub unsafe fn push_aux_output(&mut self, bus_index: usize, ptr: *mut S) {
        if bus_index < self.aux_outputs.len() {
            self.aux_outputs[bus_index].push(ptr);
        }
    }

    /// Get the main input capacity.
    #[inline]
    pub fn main_input_capacity(&self) -> usize {
        self.main_inputs.capacity()
    }

    /// Get the main output capacity.
    #[inline]
    pub fn main_output_capacity(&self) -> usize {
        self.main_outputs.capacity()
    }

    /// Get an auxiliary input bus capacity.
    #[inline]
    pub fn aux_input_capacity(&self, bus_index: usize) -> usize {
        self.aux_inputs.get(bus_index).map(|v| v.capacity()).unwrap_or(0)
    }

    /// Get an auxiliary output bus capacity.
    #[inline]
    pub fn aux_output_capacity(&self, bus_index: usize) -> usize {
        self.aux_outputs.get(bus_index).map(|v| v.capacity()).unwrap_or(0)
    }

    /// Build input slices from collected pointers.
    ///
    /// # Safety
    ///
    /// - Pointers must still be valid (within same render call)
    /// - num_samples must match what was used in collection
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
    /// - num_samples must match what was used in collection
    ///
    /// # Clippy Allow: mut_from_ref
    ///
    /// Returns `&mut [S]` from `&self` because we're converting raw pointers stored in the struct,
    /// not mutating `self`. This is a common and safe FFI pattern where:
    /// - Raw pointers (`*mut S`) are stored during collection
    /// - Those pointers are then converted back to safe references
    /// - The mutable references are to the external buffer memory, not to `self`
    /// - Host guarantees single-threaded render access, preventing aliasing
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn output_slices(&self, num_samples: usize) -> Vec<&mut [S]> {
        self.main_outputs
            .iter()
            .map(|&ptr| slice::from_raw_parts_mut(ptr, num_samples))
            .collect()
    }

    /// Build auxiliary input slices from collected pointers.
    ///
    /// Returns a Vec of buses, where each bus is a Vec of channel slices.
    ///
    /// # Safety
    ///
    /// - Pointers must still be valid (within same render call)
    /// - num_samples must match what was used in collection
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
    /// Returns a Vec of buses, where each bus is a Vec of channel slices.
    ///
    /// # Safety
    ///
    /// - Pointers must still be valid (within same render call)
    /// - num_samples must match what was used in collection
    ///
    /// # Clippy Allow: mut_from_ref
    ///
    /// Same justification as `output_slices` - converts raw pointers to mutable references.
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

    /// Get a mutable pointer to an internal output buffer channel.
    ///
    /// Returns `None` if internal buffers are not allocated or channel is out of range.
    ///
    /// # Arguments
    ///
    /// * `channel` - The channel index
    #[inline]
    pub fn internal_output_buffer_mut(&mut self, channel: usize) -> Option<&mut Vec<S>> {
        self.internal_output_buffers
            .as_mut()
            .and_then(|buffers| buffers.get_mut(channel))
    }

    /// Get a reference to an internal output buffer channel.
    ///
    /// Returns `None` if internal buffers are not allocated or channel is out of range.
    #[inline]
    pub fn internal_output_buffer(&self, channel: usize) -> Option<&Vec<S>> {
        self.internal_output_buffers
            .as_ref()
            .and_then(|buffers| buffers.get(channel))
    }

    /// Get the number of internal output buffer channels.
    #[inline]
    pub fn internal_output_buffer_count(&self) -> usize {
        self.internal_output_buffers
            .as_ref()
            .map(|b| b.len())
            .unwrap_or(0)
    }
}

impl<S: Sample> Default for ProcessBufferStorage<S> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: The raw pointers are only used within a single render call
// where the host guarantees single-threaded access.
unsafe impl<S: Sample> Send for ProcessBufferStorage<S> {}
unsafe impl<S: Sample> Sync for ProcessBufferStorage<S> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus_config::{CachedBusInfo};
    use crate::plugin::BusType;
    use crate::types::MAX_CHANNELS;

    #[test]
    fn test_validate_bus_limits_success() {
        let config = CachedBusConfig::new(
            vec![CachedBusInfo::new(2, BusType::Main)],
            vec![CachedBusInfo::new(2, BusType::Main)],
        );

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_bus_limits_too_many_channels() {
        let config = CachedBusConfig::new(
            vec![CachedBusInfo::new(MAX_CHANNELS + 1, BusType::Main)],
            vec![CachedBusInfo::new(2, BusType::Main)],
        );

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_allocate_from_config_stereo() {
        let config = CachedBusConfig::default(); // 2in/2out
        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

        assert_eq!(storage.main_inputs.capacity(), 2);
        assert_eq!(storage.main_outputs.capacity(), 2);
        assert_eq!(storage.aux_inputs.len(), 0);
        assert_eq!(storage.aux_outputs.len(), 0);
    }

    #[test]
    fn test_allocate_from_config_with_aux() {
        let config = CachedBusConfig::new(
            vec![
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(2, BusType::Aux),
            ],
            vec![CachedBusInfo::new(6, BusType::Main)],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

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
        unsafe {
            storage.push_main_input(&dummy as *const f32);
            storage.push_main_input(&dummy as *const f32);
        }

        assert_eq!(storage.main_inputs.len(), 2);

        // Clear should reset length but not capacity
        storage.clear();
        assert_eq!(storage.main_inputs.len(), 0);
        assert_eq!(storage.main_inputs.capacity(), 2);
    }

    #[test]
    fn test_allocate_from_config_mono() {
        // Mono plugin: 1 in, 1 out, no aux buses
        let config = CachedBusConfig::new(
            vec![CachedBusInfo::new(1, BusType::Main)],
            vec![CachedBusInfo::new(1, BusType::Main)],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

        assert_eq!(storage.main_inputs.capacity(), 1);
        assert_eq!(storage.main_outputs.capacity(), 1);
        assert_eq!(storage.aux_inputs.len(), 0);
        assert_eq!(storage.aux_outputs.len(), 0);
    }

    #[test]
    fn test_instrument_internal_buffers_allocated() {
        // Instruments have 0 inputs and >0 outputs - should allocate internal buffers
        let config = CachedBusConfig::new(
            vec![], // No input buses (instrument)
            vec![CachedBusInfo::new(2, BusType::Main)],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 512);

        // Internal buffers should be allocated for instruments
        assert!(storage.internal_output_buffers.is_some());
        let internal = storage.internal_output_buffers.as_ref().unwrap();
        assert_eq!(internal.len(), 2); // Stereo
        assert_eq!(internal[0].len(), 512); // max_frames
        assert_eq!(internal[1].len(), 512);
        assert_eq!(storage.max_frames, 512);
    }

    #[test]
    fn test_effect_no_internal_buffers() {
        // Effects have inputs - should NOT allocate internal buffers
        let config = CachedBusConfig::default(); // 2in/2out

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 512);

        // Effects don't need internal buffers
        assert!(storage.internal_output_buffers.is_none());
    }

    #[test]
    fn test_clear_maintains_capacity() {
        let config = CachedBusConfig::new(
            vec![
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(2, BusType::Aux),
            ],
            vec![CachedBusInfo::new(2, BusType::Main)],
        );

        let mut storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

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
        unsafe {
            storage.push_main_input(&dummy as *const f32);
            storage.push_main_input(&dummy as *const f32);
            if aux_in_count > 0 {
                storage.push_aux_input(0, &dummy as *const f32);
            }
        }

        // Clear and verify capacities are unchanged
        storage.clear();

        assert_eq!(storage.main_inputs.capacity(), main_in_cap);
        assert_eq!(storage.main_outputs.capacity(), main_out_cap);
        assert_eq!(storage.aux_inputs.len(), aux_in_count);
        if aux_in_count > 0 {
            assert_eq!(storage.aux_inputs[0].capacity(), aux_in_cap);
        }

        // Verify lengths are reset to 0
        assert_eq!(storage.main_inputs.len(), 0);
        assert_eq!(storage.main_outputs.len(), 0);
        if aux_in_count > 0 {
            assert_eq!(storage.aux_inputs[0].len(), 0);
        }
    }
}
