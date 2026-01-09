//! BeamerAudioUnit - Objective-C Audio Unit class.
//!
//! This module defines the `BeamerAudioUnit` class that bridges Beamer plugins
//! to the macOS Audio Unit system. It represents a single AU plugin instance,
//! managing the plugin's lifecycle, rendering, and parameter interaction with
//! AU host applications.
//!
//! # Architecture
//!
//! `BeamerAudioUnit` is an Objective-C class (`NSObject` subclass) that hosts a
//! Rust plugin instance. The class is defined using `objc2`'s `define_class!` macro,
//! which generates the Objective-C runtime metadata.
//!
//! # Instance Storage Strategy
//!
//! Each AU instance stores independent state in instance variables (ivars):
//! - Uses `IvarArc<T>` to store `Arc<Mutex<...>>` for thread-safe shared access
//! - Uses `IvarCell<T>` for simple mutable primitives (sample_rate, max_frames)
//! - Plugin, render block, and parameter tree are stored as ivars for lifetime management
//!
//! This approach enables proper multi-instance support - each AU instance in a DAW
//! gets its own plugin instance, render block, and parameter state.
//!
//! # Lifecycle
//!
//! 1. **Creation**: AU host creates BeamerAudioUnit instance
//! 2. **Lazy initialization**: Plugin is created on first use (initialize_plugin)
//! 3. **Allocation**: Host calls allocateRenderResourcesAndReturnError:
//! 4. **Rendering**: AU host calls the render block repeatedly during playback
//! 5. **Deallocation**: Host calls deallocateRenderResources when done
//! 6. **Destruction**: AU instance is released
//!
//! # Key Responsibilities
//!
//! - **Resource management**: Allocate and deallocate audio buffers, processors
//! - **Bus configuration**: Query and validate input/output bus layout
//! - **Parameter bridging**: Build and maintain AUParameterTree from plugin parameters
//! - **State saving/loading**: Implement full state for presets via fullState property
//! - **Tail time & latency**: Report plugin's tail samples and latency to host
//!
//! # Safety Considerations
//!
//! - AU calls happen on the main thread (initialization) and audio thread (render)
//! - The Mutex wrapping plugin ensures thread-safe access from both contexts
//! - Buffer pointers from render block have limited lifetime (only during callback)

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use block2::{Block, RcBlock};
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{class, define_class, extern_class, msg_send, ClassType, DefinedClass};
use objc2_foundation::{NSError, NSString};

use crate::buffer_storage::{validate_bus_limits_from_config, ProcessBufferStorage};
use crate::bus_config::{BusInfo, BusType, CachedBusConfig};
use crate::error::os_status;
use crate::error_helpers::{set_error_and_fail, DEFAULT_MAX_FRAMES, DEFAULT_SAMPLE_RATE};
use crate::factory;
use crate::instance::AuPluginInstance;
use crate::ivar_arc::{IvarArc, IvarCell};
use crate::parameters::build_parameter_tree;
use crate::render::{
    create_noop_render_block, create_objc_render_block, create_render_block_f32,
    create_render_block_f64, AuRenderBlockFn, RenderBlockTrait,
};

/// Opaque wrapper for Objective-C block pointers.
///
/// This type exists to provide the correct Objective-C encoding (`@?` for blocks)
/// when returning render blocks from `internalRenderBlock`. The AU framework
/// expects a block type, and objc2 validates method signatures against the
/// runtime.
///
/// The actual render block is created using block2 and stored in the ivars.
/// This wrapper allows us to return the block pointer with the correct encoding.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct OpaqueBlock(*const c_void);

// SAFETY: OpaqueBlock wraps a pointer to an Objective-C block, which has
// encoding `@?` (Encoding::Block). This is required for internalRenderBlock
// to satisfy AU's method signature validation.
unsafe impl Encode for OpaqueBlock {
    const ENCODING: Encoding = Encoding::Block;
}

unsafe impl RefEncode for OpaqueBlock {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Block);
}

