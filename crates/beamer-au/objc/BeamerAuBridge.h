/*
 * BeamerAuBridge.h
 *
 * C-ABI bridge between the native Audio Unit wrappers and the Rust plugin instance.
 *
 * This header defines the interface for the hybrid AU implementation where:
 * - Native wrappers, generated per plugin, implement the AUv2 plug-in interface
 *   and the AUv3 AUAudioUnit subclass (Beamer<Name>AuWrapper_<codes>)
 * - Rust provides all DSP, parameter handling and state management
 *
 * The bridge is designed for:
 * - Full feature parity with VST3 (aux buses, f32/f64, MIDI, parameters, state)
 * - Zero-allocation audio processing (pre-allocated buffers in Rust)
 * - Comprehensive error handling via OSStatus return codes
 *
 * Thread Safety:
 * - Lifecycle functions (create/destroy/allocate/deallocate) must not be called
 *   at the same time as each other
 * - Render function is called from real-time audio thread (no allocations, no locks)
 * - Parameter get/set may be called from any thread (uses atomics internally)
 * - State save/load may be called from any thread, also while audio is rendered
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
 * The AUv2 and AUv3 wrappers store this handle and pass it to all bridge functions.
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
    /// Number of channel labels in layout_labels: the channel count when the
    /// host set a channel layout on the bus, 0 when it set none.
    uint32_t layout_label_count;
    /// The channel label (AudioChannelLabel) of each channel of the host's
    /// channel layout.
    uint32_t layout_labels[BEAMER_AU_MAX_CHANNELS];
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
 * - Values are in AU format: actual units (e.g., -60 to +12 dB), or an index
 *   from 0 to step_count for indexed parameters
 * - The wrappers use min_value, max_value and default_value as they are
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
    /// Minimum value in AU format (e.g., -60.0 for dB, 0 for an indexed parameter)
    float min_value;
    /// Maximum value in AU format (e.g., 12.0 for dB, step_count for an indexed parameter)
    float max_value;
    /// Default value in AU format (in the range min_value to max_value)
    float default_value;
    /// Current value in AU format (in the range min_value to max_value)
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
 * automatically registered when the plugin library loads.
 *
 * Called by the wrappers before creating plugin instances to ensure the
 * factory is ready.
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
 * Calling it on a prepared instance prepares the plugin again with the new
 * configuration.
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
 * @param rendering_offline Whether the host renders without realtime deadlines
 *                      (an offline bounce). The plugin sees it as its process mode.
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
    const BeamerAuBusConfig* bus_config,
    bool rendering_offline
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
 * Delay the output of a bypassed render call of the main bus by the
 * plugin's latency, after the shim filled it with the main input (input
 * buffer i in output buffer i).
 *
 * A host that bypasses a plugin without a bypass parameter leaves the
 * plugin out: the input passes on, delayed like the plugin's output, so
 * that the track stays aligned with the host's delay compensation. Does
 * nothing for a plugin with a bypass parameter (which bypasses itself) or
 * without latency.
 *
 * An output buffer that shares memory with another or with the input
 * stays undelayed.
 *
 * Thread Safety: The render thread.
 *
 * @param instance    Handle to the plugin instance.
 * @param input_data  The input list the shim copied from, NULL without one.
 * @param output_data The output list of the render call.
 * @param frame_count Frames of the render call.
 */
void beamer_au_delay_bypassed_output(
    BeamerAuInstanceHandle _Nullable instance,
    const AudioBufferList* _Nullable input_data,
    AudioBufferList* _Nullable output_data,
    uint32_t frame_count
);

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
 * Flags for BeamerAuTransport.
 *
 * The "Has" flags tell which groups of fields the host provided. The other
 * flags describe the transport state and are valid with
 * BeamerAuTransportHasState.
 */
typedef enum {
    /// tempo and beat_position are valid
    BeamerAuTransportHasBeatAndTempo = 1 << 0,
    /// The time signature and measure_downbeat_position are valid
    BeamerAuTransportHasTimeSignature = 1 << 1,
    /// The state flags below, sample_position and the cycle range are valid
    BeamerAuTransportHasState = 1 << 2,
    /// The transport is moving
    BeamerAuTransportPlaying = 1 << 3,
    /// The host is recording
    BeamerAuTransportRecording = 1 << 4,
    /// Cycle (loop) mode is on
    BeamerAuTransportCycling = 1 << 5,
} BeamerAuTransportFlags;

