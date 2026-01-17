/*
 * BeamerAuBridge.h
 *
 * C-ABI bridge between Objective-C AUAudioUnit wrapper and Rust plugin instance.
 *
 * This header defines the interface for the hybrid AU implementation where:
 * - Objective-C provides the AUAudioUnit subclass (BeamerAuWrapper)
 * - Rust provides all DSP, parameter handling, and state management
 *
 * The bridge is designed for:
 * - Full feature parity with VST3 (aux buses, f32/f64, MIDI, parameters, state)
 * - Zero-allocation audio processing (pre-allocated buffers in Rust)
 * - Comprehensive error handling via OSStatus return codes
 *
 * Thread Safety:
 * - Lifecycle functions (create/destroy/allocate/deallocate) must be called from main thread
 * - Render function is called from real-time audio thread (no allocations, no locks)
 * - Parameter get/set may be called from any thread (uses atomics internally)
 * - State save/load should be called from main thread
 *
 * Copyright (c) 2026 Helpermedia. All rights reserved.
 */

#ifndef BEAMER_AU_BRIDGE_H
#define BEAMER_AU_BRIDGE_H

#include <AudioToolbox/AudioToolbox.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// Enable nullability annotations for better Objective-C interop
NS_ASSUME_NONNULL_BEGIN

// =============================================================================
// MARK: - Opaque Instance Handle
// =============================================================================

/**
 * Opaque handle to a Rust plugin instance.
 *
 * This handle wraps a `Box<dyn AuPluginInstance>` on the Rust side.
 * The Objective-C wrapper stores this handle and passes it to all bridge functions.
 *
 * Lifetime:
 * - Created by `beamer_au_create_instance()`
 * - Destroyed by `beamer_au_destroy_instance()`
 * - Must not be used after destruction
 *
 * Thread Safety:
 * - The handle itself is a pointer and can be copied across threads
 * - However, most operations on the instance require proper synchronization
 */
typedef void* BeamerAuInstanceHandle;

// =============================================================================
// MARK: - Bus Configuration
// =============================================================================

/**
 * Maximum number of audio buses supported per direction (input/output).
 *
 * Matches `beamer_core::MAX_BUSES` for consistency across plugin formats.
 */
#define BEAMER_AU_MAX_BUSES 16

/**
 * Maximum number of channels per audio bus.
 *
 * Matches `beamer_core::MAX_CHANNELS` for consistency across plugin formats.
 */
#define BEAMER_AU_MAX_CHANNELS 32

/**
 * Maximum number of MIDI events per render buffer.
 *
 * Matches `beamer_core::midi::MAX_MIDI_EVENTS` for consistency across plugin formats.
 * This limit accommodates dense MIDI input including MPE controllers which can
 * generate many events per buffer (pitch bend + slide + pressure per voice).
 */
#define BEAMER_AU_MAX_MIDI_EVENTS 1024

/**
 * Bus type enumeration.
 *
 * Distinguishes between main audio buses and auxiliary buses (sidechain).
 */
typedef enum {
    /// Main audio bus (bus index 0)
    BeamerAuBusTypeMain = 0,
    /// Auxiliary audio bus (sidechain, additional I/O)
    BeamerAuBusTypeAuxiliary = 1,
} BeamerAuBusType;

/**
 * Information about a single audio bus.
 *
 * Passed to Rust during `allocateRenderResources` to configure buffer allocation.
 */
typedef struct {
    /// Number of channels in this bus (1 = mono, 2 = stereo, etc.)
    uint32_t channel_count;
    /// Bus type (main or auxiliary)
    BeamerAuBusType bus_type;
} BeamerAuBusInfo;

/**
 * Complete bus configuration for the plugin.
 *
 * This structure captures the full bus layout as configured by the AU host.
 * It is passed to Rust during `allocateRenderResources` so the plugin can
 * pre-allocate appropriately sized processing buffers.
 *
 * Layout:
 * - Input buses: input_buses[0..input_bus_count]
 * - Output buses: output_buses[0..output_bus_count]
 * - Bus 0 is always the main bus; bus 1+ are auxiliary
 */
