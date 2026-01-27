# Buffer Storage Memory Optimization

## Overview

`ProcessBufferStorage` in `beamer-core` provides pre-allocated buffer storage shared by both VST3 and Audio Unit wrappers. The implementation uses **config-based allocation** instead of worst-case pre-allocation, reducing memory usage while maintaining the zero-allocation render path guarantee.

## Architecture

```
beamer-core
  ProcessBufferStorage<S>     # Shared struct + allocation logic
       |
  +----+----+
  |         |
beamer-au   beamer-vst3
  ProcessBufferStorageAuExt   # AU-specific pointer collection
  (extension trait)           # (from AudioBufferList)
```

Format wrappers use the shared struct directly. AU provides an extension trait for collecting pointers from `AudioBufferList`.

## Optimization Strategies

### 1. Smart Channel Allocation
**Before:** Would need to allocate for MAX_CHANNELS (32) per bus in worst-case designs
**After:** Allocates exact number of channels from `CachedBusConfig`

Example:
- Mono plugin (1in/1out): Allocates 1+1=2 pointers (16 bytes)
- Stereo plugin (2in/2out): Allocates 2+2=4 pointers (32 bytes)
- NOT: 32+32=64 pointers (512 bytes) worst-case

### 2. Lazy Aux Bus Allocation
**Before:** Would pre-allocate Vec containers even for plugins without aux buses
**After:** Only allocates aux bus Vecs when `aux_bus_count > 0`

```rust
// Old approach (always allocates)
let mut aux_inputs = Vec::with_capacity(aux_in_buses); // Even if aux_in_buses == 0

// New approach (lazy allocation)
let aux_inputs = if aux_in_buses > 0 {
    let mut vec = Vec::with_capacity(aux_in_buses);
    // ... allocate per-bus Vecs
    vec
} else {
    Vec::new() // Zero heap allocation
};
```

For simple mono/stereo plugins without aux buses:
- `aux_inputs` and `aux_outputs` use `Vec::new()` (zero capacity, zero heap allocation)
- Eliminates 2 heap allocations (outer Vec containers)

### 3. Per-Bus Channel Count Optimization
**Before:** If using uniform allocation, all aux buses would get same channel count
**After:** Each aux bus allocates exactly its declared channel count

Example config:
- Main input: 2 channels
- Aux input 1 (sidechain): 1 channel (mono)
- Aux input 2: 4 channels (quad)

Storage allocation:
- `main_inputs`: capacity = 2
- `aux_inputs[0]`: capacity = 1
- `aux_inputs[1]`: capacity = 4

Total: 7 pointers instead of worst-case 64 pointers (9x memory saving)

### 4. Asymmetric Bus Support
**Before:** Some designs might over-allocate to match input/output symmetrically
**After:** Input and output channels are independently allocated

Example (mono-to-stereo effect):
- Input: 1 channel
- Output: 2 channels
- Allocates: 1 input pointer + 2 output pointers = 3 pointers (24 bytes)
- NOT: 2+2 = 4 pointers (would waste 8 bytes)

### 5. Internal Output Buffers for Instruments
Instruments (plugins with no inputs) may need internal output buffers because some hosts (Logic Pro, Reaper) provide null output pointers expecting the plugin to supply its own buffers.

```rust
// Only allocated for instruments (0 inputs, >0 outputs)
internal_output_buffers: Option<Vec<Vec<S>>>,
max_frames: usize,
```

## Memory Savings

| Plugin Type | Old (Worst-Case) | New (Optimized) | Savings |
|-------------|------------------|-----------------|---------|
| Mono (1in/1out) | 512 bytes | 16 bytes | **32x** |
| Stereo (2in/2out) | 512 bytes | 32 bytes | **16x** |
| Stereo + Sidechain (2+2in/2out) | 512 bytes | 48 bytes | **10.7x** |
| Surround 5.1 (6in/6out) | 512 bytes | 96 bytes | **5.3x** |

*Note: Worst-case assumes MAX_CHANNELS (32) x 2 directions x pointer size (8 bytes) = 512 bytes*

## Real-Time Safety Guarantees

The optimization maintains all real-time safety guarantees:

### Zero Allocations in Render Path
- All allocations happen in `allocate_from_config()` (called during `setupProcessing`)
- `clear()` is O(1) - only sets Vec lengths to 0, no deallocation
- `push()` never allocates - capacity is pre-reserved
- No heap operations during audio callbacks

### Validated Tests
All operations are validated by comprehensive tests in `beamer-core`:
- `test_allocate_from_config_stereo` - verifies stereo plugin allocation
- `test_allocate_from_config_mono` - verifies mono plugin uses minimal memory
- `test_allocate_from_config_with_aux` - verifies aux bus allocation
- `test_clear_maintains_capacity` - verifies O(1) clear operation
- `test_instrument_internal_buffers_allocated` - verifies internal buffers for instruments
- `test_effect_no_internal_buffers` - verifies effects don't allocate internal buffers

And additional tests in `beamer-au` for the extension trait.

## Testing

```bash
# Test shared implementation
cargo test -p beamer-core buffer_storage

# Test AU-specific extensions
cargo test -p beamer-au buffer_storage
```

## Compatibility

The optimization is **100% backward compatible**:
- Same API surface - no breaking changes
- Same behavior - validates bus limits identically
- Same guarantees - maintains zero-allocation render path
- Existing code continues to work without changes

The optimization is transparent to users of `ProcessBufferStorage` - they get automatic memory savings without any code changes.

## Summary

This optimization achieves **up to 32x memory reduction** for simple plugins while maintaining:
- Zero allocations in render path
- O(1) clear operations
- Real-time safety guarantees
- Backward compatibility
- Comprehensive test coverage

The implementation is shared between VST3 and Audio Unit formats via `beamer-core`, ensuring consistency across plugin formats while optimizing for the actual plugin configuration rather than worst-case scenarios.