/**
 * Host transport and musical context for one render call.
 *
 * The wrapper asks the host on the render thread (the musical context and
 * transport state blocks of an AUv3, the host callbacks of an AUv2) and passes
 * the answers to beamer_au_render(). Fields of a group the host did not
 * provide are zero.
 */
typedef struct {
    /// Combination of BeamerAuTransportFlags
    uint32_t flags;
    /// Time signature denominator (e.g., 4 in 3/4)
    int32_t time_sig_denominator;
    /// Tempo in beats per minute
    double tempo;
    /// Position on the timeline in beats (quarter notes)
    double beat_position;
    /// Time signature numerator (e.g., 3 in 3/4)
    double time_sig_numerator;
    /// Position of the start of the current measure in beats
    double measure_downbeat_position;
    /// Position on the timeline in samples
    double sample_position;
    /// Start of the cycle (loop) range in beats
    double cycle_start_beat;
    /// End of the cycle (loop) range in beats
    double cycle_end_beat;
} BeamerAuTransport;

/**
 * Receives one MIDI message the plugin produced during beamer_au_render().
 *
 * The wrapper passes a function of this type when the host listens to the
 * plugin's MIDI output. It forwards the message to the host. Called on the
 * render thread, from inside beamer_au_render().
 *
 * @param context       The context of the BeamerAuMidiOutput.
 * @param sample_offset Position of the message in the current buffer.
 * @param bytes         One complete MIDI 1.0 message: a channel message without
 *                      running status, or a SysEx message from F0 to F7. Only
 *                      valid during the call.
 * @param length        Number of bytes in the message.
 */
typedef void (*BeamerAuMidiOutputFn)(
    void* _Nullable context,
    uint32_t sample_offset,
    const uint8_t* bytes,
    uint32_t length
);

/**
 * Receives one MIDI message the plugin produced during beamer_au_render(), as
 * a Universal MIDI Packet message.
 *
 * Called on the render thread, from inside beamer_au_render().
 *
 * @param context       The context of the BeamerAuMidiOutput.
 * @param sample_offset Position of the message in the current buffer.
 * @param words         One UMP message (one SysEx7 packet of a longer SysEx
 *                      message), on group 0. Only valid during the call.
 * @param word_count    Number of words in the message.
 */
typedef void (*BeamerAuUmpOutputFn)(
    void* _Nullable context,
    uint32_t sample_offset,
    const uint32_t* words,
    uint32_t word_count
);

/**
 * Where the MIDI the plugin produces in a render call goes: MIDI 1.0 bytes
 * for a host that takes them, or UMP messages in a protocol for a host that
 * takes event lists. send_words is taken over send_bytes.
 */
typedef struct {
    BeamerAuMidiOutputFn _Nullable send_bytes;
    BeamerAuUmpOutputFn _Nullable send_words;
    // The protocol of the UMP messages, a MIDIProtocolID (1 or 2).
    int32_t protocol;
    void* _Nullable context;
} BeamerAuMidiOutput;

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
 * @param transport             Transport and musical context the wrapper got from the host
 *                              for this call. May be NULL, as it is for the renders of
 *                              output buses other than 0, which do not process audio.
 * @param midi_output           Where the MIDI the plugin produces in this call goes.
 *                              NULL if the host does not listen, the MIDI output is
 *                              then discarded.
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
 * - MIDI output events (if any) have been passed to midi_output
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
    const BeamerAuTransport* _Nullable transport,
    const BeamerAuMidiOutput* _Nullable midi_output
);

