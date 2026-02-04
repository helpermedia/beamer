//! Pre-allocated buffer storage for real-time safe audio processing.
//!
//! This module re-exports [`ProcessBufferStorage`] from `beamer-core` and provides
//! the [`ProcessBufferStorageAuExt`] trait with AU-specific methods for collecting
//! pointers from `AudioBufferList`.
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
//! # Real-Time Safety
//!
//! - `clear()` is O(1) - only sets Vec lengths to 0
//! - `push()` never allocates - capacity is pre-reserved
//! - No heap operations during audio processing
//! - All allocations happen in `allocate_from_config()` (non-real-time)

use crate::buffers::AudioBufferList;
use beamer_core::Sample;

// Re-export ProcessBufferStorage from beamer-core
pub use beamer_core::ProcessBufferStorage;

#[cfg(test)]
use beamer_core::{BusType, CachedBusConfig, CachedBusInfo, MAX_CHANNELS};

// =============================================================================
// AU-specific extensions for ProcessBufferStorage
// =============================================================================

/// AU-specific extension methods for ProcessBufferStorage.
///
/// This trait provides methods for collecting pointers from AU's `AudioBufferList` type.
/// Import this trait to use these methods on `ProcessBufferStorage`.
pub trait ProcessBufferStorageAuExt<S: Sample> {
    /// Collect input pointers from an AudioBufferList.
    ///
    /// # Safety
    ///
    /// - `buffer_list` must be a valid pointer
    /// - Pointers are only valid for the current render call
    /// - num_samples must not exceed actual buffer sizes
    unsafe fn collect_inputs(&mut self, buffer_list: *const AudioBufferList, num_samples: usize);

    /// Collect output pointers from an AudioBufferList.
    ///
    /// For instruments (plugins with no inputs), if the host provides null buffer pointers,
    /// this function will use internal buffers and update the AudioBufferList to point to them.
    /// This is necessary because some hosts (Logic Pro, Reaper) expect the AU to provide buffers.
    ///
    /// # Safety
    ///
    /// - `buffer_list` must be a valid pointer
    /// - Pointers are only valid for the current render call
    /// - num_samples must not exceed actual buffer sizes or max_frames
    unsafe fn collect_outputs(&mut self, buffer_list: *mut AudioBufferList, num_samples: usize);

    /// Collect auxiliary input pointers from multiple AudioBufferLists.
    ///
    /// # Safety
    ///
    /// - `buffer_lists` must be a valid slice of valid pointers
    /// - Each pointer is only valid for the current render call
    /// - num_samples must not exceed actual buffer sizes
    unsafe fn collect_aux_inputs(
        &mut self,
        buffer_lists: &[*const AudioBufferList],
        num_samples: usize,
    );

    /// Collect auxiliary output pointers from multiple AudioBufferLists.
    ///
    /// # Safety
    ///
    /// - `buffer_lists` must be a valid slice of valid pointers
    /// - Each pointer is only valid for the current render call
    /// - num_samples must not exceed actual buffer sizes
    unsafe fn collect_aux_outputs(
        &mut self,
        buffer_lists: &[*mut AudioBufferList],
        num_samples: usize,
    );
}

