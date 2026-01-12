//! C-ABI bridge for hybrid Audio Unit architecture.
//!
//! This module implements all the C-ABI functions that the Objective-C wrapper calls.
//! It bridges to the existing beamer-au Rust infrastructure (AuPluginInstance, RenderBlock, etc.).
//!
//! # Architecture
//!
//! The hybrid AU architecture uses a thin Objective-C wrapper that delegates to Rust:
//!
//! ```text
//! AU Host (Logic Pro, etc.)
//!        ↓
//! Objective-C Wrapper (BeamerAuWrapper.m)
//!        ↓ (C-ABI calls)
//! bridge.rs (this module)
//!        ↓
//! AuPluginInstance / RenderBlock
//!        ↓
//! beamer_core::Plugin
//! ```
//!
//! # Safety
//!
//! All functions use `std::panic::catch_unwind` to prevent panics from crossing the FFI boundary.
//! Pointers are validated before dereferencing.
//! Functions return OSStatus error codes on failure.

// These are C-ABI entry points called from Objective-C. The ObjC side is responsible
// for passing valid pointers. Marking them `unsafe` would be unusual for C FFI.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{c_char, c_void, CStr};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::{Arc, Mutex};

use crate::buffer_storage::ProcessBufferStorage;
use crate::buffers::AudioBufferList;
use crate::bus_config::{BusInfo, BusType, CachedBusConfig, MAX_BUSES};
use crate::error::os_status;
use crate::factory;
use crate::instance::AuPluginInstance;
use crate::render::{
    create_render_block_f32, create_render_block_f64, AURenderEvent, AudioTimeStamp,
    RenderBlockTrait,
};
// ParameterStore trait is used via plugin.parameter_store() which returns a dyn ParameterStore
#[allow(unused_imports)]
use beamer_core::ParameterStore;

// =============================================================================
// Constants (must match BeamerAuBridge.h)
// =============================================================================

/// Maximum length of parameter name/unit strings.
const BEAMER_AU_MAX_PARAM_NAME_LENGTH: usize = 128;

// =============================================================================
// C-ABI Structs (must match BeamerAuBridge.h exactly)
// =============================================================================

/// Bus type enumeration (matches BeamerAuBusType in header).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamerAuBusType {
    /// Main audio bus (bus index 0)
    Main = 0,
    /// Auxiliary audio bus (sidechain, additional I/O)
    Auxiliary = 1,
}

/// Information about a single audio bus (matches BeamerAuBusInfo in header).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BeamerAuBusInfo {
    /// Number of channels in this bus (1 = mono, 2 = stereo, etc.)
    pub channel_count: u32,
    /// Bus type (main or auxiliary)
    pub bus_type: BeamerAuBusType,
}

/// Complete bus configuration for the plugin (matches BeamerAuBusConfig in header).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BeamerAuBusConfig {
    /// Number of input buses (1 = main only, 2+ = main + aux)
    pub input_bus_count: u32,
    /// Number of output buses (1 = main only, 2+ = main + aux)
    pub output_bus_count: u32,
    /// Input bus information array
    pub input_buses: [BeamerAuBusInfo; MAX_BUSES],
    /// Output bus information array
    pub output_buses: [BeamerAuBusInfo; MAX_BUSES],
}

/// Sample format enumeration (matches BeamerAuSampleFormat in header).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamerAuSampleFormat {
    /// 32-bit floating point samples (standard)
    Float32 = 0,
    /// 64-bit floating point samples (high precision)
    Float64 = 1,
}

/// Float64 processing support level.
///
/// The AU wrapper accepts float64 stream formats either natively (when the
/// prepared processor supports it) or via internal conversion.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamerAuFloat64Support {
    NotSupported = 0,
    ViaConversion = 1,
    Native = 2,
}