/**
 * Reset the plugin's DSP state.
 *
 * Clears delay lines, filter states and other DSP memory.
 * Called when transport stops/starts or when the plugin is bypassed/un-bypassed.
 *
 * Thread Safety: Can be called from any thread, also while audio is rendered.
 * The reset itself runs on the audio thread, at the start of the next render call.
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
 * Format a parameter value in AU format as a display string.
 *
 * Accepts the value the host holds: a plain (actual) value in the
 * parameter's native range (e.g. dB, Hz), or the index for an indexed
 * parameter. Normalizes internally using f64 precision to
 * avoid f32 round-trip artifacts in the display string.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance    Handle to the plugin instance.
 * @param param_id    Parameter ID.
 * @param value       Value in AU format (plain value, or index).
 * @param out_buffer  Buffer to write the formatted string (UTF-8, null-terminated).
 * @param buffer_len  Size of out_buffer in bytes (including null terminator).
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
 * Parse a display string to a parameter value in AU format.
 *
 * Converts a human-readable string using the parameter's string-to-value
 * function. The result is what the host holds: the plain (actual) value
 * (e.g., "-6.0 dB" -> -6.0), or the index for an indexed parameter.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param param_id   Parameter ID.
 * @param string     Display string to parse (UTF-8, null-terminated).
 * @param out_value  Pointer to receive the value in AU format.
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
 * Save the plugin state and return it in a new buffer.
 *
 * The state is serialized once, so the buffer always holds one whole state.
 * The state format is compatible with VST3 for cross-format preset sharing.
 * Release the buffer with `beamer_au_free_state()`.
 *
 * Thread Safety: Can be called from any thread, also while audio is rendered.
 *
 * @param instance Handle to the plugin instance.
 * @param out_size Receives the size of the state in bytes, 0 if it cannot
 *                 be saved.
 *
 * @return The state, or NULL if it cannot be saved.
 */
uint8_t* _Nullable beamer_au_copy_state(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t* _Nullable out_size
);

/**
 * Release a buffer returned by `beamer_au_copy_state()`.
 *
 * @param state The buffer, or NULL.
 * @param size  The size that `beamer_au_copy_state()` reported for it.
 */
void beamer_au_free_state(uint8_t* _Nullable state, uint32_t size);

/**
 * Restore plugin state from a buffer.
 *
 * The state format is compatible with VST3 for cross-format preset loading.
 *
 * Thread Safety: Can be called from any thread, also while audio is rendered.
 * The parameter smoothers jump to the loaded values on the audio thread, at the
 * start of the next render call.
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
 * @return Latency in samples as of the most recent render call (0 if no latency
 *         or if render resources are not allocated).
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
 * @return Tail time in samples as of the most recent render call (0 if no tail or
 *         if render resources are not allocated, UINT32_MAX for infinite tail).
 */
uint32_t beamer_au_get_tail_samples(BeamerAuInstanceHandle _Nullable instance);

/** The latency changed (BeamerAuHostChangeFn). */
#define BEAMER_AU_HOST_CHANGE_LATENCY (1u << 0)
/** The tail changed (BeamerAuHostChangeFn). */
#define BEAMER_AU_HOST_CHANGE_TAIL (1u << 1)
/* Bit 2 is a VST3 change (parameter values), never passed to the shim. */
/**
 * The bypass parameter changed from the plugin side (its editor,
 * automation, a loaded state), so the host's bypass did
 * (BeamerAuHostChangeFn).
 */
#define BEAMER_AU_HOST_CHANGE_BYPASS (1u << 3)

/**
 * Function that tells the host about changes, on the main thread.
 *
 * @param context The context given to beamer_au_set_host_change_callback.
 * @param changes BEAMER_AU_HOST_CHANGE_* bits: what changed since the last
 *                call.
 */
typedef void (*BeamerAuHostChangeFn)(void* _Nullable context, uint32_t changes);

/**
 * Set the function that tells the host about changes, or none (NULL).
 *
 * It runs on the main thread when a render call ended with another latency,
 * tail or bypass parameter than the host knows: the shim notifies the
 * property listeners (AUv2) or key-value observers (AUv3). Preparing and
 * releasing render resources is no change: the host asks for the values
 * then anyway.
 *
 * Clearing it waits for a call in progress, so that the context may go
 * afterwards. beamer_au_destroy_instance clears it as well. Do not call
 * this from within the callback.
 *
 * Thread Safety: Any thread except from within the callback.
 *
 * @param instance Handle to the plugin instance.
 * @param callback The function, or NULL for none.
 * @param context  Passed to the function; must stay valid while it is set.
 */
