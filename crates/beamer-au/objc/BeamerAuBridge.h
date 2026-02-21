/*
 * BeamerAuBridge.h
 *
 * C-ABI bridge between Objective-C AUAudioUnit wrapper and Rust plugin instance.
 *
 * This header defines the interface for the hybrid AU implementation where:
 * - Objective-C provides the AUAudioUnit subclass (BeamerAuWrapper)
 * - Rust provides all DSP, parameter handling and state management
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
 */

#ifndef BEAMER_AU_BRIDGE_H
#define BEAMER_AU_BRIDGE_H

// =============================================================================
// Platform Detection and Fallbacks
// =============================================================================
//
// This header uses Apple/Objective-C types and macros. When parsed by IDE
// tooling (clangd) without proper SDK configuration, we provide stub
// definitions so the header can be analyzed without errors.

#include <stdint.h>
#include <stdbool.h>

// Check if AudioToolbox is actually available (not just __APPLE__ defined)
#if defined(__has_include) && __has_include(<AudioToolbox/AudioToolbox.h>)
#include <AudioToolbox/AudioToolbox.h>
#define BEAMER_HAS_AUDIOTOOLBOX 1
#endif

#ifndef BEAMER_HAS_AUDIOTOOLBOX
// Stub type definitions for IDE parsing when SDK headers unavailable
typedef int32_t OSStatus;
typedef uint32_t AudioUnitRenderActionFlags;
typedef uint32_t AUAudioFrameCount;
typedef int64_t AUEventSampleTime;
typedef long NSInteger;
typedef struct AudioTimeStamp { uint64_t mHostTime; } AudioTimeStamp;
typedef struct AudioBufferList { uint32_t mNumberBuffers; } AudioBufferList;
typedef struct AudioComponentDescription { uint32_t componentType; } AudioComponentDescription;
typedef struct AURenderEvent { int type; } AURenderEvent;
typedef void* AURenderPullInputBlock;
typedef void* AUHostMusicalContextBlock;
typedef void* AUHostTransportStateBlock;
typedef void* AUScheduleMIDIEventBlock;
#endif

// Nullability annotation fallbacks for non-clang or missing SDK
#ifndef NS_ASSUME_NONNULL_BEGIN
#define NS_ASSUME_NONNULL_BEGIN
#define NS_ASSUME_NONNULL_END
#define _Nullable
#define _Nonnull
#endif