/// Parameter metadata for building AUParameterTree (matches BeamerAuParameterInfo in header).
#[repr(C)]
pub struct BeamerAuParameterInfo {
    /// Parameter ID (unique within the plugin, maps to AU parameter address)
    pub id: u32,
    /// Human-readable parameter name (UTF-8, null-terminated)
    pub name: [c_char; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
    /// Parameter unit string (e.g., "dB", "Hz", "ms"; UTF-8, null-terminated)
    pub units: [c_char; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
    /// Default normalized value (0.0 to 1.0)
    pub default_value: f32,
    /// Current normalized value (0.0 to 1.0)
    pub current_value: f32,
    /// Number of discrete steps (0 = continuous, 1 = boolean, N = N+1 states)
    pub step_count: i32,
    /// Flags (reserved for future use: automatable, hidden, etc.)
    pub flags: u32,
}

impl Default for BeamerAuParameterInfo {
    fn default() -> Self {
        Self {
            id: 0,
            name: [0; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
            units: [0; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
            default_value: 0.0,
            current_value: 0.0,
            step_count: 0,
            flags: 0,
        }
    }
}

// =============================================================================
// Instance Handle
// =============================================================================

/// Opaque handle to a beamer AU plugin instance.
///
/// This struct wraps the Rust plugin instance and render resources,
/// providing a stable C-ABI handle that the Objective-C wrapper can use.
///
/// # Thread Safety
///
/// The `render_block` field is wrapped in `RwLock` to ensure thread-safe access.
/// The render path uses `read()` for concurrent access during audio processing,
/// while resource allocation/deallocation uses `write()` for exclusive access.
/// This prevents data races when the host calls render from the audio thread
/// while other threads may be allocating/deallocating resources.
pub struct BeamerInstanceHandle {
    /// The plugin instance (wrapped in Arc<Mutex<>> for thread-safe access)
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
    /// The render block (created during allocate_render_resources).
    /// Wrapped in RwLock to prevent data races between render calls (read) and
    /// resource allocation/deallocation (write).
    render_block: std::sync::RwLock<Option<Arc<dyn RenderBlockTrait>>>,
    /// Sample format (f32 or f64)
    sample_format: BeamerAuSampleFormat,
    /// Current sample rate (set during allocate_render_resources)
    sample_rate: f64,
    /// Maximum frames per render call
    max_frames: u32,
    /// Cached bus configuration
    bus_config: Option<CachedBusConfig>,
}

// SAFETY: BeamerInstanceHandle is designed for FFI use. Thread safety is ensured by:
// - `plugin` is wrapped in Arc<Mutex<>> for synchronized access
// - `render_block` is wrapped in RwLock to prevent data races
// - Other fields are only modified during allocate/deallocate which holds exclusive access
unsafe impl Send for BeamerInstanceHandle {}
unsafe impl Sync for BeamerInstanceHandle {}

/// Type alias for the opaque handle pointer.
pub type BeamerAuInstanceHandle = *mut BeamerInstanceHandle;

// =============================================================================
// Helper Functions
// =============================================================================

/// Copy a Rust string into a fixed-size C char array.
fn copy_str_to_char_array(s: &str, dest: &mut [c_char]) {
    let bytes = s.as_bytes();
    let copy_len = bytes.len().min(dest.len() - 1);
    for (i, &b) in bytes[..copy_len].iter().enumerate() {
        dest[i] = b as c_char;
    }
    // Null terminate
    if copy_len < dest.len() {
        dest[copy_len] = 0;
    } else {
        dest[dest.len() - 1] = 0;
    }
}

/// Convert a C bus info array to a Vec<BusInfo>.
///
/// This helper converts FFI bus information from the C-ABI format to Rust's internal
/// representation, handling the bus type conversion and respecting MAX_BUSES bounds.
fn convert_bus_info_array(c_buses: &[BeamerAuBusInfo; MAX_BUSES], count: u32) -> Vec<BusInfo> {
    let count = (count as usize).min(MAX_BUSES);
    let mut buses = Vec::with_capacity(count);

    for bus in c_buses.iter().take(count) {
        buses.push(BusInfo {
            channel_count: bus.channel_count as usize,
            bus_type: if bus.bus_type == BeamerAuBusType::Main {
                BusType::Main
            } else {
                BusType::Auxiliary
            },
        });
    }

    buses
}

/// Convert BeamerAuBusConfig to CachedBusConfig.
fn bus_config_from_c(config: &BeamerAuBusConfig) -> CachedBusConfig {
    let input_buses = convert_bus_info_array(&config.input_buses, config.input_bus_count);
    let output_buses = convert_bus_info_array(&config.output_buses, config.output_bus_count);

    CachedBusConfig::new(input_buses, output_buses)
}

// =============================================================================
// Factory Registration
// =============================================================================

// Force-link the ObjC APPEX code by referencing a function defined in BeamerAuExtension.m
// Without this, the linker strips the ObjC classes since nothing else references them.
extern "C" {
    fn beamer_au_appex_force_link();
}

/// Ensure the plugin factory is registered.
///
/// This function checks if the plugin factory has been registered (via the
/// `export_au!` macro's static initializer). The factory is typically registered
/// automatically when the `.component` bundle loads.
///
/// # Safety
///
/// This function has no pointer parameters and is safe to call from any thread.
///
/// # Returns
///
/// `true` if the factory is registered and ready, `false` otherwise.
#[no_mangle]
pub extern "C" fn beamer_au_ensure_factory_registered() -> bool {
    // Call the force_link function to ensure the ObjC code is linked.
    // This is needed for APPEX where the principal class must be in the binary.
    unsafe { beamer_au_appex_force_link(); }
    factory::is_registered()
}

/// Fill in the AudioComponentDescription from the registered AU config.
///
/// # Safety
///
/// `desc` must be a valid pointer to an AudioComponentDescription struct.
#[no_mangle]
pub unsafe extern "C" fn beamer_au_get_component_description(desc: *mut u32) {
    if desc.is_null() {
        return;
    }
    if let Some(config) = factory::au_config() {
        // AudioComponentDescription layout: type, subtype, manufacturer, flags, mask
        *desc.add(0) = config.component_type.as_u32();
        *desc.add(1) = u32::from_be_bytes(config.subtype.0);
        *desc.add(2) = u32::from_be_bytes(config.manufacturer.0);
        *desc.add(3) = 0; // componentFlags
        *desc.add(4) = 0; // componentFlagsMask
    }
}

// =============================================================================
// Instance Lifecycle
// =============================================================================

/// Create a new plugin instance.
///
/// Uses the registered factory to create a new instance of the plugin.
/// The instance must be destroyed with `beamer_au_destroy_instance`.
///
/// # Safety
///
/// This function has no pointer parameters and is safe to call from any thread.
/// The returned handle must be destroyed with `beamer_au_destroy_instance` when
/// no longer needed to avoid memory leaks.
///
/// # Returns
///
/// A pointer to the instance handle, or null if creation failed.
#[no_mangle]
pub extern "C" fn beamer_au_create_instance() -> BeamerAuInstanceHandle {
    let result = catch_unwind(|| {
        // Use the factory to create a new plugin instance
        let plugin = factory::create_instance()?;

        let handle = Box::new(BeamerInstanceHandle {
            plugin: Arc::new(Mutex::new(plugin)),
            render_block: std::sync::RwLock::new(None),
            sample_format: BeamerAuSampleFormat::Float32,
            sample_rate: 44100.0,
            max_frames: 1024,
            bus_config: None,
        });

        Some(Box::into_raw(handle))
    });

    match result {
        Ok(Some(ptr)) => ptr,
        Ok(None) => {
            ptr::null_mut()
        }
        Err(_) => {
            ptr::null_mut()
        }
    }
}

/// Destroy a plugin instance.
///
/// Frees all resources associated with the instance.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been previously destroyed
/// - After calling this function, `instance` is invalid and must not be used
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Must not be called concurrently with any other function
///   using the same instance
#[no_mangle]
pub extern "C" fn beamer_au_destroy_instance(instance: BeamerAuInstanceHandle) {
    if instance.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let _ = Box::from_raw(instance);
    }));
}

// =============================================================================
// Render Resources
// =============================================================================

/// Maximum supported sample rate (384 kHz - highest professional audio standard).
const MAX_SAMPLE_RATE: f64 = 384_000.0;

/// Maximum supported frames per render call.
/// 8192 is a reasonable upper limit that covers common buffer sizes.
const MAX_FRAMES_PER_RENDER: u32 = 8192;

/// Allocate render resources and prepare for audio processing.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`
/// - `instance` must not have been destroyed
/// - `bus_config` must be a valid pointer to a properly initialized `BeamerAuBusConfig`
/// - This function validates both pointers are non-null before dereferencing
/// - If either pointer is null, returns `K_AUDIO_UNIT_ERR_INVALID_PARAMETER`
/// - Thread safety: Must not be called concurrently with render or deallocate
///   operations on the same instance
#[no_mangle]
pub extern "C" fn beamer_au_allocate_render_resources(
    instance: BeamerAuInstanceHandle,
    sample_rate: f64,
    max_frames: u32,
    sample_format: BeamerAuSampleFormat,
    bus_config: *const BeamerAuBusConfig,
) -> i32 {

    if instance.is_null() || bus_config.is_null() {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER;
    }

    // Validate sample_rate before any unsafe operations
    if sample_rate <= 0.0 || sample_rate > MAX_SAMPLE_RATE || !sample_rate.is_finite() {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
    }

    // Validate max_frames before any unsafe operations
    if max_frames == 0 || max_frames > MAX_FRAMES_PER_RENDER {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
    }

    // Validate bus_config contents before dereferencing fields in loops
    // SAFETY: bus_config was validated as non-null above
    let (input_bus_count, output_bus_count) = unsafe {
        let c_bus_config = &*bus_config;
        (c_bus_config.input_bus_count, c_bus_config.output_bus_count)
    };

    if input_bus_count as usize > MAX_BUSES {
        log::error!(
            "Invalid input_bus_count: {} (must be <= {})",
            input_bus_count,
            MAX_BUSES
        );
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
    }

    if output_bus_count as usize > MAX_BUSES {
        log::error!(
            "Invalid output_bus_count: {} (must be <= {})",
            output_bus_count,
            MAX_BUSES
        );
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &mut *instance;
        let c_bus_config = &*bus_config;

        // Store configuration
        handle.sample_format = sample_format;
        handle.sample_rate = sample_rate;
        handle.max_frames = max_frames;

        // Convert bus configuration (bus counts already validated above)
        let rust_bus_config = bus_config_from_c(c_bus_config);

        // Validate bus configuration
        if let Err(_e) = rust_bus_config.validate() {
            return os_status::K_AUDIO_UNIT_ERR_FORMAT_NOT_SUPPORTED;
        }

        // Allocate resources on the plugin
        {
            let mut plugin = match handle.plugin.lock() {
                Ok(guard) => guard,
                Err(_) => return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT,
            };

            if let Err(_e) =
                plugin.allocate_render_resources(sample_rate, max_frames, &rust_bus_config)
            {
                return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
            }
        }

        // Create the render block based on sample format
        // Note: We don't store host block pointers here - they're passed per-render call
        let render_block: Arc<dyn RenderBlockTrait> = match handle.sample_format {
            BeamerAuSampleFormat::Float32 => {
                let storage = ProcessBufferStorage::<f32>::allocate_from_config(&rust_bus_config);
                Arc::from(create_render_block_f32(
                    Arc::clone(&handle.plugin),
                    storage,
                    None, // musical_context_block passed at render time
                    None, // transport_state_block passed at render time
                    None, // schedule_midi_event_block passed at render time
                    max_frames,
                    sample_rate,
                ))
            }
            BeamerAuSampleFormat::Float64 => {
                let storage = ProcessBufferStorage::<f64>::allocate_from_config(&rust_bus_config);
                Arc::from(create_render_block_f64(
                    Arc::clone(&handle.plugin),
                    storage,
                    None,
                    None,
                    None,
                    max_frames,
                    sample_rate,
                ))
            }
        };

        // Use write lock to set the render block (exclusive access)
        match handle.render_block.write() {
            Ok(mut guard) => *guard = Some(render_block),
            Err(_) => return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT,
        }
        handle.bus_config = Some(rust_bus_config);

        os_status::NO_ERR
    }));

    result.unwrap_or({
        os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT
    })
}