typedef struct {
    /// Number of input buses (1 = main only, 2+ = main + aux)
    uint32_t input_bus_count;
    /// Number of output buses (1 = main only, 2+ = main + aux)
    uint32_t output_bus_count;
    /// Input bus information array (up to BEAMER_AU_MAX_BUSES)
    BeamerAuBusInfo input_buses[BEAMER_AU_MAX_BUSES];
    /// Output bus information array (up to BEAMER_AU_MAX_BUSES)
    BeamerAuBusInfo output_buses[BEAMER_AU_MAX_BUSES];
} BeamerAuBusConfig;

// =============================================================================
// MARK: - Sample Format
// =============================================================================

/**
 * Sample format enumeration for audio processing.
 *
 * AU hosts may request either 32-bit or 64-bit floating point processing.
 * The Rust side handles both formats. When a plugin doesn't support native f64
 * processing, Beamer will convert f64<->f32 internally.
 *
 * To query whether float64 is supported natively vs via conversion, use
 * beamer_au_get_float64_support().
 */
typedef enum {
    /// 32-bit floating point samples (standard)
    BeamerAuSampleFormatFloat32 = 0,
    /// 64-bit floating point samples (high precision)
    BeamerAuSampleFormatFloat64 = 1,
} BeamerAuSampleFormat;

// =============================================================================
// MARK: - Parameter Info
// =============================================================================

/**
 * Maximum length of parameter name/unit strings.
 *
 * Names and units longer than this are truncated.
 */
#define BEAMER_AU_MAX_PARAM_NAME_LENGTH 128

/**
 * Parameter metadata for building AUParameterTree.
 *
 * This structure provides all information needed to create an AUParameter
 * in Objective-C from Rust's parameter definitions.
 *
 * Value Range:
 * - All values are normalized (0.0 to 1.0)
 * - The ObjC wrapper sets min=0.0, max=1.0 on the AUParameter
 * - Display values are formatted by Rust via `beamer_au_format_parameter_value()`
 */
typedef struct {
    /// Parameter ID (unique within the plugin, maps to AU parameter address)
    uint32_t id;
    /// Human-readable parameter name (UTF-8, null-terminated)
    char name[BEAMER_AU_MAX_PARAM_NAME_LENGTH];
    /// Parameter unit string (e.g., "dB", "Hz", "ms"; UTF-8, null-terminated)
    char units[BEAMER_AU_MAX_PARAM_NAME_LENGTH];
    /// Default normalized value (0.0 to 1.0)
    float default_value;
    /// Current normalized value (0.0 to 1.0)
    float current_value;
    /// Number of discrete steps (0 = continuous, 1 = boolean, N = N+1 states)
    int32_t step_count;
    /// Flags (reserved for future use: automatable, hidden, etc.)
    uint32_t flags;
} BeamerAuParameterInfo;

/**
 * Parameter flags for BeamerAuParameterInfo.flags field.
 */
typedef enum {
    /// Parameter can be automated by the host
    BeamerAuParameterFlagAutomatable = (1 << 0),
    /// Parameter should be hidden from user (internal only)
    BeamerAuParameterFlagHidden = (1 << 1),
    /// Parameter is read-only (e.g., meter output)
    BeamerAuParameterFlagReadOnly = (1 << 2),
} BeamerAuParameterFlags;

// =============================================================================
// MARK: - Factory Registration
// =============================================================================

/**
 * Check if the plugin factory is registered.
 *
 * This function verifies that the Rust plugin factory has been registered
 * (via the `export_au!` macro's static initializer). The factory is
 * automatically registered when the .component bundle binary loads.
 *
 * Called by BeamerAuWrapper's initialization methods before creating plugin
 * instances to ensure the factory is ready.
 *
 * The function is idempotent - calling it multiple times is safe.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @return true if the factory is registered and ready, false if registration
 *         has not occurred (which indicates the plugin's `export_au!` macro
 *         was not invoked or the static initializer did not run).
 */
bool beamer_au_ensure_factory_registered(void);

/**
 * Fill in an AudioComponentDescription from the registered AU config.
 *
 * This is used by +load to register the AUAudioUnit subclass with the framework.
 *
 * @param desc Pointer to AudioComponentDescription to fill in.
 */
void beamer_au_get_component_description(AudioComponentDescription* desc);

// =============================================================================
// MARK: - Instance Lifecycle
// =============================================================================