/// Wrapper for block pointer that implements Send + Sync.
///
/// SAFETY: The wrapped pointer points to a leaked RcBlock that lives for the
/// duration of the process. The block itself is thread-safe for invocation
/// (it's a no-op that doesn't access any shared state).
struct NoopBlockPtr(*const Block<AuRenderBlockFn>);

// SAFETY: The block is a no-op with no captured state, and once created it's
// never modified. Multiple threads can safely read the pointer.
unsafe impl Send for NoopBlockPtr {}
unsafe impl Sync for NoopBlockPtr {}

/// Get a pointer to a no-op render block for use when real block isn't allocated yet.
///
/// AU hosts may query `internalRenderBlock` before `allocateRenderResources` is called.
/// Instead of returning null (which can crash some hosts), we return this no-op block.
/// It simply returns 0 (noErr) without processing any audio.
///
/// The block is lazily created on first use and intentionally leaked (never dropped)
/// to ensure the pointer remains valid for the lifetime of the process.
fn get_noop_render_block() -> *const Block<AuRenderBlockFn> {
    use std::sync::OnceLock;

    // We use OnceLock with a wrapped pointer because raw pointers aren't Sync.
    // The block is intentionally leaked so the pointer remains valid.
    static NOOP_BLOCK_PTR: OnceLock<NoopBlockPtr> = OnceLock::new();

    NOOP_BLOCK_PTR
        .get_or_init(|| {
            // Create the no-op block with proper encoding
            let block = create_noop_render_block();

            // Get pointer to the inner Block
            let block_ptr: *const Block<AuRenderBlockFn> = &*block;

            // Leak the RcBlock so it's never dropped and the pointer stays valid
            std::mem::forget(block);

            NoopBlockPtr(block_ptr)
        })
        .0
}

/// Audio Component Description structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioComponentDescription {
    pub component_type: u32,
    pub component_sub_type: u32,
    pub component_manufacturer: u32,
    pub component_flags: u32,
    pub component_flags_mask: u32,
}

/// Audio Stream Basic Description structure.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct AudioStreamBasicDescription {
    pub sample_rate: f64,
    pub format_id: u32,
    pub format_flags: u32,
    pub bytes_per_packet: u32,
    pub frames_per_packet: u32,
    pub bytes_per_frame: u32,
    pub channels_per_frame: u32,
    pub bits_per_channel: u32,
    pub reserved: u32,
}

// SAFETY: AudioStreamBasicDescription has a well-defined C memory layout
unsafe impl Encode for AudioStreamBasicDescription {
    const ENCODING: Encoding = Encoding::Struct(
        "AudioStreamBasicDescription",
        &[
            Encoding::Double, // sample_rate
            Encoding::UInt,   // format_id
            Encoding::UInt,   // format_flags
            Encoding::UInt,   // bytes_per_packet
            Encoding::UInt,   // frames_per_packet
            Encoding::UInt,   // bytes_per_frame
            Encoding::UInt,   // channels_per_frame
            Encoding::UInt,   // bits_per_channel
            Encoding::UInt,   // reserved
        ],
    );
}

unsafe impl RefEncode for AudioStreamBasicDescription {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

// SAFETY: AudioComponentDescription has a well-defined C memory layout
unsafe impl Encode for AudioComponentDescription {
    const ENCODING: Encoding = Encoding::Struct(
        "AudioComponentDescription",
        &[
            Encoding::UInt, // component_type
            Encoding::UInt, // component_sub_type
            Encoding::UInt, // component_manufacturer
            Encoding::UInt, // component_flags
            Encoding::UInt, // component_flags_mask
        ],
    );
}

unsafe impl RefEncode for AudioComponentDescription {
    const ENCODING_REF: Encoding = Encoding::Pointer(&<Self as Encode>::ENCODING);
}

// Declare AUAudioUnit as an external class
extern_class!(
    /// The base Audio Unit class from AudioToolbox.
    #[unsafe(super(NSObject))]
    #[derive(Debug, PartialEq, Eq, Hash)]
    pub struct AUAudioUnit;
);

/// Instance variables for BeamerAudioUnit.
///
/// Each AU instance has its own independent state stored here.
/// Uses `IvarArc` for Arc storage and `IvarCell` for mutable primitive access.
#[derive(Clone, Default)]
pub struct BeamerAudioUnitIvars {
    /// Plugin instance (Arc for sharing with render block)
    plugin: IvarArc<Mutex<Box<dyn AuPluginInstance>>>,

