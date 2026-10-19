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
//! - Mono plugin (1in/1out): Allocates 2 pointers
//! - Stereo plugin (2in/2out): Allocates 4 pointers
//! - Stereo with sidechain (2+2in/2out): Allocates 6 pointers
//! - Worst-case (32ch x 16 buses): Would be MAX_CHANNELS * MAX_BUSES = 512 pointers
//!
//! Next to the pointers it allocates `max_frames` samples per channel: scratch
//! memory for each input and output channel (see In-Place Processing below)
//! and internal output buffers for the main output channels.
//!
//! # Real-Time Safety
//!
//! - `clear()` is O(1) - only sets Vec lengths to 0
//! - `push()` never allocates - capacity is pre-reserved
//! - No heap operations during audio processing
//! - All allocations happen in `allocate_from_config()` (non-real-time)
//!
//! # In-Place Processing
//!
//! [`ProcessBufferStorage::prepare_in_place()`] turns the collected host
//! pointers into channels the plugin can process in place: each main output
//! channel holds its input, no output overlaps another output and no input
//! the plugin reads overlaps an output. Hosts may pass the same memory for
//! input and output, or for several outputs; the plugin then gets its own
//! memory for what would otherwise overlap.

use crate::buffer::{AuxiliaryBuffers, Buffer};
use crate::bus_config::CachedBusConfig;
use crate::sample::Sample;

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
/// The scratch memory and internal output buffers hold `max_frames` samples
/// per channel of the configuration.
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
    /// Internal output buffers for when the host provides null output pointers.
    /// Allocated for every plugin with main output channels. The format
    /// wrapper uses these buffers and updates the host's buffer list to point
    /// to them.
    pub internal_output_buffers: Option<Vec<Vec<S>>>,
    /// Max frames for internal buffers (set during allocation).
    /// Used to validate that num_samples doesn't exceed buffer capacity.
    pub max_frames: usize,
    /// Memory for input channels that overlap an output channel, one per
    /// input channel: the main bus, then the aux buses in order.
    input_scratch: Vec<Vec<S>>,
    /// Memory for output channels that overlap another output channel, one
    /// per output channel: the main bus, then the aux buses in order. What
    /// the plugin writes there is discarded.
    output_scratch: Vec<Vec<S>>,
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
            input_scratch: Vec::new(),
            output_scratch: Vec::new(),
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

        // Some hosts (Logic Pro, Reaper) may provide null output buffer pointers,
        // expecting the plugin to use its own buffers.
        let internal_output_buffers = if main_out_channels > 0 {
            let mut buffers = Vec::with_capacity(main_out_channels);
            for _ in 0..main_out_channels {
                buffers.push(vec![S::ZERO; max_frames]);
            }
            Some(buffers)
        } else {
            None
        };

        let input_channels = main_in_channels + aux_inputs.iter().map(Vec::capacity).sum::<usize>();
        let output_channels =
            main_out_channels + aux_outputs.iter().map(Vec::capacity).sum::<usize>();

        Self {
            main_inputs: Vec::with_capacity(main_in_channels),
            main_outputs: Vec::with_capacity(main_out_channels),
            aux_inputs,
            aux_outputs,
            internal_output_buffers,
            max_frames,
            input_scratch: vec![vec![S::ZERO; max_frames]; input_channels],
            output_scratch: vec![vec![S::ZERO; max_frames]; output_channels],
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
            // Manual allocation doesn't set up internal buffers or scratch.
            internal_output_buffers: None,
            max_frames: 0,
            input_scratch: Vec::new(),
            output_scratch: Vec::new(),
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

    // =========================================================================
    // In-Place Processing
    // =========================================================================

    /// Prepare the collected channels of a process call for in-place
    /// processing.
    ///
    /// Call this once per process call, after collecting the pointers and
    /// before the sub-blocks. Afterwards:
    ///
    /// - No output channel (main or aux) overlaps another. One that overlaps
    ///   an earlier output gets scratch memory, and what the plugin writes
    ///   there is discarded.
    /// - No input channel the plugin reads overlaps an output channel. One
    ///   that does is copied into scratch memory first, unless it is exactly
    ///   the memory of its own main output channel (in place).
    /// - Each main output channel holds the main input channel of its index,
    ///   or silence when the input has fewer channels. Aux outputs are
    ///   silent.
    ///
    /// Returns false when memory of its own is needed for a block longer
    /// than the storage was allocated for. The channels are then not
    /// prepared, and the caller must not process them (see
    /// [`silence_outputs()`](Self::silence_outputs)).
    ///
    /// # Safety
    ///
    /// Every collected input pointer must be valid for reads and every
    /// output pointer for writes of `num_samples` samples, and no reference
    /// to that memory may be alive.
    pub unsafe fn prepare_in_place(&mut self, num_samples: usize) -> bool {
        let fits = num_samples <= self.max_frames;

        // Outputs overlapping an earlier output get scratch. They are
        // compared with the earlier outputs as they are now, after their own
        // replacement.
        let mut flat = 0;
        for bus in 0..=self.aux_outputs.len() {
            for channel in 0..self.output_bus_len(bus) {
                let ptr = self.output_at(bus, channel);
                if self.overlaps_output_before(ptr, num_samples, bus, channel) {
                    let Some(scratch) = self.output_scratch.get_mut(flat).filter(|_| fits) else {
                        return false;
                    };
                    let scratch = scratch.as_mut_ptr();
                    self.set_output_at(bus, channel, scratch);
                }
                flat += 1;
            }
        }

        // Inputs overlapping an output are copied into scratch. Nothing is
        // written to an output before this, so all inputs are still intact.
        let mut flat = 0;
        for bus in 0..=self.aux_inputs.len() {
            for channel in 0..self.input_bus_len(bus) {
                let ptr = self.input_at(bus, channel);
                let in_place = bus == 0
                    && self
                        .main_outputs
                        .get(channel)
                        .is_some_and(|&output| std::ptr::eq(output, ptr));
                if !in_place && self.overlaps_any_output(ptr, num_samples) {
                    let Some(scratch) = self.input_scratch.get_mut(flat).filter(|_| fits) else {
                        return false;
                    };
                    // SAFETY: The input is valid for num_samples reads (caller),
                    // the scratch holds max_frames >= num_samples samples and
                    // is our own memory, so the two do not overlap.
                    unsafe { std::ptr::copy_nonoverlapping(ptr, scratch.as_mut_ptr(), num_samples) };
                    let scratch = scratch.as_ptr();
                    self.set_input_at(bus, channel, scratch);
                }
                flat += 1;
            }
        }

        // Each main output gets its input, or silence without one.
        for (channel, &output) in self.main_outputs.iter().enumerate() {
            match self.main_inputs.get(channel) {
                // SAFETY: Valid for num_samples (caller). An input that is not
                // its output's memory overlaps no output (see above).
                Some(&input) if !std::ptr::eq(input, output) => unsafe {
                    std::ptr::copy_nonoverlapping(input, output, num_samples)
                },
                Some(_) => {}
                // SAFETY: Valid for num_samples writes (caller), or scratch.
                None => unsafe { std::ptr::write_bytes(output, 0, num_samples) },
            }
        }
        for bus in &self.aux_outputs {
            for &output in bus {
                // SAFETY: Valid for num_samples writes (caller), or scratch.
                unsafe { std::ptr::write_bytes(output, 0, num_samples) };
            }
        }

        true
    }

    /// Write silence to all collected output channels, for a block the
    /// plugin does not process.
    ///
    /// # Safety
    ///
    /// Every collected output pointer must be valid for writes of
    /// `num_samples` samples, and no reference to that memory may be alive.
    pub unsafe fn silence_outputs(&self, num_samples: usize) {
        for &output in self.main_outputs.iter().chain(self.aux_outputs.iter().flatten()) {
            // SAFETY: See the function's safety section.
            unsafe { std::ptr::write_bytes(output, 0, num_samples) };
        }
    }

    /// The buffers of a sub-block of samples `start..start + len`, for
    /// `process()`.
    ///
    /// # Safety
    ///
    /// [`prepare_in_place()`](Self::prepare_in_place) returned true for this
    /// process call, `start + len` is at most its `num_samples`, and no other
    /// reference to the channels' memory is alive while the buffers are.
    pub unsafe fn sub_block_buffers(
        &self,
        start: usize,
        len: usize,
    ) -> (Buffer<'_, S>, AuxiliaryBuffers<'_, S>) {
        // After prepare_in_place no two of these slices overlap, and the
        // unpaired main inputs and the aux inputs overlap no output.
        let channels = self.main_outputs.iter().map(|&ptr| {
            // SAFETY: See the function's safety section.
            unsafe { std::slice::from_raw_parts_mut(ptr.add(start), len) }
        });
        let unpaired_inputs = self.main_inputs.iter().skip(self.main_outputs.len()).map(|&ptr| {
            // SAFETY: See the function's safety section.
            unsafe { std::slice::from_raw_parts(ptr.add(start), len) }
        });
        let aux_inputs = self.aux_inputs.iter().map(|bus| {
            bus.iter().map(|&ptr| {
                // SAFETY: See the function's safety section.
                unsafe { std::slice::from_raw_parts(ptr.add(start), len) }
            })
        });
        let aux_outputs = self.aux_outputs.iter().map(|bus| {
            bus.iter().map(|&ptr| {
                // SAFETY: See the function's safety section.
                unsafe { std::slice::from_raw_parts_mut(ptr.add(start), len) }
            })
        });

        (
            Buffer::new(channels, unpaired_inputs, self.main_inputs.len(), len),
            AuxiliaryBuffers::new(aux_inputs, aux_outputs, len),
        )
    }

    /// Number of channels of output bus `bus`: 0 is the main bus, 1 and up
    /// the aux buses.
    fn output_bus_len(&self, bus: usize) -> usize {
        if bus == 0 {
            self.main_outputs.len()
        } else {
            self.aux_outputs[bus - 1].len()
        }
    }

    fn output_at(&self, bus: usize, channel: usize) -> *mut S {
        if bus == 0 {
            self.main_outputs[channel]
        } else {
            self.aux_outputs[bus - 1][channel]
        }
    }

    fn set_output_at(&mut self, bus: usize, channel: usize, ptr: *mut S) {
        if bus == 0 {
            self.main_outputs[channel] = ptr;
        } else {
            self.aux_outputs[bus - 1][channel] = ptr;
        }
    }

    /// Number of channels of input bus `bus`: 0 is the main bus, 1 and up
    /// the aux buses.
    fn input_bus_len(&self, bus: usize) -> usize {
        if bus == 0 {
            self.main_inputs.len()
        } else {
            self.aux_inputs[bus - 1].len()
        }
    }

    fn input_at(&self, bus: usize, channel: usize) -> *const S {
        if bus == 0 {
            self.main_inputs[channel]
        } else {
            self.aux_inputs[bus - 1][channel]
        }
    }

    fn set_input_at(&mut self, bus: usize, channel: usize, ptr: *const S) {
        if bus == 0 {
            self.main_inputs[channel] = ptr;
        } else {
            self.aux_inputs[bus - 1][channel] = ptr;
        }
    }

    /// Whether `num_samples` samples at `ptr` overlap an output channel that
    /// comes before channel `channel` of output bus `bus`.
    fn overlaps_output_before(&self, ptr: *mut S, num_samples: usize, bus: usize, channel: usize) -> bool {
        (0..=bus).any(|earlier_bus| {
            let end = if earlier_bus == bus { channel } else { self.output_bus_len(earlier_bus) };
            (0..end).any(|earlier| {
                ranges_overlap(ptr, self.output_at(earlier_bus, earlier), num_samples)
            })
        })
    }

    /// Whether `num_samples` samples at `ptr` overlap any output channel.
    fn overlaps_any_output(&self, ptr: *const S, num_samples: usize) -> bool {
        self.main_outputs
            .iter()
            .chain(self.aux_outputs.iter().flatten())
            .any(|&output| ranges_overlap(ptr, output, num_samples))
    }
}