/**
 * Create a new plugin instance.
 *
 * Allocates and initializes a new Rust plugin instance in the Unprepared state.
 * The plugin is ready for parameter queries but not for audio processing.
 *
 * Thread Safety: Call from main thread only.
 *
 * @return Opaque handle to the plugin instance, or NULL on failure.
 *         The caller owns this handle and must call `beamer_au_destroy_instance()`
 *         to free it.
 *
 * Possible Failures:
 * - Memory allocation failure
 * - Plugin initialization failure
 */
BeamerAuInstanceHandle _Nullable beamer_au_create_instance(void);

/**
 * Destroy a plugin instance.
 *
 * Deallocates all resources associated with the plugin instance.
 * If render resources are allocated, they are freed first.
 *
 * Thread Safety: Call from main thread only.
 *
 * @param instance Handle to the plugin instance (may be NULL, which is a no-op).
 *
 * Post-condition:
 * - The instance handle is invalid after this call
 * - Any pointers derived from this instance are invalid
 */
void beamer_au_destroy_instance(BeamerAuInstanceHandle _Nullable instance);

// =============================================================================
// MARK: - Render Resources
// =============================================================================

/**
 * Allocate render resources and prepare for audio processing.
 *
 * This transitions the plugin from Unprepared to Prepared state.
 * After this call succeeds, the plugin is ready for `beamer_au_render()` calls.
 *
 * This function:
 * 1. Validates the bus configuration
 * 2. Allocates processing buffers (sized for max_frames)
 * 3. Calls the plugin's `prepare()` method
 * 4. Activates the audio processor
 *
 * Thread Safety: Call from main thread only.
 *
 * @param instance      Handle to the plugin instance.
 * @param sample_rate   Sample rate in Hz (e.g., 44100.0, 48000.0, 96000.0).
 * @param max_frames    Maximum number of frames per render call.
 * @param sample_format Sample format (float32 or float64).
 * @param bus_config    Pointer to bus configuration (copied internally).
 *
 * @return OSStatus:
 *         - noErr (0): Success, plugin is ready for processing
 *         - kAudioUnitErr_InvalidPropertyValue: Invalid sample rate or max_frames
 *         - kAudioUnitErr_FormatNotSupported: Bus configuration not supported
 *         - kAudioUnitErr_FailedInitialization: Plugin preparation failed
 *
 * Pre-conditions:
 * - instance is valid (not NULL, not destroyed)
 * - sample_rate > 0
 * - max_frames > 0 and <= reasonable limit (e.g., 8192)
 * - bus_config is valid pointer
 *
 * Post-conditions on success:
 * - Plugin is in Prepared state
 * - beamer_au_is_prepared() returns true
 * - beamer_au_render() can be called
 */
OSStatus beamer_au_allocate_render_resources(
    BeamerAuInstanceHandle _Nullable instance,
    double sample_rate,
    uint32_t max_frames,
    BeamerAuSampleFormat sample_format,
    const BeamerAuBusConfig* bus_config
);

/**
 * Deallocate render resources and return to unprepared state.
 *
 * This transitions the plugin from Prepared to Unprepared state.
 * After this call, `beamer_au_render()` must not be called.
 *
 * This function:
 * 1. Deactivates the audio processor
 * 2. Frees processing buffers
 * 3. Returns the plugin to initial state
 *
 * Thread Safety: Call from main thread only.
 *
 * @param instance Handle to the plugin instance.
 *
 * Post-conditions:
 * - Plugin is in Unprepared state
 * - beamer_au_is_prepared() returns false
 * - Parameter queries still work
 */
void beamer_au_deallocate_render_resources(BeamerAuInstanceHandle _Nullable instance);

/**
 * Check if render resources are currently allocated.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return true if in Prepared state (ready for rendering), false otherwise.
 */
bool beamer_au_is_prepared(BeamerAuInstanceHandle _Nullable instance);

// =============================================================================
// MARK: - Audio Rendering
// =============================================================================