#ifdef __cplusplus
extern "C" {
#endif

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
 * - Values are in actual units (e.g., -60 to +12 dB)
 * - The ObjC wrapper uses min_value and max_value for the AUParameter range
 * - Display values are formatted by Rust via `beamer_au_format_parameter_value()`
 */
typedef struct {
    /// Parameter ID (unique within the plugin, maps to AU parameter address)
    uint32_t id;
    /// Human-readable parameter name (UTF-8, null-terminated)
    char name[BEAMER_AU_MAX_PARAM_NAME_LENGTH];
    /// Parameter unit string (e.g., "dB", "Hz", "ms"; UTF-8, null-terminated)
    char units[BEAMER_AU_MAX_PARAM_NAME_LENGTH];
    /// AudioUnitParameterUnit value for host UI hints.
    ///
    /// This tells AU hosts what visual control to render:
    /// - 0 = kAudioUnitParameterUnit_Generic (slider)
    /// - 1 = kAudioUnitParameterUnit_Indexed (dropdown)
    /// - 2 = kAudioUnitParameterUnit_Boolean (checkbox)
    /// - 13 = kAudioUnitParameterUnit_Decibels
    /// - 8 = kAudioUnitParameterUnit_Hertz
    /// - etc. (see AudioUnitProperties.h for full list)
    uint32_t unit_type;
    /// Minimum actual value (e.g., -60.0 for dB)
    float min_value;
    /// Maximum actual value (e.g., 12.0 for dB)
    float max_value;
    /// Default actual value (in the range min_value to max_value)
    float default_value;
    /// Current actual value (in the range min_value to max_value)
    float current_value;
    /// Number of discrete steps (0 = continuous, 1 = boolean, N = N+1 states)
    int32_t step_count;
    /// Flags (reserved for future use: automatable, hidden, etc.)
    uint32_t flags;
    /// Group ID this parameter belongs to (0 = root/ungrouped)
    int32_t group_id;
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

/**
 * Maximum length of group name strings.
 *
 * Names longer than this are truncated.
 */
#define BEAMER_AU_MAX_GROUP_NAME_LENGTH 128

/**
 * Parameter group metadata for building hierarchical AUParameterTree.
 *
 * Groups organize parameters into folders in the DAW's parameter list.
 * Groups can be nested via parent_id references to form a tree structure.
 *
 * Special values:
 * - Group ID 0 is the root group (implicit, never returned by getGroupInfo for index > 0)
 * - parent_id = 0 means the group is at the top level
 */
typedef struct {
    /// Unique group identifier (matches VST3 UnitId)
    int32_t id;
    /// Human-readable group name (UTF-8, null-terminated)
    char name[BEAMER_AU_MAX_GROUP_NAME_LENGTH];
    /// Parent group ID (0 = top-level, i.e., child of root)
    int32_t parent_id;
} BeamerAuGroupInfo;

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
 * Clears delay lines, filter states and other DSP memory.
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
 * Get a parameter's current value in AU format (actual value).
 *
 * Returns the actual value for the parameter in its native units (e.g., dB, Hz, ms).
 * For indexed parameters, returns the index value (0 to step_count).
 *
 * This function handles the conversion from normalized to actual values internally,
 * eliminating the need for AU wrappers to duplicate the conversion logic.
 *
 * Thread Safety: Can be called from any thread (uses atomics internally).
 *
 * @param instance Handle to the plugin instance.
 * @param param_id Parameter ID (from BeamerAuParameterInfo.id).
 *
 * @return Actual value in the parameter's native units (min_value to max_value range).
 */
float beamer_au_get_parameter_value_au(BeamerAuInstanceHandle _Nullable instance, uint32_t param_id);

/**
 * Set a parameter's value from AU format (actual value).
 *
 * Accepts the actual value in the parameter's native units (e.g., dB, Hz, ms)
 * and converts it to normalized internally.
 * For indexed parameters, accepts the index value (0 to step_count).
 *
 * This function handles the conversion from actual to normalized values internally,
 * eliminating the need for AU wrappers to duplicate the conversion logic.
 *
 * Thread Safety: Can be called from any thread (uses atomics internally).
 *
 * @param instance Handle to the plugin instance.
 * @param param_id Parameter ID (from BeamerAuParameterInfo.id).
 * @param value    Actual value in the parameter's native units (min_value to max_value range).
 *
 * Note: The parameter's smoother will interpolate to the new value over time
 * to avoid zipper noise.
 */
void beamer_au_set_parameter_value_au(
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

/**
 * Get the number of discrete value strings for an indexed parameter.
 *
 * For enum/indexed parameters (unit_type = Indexed), returns the number of
 * possible values (step_count + 1). This is used to build the valueStrings
 * array for AUParameter.
 *
 * For continuous parameters or those without indexed unit type, returns 0.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance  Handle to the plugin instance.
 * @param param_id  Parameter ID (from BeamerAuParameterInfo.id).
 *
 * @return Number of value strings (0 if not an indexed parameter).
 */
uint32_t beamer_au_get_parameter_value_count(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t param_id
);

/**
 * Get the display string for a specific value of an indexed parameter.
 *
 * For enum parameters, index 0 returns the first variant name, index 1
 * returns the second, etc. This is used to populate the valueStrings array
 * for AUParameter creation.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance     Handle to the plugin instance.
 * @param param_id     Parameter ID (from BeamerAuParameterInfo.id).
 * @param value_index  Index of the value (0 to count-1).
 * @param out_string   Buffer to receive the string (UTF-8, null-terminated).
 * @param max_length   Maximum buffer length including null terminator.
 *
 * @return true if successful, false if index out of range or not indexed parameter.
 */
bool beamer_au_get_parameter_value_string(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t param_id,
    uint32_t value_index,
    char* out_string,
    uint32_t max_length
);

// =============================================================================
// MARK: - Parameter Groups
// =============================================================================

/**
 * Get the number of parameter groups (including root group).
 *
 * Returns 1 if there are no explicit groups (just the root group).
 * For nested groups, returns 1 + total nested groups.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Number of groups (minimum 1 for root), or 0 if instance invalid.
 */
uint32_t beamer_au_get_group_count(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get information about a parameter group by index.
 *
 * Index 0 returns the root group (id=0, name="", parent_id=0).
 * Used to build hierarchical AUParameterTree with AUParameterGroup nodes.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param index      Group index (0 to count-1).
 * @param out_info   Pointer to structure to fill with group info.
 *
 * @return true if successful, false if index out of range or instance invalid.
 */
bool beamer_au_get_group_info(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t index,
    BeamerAuGroupInfo* _Nonnull out_info
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
// MARK: - GUI / WebView
// =============================================================================

/**
 * Check if the plugin has a custom GUI.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return true if the plugin provides a custom WebView GUI.
 */
bool beamer_au_has_gui(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get the dev server URL.
 *
 * Returns NULL in production mode (embedded assets are used instead).
 * In dev mode, returns a null-terminated URL like "http://localhost:5173".
 *
 * The returned pointer is valid for the lifetime of the process and must
 * not be freed by the caller.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Null-terminated URL string, or NULL if not in dev mode.
 */
const char* _Nullable beamer_au_get_gui_html(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get the initial GUI size in pixels.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @param width    Pointer to receive the GUI width.
 * @param height   Pointer to receive the GUI height.
 */
void beamer_au_get_gui_size(BeamerAuInstanceHandle _Nullable instance,
                            uint32_t* _Nonnull width,
                            uint32_t* _Nonnull height);

// =============================================================================
// MARK: - WebView C-ABI (beamer-webview)
// =============================================================================

/**
 * Create a WebView serving embedded assets via custom URL scheme.
 *
 * Assets must be registered via register_assets() before calling this.
 * The WebView navigates to beamer://localhost/index.html.
 *
 * Thread Safety: Must be called from the main thread.
 *
 * @param parent    A valid NSView* pointer to attach the WebView to.
 * @param dev_tools Whether to enable Web Inspector.
 *
 * @return Opaque handle to the WebView, or NULL on failure.
 *         Must be destroyed with beamer_webview_destroy().
 */
void* _Nullable beamer_webview_create(void* _Nonnull parent,
                                      bool dev_tools);

/**
 * Create a WebView that loads from a URL (dev server mode).
 *
 * Thread Safety: Must be called from the main thread.
 *
 * @param parent    A valid NSView* pointer to attach the WebView to.
 * @param url       Null-terminated UTF-8 URL to navigate to.
 * @param dev_tools Whether to enable Web Inspector.
 *
 * @return Opaque handle to the WebView, or NULL on failure.
 *         Must be destroyed with beamer_webview_destroy().
 */
void* _Nullable beamer_webview_create_url(void* _Nonnull parent,
                                          const char* _Nonnull url,
                                          bool dev_tools);

/**
 * Update the WebView frame.
 *
 * @param handle Opaque WebView handle from beamer_webview_create().
 * @param x      X origin (bottom-left coordinate system).
 * @param y      Y origin.
 * @param width  Width in points.
 * @param height Height in points.
 */
void beamer_webview_set_frame(void* _Nonnull handle,
                              int32_t x, int32_t y,
                              int32_t width, int32_t height);

/**
 * Detach and destroy the WebView.
 *
 * @param handle Opaque WebView handle from beamer_webview_create().
 *               Must not be used after this call.
 */
void beamer_webview_destroy(void* _Nullable handle);

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
// MARK: - Channel Capabilities
// =============================================================================

/**
 * Maximum number of channel capability entries a plugin can declare.
 *
 * Most plugins only need 1-3 configurations (e.g., mono, stereo, surround).
 */
#define BEAMER_AU_MAX_CHANNEL_CAPABILITIES 16

/**
 * A single channel capability entry representing a supported [input, output] pair.
 *
 * AU channel capabilities use signed integers with special semantics:
 * - `-1` means "any number of channels" (wildcard)
 * - `0` means "no channels" (e.g., for instruments with no audio input)
 * - Positive values indicate exact channel counts
 *
 * Common patterns:
 * - `[-1, -1]`: Any matching input/output (typical for effects)
 * - `[0, 2]`: Stereo instrument (no input, stereo output)
 * - `[2, 2]`: Stereo effect (stereo in, stereo out)
 * - `[1, 1]`: Mono effect
 */
typedef struct {
    /// Number of input channels (-1 = any, 0 = none, >0 = exact count)
    int32_t input_channels;
    /// Number of output channels (-1 = any, 0 = none, >0 = exact count)
    int32_t output_channels;
} BeamerAuChannelCapability;

/**
 * Channel capabilities result containing all supported configurations.
 *
 * The AU framework uses this to populate the `channelCapabilities` property.
 */
typedef struct {
    /// Number of valid capability entries (0 means "any configuration supported")
    uint32_t count;
    /// Array of supported [input, output] channel configurations
    BeamerAuChannelCapability capabilities[BEAMER_AU_MAX_CHANNEL_CAPABILITIES];
} BeamerAuChannelCapabilities;

/**
 * Get the supported channel capabilities for the main bus.
 *
 * This function returns the [input, output] channel configurations that
 * the plugin supports, based on its component type and declared bus configuration.
 *
 * Capability semantics:
 * - Effects (aufx): Returns [-1, -1] meaning "any matching configuration"
 * - Instruments (aumu): Returns [0, N] where N is declared output channel count
 * - MIDI Processors (aumi): Returns [-1, -1] like effects
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance          Handle to the plugin instance (may be NULL for static query).
 * @param out_capabilities  Pointer to structure to fill with channel capabilities.
 *
 * @return true if capabilities were successfully retrieved, false on error.
 */
bool beamer_au_get_channel_capabilities(
    BeamerAuInstanceHandle _Nullable instance,
    BeamerAuChannelCapabilities* out_capabilities
);

// =============================================================================
// MARK: - Factory Presets
// =============================================================================

/**
 * Preset information for building AUAudioUnitPreset / AUPreset arrays.
 *
 * This structure provides information about a single factory preset,
 * including its index number and display name.
 */
typedef struct {
    /// Preset number/index (0-based, maps to AUPreset.presetNumber)
    int32_t number;
    /// Human-readable preset name (UTF-8, null-terminated)
    char name[BEAMER_AU_MAX_PARAM_NAME_LENGTH];
} BeamerAuPresetInfo;

/**
 * Get the number of factory presets.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Number of factory presets (0 if none or instance invalid).
 */
uint32_t beamer_au_get_preset_count(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get information about a factory preset by index.
 *
 * Used to build factory preset arrays for AU hosts.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance  Handle to the plugin instance.
 * @param index     Preset index (0 to count-1).
 * @param out_info  Pointer to structure to fill with preset info.
 *
 * @return true if successful, false if index out of range or instance invalid.
 */
bool beamer_au_get_preset_info(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t index,
    BeamerAuPresetInfo* _Nonnull out_info
);

/**
 * Apply a factory preset by index.
 *
 * This sets all parameters defined in the preset to their preset values.
 * Parameters not defined in the preset retain their current values (sparse application).
 *
 * Thread Safety: Can be called from any thread (parameter changes use atomics internally).
 *
 * @param instance      Handle to the plugin instance.
 * @param preset_index  Preset index (0 to count-1).
 *
 * @return true if the preset was applied successfully, false if index out of range.
 */
bool beamer_au_apply_preset(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t preset_index
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