/// Whether `len` samples at `a` and `len` samples at `b` share memory.
fn ranges_overlap<S>(a: *const S, b: *const S, len: usize) -> bool {
    let bytes = len * std::mem::size_of::<S>();
    let (a, b) = (a as usize, b as usize);
    a < b.saturating_add(bytes) && b < a.saturating_add(bytes)
}

impl<S: Sample> Default for ProcessBufferStorage<S> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: The raw pointers are only used within a single render call
// where the host guarantees single-threaded access.
unsafe impl<S: Sample> Send for ProcessBufferStorage<S> {}
// SAFETY: The raw pointers are only used within a single render call
// where the host guarantees single-threaded access.
unsafe impl<S: Sample> Sync for ProcessBufferStorage<S> {}

#[cfg(test)]
#[allow(clippy::undocumented_unsafe_blocks)]
mod tests {
    use super::*;
    use crate::bus_config::CachedBusInfo;
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
    fn test_effect_internal_buffers_allocated() {
        // Effects get internal buffers too, for a host that provides null
        // output pointers with an input of another channel count.
        let config = CachedBusConfig::default(); // 2in/2out

        let storage: ProcessBufferStorage<f32> =
            ProcessBufferStorage::allocate_from_config(&config, 512);

        let internal = storage.internal_output_buffers.as_ref().unwrap();
        assert_eq!(internal.len(), 2);
        assert_eq!(internal[0].len(), 512);
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

    // =========================================================================
    // In-place processing
    // =========================================================================

    const FRAMES: usize = 8;

    /// Storage for a plugin with the given main and aux channel counts.
    fn storage_for(
        main_in: usize,
        main_out: usize,
        aux_in: &[usize],
        aux_out: &[usize],
    ) -> ProcessBufferStorage<f32> {
        let mut inputs = vec![CachedBusInfo::new(main_in, BusType::Main)];
        inputs.extend(aux_in.iter().map(|&channels| CachedBusInfo::new(channels, BusType::Aux)));
        let mut outputs = vec![CachedBusInfo::new(main_out, BusType::Main)];
        outputs.extend(aux_out.iter().map(|&channels| CachedBusInfo::new(channels, BusType::Aux)));
        ProcessBufferStorage::allocate_from_config(&CachedBusConfig::new(inputs, outputs), FRAMES)
    }

    /// Samples 0, 1, 2 ... plus `base`, to tell channels apart.
    fn ramp(base: f32) -> Vec<f32> {
        (0..FRAMES).map(|i| base + i as f32).collect()
    }

    /// The main channels and unpaired inputs the plugin sees.
    fn seen(storage: &ProcessBufferStorage<f32>) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
        let (buffer, _aux) = unsafe { storage.sub_block_buffers(0, FRAMES) };
        (
            buffer.channels().map(<[f32]>::to_vec).collect(),
            buffer.unpaired_inputs().map(<[f32]>::to_vec).collect(),
        )
    }