impl<S: Sample> ProcessBufferStorageAuExt<S> for ProcessBufferStorage<S> {
    #[inline]
    unsafe fn collect_inputs(
        &mut self,
        buffer_list: *const AudioBufferList,
        num_samples: usize,
    ) {
        if buffer_list.is_null() {
            return;
        }

        // SAFETY: Caller guarantees buffer_list is a valid pointer. We checked non-null above.
        let list = unsafe { &*buffer_list };
        let max_channels = self.main_inputs.capacity();

        for i in 0..list.number_buffers.min(max_channels as u32) {
            // SAFETY: We iterate only up to number_buffers, so index is always valid.
            let buffer = unsafe { list.buffer_at(i) };
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

    #[inline]
    unsafe fn collect_outputs(
        &mut self,
        buffer_list: *mut AudioBufferList,
        num_samples: usize,
    ) {
        if buffer_list.is_null() {
            return;
        }

        // SAFETY: Caller guarantees buffer_list is a valid pointer. We checked non-null above.
        let list = unsafe { &mut *buffer_list };
        let max_channels = self.main_outputs.capacity();

        for i in 0..list.number_buffers.min(max_channels as u32) {
            // SAFETY: We iterate only up to number_buffers, so index is always valid.
            let buffer = unsafe { list.buffer_at_mut(i) };

            // Skip interleaved buffers (number_channels > 1)
            if buffer.number_channels != 1 {
                continue;
            }

            if !buffer.data.is_null() {
                // Host provided a buffer - use it
                let available_samples =
                    buffer.data_byte_size as usize / std::mem::size_of::<S>();
                if available_samples >= num_samples {
                    self.main_outputs.push(buffer.data as *mut S);
                }
            } else if let Some(ref mut internal_buffers) = self.internal_output_buffers {
                // Host provided null pointer - use our internal buffer (instruments only).
                // This happens in some hosts (Logic Pro, Reaper) that expect the AU to
                // provide its own output buffers for instruments/generators.
                let channel_idx = i as usize;
                if channel_idx < internal_buffers.len() && num_samples <= self.max_frames {
                    let internal_buf = &mut internal_buffers[channel_idx];
                    let ptr = internal_buf.as_mut_ptr();

                    // Update the AudioBufferList to point to our internal buffer.
                    // The host will read rendered audio from this location.
                    buffer.data = ptr as *mut std::ffi::c_void;
                    buffer.data_byte_size = (num_samples * std::mem::size_of::<S>()) as u32;

                    self.main_outputs.push(ptr);
                }
            }
            // If null data and no internal buffers (effects), skip this channel
        }
    }

    #[inline]
    unsafe fn collect_aux_inputs(
        &mut self,
        buffer_lists: &[*const AudioBufferList],
        num_samples: usize,
    ) {
        for (aux_idx, &buffer_list) in buffer_lists.iter().enumerate() {
            if buffer_list.is_null() || aux_idx >= self.aux_inputs.len() {
                continue;
            }

            // SAFETY: Caller guarantees buffer_list is a valid pointer. We checked non-null above.
            let list = unsafe { &*buffer_list };
            let max_channels = self.aux_inputs[aux_idx].capacity();

            for i in 0..list.number_buffers.min(max_channels as u32) {
                // SAFETY: We iterate only up to number_buffers, so index is always valid.
                let buffer = unsafe { list.buffer_at(i) };
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

    #[inline]
    unsafe fn collect_aux_outputs(
        &mut self,
        buffer_lists: &[*mut AudioBufferList],
        num_samples: usize,
    ) {
        for (aux_idx, &buffer_list) in buffer_lists.iter().enumerate() {
            if buffer_list.is_null() || aux_idx >= self.aux_outputs.len() {
                continue;
            }

            // SAFETY: Caller guarantees buffer_list is a valid pointer. We checked non-null above.
            let list = unsafe { &mut *buffer_list };
            let max_channels = self.aux_outputs[aux_idx].capacity();

            for i in 0..list.number_buffers.min(max_channels as u32) {
                // SAFETY: We iterate only up to number_buffers, so index is always valid.
                let buffer = unsafe { list.buffer_at_mut(i) };
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
}

#[cfg(test)]
#[allow(clippy::undocumented_unsafe_blocks)]
mod tests {
    use super::*;

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
        storage.main_inputs.push(&dummy as *const f32);
        storage.main_inputs.push(&dummy as *const f32);

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
            vec![CachedBusInfo::new(1, BusType::Main)],
            vec![CachedBusInfo::new(2, BusType::Main)],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

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
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(1, BusType::Aux), // Mono sidechain
                CachedBusInfo::new(4, BusType::Aux), // Quad input
            ],
            vec![
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(6, BusType::Aux), // 5.1 aux output
            ],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

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
            vec![CachedBusInfo::new(1, BusType::Main)],
            vec![CachedBusInfo::new(1, BusType::Main)],
        );
        let mono_storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&mono_config, 4096);

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
            ProcessBufferStorage::allocate_from_config(&stereo_config, 4096);

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
            vec![CachedBusInfo::new(2, BusType::Main)],
            vec![
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(2, BusType::Aux),
            ],
        );

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

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
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(2, BusType::Aux), // Stereo sidechain
            ],
            vec![CachedBusInfo::new(2, BusType::Main)],
        );

        let mut storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

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
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(2, BusType::Aux),
            ],
            vec![
                CachedBusInfo::new(2, BusType::Main),
                CachedBusInfo::new(2, BusType::Aux),
            ],
        );

        let mut storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 4096);

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

        // Effects don't need internal buffers (can use in-place processing)
        assert!(storage.internal_output_buffers.is_none());
    }

    #[test]
    fn test_collect_outputs_uses_internal_buffers_for_null_pointers() {
        use crate::buffers::{AudioBuffer, AudioBufferList};

        // Create instrument config (no inputs)
        let config = CachedBusConfig::new(
            vec![], // No input buses
            vec![CachedBusInfo::new(2, BusType::Main)],
        );

        let mut storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 256);

        // Create an AudioBufferList with null data pointers (simulating host behavior).
        // We need space for 2 AudioBuffers, so use a repr(C) struct.
        #[repr(C)]
        struct TestAudioBufferList {
            number_buffers: u32,
            buffers: [AudioBuffer; 2],
        }

        let mut test_abl = TestAudioBufferList {
            number_buffers: 2,
            buffers: [
                AudioBuffer {
                    number_channels: 1,
                    data_byte_size: 0,
                    data: std::ptr::null_mut(), // Null pointer - host expects AU to provide
                },
                AudioBuffer {
                    number_channels: 1,
                    data_byte_size: 0,
                    data: std::ptr::null_mut(),
                },
            ],
        };

        // Collect outputs - should use internal buffers
        let num_samples = 128;
        unsafe {
            storage.collect_outputs(
                &mut test_abl as *mut TestAudioBufferList as *mut AudioBufferList,
                num_samples,
            );
        }

        // Verify outputs were collected
        assert_eq!(storage.main_outputs.len(), 2);

        // Verify the AudioBufferList was updated to point to internal buffers
        assert!(!test_abl.buffers[0].data.is_null());
        assert!(!test_abl.buffers[1].data.is_null());
        assert_eq!(
            test_abl.buffers[0].data_byte_size,
            (num_samples * std::mem::size_of::<f32>()) as u32
        );

        // Verify we can write to the output slices and they're backed by internal buffers
        unsafe {
            let mut slices = storage.output_slices(num_samples);
            assert_eq!(slices.len(), 2);

            // Write test data
            slices[0][0] = 0.5;
            slices[1][0] = -0.5;
        }

        // Verify the data is in the internal buffers
        let internal = storage.internal_output_buffers.as_ref().unwrap();
        assert_eq!(internal[0][0], 0.5);
        assert_eq!(internal[1][0], -0.5);
    }
}