void beamer_au_set_host_change_callback(
    BeamerAuInstanceHandle _Nullable instance,
    BeamerAuHostChangeFn _Nullable callback,
    void* _Nullable context
);

/**
 * The plugin's bypass parameter, which the host's bypass goes to.
 *
 * With one, kAudioUnitProperty_BypassEffect (AUv2) and shouldBypassEffect
 * (AUv3) set and read that parameter: the plugin keeps processing and
 * ramps (and delays) its bypass itself. Without one, the shim passes the
 * input on.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @param out_id   Receives the parameter id.
 * @return true if the plugin has a bypass parameter.
 */
bool beamer_au_get_bypass_parameter(BeamerAuInstanceHandle _Nullable instance, uint32_t* _Nullable out_id);

/**
 * Note the bypass the host set, before the shim applies it, so that the
 * host is not told about it again. A diagnostics build logs it.
 *
 * Thread Safety: Host threads.
 */
void beamer_au_note_host_bypass(BeamerAuInstanceHandle _Nullable instance, bool bypassed);

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
// MARK: - Editor / WebView
// =============================================================================

/**
 * Check if the plugin has an editor (a web UI).
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return true if the plugin provides its own WebView editor.
 */
bool beamer_au_has_editor(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get the dev server URL of the web UI.
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
const char* _Nullable beamer_au_get_webview_url(BeamerAuInstanceHandle _Nullable instance);

/**
 * Get the pointer to the web UI's embedded files.
 *
 * Returns an opaque pointer suitable for passing to beamer_webview_create().
 * Returns NULL if no assets are configured (e.g. dev server mode).
 *
 * Thread Safety: Safe to call from any thread.
 */
const void* _Nullable beamer_au_get_webview_assets(void);

/**
 * Get the initial editor size in pixels.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @param width    Pointer to receive the editor width.
 * @param height   Pointer to receive the editor height.
 */
void beamer_au_get_editor_size(BeamerAuInstanceHandle _Nullable instance,
                               uint32_t* _Nonnull width,
                               uint32_t* _Nonnull height);

/**
 * Get the editor background color (RGBA, 0-255).
 *
 * Used to set the parent view's layer background so the correct color
 * shows while web content loads, preventing a white flash.
 *
 * Thread Safety: Safe to call from any thread.
 *
 * @param out Pointer to at least 4 writable bytes (R, G, B, A).
 */
void beamer_au_get_editor_background_color(uint8_t* _Nonnull out);

// =============================================================================
// MARK: - Editor
// =============================================================================

/**
 * Free a string returned by a beamer_au function (e.g.,
 * beamer_au_editor_sync_frame_script).
 *
 * @param ptr Pointer returned by a beamer_au string-producing function, or NULL (no-op).
 */
void beamer_au_free_string(char* _Nullable ptr);

/**
 * What one open editor uses to keep its web UI in step with the plugin.
 *
 * An editor holds this from the moment it opens until it closes. It stays
 * valid when the host destroys the plugin instance before the editor.
 */
typedef struct BeamerAuEditorSync* BeamerAuEditorSyncHandle;

/**
 * An editor was opened: start keeping its web UI in step.
 *
 * Events the plugin emits from now on are kept for this editor.
 *
 * Thread Safety: Main thread.
 *
 * @param instance Handle to the plugin instance.
 * @return The handle, or NULL for a NULL instance. Must be closed with
 *         beamer_au_editor_sync_close.
 */
BeamerAuEditorSyncHandle _Nullable beamer_au_editor_sync_open(BeamerAuInstanceHandle _Nullable instance);

/**
 * The script that hands a freshly loaded page its parameters and the
 * plugin format.
 *
 * Thread Safety: Main thread.
 *
 * @param sync          Handle from beamer_au_editor_sync_open, or NULL.
 * @param plugin_format "auv2" or "auv3".
 * @return JavaScript to evaluate, or NULL on failure. Caller must free with
 *         beamer_au_free_string.
 */
char* _Nullable beamer_au_editor_sync_init_script(BeamerAuEditorSyncHandle _Nullable sync,
                                                  const char* _Nonnull plugin_format);

/**
 * The script of one frame: the parameters that changed since the last
 * frame, then the events the plugin emitted.
 *
 * Thread Safety: Main thread.
 *
 * @param sync Handle from beamer_au_editor_sync_open, or NULL.
 * @return JavaScript to evaluate, or NULL when nothing changed. Caller must
 *         free with beamer_au_free_string.
 */
char* _Nullable beamer_au_editor_sync_frame_script(BeamerAuEditorSyncHandle _Nullable sync);

/**
 * What an editor does in the host when its web UI edits a parameter. Each
 * function gets `context`. A NULL function leaves the host untold.
 *
 * The AUv3 sets the AUParameter and sends its gesture events, the AUv2
 * sends the gestures and each value change as parameter events.
 */
typedef struct {
    /** Passed to each function. */
    void* _Nullable context;
    /** The web UI began a gesture on a parameter. */
    void (* _Nullable begin_gesture)(void* _Nullable context, uint32_t param_id);
    /** The web UI set a parameter to a normalized value, which the plugin
     *  holds already. The host may round it. */
    void (* _Nullable set_parameter)(void* _Nullable context, uint32_t param_id, double normalized);
    /** The web UI ended a gesture on a parameter, or its page went away
     *  during one. */
    void (* _Nullable end_gesture)(void* _Nullable context, uint32_t param_id);
} BeamerAuEditorHost;

/**
 * Handle a message of the web UI: set the parameter or State field it
 * names or pass it to the plugin, tell the host through `host` and return
 * the script that answers it.
 *
 * Thread Safety: Main thread.
 *
 * @param sync     Handle from beamer_au_editor_sync_open, or NULL.
 * @param json     The message as the web UI sent it (UTF-8 JSON).
 * @param json_len Length of the message in bytes.
 * @param host     What the editor does in the host. Its context must stay
 *                 valid during the call.
 * @return JavaScript that answers the message, or NULL for a message
 *         without an answer and on failure. Caller must free with
 *         beamer_au_free_string.
 */
char* _Nullable beamer_au_editor_sync_message(BeamerAuEditorSyncHandle _Nullable sync,
                                              const uint8_t* _Nonnull json,
                                              size_t json_len,
                                              const BeamerAuEditorHost* _Nonnull host);

/**
 * End in the host, through `host`, the gestures the web UI began and did
 * not end: its page goes away, as the editor closes or the page loads
 * again, so their ends will not come.
 *
 * Thread Safety: Main thread.
 *
 * @param sync Handle from beamer_au_editor_sync_open, or NULL.
 * @param host What the editor does in the host. Its context must stay
 *             valid during the call.
 */
void beamer_au_editor_sync_end_open_gestures(BeamerAuEditorSyncHandle _Nullable sync,
                                             const BeamerAuEditorHost* _Nonnull host);

/**
 * The editor was closed: stop keeping events for it and free the handle.
 *
 * Thread Safety: Main thread.
 *
 * @param sync Handle from beamer_au_editor_sync_open, or NULL. Invalid
 *             after this call.
 */
void beamer_au_editor_sync_close(BeamerAuEditorSyncHandle _Nullable sync);

// =============================================================================
// MARK: - WebView C-ABI (beamer-webview)
// =============================================================================

/**
 * Create a WebView serving embedded assets via custom URL scheme.
 *
 * Thread Safety: Must be called from the main thread.
 *
 * @param parent           A valid NSView* pointer to attach the WebView to.
 * @param assets           Opaque assets pointer from beamer_au_get_webview_assets().
 * @param background_color Pointer to 4 bytes (RGBA) or NULL for default.
 *
 * @return Opaque handle to the WebView, or NULL on failure.
 *         Must be destroyed with beamer_webview_destroy().
 */
void* _Nullable beamer_webview_create(void* _Nonnull parent,
                                      const void* _Nonnull assets,
                                      const uint8_t* _Nullable background_color);

/**
 * Create a WebView that loads from a URL (dev server mode).
 *
 * Thread Safety: Must be called from the main thread.
 *
 * @param parent           A valid NSView* pointer to attach the WebView to.
 * @param url              Null-terminated UTF-8 URL to navigate to.
 * @param background_color Pointer to 4 bytes (RGBA) or NULL for default.
 *
 * @return Opaque handle to the WebView, or NULL on failure.
 *         Must be destroyed with beamer_webview_destroy().
 */
void* _Nullable beamer_webview_create_url(void* _Nonnull parent,
                                          const char* _Nonnull url,
                                          const uint8_t* _Nullable background_color);

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

/**
 * Evaluate JavaScript in the WebView.
 *
 * @param handle Opaque WebView handle.
 * @param script UTF-8 script bytes.
 * @param len Length of script in bytes.
 */
void beamer_webview_eval_js(void* _Nonnull handle, const uint8_t* _Nonnull script, size_t len);

/**
 * Create a WebView with IPC support, serving embedded assets.
 *
 * @param parent           NSView* to attach the WebView to.
 * @param assets           Opaque assets pointer from beamer_au_get_webview_assets().
 * @param background_color Pointer to 4 bytes (RGBA) or NULL for default.
 * @param message_callback   Called when JS sends a message to native.
 * @param page_gone_callback Called when the page the WebView showed is gone:
 *                           a new page committed in its place, or its web
 *                           content process ended. Every message of the old
 *                           page came before it.
 * @param loaded_callback    Called each time the WebView finishes loading the
 *                           page, also after its web content process ended.
 * @param callback_context   Opaque pointer passed to all callbacks.
 * @return Opaque WebView handle, or NULL on failure.
 */
void* _Nullable beamer_webview_create_with_ipc(
    void* _Nonnull parent,
    const void* _Nonnull assets,
    const uint8_t* _Nullable background_color,
    void (* _Nonnull message_callback)(void* context, const uint8_t* json, size_t len),
    void (* _Nonnull page_gone_callback)(void* context),
    void (* _Nonnull loaded_callback)(void* context),
    void* _Nullable callback_context
);

/**
 * Create a WebView with IPC support, loading from a URL (dev server mode).
 *
 * @param parent           NSView* to attach the WebView to.
 * @param url              Null-terminated UTF-8 URL.
 * @param background_color Pointer to 4 bytes (RGBA) or NULL for default.
 * @param message_callback   Called when JS sends a message to native.
 * @param page_gone_callback Called when the page the WebView showed is gone:
 *                           a new page committed in its place, or its web
 *                           content process ended. Every message of the old
 *                           page came before it.
 * @param loaded_callback    Called each time the WebView finishes loading the
 *                           page, also after its web content process ended.
 * @param callback_context   Opaque pointer passed to all callbacks.
 * @return Opaque WebView handle, or NULL on failure.
 */
void* _Nullable beamer_webview_create_url_with_ipc(
    void* _Nonnull parent,
    const char* _Nonnull url,
    const uint8_t* _Nullable background_color,
    void (* _Nonnull message_callback)(void* context, const uint8_t* json, size_t len),
    void (* _Nonnull page_gone_callback)(void* context),
    void (* _Nonnull loaded_callback)(void* context),
    void* _Nullable callback_context
);

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
 * Check whether an input bus is on by default: the main bus, or an aux bus
 * the plugin declares active by default.
 *
 * The AUv3 wrapper seeds the enabled flag of its buses with this.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance  Handle to the plugin instance.
 * @param bus_index Index of the input bus.
 *
 * @return true if the bus is on by default (false if bus index is invalid).
 */
bool beamer_au_is_input_bus_default_active(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t bus_index
);

/**
 * Check whether an output bus is on by default, as for
 * beamer_au_is_input_bus_default_active().
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance  Handle to the plugin instance.
 * @param bus_index Index of the output bus.
 *
 * @return true if the bus is on by default (false if bus index is invalid).
 */
bool beamer_au_is_output_bus_default_active(
    BeamerAuInstanceHandle _Nullable instance,
    uint32_t bus_index
);

/**
 * Copy the name the plugin gives a bus into out_name.
 *
 * The wrappers name their buses with it. The name is UTF-8 with a
 * terminating null, at most capacity bytes, cut at a character.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance  Handle to the plugin instance.
 * @param is_input  true for an input bus, false for an output bus.
 * @param bus_index Index of the bus.
 * @param out_name  Buffer for the name.
 * @param capacity  Size of out_name in bytes.
 *
 * @return true if the bus exists and its name was copied.
 */
bool beamer_au_get_bus_name(
    BeamerAuInstanceHandle _Nullable instance,
    bool is_input,
    uint32_t bus_index,
    char* _Nullable out_name,
    uint32_t capacity
);

/**
 * Check if a proposed channel configuration is valid.
 *
 * Called when render resources are allocated (AUv3) and on Initialize (AUv2)
 * to check the main bus formats: one of the plugin's layouts has to take
 * these channel counts on its main input and output bus (0 for a direction
 * without buses).
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
 * One [input, output] pair per layout of the plugin, without repeats: the
 * channel counts of its main input and output bus. The input is 0 for a
 * layout without input buses, the normal case for instruments (aumu) and
 * generators (augn). A bus that takes any set is -1; an output that follows
 * the input is -1 with an input of any count and the input's count
 * otherwise; any output with any input is -2.
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

/**
 * Check whether one of the plugin's layouts takes a channel count on a bus.
 *
 * A bus that follows the main input takes any count here; the whole
 * combination is checked when render resources are allocated.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param is_input   Whether the bus is an input bus.
 * @param bus_index  Index of the bus.
 * @param channels   Number of channels.
 *
 * @return true if a layout takes the count (false for 0 channels or a bus
 *         that does not exist).
 */
bool beamer_au_bus_supports_channel_count(
    BeamerAuInstanceHandle _Nullable instance,
    bool is_input,
    uint32_t bus_index,
    uint32_t channels
);

/**
 * Check whether one of the plugin's layouts takes a channel layout on a bus,
 * given as the channel label (AudioChannelLabel) of each channel, in any
 * order of its channels.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param is_input   Whether the bus is an input bus.
 * @param bus_index  Index of the bus.
 * @param labels     The channel labels, count of them.
 * @param count      Number of channels.
 *
 * @return true if a layout takes the channel layout.
 */
bool beamer_au_bus_supports_layout(
    BeamerAuInstanceHandle _Nullable instance,
    bool is_input,
    uint32_t bus_index,
    const uint32_t* _Nullable labels,
    uint32_t count
);

/**
 * Get the layout tags (AudioChannelLayoutTag) of the channel layouts a bus
 * takes: the sets the plugin's layouts name, every known set for a bus that
 * takes any set.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param is_input   Whether the bus is an input bus.
 * @param bus_index  Index of the bus.
 * @param out_tags   Where to write at most capacity tags, or NULL.
 * @param capacity   Number of tags out_tags holds.
 *
 * @return The number of tags the bus has.
 */
uint32_t beamer_au_get_supported_layout_tags(
    BeamerAuInstanceHandle _Nullable instance,
    bool is_input,
    uint32_t bus_index,
    uint32_t* _Nullable out_tags,
    uint32_t capacity
);

/**
 * Get the layout tag (AudioChannelLayoutTag) of the set a bus has with a
 * channel count when the host sets no channel layout.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance   Handle to the plugin instance.
 * @param is_input   Whether the bus is an input bus.
 * @param bus_index  Index of the bus.
 * @param channels   Number of channels.
 *
 * @return The layout tag, 0 for 0 channels.
 */
uint32_t beamer_au_get_layout_tag(
    BeamerAuInstanceHandle _Nullable instance,
    bool is_input,
    uint32_t bus_index,
    uint32_t channels
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
 * The preset sets the parameters it names. The others keep their current
 * values.
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
 * Check if the plugin produces MIDI output.
 *
 * Returns midi_output of the plugin's config, on by default for MIDI
 * effects (aumi).
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return true if plugin produces MIDI output events.
 */
bool beamer_au_produces_midi(BeamerAuInstanceHandle _Nullable instance);

/**
 * Read the MIDI input of a render call that the host bypassed, without the
 * plugin; pass it on to midi_output if given: a bypassed MIDI effect passes
 * its MIDI on, in the same form as the plugin's output. Reading it keeps a
 * SysEx message or an RPN sequence that spans the bypass whole.
 *
 * Thread Safety: The render thread; does not wait for a lock.
 *
 * @param instance              Handle to the plugin instance.
 * @param timestamp             The render call's time stamp.
 * @param frame_count           Frames of the render call.
 * @param events                The render call's events, or NULL.
 * @param midi_output           Where the MIDI passed on goes, or NULL.
 * @return false when the MIDI could not be read now (a host thread prepares
 *         the instance): keep it for the next call.
 */
bool beamer_au_pass_midi_through(
    BeamerAuInstanceHandle _Nullable instance,
    const AudioTimeStamp* _Nonnull timestamp,
    uint32_t frame_count,
    const AURenderEvent* _Nullable events,
    const BeamerAuMidiOutput* _Nullable midi_output
);

/**
 * The MIDI protocol the plugin asks the host for (midi_protocol in
 * Config.toml), as a MIDIProtocolID: 1 for MIDI 1.0, 2 for MIDI 2.0. 0 for a
 * plugin without MIDI input, which reports no protocol.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return The protocol, or 0.
 */
int32_t beamer_au_get_midi_protocol(BeamerAuInstanceHandle _Nullable instance);

/**
 * The SysEx bytes one render call holds in each direction for a plugin with
 * MIDI input (sysex_bytes_per_block in Config.toml), 0 without MIDI input.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Bytes, counting the F0 and F7 of each message.
 */
uint32_t beamer_au_get_sysex_input_bytes(BeamerAuInstanceHandle _Nullable instance);

/**
 * The SysEx bytes one render call holds in each direction for a plugin with
 * MIDI output (sysex_bytes_per_block in Config.toml), 0 without MIDI output.
 *
 * Thread Safety: Can be called from any thread.
 *
 * @param instance Handle to the plugin instance.
 * @return Bytes, counting the F0 and F7 of each message.
 */
uint32_t beamer_au_get_sysex_output_bytes(BeamerAuInstanceHandle _Nullable instance);

/**
 * Note SysEx messages from the host that were dropped before a render call:
 * the queue was full or a message was larger than a render call holds.
 *
 * Thread Safety: Safe on the render thread (no lock, no allocation).
 *
 * @param instance Handle to the plugin instance.
 * @param count Messages dropped.
 */
void beamer_au_note_sysex_input_dropped(BeamerAuInstanceHandle _Nullable instance, uint32_t count);

// =============================================================================
// MARK: - Diagnostics
// =============================================================================

#ifdef BEAMER_DIAGNOSTICS

/**
 * Log a line about a host call, such as a stream format the host set.
 *
 * Thread Safety: Host threads.
 */
void beamer_au_diagnostics_note(BeamerAuInstanceHandle _Nullable instance, const char* _Nullable text);

/**
 * Note whether the host's main output buffers have memory of their own
 * in a render call of the main bus, before the shim points any at its own.
 *
 * Thread Safety: Render thread.
 */
void beamer_au_diagnostics_host_output_memory(BeamerAuInstanceHandle _Nullable instance, bool has_memory);

/**
 * Note the time stamp of a packet the host sent through
 * MusicDeviceMIDIEventList, before the shim folds it into the event's time.
 *
 * Thread Safety: Any thread.
 */
void beamer_au_diagnostics_host_packet_time_stamp(BeamerAuInstanceHandle _Nullable instance, uint64_t time_stamp);

/**
 * Measure the output of a render call of the main bus that the host
 * bypassed: the plugin was left out and the shim filled the output.
 *
 * Thread Safety: Render thread.
 */
void beamer_au_diagnostics_bypassed_render(
    BeamerAuInstanceHandle _Nullable instance,
    const AudioBufferList* output_data,
    AUAudioFrameCount frame_count
);

/**
 * Whether every buffer of a list has memory, for the diagnostics.
 */
static inline bool beamer_au_diagnostics_has_memory(const AudioBufferList* _Nullable list) {
    if (list == NULL || list->mNumberBuffers == 0) {
        return false;
    }
    for (UInt32 i = 0; i < list->mNumberBuffers; i++) {
        if (list->mBuffers[i].mData == NULL) {
            return false;
        }
    }
    return true;
}

#endif /* BEAMER_DIAGNOSTICS */

NS_ASSUME_NONNULL_END

#ifdef __cplusplus
}
#endif

#endif /* BEAMER_AU_BRIDGE_H */