    /// Whether no two of the plugin's slices share memory, and no input
    /// slice overlaps an output.
    fn no_overlaps(storage: &ProcessBufferStorage<f32>) -> bool {
        let outputs: Vec<*const f32> = storage
            .main_outputs
            .iter()
            .chain(storage.aux_outputs.iter().flatten())
            .map(|&ptr| ptr as *const f32)
            .collect();
        let inputs: Vec<*const f32> = storage
            .main_inputs
            .iter()
            .skip(storage.main_outputs.len())
            .chain(storage.aux_inputs.iter().flatten())
            .copied()
            .collect();
        let outputs_apart = outputs.iter().enumerate().all(|(i, &a)| {
            outputs[..i].iter().all(|&b| !ranges_overlap(a, b, FRAMES))
        });
        let inputs_apart = inputs
            .iter()
            .all(|&input| outputs.iter().all(|&output| !ranges_overlap(input, output, FRAMES)));
        outputs_apart && inputs_apart
    }

    #[test]
    fn test_prepare_copies_separate_input_into_output() {
        let mut storage = storage_for(2, 2, &[], &[]);
        let inputs = [ramp(10.0), ramp(20.0)];
        let mut outputs = [vec![0.5; FRAMES], vec![0.5; FRAMES]];
        unsafe {
            for input in &inputs {
                storage.push_main_input(input.as_ptr());
            }
            for output in &mut outputs {
                storage.push_main_output(output.as_mut_ptr());
            }
            assert!(storage.prepare_in_place(FRAMES));
        }

        assert_eq!(seen(&storage), (inputs.to_vec(), vec![]));
        assert_eq!(outputs, inputs);
    }