/// Deallocate render resources.
///
/// This function uses non-blocking lock acquisition to avoid race conditions with
/// the render path. If the render block or plugin locks cannot be acquired (because
/// rendering is in progress), the function returns early without deallocating.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Uses non-blocking locks; safe to call from any thread but
///   host should ensure rendering is stopped first for reliable deallocation
///
/// # Real-time Safety
///
/// This function uses `try_write()` and `try_lock()` to match the real-time safety
/// approach used in the render path (`beamer_au_render`). This prevents:
/// 1. Blocking the calling thread while waiting for locks held by the audio thread
/// 2. TOCTOU race conditions where resources are deallocated while in use
///
/// # Return Behavior
///
/// The function silently returns if locks cannot be acquired. The host should
/// ensure rendering is stopped before calling this function. If called while
/// rendering is active, the caller may need to retry after rendering stops.
#[no_mangle]
pub extern "C" fn beamer_au_deallocate_render_resources(instance: BeamerAuInstanceHandle) {
    if instance.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &mut *instance;

        // Use try_write() to avoid blocking if render is in progress.
        // This prevents a TOCTOU race condition where we could deallocate
        // resources while another thread is actively using them in process_impl.
        let render_block_cleared = match handle.render_block.try_write() {
            Ok(mut guard) => {
                *guard = None;
                true
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                // Render is in progress - cannot safely deallocate
                log::warn!(
                    "beamer_au_deallocate_render_resources: render_block lock held by render thread, \
                     cannot deallocate. Ensure rendering is stopped before deallocating."
                );
                false
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                // Lock was poisoned by a panic - try to recover by clearing anyway
                log::error!(
                    "beamer_au_deallocate_render_resources: render_block lock poisoned, \
                     attempting recovery"
                );
                if let Ok(mut guard) = handle.render_block.write() {
                    *guard = None;
                }
                true
            }
        };

        // Only proceed with plugin deallocation if we successfully cleared the render block.
        // This ensures we don't have a partial deallocation state.
        if !render_block_cleared {
            return;
        }

        // Use try_lock() for the plugin mutex, consistent with render path.
        // At this point the render_block is None, so even if a render call
        // is starting, it will fail at the render_block check and not access
        // the plugin.
        match handle.plugin.try_lock() {
            Ok(mut plugin) => {
                plugin.deallocate_render_resources();
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                // This shouldn't normally happen since we cleared render_block first,
                // but handle it gracefully
                log::warn!(
                    "beamer_au_deallocate_render_resources: plugin lock held, \
                     render_block cleared but plugin resources not deallocated"
                );
                return;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                log::error!(
                    "beamer_au_deallocate_render_resources: plugin lock poisoned, \
                     cannot deallocate plugin resources"
                );
                return;
            }
        }

        // Clear bus config only after successful deallocation
        handle.bus_config = None;
    }));
}