/**
 * Process audio through the plugin.
 *
 * This is the main audio processing entry point, called from the AU host's
 * render callback (real-time audio thread).
 *
 * REAL-TIME SAFETY:
 * - This function must not allocate memory
 * - This function must not block (no locks, no I/O)
 * - This function must complete quickly (sub-millisecond)
 *
 * Thread Safety: Call from real-time audio thread only.
 *
 * @param instance              Handle to the plugin instance.
 * @param action_flags          Pointer to AudioUnitRenderActionFlags (may be modified).
 * @param timestamp             Pointer to AudioTimeStamp for this render call.
 * @param frame_count           Number of frames to process in this call.
 * @param output_bus_number     Index of the output bus being rendered (usually 0).
 * @param output_data           Pointer to AudioBufferList for output audio.
 *                              For effects, also contains input audio (in-place processing).
 * @param events                Pointer to linked list of AURenderEvent (MIDI, parameter changes).
 *                              May be NULL if no events.
 * @param pull_input_block      Block to pull audio from auxiliary input buses.
 *                              May be NULL if no aux inputs or for instruments.
 * @param musical_context_block Block to query host musical context (tempo, time signature).
 *                              May be NULL if host doesn't provide musical context.
 * @param transport_state_block Block to query host transport state (playing, recording).
 *                              May be NULL if host doesn't provide transport state.
 * @param schedule_midi_block   Block to schedule MIDI output events.
 *                              May be NULL for effect plugins (only available for
 *                              aumu instruments and aumf MIDI effects).
 *
 * @return OSStatus:
 *         - noErr (0): Success
 *         - kAudioUnitErr_Uninitialized: Render resources not allocated
 *         - kAudioUnitErr_CannotDoInCurrentContext: Lock contention (try_lock failed)
 *         - kAudioUnitErr_TooManyFramesToProcess: frame_count exceeds max_frames
 *         - kAudioUnitErr_Render: Processing error
 *
 * Pre-conditions:
 * - beamer_au_is_prepared() returns true
 * - output_data has valid buffers with space for frame_count samples
 * - timestamp is valid
 * - frame_count <= max_frames passed to allocate_render_resources
 *
 * Post-conditions on success:
 * - output_data buffers contain processed audio
 * - MIDI output events (if any) have been scheduled via schedule_midi_block
 */
OSStatus beamer_au_render(
    BeamerAuInstanceHandle _Nullable instance,
    AudioUnitRenderActionFlags* action_flags,
    const AudioTimeStamp* timestamp,
    AUAudioFrameCount frame_count,
    NSInteger output_bus_number,
    AudioBufferList* output_data,
    const AURenderEvent* _Nullable events,
    AURenderPullInputBlock _Nullable pull_input_block,
    const AudioBufferList* _Nullable input_data,
    AUHostMusicalContextBlock _Nullable musical_context_block,
    AUHostTransportStateBlock _Nullable transport_state_block,
    AUScheduleMIDIEventBlock _Nullable schedule_midi_block
);

/**
 * Reset the plugin's DSP state.
 *
 * Clears delay lines, filter states, and other DSP memory.
 * Called when transport stops/starts or when the plugin is bypassed/un-bypassed.
 *
 * Thread Safety: Call from main thread only.
 *
 * @param instance Handle to the plugin instance.
 *
 * Note: This is different from deallocate/reallocate. The plugin remains in
 * Prepared state but with cleared DSP state.
 */
void beamer_au_reset(BeamerAuInstanceHandle _Nullable instance);

// =============================================================================
// MARK: - Parameters
// =============================================================================

/**
 * Get the number of parameters exposed by the plugin.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Number of parameters (0 if instance is invalid).
 */