    #[test]
    fn test_prepare_keeps_shared_input_and_output_in_place() {
        let mut storage = storage_for(2, 2, &[], &[]);
        let mut channels = [ramp(10.0), ramp(20.0)];
        unsafe {
            for channel in &mut channels {
                storage.push_main_input(channel.as_ptr());
                storage.push_main_output(channel.as_mut_ptr());
            }
            assert!(storage.prepare_in_place(FRAMES));
        }

        // The plugin works in the host's memory, which holds the input.
        assert_eq!(storage.main_outputs[0], channels[0].as_mut_ptr());
        assert_eq!(seen(&storage).0, vec![ramp(10.0), ramp(20.0)]);
    }

    #[test]
    fn test_prepare_handles_crossed_channels() {
        // Input 0 is in output 1's memory and input 1 in output 0's.
        let mut storage = storage_for(2, 2, &[], &[]);
        let mut memory = [ramp(10.0), ramp(20.0)];
        unsafe {
            storage.push_main_input(memory[1].as_ptr());
            storage.push_main_input(memory[0].as_ptr());
            storage.push_main_output(memory[0].as_mut_ptr());
            storage.push_main_output(memory[1].as_mut_ptr());
            assert!(storage.prepare_in_place(FRAMES));
        }

        assert_eq!(memory, [ramp(20.0), ramp(10.0)]);
        assert!(no_overlaps(&storage));
    }