    /// Parameter tree (optional retained ObjC object)
    param_tree: IvarArc<Mutex<Option<Retained<AnyObject>>>>,

    /// Render block (Arc for lifetime management, type-erased for f32/f64 support)
    render_block: IvarArc<dyn RenderBlockTrait>,

    /// Objective-C render block returned by internalRenderBlock.
    /// This wraps the Rust render_block in an ObjC block that AU can invoke.
    /// Must be kept alive as long as render resources are allocated.
    objc_render_block: IvarCell<Option<RcBlock<AuRenderBlockFn>>>,

    /// Cached bus configuration extracted from AU during allocation
    bus_config: IvarCell<Option<CachedBusConfig>>,

    /// Current sample rate (set in allocateRenderResources)
    sample_rate: IvarCell<f64>,

    /// Maximum frames per render call
    max_frames: IvarCell<u32>,
}

// SAFETY: All fields are either Send+Sync (IvarArc, UnsafeCell with primitives)
// or protected by Mutex. AU guarantees proper synchronization.
unsafe impl Send for BeamerAudioUnitIvars {}
unsafe impl Sync for BeamerAudioUnitIvars {}

/// Extract bus configuration from Audio Unit's bus arrays.
///
/// Queries the AU's inputBusses and outputBusses arrays to determine the
/// number of buses and channels per bus. This configuration is then cached
/// for efficient access during render.
///
/// # Safety
///
/// Must be called from within an AU method with valid `au` reference.
/// Uses Objective-C messaging to query bus properties.
unsafe fn extract_bus_config_from_au(au: &BeamerAudioUnit) -> Result<CachedBusConfig, String> {
    let mut input_buses = Vec::new();
    let mut output_buses = Vec::new();

    // Query input busses
    let input_busses: Option<Retained<AnyObject>> = msg_send![au, inputBusses];
    if let Some(busses) = input_busses {
        let count: usize = msg_send![&busses, count];
        for i in 0..count {
            let bus: Option<Retained<AnyObject>> = msg_send![&busses, objectAtIndexedSubscript: i];
            if let Some(bus) = bus {
                let format: Option<Retained<AnyObject>> = msg_send![&bus, format];
                if let Some(fmt) = format {
                    let ch: u32 = msg_send![&fmt, channelCount];
                    input_buses.push(BusInfo {
                        channel_count: ch as usize,
                        bus_type: if i == 0 {
                            BusType::Main
                        } else {
                            BusType::Auxiliary
                        },
                    });
                }
            }
        }
    }

    // Query output busses
    let output_busses: Option<Retained<AnyObject>> = msg_send![au, outputBusses];
    if let Some(busses) = output_busses {
        let count: usize = msg_send![&busses, count];
        for i in 0..count {
            let bus: Option<Retained<AnyObject>> = msg_send![&busses, objectAtIndexedSubscript: i];
            if let Some(bus) = bus {
                let format: Option<Retained<AnyObject>> = msg_send![&bus, format];
                if let Some(fmt) = format {
                    let ch: u32 = msg_send![&fmt, channelCount];
                    output_buses.push(BusInfo {
                        channel_count: ch as usize,
                        bus_type: if i == 0 {
                            BusType::Main
                        } else {
                            BusType::Auxiliary
                        },
                    });
                }
            }
        }
    }

    Ok(CachedBusConfig::new(input_buses, output_buses))
}

define_class!(
    /// BeamerAudioUnit - the Objective-C class for Beamer plugins.
    #[unsafe(super(AUAudioUnit))]
    #[name = "BeamerAudioUnit"]
    #[ivars = BeamerAudioUnitIvars]
    pub struct BeamerAudioUnit;

    impl BeamerAudioUnit {
        /// Allocate resources for rendering.
        #[unsafe(method(allocateRenderResourcesAndReturnError:))]
        fn __allocate_render_resources(&self, error: *mut *mut NSError) -> Bool {
            log::debug!("allocateRenderResourcesAndReturnError called");

            // Initialize plugin if not already done
            self.initialize_plugin();

            // SAFETY: Calling super's allocateRenderResourcesAndReturnError via ObjC
            // messaging. This is safe because `self` is a valid BeamerAudioUnit instance
            // and the error pointer is provided by the AU host.
            let super_result: Bool = unsafe {
                msg_send![super(self), allocateRenderResourcesAndReturnError: error]
            };

            if !super_result.as_bool() {
                return super_result;
            }

            // SAFETY: We have a valid BeamerAudioUnit reference (`self`) within an AU method.
            // The extract_bus_config_from_au function queries AU bus properties via ObjC messaging.
            let bus_config = unsafe {
                match extract_bus_config_from_au(self) {
                    Ok(config) => config,
                    Err(e) => {
                        log::error!("Failed to extract bus config: {}", e);
                        return set_error_and_fail(
                            error,
                            os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
                            &format!("Failed to extract bus configuration: {}", e),
                        );
                    }
                }
            };

            // Validate bus configuration
            if let Err(e) = bus_config.validate() {
                log::error!("Bus config validation failed: {}", e);
                return unsafe {
                    set_error_and_fail(
                        error,
                        os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
                        &format!("Invalid bus configuration: {}", e),
                    )
                };
            }

            // Validate against system limits
            if let Err(e) = validate_bus_limits_from_config(&bus_config) {
                log::error!("Bus limits validation failed: {}", e);
                return unsafe {
                    set_error_and_fail(
                        error,
                        os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
                        &format!("Bus configuration exceeds limits: {}", e),
                    )
                };
            }

            log::debug!(
                "Bus config: {} input buses ({} channels), {} output buses ({} channels)",
                bus_config.input_bus_count,
                bus_config.total_input_channels(),
                bus_config.output_bus_count,
                bus_config.total_output_channels()
            );

            // SAFETY: Ivar access is safe here because AU guarantees this method is called
            // on the main thread, and no render calls can happen until allocation succeeds.
            unsafe {
                *self.ivars().bus_config.get() = Some(bus_config.clone());
            }

            // SAFETY: ObjC messaging to query AU bus format properties. All objects
            // (outputBusses, bus, format) are Retained and thus kept alive for the
            // duration of these queries. The streamDescription pointer is valid while
            // the format object is retained.
            let (sample_rate, is_float64): (f64, bool) = unsafe {
                // Get outputBusses property (AUAudioUnitBusArray)
                let output_busses: Option<Retained<AnyObject>> = msg_send![self, outputBusses];
                if let Some(busses) = output_busses {
                    // Get bus at index 0 (main output bus)
                    let bus_0: Option<Retained<AnyObject>> =
                        msg_send![&busses, objectAtIndexedSubscript: 0usize];
                    if let Some(bus) = bus_0 {
                        // Get format (AVAudioFormat)
                        let format: Option<Retained<AnyObject>> = msg_send![&bus, format];
                        if let Some(fmt) = format {
                            // Get sample rate (double)
                            let sr: f64 = msg_send![&fmt, sampleRate];
                            // Get stream description to check if it's float64
                            let stream_desc: *const AudioStreamBasicDescription = msg_send![&fmt, streamDescription];
                            let is_f64 = if !stream_desc.is_null() {
                                (*stream_desc).bits_per_channel == 64
                            } else {
                                false
                            };
                            (sr, is_f64)
                        } else {
                            (DEFAULT_SAMPLE_RATE, false)
                        }
                    } else {
                        (DEFAULT_SAMPLE_RATE, false)
                    }
                } else {
                    (DEFAULT_SAMPLE_RATE, false)
                }
            };

            // SAFETY: ObjC message to query AU's maximumFramesToRender property.
            // `self` is a valid BeamerAudioUnit instance.
            let max_frames: u32 = unsafe {
                let frames: u32 = msg_send![self, maximumFramesToRender];
                if frames > 0 {
                    frames
                } else {
                    DEFAULT_MAX_FRAMES
                }
            };

            log::debug!(
                "Sample rate: {}, max frames: {}, format: {}",
                sample_rate,
                max_frames,
                if is_float64 { "f64" } else { "f32" }
            );

            // Store in ivars
            unsafe {
                *self.ivars().sample_rate.get() = sample_rate;
                *self.ivars().max_frames.get() = max_frames;
            }

            // Get plugin and prepare it
            let plugin: Arc<Mutex<Box<dyn AuPluginInstance>>> = unsafe {
                match self.ivars().plugin.get() {
                    Some(p) => p,
                    None => {
                        log::error!("Plugin not initialized");
                        return set_error_and_fail(
                            error,
                            os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED,
                            "Plugin not initialized",
                        );
                    }
                }
            };

            // Prepare the plugin with the actual bus configuration
            {
                let mut plugin_guard: std::sync::MutexGuard<'_, Box<dyn AuPluginInstance>> =
                    match plugin.lock() {
                        Ok(g) => g,
                        Err(_) => {
                            return unsafe {
                                set_error_and_fail(
                                    error,
                                    os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT,
                                    "Failed to lock plugin",
                                )
                            };
                        }
                    };

                // Pass CachedBusConfig directly to allocate_render_resources
                // This allows proper derivation of aux bus channel counts for conversion buffers
                if let Err(e) = plugin_guard.allocate_render_resources(sample_rate, max_frames, &bus_config) {
                    log::error!("Failed to allocate render resources: {:?}", e);
                    return unsafe {
                        set_error_and_fail(
                            error,
                            os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY,
                            &format!("Failed to prepare plugin: {:?}", e),
                        )
                    };
                }
            }

            // Query AU host for musical context block
            // The musicalContextBlock property provides tempo, time signature, and beat position
            // Some hosts (Logic Pro, GarageBand) provide this, others may return null
            let musical_context_block: Option<*const std::ffi::c_void> = unsafe {
                let block_ptr: *const c_void = msg_send![self, musicalContextBlock];
                if block_ptr.is_null() {
                    log::debug!("Host does not provide musicalContextBlock - using basic transport");
                    None
                } else {
                    log::debug!("Host provides musicalContextBlock for musical timing info");
                    Some(block_ptr)
                }
            };

            // Query AU host for transport state block
            // The transportStateBlock property provides is_playing, is_recording, is_cycling flags
            // Some hosts provide this, others may return null (fallback to stopped state)
            let transport_state_block: Option<*const std::ffi::c_void> = unsafe {
                let block_ptr: *const c_void = msg_send![self, transportStateBlock];
                if block_ptr.is_null() {
                    log::debug!("Host does not provide transportStateBlock - defaulting to stopped");
                    None
                } else {
                    log::debug!("Host provides transportStateBlock for playback state");
                    Some(block_ptr)
                }
            };

            // Query AU host for MIDI output block (scheduleMIDIEventBlock)
            // This block is used to send MIDI events from the plugin to the host.
            // Only available for component types that support MIDI output:
            // - `aumu` (Music Device/Instrument): Supported
            // - `aumf` (MIDI Effect): Supported
            // - `aufx` (Effect): NOT typically supported by hosts
            //
            // For effect plugins, hosts typically don't provide this block, and
            // any MIDI output events will be dropped with a debug log message.
            let schedule_midi_event_block: Option<*const std::ffi::c_void> = unsafe {
                let block_ptr: *const c_void = msg_send![self, scheduleMIDIEventBlock];
                if block_ptr.is_null() {
                    log::debug!(
                        "Host does not provide scheduleMIDIEventBlock - MIDI output disabled. \
                         This is normal for effect plugins (aufx)."
                    );
                    None
                } else {
                    log::debug!("Host provides scheduleMIDIEventBlock for MIDI output");
                    Some(block_ptr)
                }
            };

            // Create render block with pre-allocated storage (f32 or f64 based on format)
            let render_block: Box<dyn RenderBlockTrait> = if is_float64 {
                // f64 (double precision) format
                let storage_f64 = ProcessBufferStorage::<f64>::allocate_from_config(&bus_config);
                create_render_block_f64(
                    Arc::clone(&plugin),
                    storage_f64,
                    musical_context_block,
                    transport_state_block,
                    schedule_midi_event_block,
                    max_frames,
                    sample_rate,
                )
            } else {
                // f32 (single precision) format
                let storage_f32 = ProcessBufferStorage::<f32>::allocate_from_config(&bus_config);
                create_render_block_f32(
                    Arc::clone(&plugin),
                    storage_f32,
                    musical_context_block,
                    transport_state_block,
                    schedule_midi_event_block,
                    max_frames,
                    sample_rate,
                )
            };

            // Store the Rust render block (Arc::from converts Box<dyn Trait> to Arc<dyn Trait>)
            let render_block_arc: Arc<dyn RenderBlockTrait> = Arc::from(render_block);
            unsafe {
                self.ivars().render_block.init(Arc::clone(&render_block_arc));
            }

            // Create Objective-C block that wraps the Rust render block.
            // This is what AU actually invokes during audio processing.
            //
            // We use block2 to create a proper ObjC block. The block captures
            // an Arc to the render block and forwards calls to it.
            let objc_block = create_objc_render_block(Arc::clone(&render_block_arc));

            unsafe {
                *self.ivars().objc_render_block.get() = Some(objc_block);
            }

            log::debug!("Render resources allocated");
            Bool::YES
        }

        /// Deallocate rendering resources.
        #[unsafe(method(deallocateRenderResources))]
        fn __deallocate_render_resources(&self) {
            log::debug!("deallocateRenderResources called");

            // Clear ObjC render block first (it holds reference to Rust block)
            unsafe {
                *self.ivars().objc_render_block.get() = None;
            }

            // Clear Rust render block
            unsafe {
                self.ivars().render_block.clear();
            }

            // Deallocate plugin resources
            if let Some(plugin) = unsafe { self.ivars().plugin.get() } {
                if let Ok(mut plugin_guard) = plugin.lock() {
                    plugin_guard.deallocate_render_resources();
                }
            }

            // Call super's implementation
            unsafe {
                let _: () = msg_send![super(self), deallocateRenderResources];
            }
        }

        /// Return the internal render block.
        ///
        /// Returns an Objective-C block that AU invokes during audio processing.
        /// The block wraps our Rust render_block and forwards calls to it.
        ///
        /// If render resources haven't been allocated yet, returns a static no-op
        /// block instead of null. This prevents crashes in AU hosts that don't
        /// properly check for nil blocks.
        #[unsafe(method(internalRenderBlock))]
        fn __internal_render_block(&self) -> OpaqueBlock {
            unsafe {
                if let Some(block) = &*self.ivars().objc_render_block.get() {
                    // Get pointer to the Block (deref RcBlock to &Block, then to raw ptr)
                    let block_ref: &Block<AuRenderBlockFn> = block;
                    OpaqueBlock(block_ref as *const Block<AuRenderBlockFn> as *const c_void)
                } else {
                    // Return no-op block instead of null to prevent crashes
                    OpaqueBlock(get_noop_render_block() as *const c_void)
                }
            }
        }

        /// Return the parameter tree.
        #[unsafe(method_id(parameterTree))]
        fn __parameter_tree(&self) -> Option<Retained<AnyObject>> {
            unsafe {
                if let Some(pt_arc) = self.ivars().param_tree.get() {
                    if let Ok(guard) = pt_arc.lock() {
                        guard.clone()
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }

        /// Return the tail time in seconds.
        #[unsafe(method(tailTime))]
        fn __tail_time(&self) -> f64 {
            unsafe {
                if let Some(plugin) = self.ivars().plugin.get() {
                    if let Ok(plugin_guard) = plugin.lock() {
                        let sample_rate = *self.ivars().sample_rate.get();
                        let sample_rate = if sample_rate > 0.0 { sample_rate } else { 44100.0 };
                        let tail_samples = plugin_guard.tail_samples();
                        return tail_samples as f64 / sample_rate;
                    }
                }
                0.0
            }
        }

        /// Return the latency in seconds.
        #[unsafe(method(latency))]
        fn __latency(&self) -> f64 {
            unsafe {
                if let Some(plugin) = self.ivars().plugin.get() {
                    if let Ok(plugin_guard) = plugin.lock() {
                        let sample_rate = *self.ivars().sample_rate.get();
                        let sample_rate = if sample_rate > 0.0 { sample_rate } else { 44100.0 };
                        let latency_samples = plugin_guard.latency_samples();
                        return latency_samples as f64 / sample_rate;
                    }
                }
                0.0
            }
        }

        /// Get the full state of the audio unit for preset saving.
        ///
        /// Returns an NSDictionary containing the plugin state wrapped in a format
        /// compatible with the AU state saving system.
        #[unsafe(method_id(fullState))]
        fn __full_state(&self) -> Option<Retained<AnyObject>> {
            self.get_full_state_impl()
        }

        /// Set the full state of the audio unit for preset loading.
        ///
        /// Accepts an NSDictionary and restores the plugin state from it.
        #[unsafe(method(setFullState:))]
        fn __set_full_state(&self, state: *const AnyObject) {
            // SAFETY: The state pointer is provided by the AU host and is either null
            // (which we check) or a valid NSDictionary. All ObjC objects are properly
            // retained during the scope of this function. The bytes pointer from NSData
            // is valid for the lifetime of the ns_data Retained.
            unsafe {
                if state.is_null() {
                    return;
                }

                let state = &*state;

                // Get beamerState NSData from dictionary
                let state_key = NSString::from_str("beamerState");
                let ns_data: Option<Retained<AnyObject>> = msg_send![state, objectForKey: &*state_key];
                let ns_data = match ns_data {
                    Some(d) => d,
                    None => {
                        log::warn!("No beamerState key in state dictionary");
                        return;
                    }
                };

                // Get bytes from NSData
                let length: usize = msg_send![&ns_data, length];
                if length == 0 {
                    return;
                }

                let bytes_ptr: *const u8 = msg_send![&ns_data, bytes];
                if bytes_ptr.is_null() {
                    return;
                }

                let state_data = std::slice::from_raw_parts(bytes_ptr, length);

                // Load state into plugin
                if let Some(plugin) = self.ivars().plugin.get() {
                    if let Ok(mut plugin_guard) = plugin.lock() {
                        if let Err(e) = plugin_guard.load_state(state_data) {
                            log::error!("Failed to load state: {:?}", e);
                        } else {
                            log::debug!("Loaded state: {} bytes", length);
                        }
                    }
                }
            }
        }
    }
);

impl BeamerAudioUnit {
    /// Implementation of fullState getter.
    fn get_full_state_impl(&self) -> Option<Retained<AnyObject>> {
        unsafe {
            // Get plugin state
            let plugin = self.ivars().plugin.get()?;
            let plugin_guard = plugin.lock().ok()?;
            let state_data = plugin_guard.save_state();

            if state_data.is_empty() {
                return None;
            }

            // Create NSData from state bytes
            let data_class: &AnyClass = class!(NSData);
            let ns_data: Option<Retained<AnyObject>> = msg_send![
                data_class,
                dataWithBytes: state_data.as_ptr() as *const c_void,
                length: state_data.len()
            ];
            let ns_data = ns_data?;

            // Create dictionary with state key
            let dict_class: &AnyClass = class!(NSMutableDictionary);
            let dict: Retained<AnyObject> = msg_send![dict_class, new];

            let state_key = NSString::from_str("beamerState");
            let version_key = NSString::from_str("beamerVersion");
            let version_value: Retained<AnyObject> =
                msg_send![class!(NSNumber), numberWithInt: 1i32];

            let _: () = msg_send![&dict, setObject: &*ns_data, forKey: &*state_key];
            let _: () = msg_send![&dict, setObject: &*version_value, forKey: &*version_key];

            log::debug!("Saved state: {} bytes", state_data.len());
            Some(dict)
        }
    }

    /// Initialize the plugin from the factory.
    ///
    /// This is called lazily when the AU is first used.
    fn initialize_plugin(&self) {
        // SAFETY: Called from AU methods on the main thread. The IvarArc::init
        // and IvarArc::is_initialized methods handle the interior mutability
        // safely for single-threaded initialization.
        unsafe {
            if self.ivars().plugin.is_initialized() {
                return; // Already initialized
            }

            // Create plugin instance from factory
            let plugin = factory::create_instance();
            if plugin.is_none() {
                log::error!("Failed to create plugin instance - no factory registered");
                return;
            }

            let plugin = Arc::new(Mutex::new(plugin.unwrap()));

            // Store in ivars
            self.ivars().plugin.init(Arc::clone(&plugin));

            // Build parameter tree
            let param_tree = build_parameter_tree(Arc::clone(&plugin));

            // Store parameter tree in ivar
            // Note: Retained<AnyObject> is not Send/Sync, but parameter tree is only
            // accessed on the main thread (Audio Unit main thread affinity)
            #[allow(clippy::arc_with_non_send_sync)]
            let pt_arc = Arc::new(Mutex::new(param_tree));
            self.ivars().param_tree.init(pt_arc);

            log::debug!("BeamerAudioUnit plugin initialized");
        }
    }

    /// Get access to the plugin instance.
    pub fn plugin(&self) -> Option<Arc<Mutex<Box<dyn AuPluginInstance>>>> {
        unsafe { self.ivars().plugin.get() }
    }

    /// Get the current sample rate.
    pub fn sample_rate(&self) -> f64 {
        unsafe { *self.ivars().sample_rate.get() }
    }

    /// Get the maximum frames per render.
    pub fn max_frames(&self) -> u32 {
        unsafe { *self.ivars().max_frames.get() }
    }
}

/// Register the BeamerAudioUnit class with the Objective-C runtime.
///
/// This should be called during module initialization.
pub fn register_class() {
    // The class is automatically registered when first accessed
    let _ = BeamerAudioUnit::class();
    log::debug!("BeamerAudioUnit class registered");
}

/// Create a new BeamerAudioUnit instance.
///
/// This is the implementation of the AU factory function. It creates a new
/// BeamerAudioUnit instance using the Objective-C runtime and returns
/// a retained pointer that the caller takes ownership of.
///
/// # Safety
///
/// - `desc` must be a valid pointer to an AudioComponentDescription, or null
/// - The returned pointer is retained and ownership is transferred to the caller
/// - Returns null if instance creation fails
pub unsafe fn create_audio_unit_instance(
    desc: *const AudioComponentDescription,
) -> *mut c_void {
    // Ensure class is registered
    register_class();

    // Allocate BeamerAudioUnit
    let class = BeamerAudioUnit::class();
    let alloc: *mut AnyObject = msg_send![class, alloc];

    if alloc.is_null() {
        log::error!("Failed to allocate BeamerAudioUnit");
        return std::ptr::null_mut();
    }

    // Initialize with component description
    // initWithComponentDescription:error: is the designated initializer for AUAudioUnit
    let mut error_ptr: *mut NSError = std::ptr::null_mut();
    let instance: *mut AnyObject = msg_send![
        alloc,
        initWithComponentDescription: *desc,
        error: &mut error_ptr as *mut *mut NSError
    ];

    if instance.is_null() {
        if !error_ptr.is_null() {
            let error_ref = &*error_ptr;
            let description: Retained<NSString> = msg_send![error_ref, localizedDescription];
            log::error!("Failed to init BeamerAudioUnit: {}", description);
        } else {
            log::error!("Failed to init BeamerAudioUnit (unknown error)");
        }
        return std::ptr::null_mut();
    }

    log::debug!("BeamerAudioUnit instance created successfully");
    instance as *mut c_void
}