uint32_t beamer_au_get_parameter_count(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get information about a parameter by index.
 *
 * Used to build the AUParameterTree when the AU is instantiated.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance    Handle to the plugin instance.
 * @param index       Parameter index (0 to count-1).
 * @param out_info    Pointer to structure to fill with parameter info.
 *
 * @return true if successful, false if index out of range or instance invalid.
 */
bool beamer_au_get_parameter_info(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t index,
    BeamerAuParameterInfo* out_info
);

/**
 * Get a parameter's current normalized value.
 *
 * Thread Safety: Can be called from any thread (uses atomics internally).
 *
 * @param instance Handle to the plugin instance.
 * @param param_id Parameter ID (from BeamerAuParameterInfo.id).
 *
 * @return Normalized value (0.0 to 1.0), or 0.0 if parameter not found.
 */
float beamer_au_get_parameter_value(BeamerAuInstanceHandle _Nullable instance, uint32_t param_id);

/**
 * Set a parameter's normalized value.
 *
 * This is called from the AU host when the user changes a parameter or
 * during automation playback.
 *
 * Thread Safety: Can be called from any thread (uses atomics internally).
 *
 * @param instance Handle to the plugin instance.
 * @param param_id Parameter ID (from BeamerAuParameterInfo.id).
 * @param value    Normalized value (0.0 to 1.0, clamped internally).
 *
 * Note: The parameter's smoother will interpolate to the new value over time
 * to avoid zipper noise.
 */
void beamer_au_set_parameter_value(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t param_id,
    float value
);

/**
 * Format a parameter value as a display string.
 *
 * Converts a normalized value to a human-readable string using the parameter's
 * value-to-string function (e.g., "0.5" -> "-6.0 dB").
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param param_id   Parameter ID.
 * @param value      Normalized value to format (0.0 to 1.0).
 * @param out_buffer Buffer to write the formatted string (UTF-8, null-terminated).
 * @param buffer_len Size of out_buffer in bytes (including null terminator).
 *
 * @return Number of bytes written (excluding null terminator), or 0 on error.
 */
uint32_t beamer_au_format_parameter_value(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t param_id,
    float value,
    char* out_buffer,
    uint32_t buffer_len
);

/**
 * Parse a display string to a normalized value.
 *
 * Converts a human-readable string to a normalized value using the parameter's
 * string-to-value function (e.g., "-6.0 dB" -> 0.5).
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param param_id   Parameter ID.
 * @param string     Display string to parse (UTF-8, null-terminated).
 * @param out_value  Pointer to receive the normalized value.
 *
 * @return true if parsing succeeded, false if string is invalid.
 */
bool beamer_au_parse_parameter_value(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t param_id,
    const char* string,
    float* out_value
);

// =============================================================================
// MARK: - State Persistence
// =============================================================================

/**
 * Get the size of the serialized state in bytes.
 *
 * Call this before `beamer_au_get_state()` to allocate an appropriately sized buffer.
 *
 * Thread Safety: Call from main thread only.
 *
 * @param instance Handle to the plugin instance.
 * @return Size of state in bytes, or 0 if no state to save.
 */
uint32_t beamer_au_get_state_size(BeamerAuInstanceHandle _Nullable instance);

/**
 * Serialize the plugin state to a buffer.
 *
 * The state format is compatible with VST3 for cross-format preset sharing.
 * The buffer must be at least `beamer_au_get_state_size()` bytes.
 *
 * Thread Safety: Call from main thread only.
 *
 * @param instance Handle to the plugin instance.
 * @param buffer   Buffer to write state data.
 * @param size     Size of buffer in bytes.
 *
 * @return Number of bytes written, or 0 on error.
 */
uint32_t beamer_au_get_state(
    BeamerAuInstanceHandle _Nullable instance,
    uint8_t* buffer,
    uint32_t size
);

/**
 * Restore plugin state from a buffer.
 *
 * The state format is compatible with VST3 for cross-format preset loading.
 *
 * Thread Safety: Call from main thread only.
 *
 * @param instance Handle to the plugin instance.
 * @param buffer   Buffer containing state data.
 * @param size     Size of data in bytes.
 *
 * @return OSStatus:
 *         - noErr: Success
 *         - kAudioUnitErr_InvalidPropertyValue: Invalid state data format
 */
OSStatus beamer_au_set_state(
    BeamerAuInstanceHandle _Nullable instance,
    const uint8_t* _Nullable buffer,
    uint32_t size
);

// =============================================================================
// MARK: - Properties
// =============================================================================

/**
 * Get the plugin's processing latency in samples.
 *
 * The host uses this for delay compensation to align tracks.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Latency in samples (0 if no latency).
 */
uint32_t beamer_au_get_latency_samples(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get the plugin's tail time in samples.
 *
 * This is the number of samples the plugin will continue to output after
 * input has stopped (e.g., reverb/delay tail). The host uses this to know
 * when to stop processing after playback ends.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Tail time in samples (0 if no tail, UINT32_MAX for infinite tail).
 */
uint32_t beamer_au_get_tail_samples(BeamerAuInstanceHandle _Nullable instance);

/**
 * Float64 processing support level.
 *
 * Beamer supports float64 streams in AU either:
 * - natively (the processor implements f64 processing), or
 * - via internal conversion (f64<->f32 around the f32 processing path).
 */
typedef enum BeamerAuFloat64Support {
    /// Float64 is not supported.
    BeamerAuFloat64SupportNotSupported = 0,
    /// Float64 is supported via internal conversion (always available).
    BeamerAuFloat64SupportViaConversion = 1,
    /// Float64 is supported natively by the processor.
    BeamerAuFloat64SupportNative = 2,
} BeamerAuFloat64Support;

/**
 * Get float64 processing support level.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Float64 support level.
 */
BeamerAuFloat64Support beamer_au_get_float64_support(BeamerAuInstanceHandle _Nullable instance);

// =============================================================================
// MARK: - Plugin Metadata
// =============================================================================

/**
 * Get the plugin's display name.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param out_buffer Buffer to write the name (UTF-8, null-terminated).
 * @param buffer_len Size of out_buffer in bytes.
 *
 * @return Number of bytes written (excluding null terminator).
 */
uint32_t beamer_au_get_name(
    BeamerAuInstanceHandle _Nullable instance,
    char* out_buffer,
    uint32_t buffer_len
);

/**
 * Get the plugin vendor/manufacturer name.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param out_buffer Buffer to write the vendor name (UTF-8, null-terminated).
 * @param buffer_len Size of out_buffer in bytes.
 *
 * @return Number of bytes written (excluding null terminator).
 */
uint32_t beamer_au_get_vendor(
    BeamerAuInstanceHandle _Nullable instance,
    char* out_buffer,
    uint32_t buffer_len
);

// =============================================================================
// MARK: - Bus Queries
// =============================================================================

/**
 * Get the number of input buses the plugin supports.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Number of input buses (0 for generator/instrument, 1+ for effects).
 */
uint32_t beamer_au_get_input_bus_count(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get the number of output buses the plugin supports.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Number of output buses (usually 1, more for multi-output plugins).
 */
uint32_t beamer_au_get_output_bus_count(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get the default channel count for an input bus.
 *
 * Used when setting up bus formats before allocateRenderResources.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance  Handle to the plugin instance.
 * @param bus_index Index of the input bus.
 *
 * @return Default channel count (0 if bus index is invalid).
 */
uint32_t beamer_au_get_input_bus_channel_count(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t bus_index
);

/**
 * Get the default channel count for an output bus.
 *
 * Used when setting up bus formats before allocateRenderResources.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance  Handle to the plugin instance.
 * @param bus_index Index of the output bus.
 *
 * @return Default channel count (0 if bus index is invalid).
 */
uint32_t beamer_au_get_output_bus_channel_count(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t bus_index
);

/**
 * Check if a proposed channel configuration is valid.
 *
 * This is used by shouldChangeToFormat:forBus: to validate that a proposed
 * format change would result in a valid overall configuration. For example,
 * an effect plugin with [-1,-1] capability requires input channels to equal
 * output channels on the main bus.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance               Handle to the plugin instance.
 * @param main_input_channels    Proposed number of channels for main input bus.
 * @param main_output_channels   Proposed number of channels for main output bus.
 *
 * @return true if the channel configuration is valid, false otherwise.
 */
bool beamer_au_is_channel_config_valid(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t main_input_channels,
    uint32_t main_output_channels
);

// =============================================================================
// MARK: - MIDI Support
// =============================================================================

/**
 * Check if the plugin accepts MIDI input.
 *
 * Returns true for instruments (aumu) and MIDI effects (aumf).
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return true if plugin accepts MIDI input events.
 */
bool beamer_au_accepts_midi(BeamerAuInstanceHandle _Nullable instance);

/**
 * Check if the plugin produces MIDI output.
 *
 * Returns true for instruments (aumu) that output MIDI and MIDI effects (aumf).
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return true if plugin produces MIDI output events.
 */
bool beamer_au_produces_midi(BeamerAuInstanceHandle _Nullable instance);

NS_ASSUME_NONNULL_END

#ifdef __cplusplus
}
#endif

#endif /* BEAMER_AU_BRIDGE_H */