    #[test]
    fn test_prepare_handles_a_partly_overlapping_input() {
        // The input starts one sample into the output's memory.
        let mut storage = storage_for(1, 1, &[], &[]);
        let mut memory = ramp(0.0);
        memory.extend([100.0]);
        let expected: Vec<f32> = memory[1..].to_vec();
        unsafe {
            storage.push_main_input(memory.as_ptr().add(1));
            storage.push_main_output(memory.as_mut_ptr());
            assert!(storage.prepare_in_place(FRAMES));
        }

        assert_eq!(seen(&storage).0, vec![expected]);
        assert!(no_overlaps(&storage));
    }

    #[test]
    fn test_prepare_gives_a_repeated_output_memory_of_its_own() {
        // The host gives both outputs the same memory. The second one gets
        // scratch, so that the plugin holds two separate channels.
        let mut storage = storage_for(2, 2, &[], &[]);
        let inputs = [ramp(10.0), ramp(20.0)];
        let mut output = vec![0.0; FRAMES];
        unsafe {
            for input in &inputs {
                storage.push_main_input(input.as_ptr());
            }
            storage.push_main_output(output.as_mut_ptr());
            storage.push_main_output(output.as_mut_ptr());
            assert!(storage.prepare_in_place(FRAMES));
        }

        assert_eq!(seen(&storage).0, inputs.to_vec());
        assert_eq!(output, inputs[0]);
        assert!(no_overlaps(&storage));
    }