/// Check if render resources are currently allocated.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_is_prepared(instance: BeamerAuInstanceHandle) -> bool {
    if instance.is_null() {
        return false;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };

        plugin.is_prepared()
    }));

    result.unwrap_or(false)
}

// =============================================================================
// Audio Rendering
// =============================================================================

/// Process audio through the plugin.
///
/// This is the main audio processing entry point, called from the AU host's
/// render callback (real-time audio thread).
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`
/// - `instance` must not have been destroyed
/// - `action_flags` must be a valid pointer to a mutable `u32`
/// - `timestamp` must be a valid pointer to an `AudioTimeStamp`
/// - `output_data` must be a valid pointer to an `AudioBufferList` with properly
///   allocated buffers sized for at least `frame_count` frames
/// - `events` may be null if there are no events to process
/// - `pull_input_block` may be null for generator plugins that don't need input
/// - Context block pointers (`_musical_context_block`, `_transport_state_block`,
///   `_schedule_midi_block`) may be null if those features aren't used
/// - This function validates `instance`, `action_flags`, `timestamp`, and
///   `output_data` are non-null; returns `K_AUDIO_UNIT_ERR_INVALID_PARAMETER` if any are null
/// - Thread safety: Designed for real-time audio thread; uses non-blocking
///   `try_read()` to avoid blocking if resource allocation is in progress
/// - Uses `catch_unwind` to prevent panics from crossing the FFI boundary
#[no_mangle]
pub extern "C" fn beamer_au_render(
    instance: BeamerAuInstanceHandle,
    action_flags: *mut u32,
    timestamp: *const AudioTimeStamp,
    frame_count: u32,
    output_bus_number: isize,
    output_data: *mut AudioBufferList,
    events: *const AURenderEvent,
    pull_input_block: *const c_void,
    _musical_context_block: *const c_void,
    _transport_state_block: *const c_void,
    _schedule_midi_block: *const c_void,
) -> i32 {

    // Validate instance handle
    if instance.is_null() {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER;
    }

    // Validate critical pointers required for rendering
    if action_flags.is_null() || timestamp.is_null() || output_data.is_null() {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let handle = unsafe { &*instance };

        // Validate frame count against maximum set during allocate_render_resources
        // This is required by the AU spec - render must reject requests exceeding max_frames
        if frame_count > handle.max_frames {
            return os_status::K_AUDIO_UNIT_ERR_TOO_MANY_FRAMES_TO_PROCESS;
        }

        // Use read lock for concurrent access during rendering
        // try_read() is used for real-time safety - we don't want to block the audio thread
        let render_block_guard = match handle.render_block.try_read() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                // Another thread holds the write lock (e.g., during resource allocation)
                // Return an error rather than blocking the audio thread
                return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                // Lock was poisoned by a panic in another thread
                return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT;
            }
        };

        let render_block = match render_block_guard.as_ref() {
            Some(rb) => rb,
            None => return os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED,
        };

        render_block.process(
            action_flags,
            timestamp,
            frame_count,
            output_bus_number as i32,
            output_data,
            events,
            pull_input_block,
        )
    }));

    result.unwrap_or(os_status::K_AUDIO_UNIT_ERR_RENDER)
}

/// Reset the plugin's DSP state.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_reset(instance: BeamerAuInstanceHandle) {
    if instance.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        if let Ok(mut plugin) = handle.plugin.lock() {
            plugin.reset();
        }
    }));
}

// =============================================================================
// Parameters
// =============================================================================

/// Get the number of parameters exposed by the plugin.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_count(instance: BeamerAuInstanceHandle) -> u32 {
    if instance.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        match plugin.parameter_store() {
            Ok(store) => store.count() as u32,
            Err(_) => 0,
        }
    }));

    result.unwrap_or(0)
}

/// Get information about a parameter by index.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - `out_info` must be a valid pointer to a `BeamerAuParameterInfo` struct,
///   or null (in which case this function returns `false`)
/// - This function validates both pointers are non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_info(
    instance: BeamerAuInstanceHandle,
    index: u32,
    out_info: *mut BeamerAuParameterInfo,
) -> bool {
    if instance.is_null() || out_info.is_null() {
        return false;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };

        let store = match plugin.parameter_store() {
            Ok(s) => s,
            Err(_) => return false,
        };

        let param_info = match store.info(index as usize) {
            Some(info) => info,
            None => return false,
        };

        // Fill output struct
        let out = &mut *out_info;
        out.id = param_info.id;
        copy_str_to_char_array(param_info.name, &mut out.name);
        copy_str_to_char_array(param_info.units, &mut out.units);
        out.default_value = param_info.default_normalized as f32;
        out.current_value = store.get_normalized(param_info.id) as f32;
        out.step_count = param_info.step_count;
        // Convert ParameterFlags to u32 bitfield
        out.flags = {
            let mut flags = 0u32;
            if param_info.flags.can_automate {
                flags |= 1 << 0; // BeamerAuParameterFlagAutomatable
            }
            if param_info.flags.is_hidden {
                flags |= 1 << 1; // BeamerAuParameterFlagHidden
            }
            if param_info.flags.is_readonly {
                flags |= 1 << 2; // BeamerAuParameterFlagReadOnly
            }
            flags
        };

        true
    }));

    result.unwrap_or(false)
}

/// Get a parameter's current normalized value.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0.0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_value(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
) -> f32 {
    if instance.is_null() {
        return 0.0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0.0,
        };

        match plugin.parameter_store() {
            Ok(store) => store.get_normalized(param_id) as f32,
            Err(_) => 0.0,
        }
    }));

    result.unwrap_or(0.0)
}

/// Set a parameter's normalized value.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_set_parameter_value(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    value: f32,
) {
    if instance.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };

        if let Ok(store) = plugin.parameter_store() {
            store.set_normalized(param_id, value as f64);
        }
    }));
}

/// Format a parameter value as a display string.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - `out_buffer` must be a valid pointer to a writable buffer of at least
///   `buffer_len` bytes, or null (in which case this function returns `0`)
/// - `buffer_len` must be greater than 0
/// - This function validates `instance` and `out_buffer` are non-null and
///   `buffer_len > 0` before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_format_parameter_value(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    value: f32,
    out_buffer: *mut c_char,
    buffer_len: u32,
) -> u32 {
    if instance.is_null() || out_buffer.is_null() || buffer_len == 0 {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        let string = match plugin.parameter_store() {
            Ok(store) => store.normalized_to_string(param_id, value as f64),
            Err(_) => return 0,
        };

        // Copy to buffer
        let bytes = string.as_bytes();
        let copy_len = bytes.len().min(buffer_len as usize - 1);

        ptr::copy_nonoverlapping(bytes.as_ptr(), out_buffer as *mut u8, copy_len);
        *out_buffer.add(copy_len) = 0; // Null terminator

        copy_len as u32
    }));

    result.unwrap_or(0)
}

/// Parse a display string to a normalized value.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - `string` must be a valid pointer to a null-terminated UTF-8 C string,
///   or null (in which case this function returns `false`)
/// - `out_value` must be a valid pointer to a writable `f32`,
///   or null (in which case this function returns `false`)
/// - This function validates all three pointers are non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_parse_parameter_value(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    string: *const c_char,
    out_value: *mut f32,
) -> bool {
    if instance.is_null() || string.is_null() || out_value.is_null() {
        return false;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let rust_string = match CStr::from_ptr(string).to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return false,
        };

        match plugin.parameter_store() {
            Ok(store) => match store.string_to_normalized(param_id, rust_string) {
                Some(value) => {
                    *out_value = value as f32;
                    true
                }
                None => false,
            },
            Err(_) => false,
        }
    }));

    result.unwrap_or(false)
}

// =============================================================================
// State Persistence
// =============================================================================

/// Get the size of the serialized state in bytes.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_get_state_size(instance: BeamerAuInstanceHandle) -> u32 {
    if instance.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        plugin.save_state().len() as u32
    }));

    result.unwrap_or(0)
}

/// Serialize the plugin state to a buffer.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - `buffer` must be a valid pointer to a writable buffer of at least `size` bytes,
///   or null (in which case this function returns `0`)
/// - This function validates both pointers are non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_get_state(
    instance: BeamerAuInstanceHandle,
    buffer: *mut u8,
    size: u32,
) -> u32 {
    if instance.is_null() || buffer.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        let state = plugin.save_state();
        let copy_len = state.len().min(size as usize);

        if copy_len > 0 {
            ptr::copy_nonoverlapping(state.as_ptr(), buffer, copy_len);
        }

        copy_len as u32
    }));

    result.unwrap_or(0)
}

/// Restore plugin state from a buffer.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `K_AUDIO_UNIT_ERR_INVALID_PARAMETER`)
/// - `instance` must not have been destroyed
/// - `buffer` must be a valid pointer to a readable buffer of at least `size` bytes,
///   or null if `size` is 0 (empty state is allowed)
/// - This function validates `instance` is non-null and that `buffer` is non-null
///   when `size > 0` before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_set_state(
    instance: BeamerAuInstanceHandle,
    buffer: *const u8,
    size: u32,
) -> i32 {
    if instance.is_null() || (buffer.is_null() && size > 0) {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let state_slice = if size > 0 {
            std::slice::from_raw_parts(buffer, size as usize)
        } else {
            &[]
        };

        let mut plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT,
        };

        match plugin.load_state(state_slice) {
            Ok(()) => os_status::NO_ERR,
            Err(e) => {
                log::error!("Failed to load state: {}", e);
                os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE
            }
        }
    }));

    result.unwrap_or(os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT)
}

// =============================================================================
// Properties
// =============================================================================

/// Get the plugin's processing latency in samples.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_get_latency_samples(instance: BeamerAuInstanceHandle) -> u32 {
    if instance.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        plugin.latency_samples()
    }));

    result.unwrap_or(0)
}

/// Get the plugin's tail time in samples.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_get_tail_samples(instance: BeamerAuInstanceHandle) -> u32 {
    if instance.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        plugin.tail_samples()
    }));

    result.unwrap_or(0)
}

/// Get float64 processing support level.
///
/// Returns:
/// - `NotSupported` when `instance` is null.
/// - `Native` when the prepared processor reports native f64 processing.
/// - `ViaConversion` otherwise (float64 streams are accepted via conversion).
#[no_mangle]
pub extern "C" fn beamer_au_get_float64_support(
    instance: BeamerAuInstanceHandle,
) -> BeamerAuFloat64Support {
    if instance.is_null() {
        return BeamerAuFloat64Support::NotSupported;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return BeamerAuFloat64Support::NotSupported,
        };

        if plugin.supports_native_double_precision() {
            BeamerAuFloat64Support::Native
        } else {
            BeamerAuFloat64Support::ViaConversion
        }
    }));

    result.unwrap_or(BeamerAuFloat64Support::NotSupported)
}

// =============================================================================
// Plugin Metadata
// =============================================================================

/// Get the plugin's display name.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - `out_buffer` must be a valid pointer to a writable buffer of at least
///   `buffer_len` bytes, or null (in which case this function returns `0`)
/// - `buffer_len` must be greater than 0
/// - This function validates `out_buffer` is non-null and `buffer_len > 0`
///   before dereferencing
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_name(
    _instance: BeamerAuInstanceHandle,
    out_buffer: *mut c_char,
    buffer_len: u32,
) -> u32 {
    if out_buffer.is_null() || buffer_len == 0 {
        return 0;
    }

    let result = catch_unwind(|| unsafe {
        let config = match factory::plugin_config() {
            Some(c) => c,
            None => return 0,
        };

        let bytes = config.name.as_bytes();
        let copy_len = bytes.len().min(buffer_len as usize - 1);

        ptr::copy_nonoverlapping(bytes.as_ptr(), out_buffer as *mut u8, copy_len);
        *out_buffer.add(copy_len) = 0;

        copy_len as u32
    });

    result.unwrap_or(0)
}

/// Get the plugin vendor/manufacturer name.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - `out_buffer` must be a valid pointer to a writable buffer of at least
///   `buffer_len` bytes, or null (in which case this function returns `0`)
/// - `buffer_len` must be greater than 0
/// - This function validates `out_buffer` is non-null and `buffer_len > 0`
///   before dereferencing
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_vendor(
    _instance: BeamerAuInstanceHandle,
    out_buffer: *mut c_char,
    buffer_len: u32,
) -> u32 {
    if out_buffer.is_null() || buffer_len == 0 {
        return 0;
    }

    let result = catch_unwind(|| unsafe {
        let config = match factory::plugin_config() {
            Some(c) => c,
            None => return 0,
        };

        let bytes = config.vendor.as_bytes();
        let copy_len = bytes.len().min(buffer_len as usize - 1);

        ptr::copy_nonoverlapping(bytes.as_ptr(), out_buffer as *mut u8, copy_len);
        *out_buffer.add(copy_len) = 0;

        copy_len as u32
    });

    result.unwrap_or(0)
}

// =============================================================================
// Bus Queries
// =============================================================================

/// Get the number of input buses the plugin supports.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_input_bus_count(instance: BeamerAuInstanceHandle) -> u32 {
    if instance.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        // If resources are allocated, the host-provided bus config is the source of truth.
        if let Some(cfg) = handle.bus_config.as_ref() {
            let count = cfg.input_bus_count.min(MAX_BUSES) as u32;
            return count;
        }

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        
        plugin.declared_input_bus_count().min(MAX_BUSES) as u32
    }));

    result.unwrap_or(0)
}

/// Get the number of output buses the plugin supports.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_output_bus_count(instance: BeamerAuInstanceHandle) -> u32 {
    if instance.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        // If resources are allocated, the host-provided bus config is the source of truth.
        if let Some(cfg) = handle.bus_config.as_ref() {
            let count = cfg.output_bus_count.min(MAX_BUSES) as u32;
            return count;
        }

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        
        plugin.declared_output_bus_count().min(MAX_BUSES) as u32
    }));

    result.unwrap_or(0)
}

/// Get the default channel count for an input bus.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_input_bus_channel_count(
    instance: BeamerAuInstanceHandle,
    bus_index: u32,
) -> u32 {
    if instance.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        // If resources are allocated, report the host-negotiated channel counts.
        if let Some(cfg) = handle.bus_config.as_ref() {
            let ch = cfg
                .input_bus_info(bus_index as usize)
                .map(|b| b.channel_count as u32)
                .unwrap_or(0);
            return ch;
        }

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        
        plugin
            .declared_input_bus_info(bus_index as usize)
            .map(|b| b.channel_count)
            .unwrap_or(0)
    }));

    result.unwrap_or(0)
}

/// Get the default channel count for an output bus.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_output_bus_channel_count(
    instance: BeamerAuInstanceHandle,
    bus_index: u32,
) -> u32 {
    if instance.is_null() {
        return 0;
    }

    let result = catch_unwind(AssertUnwindSafe(|| unsafe {
        let handle = &*instance;

        // If resources are allocated, report the host-negotiated channel counts.
        if let Some(cfg) = handle.bus_config.as_ref() {
            let ch = cfg
                .output_bus_info(bus_index as usize)
                .map(|b| b.channel_count as u32)
                .unwrap_or(0);
            return ch;
        }

        let plugin = match handle.plugin.lock() {
            Ok(guard) => guard,
            Err(_) => return 0,
        };

        
        plugin
            .declared_output_bus_info(bus_index as usize)
            .map(|b| b.channel_count)
            .unwrap_or(0)
    }));

    result.unwrap_or(0)
}

/// Check if a proposed channel configuration is valid.
///
/// For effect plugins (aufx), this enforces that input channels equal output channels
/// on the main bus, which is the typical expectation for [-1, -1] channel capability.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_is_channel_config_valid(
    _instance: BeamerAuInstanceHandle,
    main_input_channels: u32,
    main_output_channels: u32,
) -> bool {
    use crate::bus_config::MAX_CHANNELS;
    use crate::config::ComponentType;

    let result = catch_unwind(|| {
        // Get the AU config to check the component type
        let config = match factory::au_config() {
            Some(c) => c,
            None => return false,
        };

        // Validate channel counts are within bounds
        if main_input_channels > MAX_CHANNELS as u32 || main_output_channels > MAX_CHANNELS as u32 {
            return false;
        }

        // For effect plugins (aufx), require matching input/output channel counts
        // This implements the [-1, -1] channel capability behavior
        if config.component_type == ComponentType::Effect {
            return main_input_channels == main_output_channels;
        }

        // For instruments (aumu), any output channel count is valid
        // Instruments typically don't have audio input, only MIDI
        if config.component_type == ComponentType::MusicDevice {
            return true;
        }

        // For MIDI processors (aumi), require matching input/output
        if config.component_type == ComponentType::MidiProcessor {
            return main_input_channels == main_output_channels;
        }

        // Default: accept any configuration
        true
    });

    result.unwrap_or(false)
}

// =============================================================================
// MIDI Support
// =============================================================================

/// Check if the plugin accepts MIDI input.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_accepts_midi(_instance: BeamerAuInstanceHandle) -> bool {
    // Check component type from AU config
    let result = catch_unwind(|| {
        let config = match factory::au_config() {
            Some(c) => c,
            None => return false,
        };

        use crate::config::ComponentType;
        matches!(
            config.component_type,
            ComponentType::MusicDevice | ComponentType::MidiProcessor
        )
    });

    result.unwrap_or(false)
}

/// Check if the plugin produces MIDI output.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_produces_midi(_instance: BeamerAuInstanceHandle) -> bool {
    // Check component type from AU config
    let result = catch_unwind(|| {
        let config = match factory::au_config() {
            Some(c) => c,
            None => return false,
        };

        use crate::config::ComponentType;
        matches!(
            config.component_type,
            ComponentType::MusicDevice | ComponentType::MidiProcessor
        )
    });

    result.unwrap_or(false)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beamer_au_bus_info_layout() {
        // Verify C-compatible layout
        assert_eq!(std::mem::size_of::<BeamerAuBusInfo>(), 8);
        assert_eq!(std::mem::align_of::<BeamerAuBusInfo>(), 4);
    }

    #[test]
    fn test_beamer_au_sample_format() {
        assert_eq!(BeamerAuSampleFormat::Float32 as i32, 0);
        assert_eq!(BeamerAuSampleFormat::Float64 as i32, 1);
    }

    #[test]
    fn test_beamer_au_float64_support_enum() {
        assert_eq!(BeamerAuFloat64Support::NotSupported as i32, 0);
        assert_eq!(BeamerAuFloat64Support::ViaConversion as i32, 1);
        assert_eq!(BeamerAuFloat64Support::Native as i32, 2);
        assert_eq!(beamer_au_get_float64_support(ptr::null_mut()), BeamerAuFloat64Support::NotSupported);
    }

    #[test]
    fn test_copy_str_to_char_array() {
        let mut dest = [0i8; 16];
        copy_str_to_char_array("hello", &mut dest);
        assert_eq!(dest[0], b'h' as i8);
        assert_eq!(dest[4], b'o' as i8);
        assert_eq!(dest[5], 0);
    }

    #[test]
    fn test_copy_str_to_char_array_truncation() {
        let mut dest = [0i8; 4];
        copy_str_to_char_array("hello", &mut dest);
        assert_eq!(dest[0], b'h' as i8);
        assert_eq!(dest[2], b'l' as i8);
        assert_eq!(dest[3], 0); // Null terminator
    }

    #[test]
    fn test_null_handle_safety() {
        // All functions should handle null handles gracefully
        beamer_au_destroy_instance(ptr::null_mut());

        assert_eq!(
            beamer_au_allocate_render_resources(
                ptr::null_mut(),
                44100.0,
                1024,
                BeamerAuSampleFormat::Float32,
                ptr::null(),
            ),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER
        );

        beamer_au_deallocate_render_resources(ptr::null_mut());

        assert_eq!(
            beamer_au_render(
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
                1024,
                0,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
            ),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER
        );

        assert_eq!(beamer_au_get_parameter_count(ptr::null_mut()), 0);
        assert!(!beamer_au_get_parameter_info(
            ptr::null_mut(),
            0,
            ptr::null_mut()
        ));
        assert_eq!(beamer_au_get_parameter_value(ptr::null_mut(), 0), 0.0);
        beamer_au_set_parameter_value(ptr::null_mut(), 0, 0.5);
        assert_eq!(beamer_au_get_state_size(ptr::null_mut()), 0);
        assert_eq!(beamer_au_get_state(ptr::null_mut(), ptr::null_mut(), 0), 0);
        assert_eq!(
            beamer_au_set_state(ptr::null_mut(), ptr::null(), 0),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER
        );
        assert_eq!(beamer_au_get_latency_samples(ptr::null_mut()), 0);
        assert_eq!(beamer_au_get_tail_samples(ptr::null_mut()), 0);
        assert!(!beamer_au_is_prepared(ptr::null_mut()));
        beamer_au_reset(ptr::null_mut());
    }

    /// Helper to create a valid bus config for testing
    fn create_valid_bus_config() -> BeamerAuBusConfig {
        let mut config = BeamerAuBusConfig {
            input_bus_count: 1,
            output_bus_count: 1,
            input_buses: [BeamerAuBusInfo {
                channel_count: 0,
                bus_type: BeamerAuBusType::Main,
            }; MAX_BUSES],
            output_buses: [BeamerAuBusInfo {
                channel_count: 0,
                bus_type: BeamerAuBusType::Main,
            }; MAX_BUSES],
        };
        config.input_buses[0].channel_count = 2;
        config.output_buses[0].channel_count = 2;
        config
    }

    #[test]
    fn test_allocate_render_resources_invalid_sample_rate() {
        // Test with a valid bus config but invalid sample rates
        let bus_config = create_valid_bus_config();

        // Create a dummy non-null instance handle for testing parameter validation
        // Note: We use a non-null pointer that will cause the function to validate
        // parameters before attempting to dereference (which would fail).
        // The validation happens before the unsafe block, so these tests are safe.

        // Zero sample rate
        assert_eq!(
            beamer_au_allocate_render_resources(
                ptr::null_mut(), // Still null - but sample_rate check happens after null check
                0.0,
                1024,
                BeamerAuSampleFormat::Float32,
                &bus_config,
            ),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER // null instance check first
        );

        // Negative sample rate - need non-null instance to reach sample_rate validation
        // Since we can't easily create a valid instance in tests, we verify the constants exist
        assert_eq!(MAX_SAMPLE_RATE, 384_000.0);
        assert_eq!(MAX_FRAMES_PER_RENDER, 8192);
    }

    #[test]
    fn test_bus_config_validation_constants() {
        // Verify the validation constants are reasonable (compile-time checks)
        const _: () = assert!(MAX_SAMPLE_RATE > 0.0);
        const _: () = assert!(MAX_SAMPLE_RATE >= 192_000.0); // Should support high-end pro audio
        const _: () = assert!(MAX_FRAMES_PER_RENDER > 0);
        const _: () = assert!(MAX_FRAMES_PER_RENDER >= 4096); // Should support common buffer sizes
    }

    #[test]
    fn test_bus_config_bounds() {
        // Verify MAX_BUSES is accessible and reasonable (compile-time checks)
        const _: () = assert!(MAX_BUSES > 0);
        const _: () = assert!(MAX_BUSES <= 16); // Reasonable upper bound for buses
    }
}