    #[test]
    fn test_prepare_copies_an_unpaired_input_over_an_output() {
        // Stereo to mono, and the host gives the output the right input's
        // memory. The right input is copied before the output is written.
        let mut storage = storage_for(2, 1, &[], &[]);
        let left = ramp(10.0);
        let mut right = ramp(20.0);
        unsafe {
            storage.push_main_input(left.as_ptr());
            storage.push_main_input(right.as_ptr());
            storage.push_main_output(right.as_mut_ptr());
            assert!(storage.prepare_in_place(FRAMES));
        }

        assert_eq!(seen(&storage), (vec![ramp(10.0)], vec![ramp(20.0)]));
        assert!(no_overlaps(&storage));

        let (buffer, _aux) = unsafe { storage.sub_block_buffers(0, FRAMES) };
        assert_eq!(buffer.num_input_channels(), 2);
        assert_eq!(buffer.unpaired_input(1), &ramp(20.0)[..]);
        assert!(buffer.unpaired_input(0).is_empty());
    }

    #[test]
    fn test_prepare_silences_outputs_without_input() {
        // Mono to stereo: the second channel starts silent.
        let mut storage = storage_for(1, 2, &[], &[]);
        let input = ramp(10.0);
        let mut outputs = [vec![0.5; FRAMES], vec![0.5; FRAMES]];
        unsafe {
            storage.push_main_input(input.as_ptr());
            for output in &mut outputs {
                storage.push_main_output(output.as_mut_ptr());
            }
            assert!(storage.prepare_in_place(FRAMES));
        }

        assert_eq!(seen(&storage).0, vec![ramp(10.0), vec![0.0; FRAMES]]);
    }

    #[test]
    fn test_prepare_copies_aux_input_over_an_output_and_silences_aux_outputs() {
        let mut storage = storage_for(1, 1, &[1], &[1]);
        let main_input = ramp(10.0);
        let mut main_output = ramp(20.0);
        let mut aux_output = vec![0.5; FRAMES];
        unsafe {
            storage.push_main_input(main_input.as_ptr());
            storage.push_main_output(main_output.as_mut_ptr());
            // The sidechain lies in the main output's memory.
            storage.push_aux_input(0, main_output.as_ptr());
            storage.push_aux_output(0, aux_output.as_mut_ptr());
            assert!(storage.prepare_in_place(FRAMES));
        }

        assert!(no_overlaps(&storage));
        let (mut buffer, mut aux) = unsafe { storage.sub_block_buffers(0, FRAMES) };
        assert_eq!(buffer.channel(0), &ramp(10.0)[..]);
        assert_eq!(aux.sidechain().unwrap().input(0), &ramp(20.0)[..]);
        assert_eq!(aux.output(0).unwrap().output(0), &[0.0; FRAMES][..]);
    }

    #[test]
    fn test_prepare_refuses_a_too_long_block_only_when_it_needs_scratch() {
        let long = FRAMES + 1;

        // Separate buffers need no scratch: the block is prepared.
        let mut storage = storage_for(1, 1, &[], &[]);
        let input = vec![1.0; long];
        let mut output = vec![0.0; long];
        unsafe {
            storage.push_main_input(input.as_ptr());
            storage.push_main_output(output.as_mut_ptr());
            assert!(storage.prepare_in_place(long));
        }
        assert_eq!(output, input);

        // A repeated output needs scratch, which is too short.
        let mut storage = storage_for(1, 2, &[], &[]);
        let mut output = vec![0.5; long];
        unsafe {
            storage.push_main_input(input.as_ptr());
            storage.push_main_output(output.as_mut_ptr());
            storage.push_main_output(output.as_mut_ptr());
            assert!(!storage.prepare_in_place(long));
            storage.silence_outputs(long);
        }
        assert_eq!(output, vec![0.0; long]);
    }

}
