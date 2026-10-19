// =============================================================================
// AUv2 Plugin Implementation for {{PLUGIN_NAME}}
// =============================================================================
// Auto-generated proper AUv2 implementation that returns AudioComponentPlugInInterface*.
// All plugin logic is handled by the beamer_au_* bridge functions.

#include <AudioToolbox/AudioToolbox.h>
#include <AudioUnit/AudioUnit.h>
#include <CoreFoundation/CoreFoundation.h>
#include <CoreMIDI/CoreMIDI.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <stdatomic.h>

#import <Cocoa/Cocoa.h>
#import <AudioUnit/AUCocoaUIView.h>

// Include the bridge header for Rust plugin access
#include "BeamerAuBridge.h"
#include "au_ipc_helpers.h"

// CocoaUI class interfaces (implementations at end of file)
@interface {{COCOA_GUI_VIEW_CLASS}} : NSView {
@public
    void* _webviewHandle;
    BeamerAuInstanceHandle _rustInstance;
    AudioUnit _audioUnit;
    NSTimer* _syncTimer;
    BeamerAuEditorSyncHandle _sync;
}
- (instancetype)initWithFrame:(NSRect)frame
                 webviewHandle:(void*)handle
                  rustInstance:(BeamerAuInstanceHandle)rustInstance
                     audioUnit:(AudioUnit)audioUnit;
@end

@interface {{COCOA_VIEW_FACTORY_CLASS}} : NSObject <AUCocoaUIBase>
@end

// =============================================================================
// MARK: - Constants
// =============================================================================

#define MAX_PROPERTY_LISTENERS 64
#define MAX_RENDER_NOTIFY 32

// Parameter changes the host schedules for the next render call. A power of
// two, like the size of every event queue.
#define PARAMETER_QUEUE_SIZE 512

// Size of the packet list that collects the MIDI output of one render call.
// Enough for a full buffer of channel messages. When more is produced, the
// list goes to the host in parts.
#define MIDI_OUTPUT_LIST_BYTES 32768

// Private property for view factory to retrieve the Rust instance handle.
// Uses the AU custom property range (64000+).
#define kBeamerAuPropertyRustInstance 64000

// =============================================================================
// MARK: - Data Structures
// =============================================================================

typedef struct {
    AudioUnitPropertyID propID;
    AudioUnitPropertyListenerProc proc;
    void* userData;
} PropertyListener;

typedef struct {
    AURenderCallback proc;
    void* userData;
} RenderNotify;

typedef struct {
    AudioUnit sourceAU;
    UInt32 sourceOutputNumber;
} InputConnection;

// What the host sets for the render thread to call: the input source (a
// callback or a connection), the MIDI output callback and the host
// callbacks.
typedef struct {
    AURenderCallbackStruct input;
    InputConnection inputConnection;
    AUMIDIOutputCallbackStruct midiOutput;
    HostCallbackInfo host;
} CallbackSet;

// One place in an event queue. The sequence number says whose turn it is:
// equal to the position, a producer may fill the slot; one more, the event
// in it is ready for the consumer.
typedef struct {
    _Atomic UInt32 sequence;
    AURenderEvent event;
} EventSlot;

// Render events for the next render call, in a bounded ring that several
// threads may add to at once and one thread at a time takes from, without
// locks (a ring with a sequence number per slot). Producers claim a position
// with compare-and-swap. The consumer is whoever holds queueReaderMutex.
typedef struct {
    EventSlot* slots;
    UInt32 mask;
    // Next position to fill, shared by the producers.
    _Atomic UInt32 writePosition;
    // Next position to take. Only the consumer uses it.
    UInt32 readPosition;
} EventQueue;

typedef struct BeamerAuv2Instance {
    // AudioComponentPlugInInterface MUST be first (ABI requirement)
    AudioComponentPlugInInterface interface;
    AudioComponentInstance componentInstance;
    BeamerAuInstanceHandle rustInstance;

    // Audio configuration
    Float64 sampleRate;
    UInt32 maxFramesPerSlice;
    _Atomic bool initialized;
    _Atomic bool bypassed;
    bool offlineRender;

    // Stream formats per bus (indexed by element number)
    AudioStreamBasicDescription inputFormats[BEAMER_AU_MAX_BUSES];
    AudioStreamBasicDescription outputFormats[BEAMER_AU_MAX_BUSES];

    // Allocated input buffer for pulling
    AudioBufferList* inputBufferList;
    UInt32 inputBufferCapacity;
    void* ownedInputBuffers[BEAMER_AU_MAX_CHANNELS]; // tracks our calloc'd mData pointers

    // Property listeners
    PropertyListener propertyListeners[MAX_PROPERTY_LISTENERS];
    UInt32 propertyListenerCount;
    pthread_mutex_t listenerMutex;

    // Render notifications
    RenderNotify renderNotify[MAX_RENDER_NOTIFY];
    UInt32 renderNotifyCount;
    pthread_mutex_t renderNotifyMutex;

    // Callbacks as the host set them. Host threads write them under the
    // mutex. The render thread only tries the mutex and copies them into
    // renderCallbacks, which only it uses: it never waits, and a copy is
    // never half written.
    CallbackSet callbacks;
    pthread_mutex_t callbacksMutex;
    CallbackSet renderCallbacks;

    // Factory presets
    CFArrayRef factoryPresets;         // CFArray of AUPreset pointers (NULL callbacks)
    AUPreset* presetStorage;           // Backing storage for preset structs
    uint32_t presetCount;              // Number of factory presets
    int32_t currentPresetIndex;        // -1 = no preset, >=0 = factory preset index
    CFStringRef currentPresetName;     // Current preset name for ClassInfo round-trip

    // MIDI input for the next render call. MIDIEvent adds to it, from the
    // render thread or from another thread.
    EventSlot midiSlots[BEAMER_AU_MAX_MIDI_EVENTS];
    EventQueue midiQueue;

    // Parameter changes with a sample offset, as render events.
    // ScheduleParameters and SetParameter add to it.
    EventSlot parameterSlots[PARAMETER_QUEUE_SIZE];
    EventQueue parameterQueue;

    // Held while taking events from the two queues. The render thread only
    // tries it and leaves the events for its next call when it is held.
    // Reset, Initialize and Uninitialize take it.
    pthread_mutex_t queueReaderMutex;

    // MIDI output. The list is only allocated for a plugin with MIDI
    // output, and the host sets the callback when it listens.
    bool isMidiEffect;
    MIDIPacketList* midiOutputList;
} BeamerAuv2Instance;

// MIDI output of one render call.
typedef struct {
    AUMIDIOutputCallbackStruct callback;
    const AudioTimeStamp* timeStamp;
    MIDIPacketList* list;
    // Write position in the list.
    MIDIPacket* packet;
} BeamerAuv2MidiOutput;

// =============================================================================
// MARK: - Forward Declarations
// =============================================================================

static OSStatus BeamerAuv2Open(void* self, AudioComponentInstance ci);
static OSStatus BeamerAuv2Close(void* self);
static AudioComponentMethod BeamerAuv2Lookup(SInt16 selector);
static void FreeInputBufferList(BeamerAuv2Instance* inst);

static OSStatus BeamerAuv2Initialize(void* self);
static OSStatus BeamerAuv2Uninitialize(void* self);
static OSStatus BeamerAuv2GetPropertyInfo(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, UInt32* outDataSize, Boolean* outWritable);
static OSStatus BeamerAuv2GetProperty(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, void* outData, UInt32* ioDataSize);
static OSStatus BeamerAuv2SetProperty(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, const void* inData, UInt32 inDataSize);
static OSStatus BeamerAuv2AddPropertyListener(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc, void* userData);
static OSStatus BeamerAuv2RemovePropertyListener(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc);
static OSStatus BeamerAuv2RemovePropertyListenerWithUserData(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc, void* userData);
static OSStatus BeamerAuv2GetParameter(void* self, AudioUnitParameterID paramID,
    AudioUnitScope scope, AudioUnitElement element, AudioUnitParameterValue* outValue);
static OSStatus BeamerAuv2SetParameter(void* self, AudioUnitParameterID paramID,
    AudioUnitScope scope, AudioUnitElement element, AudioUnitParameterValue value, UInt32 bufferOffset);
static OSStatus BeamerAuv2ScheduleParameters(void* self, const AudioUnitParameterEvent* events, UInt32 numEvents);
static OSStatus BeamerAuv2Render(void* self, AudioUnitRenderActionFlags* ioActionFlags,
    const AudioTimeStamp* inTimeStamp, UInt32 inOutputBusNumber, UInt32 inNumberFrames, AudioBufferList* ioData);
static OSStatus BeamerAuv2Reset(void* self, AudioUnitScope scope, AudioUnitElement element);
static OSStatus BeamerAuv2AddRenderNotify(void* self, AURenderCallback proc, void* userData);
static OSStatus BeamerAuv2RemoveRenderNotify(void* self, AURenderCallback proc, void* userData);
static OSStatus BeamerAuv2MIDIEvent(void* self, UInt32 inStatus, UInt32 inData1, UInt32 inData2, UInt32 inOffsetSampleFrame);
static void BeamerAuv2ApplyPendingParameters(BeamerAuv2Instance* inst);
static void BeamerAuv2DiscardMidi(BeamerAuv2Instance* inst);

// =============================================================================
// MARK: - Helper Functions
// =============================================================================

static void InitDefaultFormat(AudioStreamBasicDescription* format, Float64 sampleRate, UInt32 channels) {
    memset(format, 0, sizeof(AudioStreamBasicDescription));
    format->mSampleRate = sampleRate;
    format->mFormatID = kAudioFormatLinearPCM;
    format->mFormatFlags = kAudioFormatFlagsNativeFloatPacked | kAudioFormatFlagIsNonInterleaved;
    format->mBytesPerPacket = sizeof(Float32);
    format->mFramesPerPacket = 1;
    format->mBytesPerFrame = sizeof(Float32);
    format->mChannelsPerFrame = channels;
    format->mBitsPerChannel = 32;
}

// Whether two stream formats are the same, field by field.
static bool StreamFormatsEqual(const AudioStreamBasicDescription* a, const AudioStreamBasicDescription* b) {
    return a->mSampleRate == b->mSampleRate
        && a->mFormatID == b->mFormatID
        && a->mFormatFlags == b->mFormatFlags
        && a->mBytesPerPacket == b->mBytesPerPacket
        && a->mFramesPerPacket == b->mFramesPerPacket
        && a->mBytesPerFrame == b->mBytesPerFrame
        && a->mChannelsPerFrame == b->mChannelsPerFrame
        && a->mBitsPerChannel == b->mBitsPerChannel;
}

static void NotifyPropertyListeners(BeamerAuv2Instance* inst, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element) {
    pthread_mutex_lock(&inst->listenerMutex);
    for (UInt32 i = 0; i < inst->propertyListenerCount; i++) {
        if (inst->propertyListeners[i].propID == propID) {
            inst->propertyListeners[i].proc(
                inst->propertyListeners[i].userData,
                inst->componentInstance,
                propID, scope, element);
        }
    }
    pthread_mutex_unlock(&inst->listenerMutex);
}

static OSStatus EnsureInputBufferList(BeamerAuv2Instance* inst, UInt32 channels, UInt32 frames) {
    if (channels > BEAMER_AU_MAX_CHANNELS) return kAudio_ParamError;
    UInt32 neededCapacity = frames * channels;
    if (inst->inputBufferList && inst->inputBufferCapacity >= neededCapacity) {
        // Existing buffer is large enough. Restore our owned mData pointers
        // because the host may have replaced them during a previous render.
        for (UInt32 i = 0; i < inst->inputBufferList->mNumberBuffers; i++) {
            inst->inputBufferList->mBuffers[i].mDataByteSize = frames * sizeof(Float32);
            inst->inputBufferList->mBuffers[i].mData = inst->ownedInputBuffers[i];
        }
        return noErr;
    }

    // Free old buffer if it exists
    if (inst->inputBufferList) {
        for (UInt32 i = 0; i < inst->inputBufferList->mNumberBuffers; i++) {
            free(inst->ownedInputBuffers[i]);
            inst->ownedInputBuffers[i] = NULL;
        }
        free(inst->inputBufferList);
    }

    // Allocate new buffer list (non-interleaved: one buffer per channel)
    size_t listSize = sizeof(AudioBufferList) + (channels > 0 ? (channels - 1) * sizeof(AudioBuffer) : 0);
    inst->inputBufferList = (AudioBufferList*)calloc(1, listSize);
    if (!inst->inputBufferList) return kAudio_MemFullError;

    inst->inputBufferList->mNumberBuffers = channels;
    for (UInt32 i = 0; i < channels; i++) {
        inst->inputBufferList->mBuffers[i].mNumberChannels = 1;
        inst->inputBufferList->mBuffers[i].mDataByteSize = frames * sizeof(Float32);
        inst->ownedInputBuffers[i] = calloc(frames, sizeof(Float32));
        inst->inputBufferList->mBuffers[i].mData = inst->ownedInputBuffers[i];
        if (!inst->ownedInputBuffers[i]) {
            FreeInputBufferList(inst);
            return kAudio_MemFullError;
        }
    }

    inst->inputBufferCapacity = neededCapacity;
    return noErr;
}

static void FreeInputBufferList(BeamerAuv2Instance* inst) {
    if (inst->inputBufferList) {
        // Free our owned pointers, not mData (host may have replaced them)
        for (UInt32 i = 0; i < inst->inputBufferList->mNumberBuffers; i++) {
            free(inst->ownedInputBuffers[i]);
            inst->ownedInputBuffers[i] = NULL;
        }
        free(inst->inputBufferList);
        inst->inputBufferList = NULL;
        inst->inputBufferCapacity = 0;
    }
}

// =============================================================================
// MARK: - Event Queues
// =============================================================================

static void EventQueueInit(EventQueue* queue, EventSlot* slots, UInt32 size) {
    queue->slots = slots;
    queue->mask = size - 1;
    for (UInt32 i = 0; i < size; i++) {
        atomic_init(&slots[i].sequence, i);
    }
    atomic_init(&queue->writePosition, 0);
    queue->readPosition = 0;
}

// Add an event. Any thread may call it, also several at once. Returns false
// when the queue is full.
static bool EventQueuePush(EventQueue* queue, const AURenderEvent* event) {
    UInt32 position = atomic_load_explicit(&queue->writePosition, memory_order_relaxed);
    for (;;) {
        EventSlot* slot = &queue->slots[position & queue->mask];
        UInt32 sequence = atomic_load_explicit(&slot->sequence, memory_order_acquire);
        int32_t difference = (int32_t)(sequence - position);
        if (difference == 0) {
            // The slot is free: claim the position. On failure position holds
            // the one another producer moved on to.
            if (atomic_compare_exchange_weak_explicit(&queue->writePosition, &position,
                    position + 1, memory_order_relaxed, memory_order_relaxed)) {
                slot->event = *event;
                atomic_store_explicit(&slot->sequence, position + 1, memory_order_release);
                return true;
            }
        } else if (difference < 0) {
            // The consumer has not given this slot back yet.
            return false;
        } else {
            // Another producer claimed this position.
            position = atomic_load_explicit(&queue->writePosition, memory_order_relaxed);
        }
    }
}

// The number of events ready for the consumer, in order from the read
// position. An event whose producer is still writing it ends the count.
static UInt32 EventQueueReadyCount(const EventQueue* queue) {
    UInt32 count = 0;
    while (count <= queue->mask) {
        UInt32 position = queue->readPosition + count;
        const EventSlot* slot = &queue->slots[position & queue->mask];
        if (atomic_load_explicit(&slot->sequence, memory_order_acquire) != position + 1) {
            break;
        }
        count++;
    }
    return count;
}

// A ready event, by its index from the read position. It stays the
// consumer's until it is released.
static AURenderEvent* EventQueueAt(EventQueue* queue, UInt32 index) {
    return &queue->slots[(queue->readPosition + index) & queue->mask].event;
}

// Give the first count ready events back to the producers.
static void EventQueueRelease(EventQueue* queue, UInt32 count) {
    for (UInt32 i = 0; i < count; i++) {
        UInt32 position = queue->readPosition + i;
        atomic_store_explicit(&queue->slots[position & queue->mask].sequence,
            position + queue->mask + 1, memory_order_release);
    }
    queue->readPosition += count;
}

// =============================================================================
// MARK: - Factory Function
// =============================================================================

__attribute__((visibility("default")))
void* {{FACTORY_NAME}}(const AudioComponentDescription* inDesc) {
    (void)inDesc;

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)calloc(1, sizeof(BeamerAuv2Instance));
    if (!inst) return NULL;

    // Set up the interface function pointers
    inst->interface.Open = BeamerAuv2Open;
    inst->interface.Close = BeamerAuv2Close;
    inst->interface.Lookup = BeamerAuv2Lookup;
    inst->interface.reserved = NULL;

    // Set defaults
    inst->sampleRate = 44100.0;
    inst->maxFramesPerSlice = 1024;
    atomic_init(&inst->initialized, false);
    atomic_init(&inst->bypassed, false);

    // Initialize mutexes
    pthread_mutex_init(&inst->listenerMutex, NULL);
    pthread_mutex_init(&inst->renderNotifyMutex, NULL);
    pthread_mutex_init(&inst->callbacksMutex, NULL);
    pthread_mutex_init(&inst->queueReaderMutex, NULL);

    EventQueueInit(&inst->midiQueue, inst->midiSlots, BEAMER_AU_MAX_MIDI_EVENTS);
    EventQueueInit(&inst->parameterQueue, inst->parameterSlots, PARAMETER_QUEUE_SIZE);

    return &inst->interface;
}

// =============================================================================
// MARK: - Open/Close/Lookup
// =============================================================================

static OSStatus BeamerAuv2Open(void* self, AudioComponentInstance ci) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    inst->componentInstance = ci;

    // Ensure Rust factory is registered
    if (!beamer_au_ensure_factory_registered()) {
        return kAudioUnitErr_FailedInitialization;
    }

    // Create Rust plugin instance
    inst->rustInstance = beamer_au_create_instance();
    if (!inst->rustInstance) {
        return kAudioUnitErr_FailedInitialization;
    }

    // Query bus configuration from Rust and set up default formats per bus
    uint32_t inputBusCount = beamer_au_get_input_bus_count(inst->rustInstance);
    uint32_t outputBusCount = beamer_au_get_output_bus_count(inst->rustInstance);

    for (uint32_t i = 0; i < inputBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
        uint32_t ch = beamer_au_get_input_bus_channel_count(inst->rustInstance, i);
        if (ch == 0) ch = 2; // Default to stereo
        InitDefaultFormat(&inst->inputFormats[i], inst->sampleRate, ch);
    }
    for (uint32_t i = 0; i < outputBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
        uint32_t ch = beamer_au_get_output_bus_channel_count(inst->rustInstance, i);
        if (ch == 0) ch = 2; // Default to stereo
        InitDefaultFormat(&inst->outputFormats[i], inst->sampleRate, ch);
    }

    // Prepare MIDI output for a plugin with MIDI output.
    AudioComponentDescription desc;
    beamer_au_get_component_description(&desc);
    inst->isMidiEffect = (desc.componentType == kAudioUnitType_MIDIProcessor);
    if (beamer_au_produces_midi(inst->rustInstance)) {
        inst->midiOutputList = (MIDIPacketList*)calloc(1, MIDI_OUTPUT_LIST_BYTES);
    }

    // Build factory presets cache
    uint32_t presetCount = beamer_au_get_preset_count(inst->rustInstance);
    inst->presetCount = presetCount;
    inst->currentPresetIndex = -1;
    inst->currentPresetName = NULL;

    if (presetCount > 0) {
        // Allocate backing storage for AUPreset structs
        inst->presetStorage = (AUPreset*)calloc(presetCount, sizeof(AUPreset));
        if (inst->presetStorage) {
            // Initialize each preset from Rust
            for (uint32_t i = 0; i < presetCount; i++) {
                BeamerAuPresetInfo info;
                memset(&info, 0, sizeof(info));
                if (beamer_au_get_preset_info(inst->rustInstance, i, &info)) {
                    inst->presetStorage[i].presetNumber = (SInt32)info.number;
                    inst->presetStorage[i].presetName = CFStringCreateWithCString(
                        kCFAllocatorDefault, info.name, kCFStringEncodingUTF8);
                }
            }

            // Build CFArray with NULL callbacks (stores raw pointers to AUPreset)
            CFMutableArrayRef presets = CFArrayCreateMutable(kCFAllocatorDefault, presetCount, NULL);
            if (presets) {
                for (uint32_t i = 0; i < presetCount; i++) {
                    CFArrayAppendValue(presets, &inst->presetStorage[i]);
                }
                inst->factoryPresets = presets;
            } else {
                inst->factoryPresets = NULL;
            }
        } else {
            inst->factoryPresets = NULL;
            inst->presetCount = 0;
        }
    } else {
        inst->factoryPresets = NULL;
        inst->presetStorage = NULL;
    }

    return noErr;
}

static OSStatus BeamerAuv2Close(void* self) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
        beamer_au_deallocate_render_resources(inst->rustInstance);
        atomic_store_explicit(&inst->initialized, false, memory_order_release);
    }

    if (inst->rustInstance) {
        beamer_au_destroy_instance(inst->rustInstance);
        inst->rustInstance = NULL;
    }

    FreeInputBufferList(inst);

    free(inst->midiOutputList);
    inst->midiOutputList = NULL;

    // Release current preset name
    if (inst->currentPresetName) {
        CFRelease(inst->currentPresetName);
        inst->currentPresetName = NULL;
    }

    // Release factory presets
    if (inst->factoryPresets) {
        CFRelease(inst->factoryPresets);
        inst->factoryPresets = NULL;
    }

    // Free preset storage and release dynamically created CFStrings
    if (inst->presetStorage) {
        for (uint32_t i = 0; i < inst->presetCount; i++) {
            if (inst->presetStorage[i].presetName) {
                CFRelease(inst->presetStorage[i].presetName);
            }
        }
        free(inst->presetStorage);
        inst->presetStorage = NULL;
    }

    pthread_mutex_destroy(&inst->listenerMutex);
    pthread_mutex_destroy(&inst->renderNotifyMutex);
    pthread_mutex_destroy(&inst->callbacksMutex);
    pthread_mutex_destroy(&inst->queueReaderMutex);

    free(inst);
    return noErr;
}

static AudioComponentMethod BeamerAuv2Lookup(SInt16 selector) {
    switch (selector) {
        case kAudioUnitInitializeSelect:
            return (AudioComponentMethod)BeamerAuv2Initialize;
        case kAudioUnitUninitializeSelect:
            return (AudioComponentMethod)BeamerAuv2Uninitialize;
        case kAudioUnitGetPropertyInfoSelect:
            return (AudioComponentMethod)BeamerAuv2GetPropertyInfo;
        case kAudioUnitGetPropertySelect:
            return (AudioComponentMethod)BeamerAuv2GetProperty;
        case kAudioUnitSetPropertySelect:
            return (AudioComponentMethod)BeamerAuv2SetProperty;
        case kAudioUnitAddPropertyListenerSelect:
            return (AudioComponentMethod)BeamerAuv2AddPropertyListener;
        case kAudioUnitRemovePropertyListenerSelect:
            return (AudioComponentMethod)BeamerAuv2RemovePropertyListener;
        case kAudioUnitRemovePropertyListenerWithUserDataSelect:
            return (AudioComponentMethod)BeamerAuv2RemovePropertyListenerWithUserData;
        case kAudioUnitGetParameterSelect:
            return (AudioComponentMethod)BeamerAuv2GetParameter;
        case kAudioUnitSetParameterSelect:
            return (AudioComponentMethod)BeamerAuv2SetParameter;
        case kAudioUnitScheduleParametersSelect:
            return (AudioComponentMethod)BeamerAuv2ScheduleParameters;
        case kAudioUnitRenderSelect:
            return (AudioComponentMethod)BeamerAuv2Render;
        case kAudioUnitResetSelect:
            return (AudioComponentMethod)BeamerAuv2Reset;
        case kAudioUnitAddRenderNotifySelect:
            return (AudioComponentMethod)BeamerAuv2AddRenderNotify;
        case kAudioUnitRemoveRenderNotifySelect:
            return (AudioComponentMethod)BeamerAuv2RemoveRenderNotify;
{{MIDI_EVENT_CASE}}
        default:
            return NULL;
    }
}

// =============================================================================
// MARK: - Initialize/Uninitialize
// =============================================================================

// Allocate the render resources in Rust for the current stream formats, frame
// count and offline state. On an initialized unit this prepares the plugin
// again with the new configuration.
static OSStatus BeamerAuv2AllocateRenderResources(BeamerAuv2Instance* inst) {
    // Build bus config from current stream formats
    BeamerAuBusConfig busConfig;
    memset(&busConfig, 0, sizeof(busConfig));

    uint32_t inputBusCount = beamer_au_get_input_bus_count(inst->rustInstance);
    uint32_t outputBusCount = beamer_au_get_output_bus_count(inst->rustInstance);

    busConfig.input_bus_count = inputBusCount;
    busConfig.output_bus_count = outputBusCount;

    uint32_t inputChannels = 0;
    uint32_t outputChannels = 0;

    for (uint32_t i = 0; i < inputBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
        busConfig.input_buses[i].channel_count = inst->inputFormats[i].mChannelsPerFrame;
        busConfig.input_buses[i].bus_type = (i == 0) ? BeamerAuBusTypeMain : BeamerAuBusTypeAuxiliary;
    }
    if (inputBusCount > 0) {
        inputChannels = inst->inputFormats[0].mChannelsPerFrame;
    }

    for (uint32_t i = 0; i < outputBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
        busConfig.output_buses[i].channel_count = inst->outputFormats[i].mChannelsPerFrame;
        busConfig.output_buses[i].bus_type = (i == 0) ? BeamerAuBusTypeMain : BeamerAuBusTypeAuxiliary;
    }
    if (outputBusCount > 0) {
        outputChannels = inst->outputFormats[0].mChannelsPerFrame;
    }

    // Validate channel configuration before proceeding
    bool configValid = beamer_au_is_channel_config_valid(inst->rustInstance, inputChannels, outputChannels);
    if (!configValid) {
        return kAudioUnitErr_FormatNotSupported;
    }

    // Determine sample format
    BeamerAuSampleFormat format = BeamerAuSampleFormatFloat32;
    if (outputBusCount > 0 && inst->outputFormats[0].mBitsPerChannel == 64) {
        format = BeamerAuSampleFormatFloat64;
    }

    // Allocate render resources in Rust
    return beamer_au_allocate_render_resources(
        inst->rustInstance,
        inst->sampleRate,
        inst->maxFramesPerSlice,
        format,
        &busConfig,
        inst->offlineRender
    );
}

static OSStatus BeamerAuv2Initialize(void* self) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
        return noErr; // Already initialized
    }

    OSStatus status = BeamerAuv2AllocateRenderResources(inst);

    if (status == noErr) {
        // MIDI that arrived before does not play. Parameter changes are only
        // queued while initialized, so none should be left; one that is
        // still applies.
        pthread_mutex_lock(&inst->queueReaderMutex);
        BeamerAuv2DiscardMidi(inst);
        BeamerAuv2ApplyPendingParameters(inst);
        pthread_mutex_unlock(&inst->queueReaderMutex);

        // Pre-allocate input buffer if we have input buses
        if (beamer_au_get_input_bus_count(inst->rustInstance) > 0) {
            EnsureInputBufferList(inst, inst->inputFormats[0].mChannelsPerFrame, inst->maxFramesPerSlice);
        }

        // The first copy of the callbacks, so that a render call that cannot
        // take the mutex still has the ones the host set before.
        pthread_mutex_lock(&inst->callbacksMutex);
        inst->renderCallbacks = inst->callbacks;
        pthread_mutex_unlock(&inst->callbacksMutex);

        atomic_store_explicit(&inst->initialized, true, memory_order_release);
    }

    return status;
}

static OSStatus BeamerAuv2Uninitialize(void* self) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
        // From now on parameter changes are set right away. Changes queued
        // for a render call that will not come still apply, and MIDI queued
        // for it is dropped.
        atomic_store_explicit(&inst->initialized, false, memory_order_release);
        atomic_thread_fence(memory_order_seq_cst);
        pthread_mutex_lock(&inst->queueReaderMutex);
        BeamerAuv2ApplyPendingParameters(inst);
        BeamerAuv2DiscardMidi(inst);
        pthread_mutex_unlock(&inst->queueReaderMutex);
        beamer_au_deallocate_render_resources(inst->rustInstance);
    }

    return noErr;
}

// =============================================================================
// MARK: - Property Handling
// =============================================================================

static OSStatus BeamerAuv2GetPropertyInfo(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, UInt32* outDataSize, Boolean* outWritable) {

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    // Default to not writable
    if (outWritable) *outWritable = false;

    switch (propID) {
        // Stream format (only valid for existing buses). The plugin is
        // prepared for the format, so it can only change while uninitialized.
        case kAudioUnitProperty_StreamFormat:
            if (scope == kAudioUnitScope_Input) {
                if (element >= beamer_au_get_input_bus_count(inst->rustInstance))
                    return kAudioUnitErr_InvalidElement;
            } else if (scope == kAudioUnitScope_Output) {
                if (element >= beamer_au_get_output_bus_count(inst->rustInstance))
                    return kAudioUnitErr_InvalidElement;
            }
            if (outDataSize) *outDataSize = sizeof(AudioStreamBasicDescription);
            if (outWritable) *outWritable = !atomic_load_explicit(&inst->initialized, memory_order_acquire);
            return noErr;

        // Sample rate, part of the stream format.
        case kAudioUnitProperty_SampleRate:
            if (outDataSize) *outDataSize = sizeof(Float64);
            if (outWritable) *outWritable = !atomic_load_explicit(&inst->initialized, memory_order_acquire);
            return noErr;

        // Maximum frames per slice
        case kAudioUnitProperty_MaximumFramesPerSlice:
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = true;
            return noErr;

        // Parameter list
        case kAudioUnitProperty_ParameterList:
            if (scope == kAudioUnitScope_Global && element == 0) {
                uint32_t count = beamer_au_get_parameter_count(inst->rustInstance);
                if (outDataSize) *outDataSize = count * sizeof(AudioUnitParameterID);
                if (outWritable) *outWritable = false;
                return noErr;
            }
            return kAudioUnitErr_InvalidScope;

        // Parameter info (element is param ID)
        case kAudioUnitProperty_ParameterInfo:
            if (scope == kAudioUnitScope_Global) {
                if (outDataSize) *outDataSize = sizeof(AudioUnitParameterInfo);
                if (outWritable) *outWritable = false;
                return noErr;
            }
            return kAudioUnitErr_InvalidScope;

        // Parameter value strings (for indexed params)
        case kAudioUnitProperty_ParameterValueStrings:
            if (scope == kAudioUnitScope_Global) {
                uint32_t count = beamer_au_get_parameter_value_count(inst->rustInstance, element);
                if (count > 0) {
                    if (outDataSize) *outDataSize = sizeof(CFArrayRef);
                    if (outWritable) *outWritable = false;
                    return noErr;
                }
            }
            return kAudioUnitErr_InvalidProperty;

        // Parameter string from value (convert value to display string)
        case kAudioUnitProperty_ParameterStringFromValue:
            if (scope == kAudioUnitScope_Global) {
                if (outDataSize) *outDataSize = sizeof(AudioUnitParameterStringFromValue);
                if (outWritable) *outWritable = false;
                return noErr;
            }
            return kAudioUnitErr_InvalidScope;

        // Parameter value from string (convert display string to value)
        case kAudioUnitProperty_ParameterValueFromString:
            if (scope == kAudioUnitScope_Global) {
                if (outDataSize) *outDataSize = sizeof(AudioUnitParameterValueFromString);
                if (outWritable) *outWritable = true;
                return noErr;
            }
            return kAudioUnitErr_InvalidScope;

        // Latency (Global scope only)
        case kAudioUnitProperty_Latency:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (outDataSize) *outDataSize = sizeof(Float64);
            if (outWritable) *outWritable = false;
            return noErr;

        // Tail time (Global scope only)
        case kAudioUnitProperty_TailTime:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (outDataSize) *outDataSize = sizeof(Float64);
            if (outWritable) *outWritable = false;
            return noErr;

        // Bypass (Global scope only)
        case kAudioUnitProperty_BypassEffect:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = true;
            return noErr;

        // Present preset
        case kAudioUnitProperty_PresentPreset:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (outDataSize) *outDataSize = sizeof(AUPreset);
            if (outWritable) *outWritable = true;
            return noErr;

        // Factory presets - only supported when plugin has presets
        case kAudioUnitProperty_FactoryPresets:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            // Only report as supported if we actually have presets
            if (inst->presetCount == 0) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (outDataSize) *outDataSize = sizeof(CFArrayRef);
            if (outWritable) *outWritable = false;  // Factory presets are read-only
            return noErr;

        // Render callback (for setting input source)
        case kAudioUnitProperty_SetRenderCallback:
            if (scope == kAudioUnitScope_Input && element == 0) {
                if (outDataSize) *outDataSize = sizeof(AURenderCallbackStruct);
                if (outWritable) *outWritable = true;
                return noErr;
            }
            return kAudioUnitErr_InvalidScope;

        // Audio unit connection
        case kAudioUnitProperty_MakeConnection:
            if (scope == kAudioUnitScope_Input && element == 0) {
                if (outDataSize) *outDataSize = sizeof(AudioUnitConnection);
                if (outWritable) *outWritable = true;
                return noErr;
            }
            return kAudioUnitErr_InvalidScope;

        // Supported channel layouts
        case kAudioUnitProperty_SupportedNumChannels:
            if (scope == kAudioUnitScope_Global) {
                BeamerAuChannelCapabilities caps;
                if (beamer_au_get_channel_capabilities(inst->rustInstance, &caps)) {
                    if (outDataSize) *outDataSize = caps.count * sizeof(AUChannelInfo);
                    if (outWritable) *outWritable = false;
                    return noErr;
                }
            }
            return kAudioUnitErr_InvalidProperty;

        // Class info (state save/restore)
        case kAudioUnitProperty_ClassInfo:
            if (outDataSize) *outDataSize = sizeof(CFPropertyListRef);
            if (outWritable) *outWritable = true;
            return noErr;

        // Host callbacks
        case kAudioUnitProperty_HostCallbacks:
            if (outDataSize) *outDataSize = sizeof(HostCallbackInfo);
            if (outWritable) *outWritable = true;
            return noErr;

        // Element count
        case kAudioUnitProperty_ElementCount:
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = false;
            return noErr;

        // In-place processing
        case kAudioUnitProperty_InPlaceProcessing:
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = true;
            return noErr;

        // Offline render (Global scope only)
        case kAudioUnitProperty_OfflineRender:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = true;
            return noErr;

        // Should allocate buffer
        case kAudioUnitProperty_ShouldAllocateBuffer:
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = true;
            return noErr;

        // Last render error
        case kAudioUnitProperty_LastRenderError:
            if (outDataSize) *outDataSize = sizeof(OSStatus);
            if (outWritable) *outWritable = false;
            return noErr;

        // MIDI output - only supported when the plugin has MIDI output.
        case kAudioUnitProperty_MIDIOutputCallbackInfo:
        case kAudioUnitProperty_MIDIOutputCallback:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inst->midiOutputList) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (propID == kAudioUnitProperty_MIDIOutputCallbackInfo) {
                if (outDataSize) *outDataSize = sizeof(CFArrayRef);
                if (outWritable) *outWritable = false;
            } else {
                if (outDataSize) *outDataSize = sizeof(AUMIDIOutputCallbackStruct);
                if (outWritable) *outWritable = true;
            }
            return noErr;

        // CocoaUI - only supported when the plugin has a GUI
        case kAudioUnitProperty_CocoaUI:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!beamer_au_has_gui(inst->rustInstance)) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (outDataSize) *outDataSize = sizeof(AudioUnitCocoaViewInfo);
            if (outWritable) *outWritable = false;
            return noErr;

        // Private property: expose Rust instance handle for the view factory
        case kBeamerAuPropertyRustInstance:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (outDataSize) *outDataSize = sizeof(BeamerAuInstanceHandle);
            if (outWritable) *outWritable = false;
            return noErr;

        default:
            return kAudioUnitErr_InvalidProperty;
    }
}

static OSStatus BeamerAuv2GetProperty(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, void* outData, UInt32* ioDataSize) {

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    switch (propID) {
        case kAudioUnitProperty_StreamFormat: {
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioStreamBasicDescription)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            AudioStreamBasicDescription* desc = (AudioStreamBasicDescription*)outData;
            if (scope == kAudioUnitScope_Input) {
                if (element >= beamer_au_get_input_bus_count(inst->rustInstance))
                    return kAudioUnitErr_InvalidElement;
                *desc = inst->inputFormats[element];
            } else if (scope == kAudioUnitScope_Output) {
                if (element >= beamer_au_get_output_bus_count(inst->rustInstance))
                    return kAudioUnitErr_InvalidElement;
                *desc = inst->outputFormats[element];
            } else {
                return kAudioUnitErr_InvalidScope;
            }
            *ioDataSize = sizeof(AudioStreamBasicDescription);
            return noErr;
        }

        case kAudioUnitProperty_SampleRate: {
            if (!outData || !ioDataSize || *ioDataSize < sizeof(Float64)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(Float64*)outData = inst->sampleRate;
            *ioDataSize = sizeof(Float64);
            return noErr;
        }

        case kAudioUnitProperty_MaximumFramesPerSlice: {
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(UInt32*)outData = inst->maxFramesPerSlice;
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }

        case kAudioUnitProperty_ParameterList: {
            if (scope != kAudioUnitScope_Global || element != 0) {
                return kAudioUnitErr_InvalidScope;
            }
            uint32_t count = beamer_au_get_parameter_count(inst->rustInstance);
            UInt32 needed = count * sizeof(AudioUnitParameterID);
            if (!outData || !ioDataSize || *ioDataSize < needed) {
                if (ioDataSize) *ioDataSize = needed;
                return outData ? kAudioUnitErr_InvalidPropertyValue : noErr;
            }
            AudioUnitParameterID* ids = (AudioUnitParameterID*)outData;
            for (uint32_t i = 0; i < count; i++) {
                BeamerAuParameterInfo info;
                if (beamer_au_get_parameter_info(inst->rustInstance, i, &info)) {
                    ids[i] = info.id;
                } else {
                    ids[i] = 0;
                }
            }
            *ioDataSize = needed;
            return noErr;
        }

        case kAudioUnitProperty_ParameterInfo: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioUnitParameterInfo)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            // element is the parameter ID - find it by iterating
            uint32_t count = beamer_au_get_parameter_count(inst->rustInstance);
            for (uint32_t i = 0; i < count; i++) {
                BeamerAuParameterInfo bInfo;
                if (beamer_au_get_parameter_info(inst->rustInstance, i, &bInfo) && bInfo.id == element) {
                    AudioUnitParameterInfo* auInfo = (AudioUnitParameterInfo*)outData;
                    memset(auInfo, 0, sizeof(AudioUnitParameterInfo));

                    // Copy name (CFString)
                    auInfo->cfNameString = CFStringCreateWithCString(NULL, bInfo.name, kCFStringEncodingUTF8);
                    // The plugin formats and parses every value, so a host
                    // asks it for the text of a value and for the value of
                    // a text instead of formatting the number itself.
                    auInfo->flags = kAudioUnitParameterFlag_HasCFNameString |
                                    kAudioUnitParameterFlag_IsReadable |
                                    kAudioUnitParameterFlag_ValuesHaveStrings;
                    if (!(bInfo.flags & BeamerAuParameterFlagReadOnly)) {
                        auInfo->flags |= kAudioUnitParameterFlag_IsWritable;
                    }

                    if (bInfo.flags & BeamerAuParameterFlagAutomatable) {
                        auInfo->flags |= kAudioUnitParameterFlag_IsHighResolution;
                    }

                    // Map unit type
                    auInfo->unit = bInfo.unit_type;

                    // The range from Rust is in AU format: actual values, or
                    // an index (0..step_count) for indexed parameters.
                    auInfo->minValue = bInfo.min_value;
                    auInfo->maxValue = bInfo.max_value;
                    auInfo->defaultValue = bInfo.default_value;

                    // Copy unit label if present
                    if (bInfo.units[0] != '\0') {
                        auInfo->unitName = CFStringCreateWithCString(NULL, bInfo.units, kCFStringEncodingUTF8);
                    }

                    *ioDataSize = sizeof(AudioUnitParameterInfo);
                    return noErr;
                }
            }
            return kAudioUnitErr_InvalidParameter;
        }

        case kAudioUnitProperty_ParameterValueStrings: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            uint32_t count = beamer_au_get_parameter_value_count(inst->rustInstance, element);
            if (count == 0) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(CFArrayRef)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            CFMutableArrayRef array = CFArrayCreateMutable(NULL, count, &kCFTypeArrayCallBacks);
            char buffer[256];
            for (uint32_t i = 0; i < count; i++) {
                if (beamer_au_get_parameter_value_string(inst->rustInstance, element, i, buffer, sizeof(buffer))) {
                    CFStringRef str = CFStringCreateWithCString(NULL, buffer, kCFStringEncodingUTF8);
                    CFArrayAppendValue(array, str);
                    CFRelease(str);
                }
            }
            *(CFArrayRef*)outData = array;
            *ioDataSize = sizeof(CFArrayRef);
            return noErr;
        }

        case kAudioUnitProperty_ParameterStringFromValue: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioUnitParameterStringFromValue)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            AudioUnitParameterStringFromValue* params = (AudioUnitParameterStringFromValue*)outData;
            AudioUnitParameterID paramID = params->inParamID;

            // Get the value to convert (either provided or current)
            float value;
            if (params->inValue != NULL) {
                value = *(params->inValue);
            } else {
                value = beamer_au_get_parameter_value_au(inst->rustInstance, paramID);
            }

            // Pass the AU value directly to Rust which normalizes in f64
            // precision, avoiding f32 round-trip artifacts in display strings.
            char buffer[256];
            uint32_t written = beamer_au_format_parameter_value(inst->rustInstance, paramID, value, buffer, sizeof(buffer));
            if (written > 0) {
                params->outString = CFStringCreateWithCString(NULL, buffer, kCFStringEncodingUTF8);
            } else {
                // Fallback: format as number
                char fallback[64];
                snprintf(fallback, sizeof(fallback), "%.2f", value);
                params->outString = CFStringCreateWithCString(NULL, fallback, kCFStringEncodingUTF8);
            }

            *ioDataSize = sizeof(AudioUnitParameterStringFromValue);
            return noErr;
        }

        case kAudioUnitProperty_ParameterValueFromString: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioUnitParameterValueFromString)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            AudioUnitParameterValueFromString* params = (AudioUnitParameterValueFromString*)outData;
            AudioUnitParameterID paramID = params->inParamID;
            CFStringRef inputString = params->inString;

            if (inputString == NULL) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            char buffer[256];
            if (!CFStringGetCString(inputString, buffer, sizeof(buffer), kCFStringEncodingUTF8)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            // Rust parses to AU format: the plain value, or the index for an
            // indexed parameter. Text it cannot parse is not a value.
            float parsedValue = 0.0f;
            if (!beamer_au_parse_parameter_value(inst->rustInstance, paramID, buffer, &parsedValue)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            params->outValue = parsedValue;

            *ioDataSize = sizeof(AudioUnitParameterValueFromString);
            return noErr;
        }

        case kAudioUnitProperty_Latency: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(Float64)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            uint32_t samples = beamer_au_get_latency_samples(inst->rustInstance);
            *(Float64*)outData = (inst->sampleRate > 0) ? (Float64)samples / inst->sampleRate : 0.0;
            *ioDataSize = sizeof(Float64);
            return noErr;
        }

        case kAudioUnitProperty_TailTime: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(Float64)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            uint32_t samples = beamer_au_get_tail_samples(inst->rustInstance);
            if (samples == UINT32_MAX) {
                *(Float64*)outData = INFINITY;
            } else {
                *(Float64*)outData = (inst->sampleRate > 0) ? (Float64)samples / inst->sampleRate : 0.0;
            }
            *ioDataSize = sizeof(Float64);
            return noErr;
        }

        case kAudioUnitProperty_OfflineRender: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(UInt32*)outData = inst->offlineRender ? 1 : 0;
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }

        case kAudioUnitProperty_BypassEffect: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(UInt32*)outData = atomic_load_explicit(&inst->bypassed, memory_order_relaxed) ? 1 : 0;
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }

        case kAudioUnitProperty_SupportedNumChannels: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            BeamerAuChannelCapabilities caps;
            if (!beamer_au_get_channel_capabilities(inst->rustInstance, &caps)) {
                return kAudioUnitErr_InvalidProperty;
            }
            UInt32 needed = caps.count * sizeof(AUChannelInfo);
            if (!outData || !ioDataSize || *ioDataSize < needed) {
                if (ioDataSize) *ioDataSize = needed;
                return outData ? kAudioUnitErr_InvalidPropertyValue : noErr;
            }
            AUChannelInfo* info = (AUChannelInfo*)outData;
            for (uint32_t i = 0; i < caps.count; i++) {
                info[i].inChannels = (SInt16)caps.capabilities[i].input_channels;
                info[i].outChannels = (SInt16)caps.capabilities[i].output_channels;
            }
            *ioDataSize = needed;
            return noErr;
        }

        case kAudioUnitProperty_ClassInfo: {
            if (!outData || !ioDataSize || *ioDataSize < sizeof(CFPropertyListRef)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            // Get component description for type/subtype/manufacturer
            AudioComponentDescription desc;
            beamer_au_get_component_description(&desc);

            CFMutableDictionaryRef dict = CFDictionaryCreateMutable(NULL, 0,
                &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);

            // Add required type/subtype/manufacturer fields
            SInt32 compType = (SInt32)desc.componentType;
            SInt32 compSubType = (SInt32)desc.componentSubType;
            SInt32 compManu = (SInt32)desc.componentManufacturer;
            CFNumberRef typeNum = CFNumberCreate(NULL, kCFNumberSInt32Type, &compType);
            CFNumberRef subTypeNum = CFNumberCreate(NULL, kCFNumberSInt32Type, &compSubType);
            CFNumberRef manuNum = CFNumberCreate(NULL, kCFNumberSInt32Type, &compManu);
            CFDictionarySetValue(dict, CFSTR("type"), typeNum);
            CFDictionarySetValue(dict, CFSTR("subtype"), subTypeNum);
            CFDictionarySetValue(dict, CFSTR("manufacturer"), manuNum);
            CFRelease(typeNum);
            CFRelease(subTypeNum);
            CFRelease(manuNum);

            // Add current preset name (required by auval ClassInfo round-trip)
            if (inst->currentPresetName) {
                CFDictionarySetValue(dict, CFSTR("name"), inst->currentPresetName);
            } else if (inst->currentPresetIndex >= 0 &&
                       (uint32_t)inst->currentPresetIndex < inst->presetCount &&
                       inst->presetStorage) {
                CFDictionarySetValue(dict, CFSTR("name"),
                    inst->presetStorage[inst->currentPresetIndex].presetName);
            } else {
                CFDictionarySetValue(dict, CFSTR("name"), CFSTR("Untitled"));
            }

            // Store format version
            SInt32 version = 0;
            CFNumberRef versionNum = CFNumberCreate(NULL, kCFNumberSInt32Type, &version);
            CFDictionarySetValue(dict, CFSTR("version"), versionNum);
            CFRelease(versionNum);

            // Get state from Rust (save as "data" key which is the standard AU key)
            uint32_t stateSize = 0;
            uint8_t* stateBuffer = beamer_au_copy_state(inst->rustInstance, &stateSize);
            if (stateBuffer) {
                CFDataRef data = CFDataCreate(NULL, stateBuffer, stateSize);
                CFDictionarySetValue(dict, CFSTR("data"), data);
                CFRelease(data);
                beamer_au_free_state(stateBuffer, stateSize);
            }

            *(CFPropertyListRef*)outData = dict;
            *ioDataSize = sizeof(CFPropertyListRef);
            return noErr;
        }

        case kAudioUnitProperty_ElementCount: {
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            if (scope == kAudioUnitScope_Input) {
                *(UInt32*)outData = beamer_au_get_input_bus_count(inst->rustInstance);
            } else if (scope == kAudioUnitScope_Output) {
                *(UInt32*)outData = beamer_au_get_output_bus_count(inst->rustInstance);
            } else if (scope == kAudioUnitScope_Global) {
                *(UInt32*)outData = 1;
            } else {
                return kAudioUnitErr_InvalidScope;
            }
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }

        case kAudioUnitProperty_InPlaceProcessing: {
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(UInt32*)outData = 0; // Not using in-place processing
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }

        case kAudioUnitProperty_PresentPreset: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AUPreset)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            AUPreset* preset = (AUPreset*)outData;
            if (inst->currentPresetIndex >= 0 && (uint32_t)inst->currentPresetIndex < inst->presetCount && inst->presetStorage) {
                preset->presetNumber = inst->presetStorage[inst->currentPresetIndex].presetNumber;
                preset->presetName = inst->presetStorage[inst->currentPresetIndex].presetName;
            } else {
                preset->presetNumber = -1;
                preset->presetName = inst->currentPresetName ? inst->currentPresetName : CFSTR("Untitled");
            }
            *ioDataSize = sizeof(AUPreset);
            return noErr;
        }

        case kAudioUnitProperty_FactoryPresets: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            // Only supported when plugin has presets
            if (inst->presetCount == 0) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(CFArrayRef)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            if (inst->factoryPresets) {
                CFRetain(inst->factoryPresets);  // Caller owns reference
                *(CFArrayRef*)outData = inst->factoryPresets;
            } else {
                *(CFArrayRef*)outData = NULL;
            }
            *ioDataSize = sizeof(CFArrayRef);
            return noErr;
        }

        case kAudioUnitProperty_MIDIOutputCallbackInfo: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inst->midiOutputList) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(CFArrayRef)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            // One MIDI output. The caller owns the array.
            CFStringRef name = CFSTR("MIDI Out");
            *(CFArrayRef*)outData = CFArrayCreate(kCFAllocatorDefault,
                (const void**)&name, 1, &kCFTypeArrayCallBacks);
            *ioDataSize = sizeof(CFArrayRef);
            return noErr;
        }

        case kAudioUnitProperty_LastRenderError: {
            if (!outData || !ioDataSize || *ioDataSize < sizeof(OSStatus)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(OSStatus*)outData = noErr;
            *ioDataSize = sizeof(OSStatus);
            return noErr;
        }

        case kAudioUnitProperty_CocoaUI: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!beamer_au_has_gui(inst->rustInstance)) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioUnitCocoaViewInfo)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            // Get the bundle URL from the view factory class (it lives in our bundle)
            NSBundle* bundle = [NSBundle bundleForClass:[{{COCOA_VIEW_FACTORY_CLASS}} class]];
            CFURLRef bundleURL = (__bridge_retained CFURLRef)[bundle bundleURL];

            AudioUnitCocoaViewInfo* info = (AudioUnitCocoaViewInfo*)outData;
            info->mCocoaAUViewBundleLocation = bundleURL; // caller owns this reference
            info->mCocoaAUViewClass[0] = CFStringCreateWithCString(
                kCFAllocatorDefault, "{{COCOA_VIEW_FACTORY_CLASS}}", kCFStringEncodingUTF8);

            *ioDataSize = sizeof(AudioUnitCocoaViewInfo);
            return noErr;
        }

        case kBeamerAuPropertyRustInstance: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(BeamerAuInstanceHandle)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(BeamerAuInstanceHandle*)outData = inst->rustInstance;
            *ioDataSize = sizeof(BeamerAuInstanceHandle);
            return noErr;
        }

        default:
            return kAudioUnitErr_InvalidProperty;
    }
}

static OSStatus BeamerAuv2SetProperty(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, const void* inData, UInt32 inDataSize) {

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    switch (propID) {
        case kAudioUnitProperty_StreamFormat: {
            if (!inData || inDataSize < sizeof(AudioStreamBasicDescription)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            const AudioStreamBasicDescription* desc = (const AudioStreamBasicDescription*)inData;

            // Validate format (must be float, non-interleaved)
            if (desc->mFormatID != kAudioFormatLinearPCM) {
                return kAudioUnitErr_FormatNotSupported;
            }
            if (!(desc->mFormatFlags & kAudioFormatFlagIsFloat)) {
                return kAudioUnitErr_FormatNotSupported;
            }

            // Validate scope and element (bus must exist)
            if (scope == kAudioUnitScope_Input) {
                uint32_t inputBusCount = beamer_au_get_input_bus_count(inst->rustInstance);
                if (element >= inputBusCount) {
                    return kAudioUnitErr_InvalidElement;
                }
            } else if (scope == kAudioUnitScope_Output) {
                uint32_t outputBusCount = beamer_au_get_output_bus_count(inst->rustInstance);
                if (element >= outputBusCount) {
                    return kAudioUnitErr_InvalidElement;
                }
            } else {
                return kAudioUnitErr_InvalidScope;
            }

            // Validate channel count is reasonable (1-64 channels)
            UInt32 proposedChannels = desc->mChannelsPerFrame;
            if (proposedChannels == 0 || proposedChannels > 64) {
                return kAudioUnitErr_FormatNotSupported;
            }

            // Validate channel count against declared capability for MAIN bus (element 0).
            // This enforces the [N, M] capability we report in SupportedNumChannels.
            // Auxiliary buses (sidechain, etc.) can have any reasonable channel count.
            if (element == 0) {
                uint32_t declaredChannels;
                if (scope == kAudioUnitScope_Input) {
                    declaredChannels = beamer_au_get_input_bus_channel_count(inst->rustInstance, 0);
                } else {
                    declaredChannels = beamer_au_get_output_bus_channel_count(inst->rustInstance, 0);
                }
                if (declaredChannels > 0 && proposedChannels != declaredChannels) {
                    return kAudioUnitErr_FormatNotSupported;
                }
            }

            // The plugin is prepared for the current format: an initialized
            // unit only accepts it again.
            const AudioStreamBasicDescription* current = (scope == kAudioUnitScope_Input)
                ? &inst->inputFormats[element] : &inst->outputFormats[element];
            if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
                return StreamFormatsEqual(desc, current) ? noErr : kAudioUnitErr_PropertyNotWritable;
            }

            // Apply the format change
            if (scope == kAudioUnitScope_Input) {
                inst->inputFormats[element] = *desc;
            } else {
                inst->outputFormats[element] = *desc;
            }
            inst->sampleRate = desc->mSampleRate;

            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }

        case kAudioUnitProperty_SampleRate: {
            if (!inData || inDataSize < sizeof(Float64)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            Float64 sampleRate = *(Float64*)inData;
            uint32_t inBusCount = beamer_au_get_input_bus_count(inst->rustInstance);
            uint32_t outBusCount = beamer_au_get_output_bus_count(inst->rustInstance);

            // The plugin is prepared for the current rate: an initialized
            // unit only accepts it again.
            if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
                bool changes = (inst->sampleRate != sampleRate);
                for (uint32_t i = 0; i < inBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
                    changes |= (inst->inputFormats[i].mSampleRate != sampleRate);
                }
                for (uint32_t i = 0; i < outBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
                    changes |= (inst->outputFormats[i].mSampleRate != sampleRate);
                }
                return changes ? kAudioUnitErr_PropertyNotWritable : noErr;
            }

            inst->sampleRate = sampleRate;
            for (uint32_t i = 0; i < inBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
                inst->inputFormats[i].mSampleRate = inst->sampleRate;
            }
            for (uint32_t i = 0; i < outBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
                inst->outputFormats[i].mSampleRate = inst->sampleRate;
            }
            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }

        case kAudioUnitProperty_MaximumFramesPerSlice: {
            if (!inData || inDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            // The plugin and the input buffer are prepared for the current
            // limit, so it can only change while uninitialized.
            if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
                return kAudioUnitErr_Initialized;
            }
            inst->maxFramesPerSlice = *(UInt32*)inData;
            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }

        case kAudioUnitProperty_BypassEffect: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inData || inDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            atomic_store_explicit(&inst->bypassed, *(UInt32*)inData != 0, memory_order_relaxed);
            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }

        case kAudioUnitProperty_SetRenderCallback: {
            if (scope != kAudioUnitScope_Input || element != 0) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inData || inDataSize < sizeof(AURenderCallbackStruct)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            pthread_mutex_lock(&inst->callbacksMutex);
            inst->callbacks.input = *(AURenderCallbackStruct*)inData;
            // Clear connection when callback is set.
            inst->callbacks.inputConnection.sourceAU = NULL;
            pthread_mutex_unlock(&inst->callbacksMutex);
            return noErr;
        }

        case kAudioUnitProperty_MakeConnection: {
            if (scope != kAudioUnitScope_Input || element != 0) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inData || inDataSize < sizeof(AudioUnitConnection)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            const AudioUnitConnection* conn = (const AudioUnitConnection*)inData;
            pthread_mutex_lock(&inst->callbacksMutex);
            inst->callbacks.inputConnection.sourceAU = conn->sourceAudioUnit;
            inst->callbacks.inputConnection.sourceOutputNumber = conn->sourceOutputNumber;
            // Clear callback when connection is set.
            inst->callbacks.input.inputProc = NULL;
            inst->callbacks.input.inputProcRefCon = NULL;
            pthread_mutex_unlock(&inst->callbacksMutex);
            return noErr;
        }

        case kAudioUnitProperty_MIDIOutputCallback: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inst->midiOutputList) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (!inData || inDataSize < sizeof(AUMIDIOutputCallbackStruct)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            pthread_mutex_lock(&inst->callbacksMutex);
            inst->callbacks.midiOutput = *(const AUMIDIOutputCallbackStruct*)inData;
            pthread_mutex_unlock(&inst->callbacksMutex);
            return noErr;
        }

        case kAudioUnitProperty_HostCallbacks: {
            if (!inData) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            // The struct has grown over time, so a host may pass a shorter one.
            // The callbacks it does not know stay NULL.
            HostCallbackInfo callbacks;
            memset(&callbacks, 0, sizeof(callbacks));
            memcpy(&callbacks, inData,
                inDataSize < sizeof(callbacks) ? inDataSize : sizeof(callbacks));
            pthread_mutex_lock(&inst->callbacksMutex);
            inst->callbacks.host = callbacks;
            pthread_mutex_unlock(&inst->callbacksMutex);
            return noErr;
        }

        case kAudioUnitProperty_ClassInfo: {
            if (!inData || inDataSize < sizeof(CFPropertyListRef)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            CFDictionaryRef dict = *(CFDictionaryRef*)inData;
            if (!dict || CFGetTypeID(dict) != CFDictionaryGetTypeID()) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            // Restore preset name from "name" key
            CFStringRef nameStr = (CFStringRef)CFDictionaryGetValue(dict, CFSTR("name"));
            if (nameStr && CFGetTypeID(nameStr) == CFStringGetTypeID()) {
                if (inst->currentPresetName) {
                    CFRelease(inst->currentPresetName);
                }
                inst->currentPresetName = CFStringCreateCopy(NULL, nameStr);
            }

            // Try "data" key (standard AU) first, then fallback to "beamer-state"
            CFDataRef stateData = (CFDataRef)CFDictionaryGetValue(dict, CFSTR("data"));
            if (!stateData) {
                stateData = (CFDataRef)CFDictionaryGetValue(dict, CFSTR("beamer-state"));
            }
            if (stateData && CFGetTypeID(stateData) == CFDataGetTypeID()) {
                const uint8_t* bytes = CFDataGetBytePtr(stateData);
                CFIndex length = CFDataGetLength(stateData);
                beamer_au_set_state(inst->rustInstance, bytes, (uint32_t)length);
            }

            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }

        case kAudioUnitProperty_PresentPreset: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inData || inDataSize < sizeof(AUPreset)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }

            const AUPreset* newPreset = (const AUPreset*)inData;

            // Release old preset name
            if (inst->currentPresetName) {
                CFRelease(inst->currentPresetName);
                inst->currentPresetName = NULL;
            }

            if (newPreset->presetNumber >= 0 && (uint32_t)newPreset->presetNumber < inst->presetCount) {
                inst->currentPresetIndex = newPreset->presetNumber;
                beamer_au_apply_preset(inst->rustInstance, (uint32_t)newPreset->presetNumber);
                // Copy factory preset name
                if (inst->presetStorage && inst->presetStorage[inst->currentPresetIndex].presetName) {
                    inst->currentPresetName = (CFStringRef)CFRetain(
                        inst->presetStorage[inst->currentPresetIndex].presetName);
                }
            } else {
                // User preset (negative number) - track the provided name
                inst->currentPresetIndex = -1;
                if (newPreset->presetName) {
                    inst->currentPresetName = CFStringCreateCopy(NULL, newPreset->presetName);
                }
            }

            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }

        case kAudioUnitProperty_OfflineRender: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inData || inDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            bool offline = (*(const UInt32*)inData != 0);
            if (offline == inst->offlineRender) {
                return noErr;
            }
            inst->offlineRender = offline;
            // The plugin learns its process mode when it is prepared, so an
            // initialized unit is prepared again.
            if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
                OSStatus status = BeamerAuv2AllocateRenderResources(inst);
                if (status != noErr) {
                    // Go back to the state the plugin was prepared for.
                    inst->offlineRender = !offline;
                    BeamerAuv2AllocateRenderResources(inst);
                    return status;
                }
            }
            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }

        case kAudioUnitProperty_InPlaceProcessing:
        case kAudioUnitProperty_ShouldAllocateBuffer:
            // Accept but ignore these
            return noErr;

        default:
            return kAudioUnitErr_InvalidProperty;
    }
}

// =============================================================================
// MARK: - Property Listeners
// =============================================================================

static OSStatus BeamerAuv2AddPropertyListener(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc, void* userData) {

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->listenerMutex);
    if (inst->propertyListenerCount >= MAX_PROPERTY_LISTENERS) {
        pthread_mutex_unlock(&inst->listenerMutex);
        return kAudio_TooManyFilesOpenError;
    }

    PropertyListener* listener = &inst->propertyListeners[inst->propertyListenerCount++];
    listener->propID = propID;
    listener->proc = proc;
    listener->userData = userData;

    pthread_mutex_unlock(&inst->listenerMutex);
    return noErr;
}

static OSStatus BeamerAuv2RemovePropertyListener(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc) {

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->listenerMutex);
    for (UInt32 i = 0; i < inst->propertyListenerCount; i++) {
        if (inst->propertyListeners[i].propID == propID && inst->propertyListeners[i].proc == proc) {
            // Shift remaining listeners down
            for (UInt32 j = i; j < inst->propertyListenerCount - 1; j++) {
                inst->propertyListeners[j] = inst->propertyListeners[j + 1];
            }
            inst->propertyListenerCount--;
            pthread_mutex_unlock(&inst->listenerMutex);
            return noErr;
        }
    }
    pthread_mutex_unlock(&inst->listenerMutex);
    return noErr;
}

static OSStatus BeamerAuv2RemovePropertyListenerWithUserData(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc, void* userData) {

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->listenerMutex);
    for (UInt32 i = 0; i < inst->propertyListenerCount; i++) {
        if (inst->propertyListeners[i].propID == propID &&
            inst->propertyListeners[i].proc == proc &&
            inst->propertyListeners[i].userData == userData) {
            // Shift remaining listeners down
            for (UInt32 j = i; j < inst->propertyListenerCount - 1; j++) {
                inst->propertyListeners[j] = inst->propertyListeners[j + 1];
            }
            inst->propertyListenerCount--;
            pthread_mutex_unlock(&inst->listenerMutex);
            return noErr;
        }
    }
    pthread_mutex_unlock(&inst->listenerMutex);
    return noErr;
}

// =============================================================================
// MARK: - Parameters
// =============================================================================

static OSStatus BeamerAuv2GetParameter(void* self, AudioUnitParameterID paramID,
    AudioUnitScope scope, AudioUnitElement element, AudioUnitParameterValue* outValue) {

    (void)element;

    if (scope != kAudioUnitScope_Global) {
        return kAudioUnitErr_InvalidScope;
    }

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    // Use AU-format getter which handles indexed parameter conversion internally
    *outValue = beamer_au_get_parameter_value_au(inst->rustInstance, paramID);
    return noErr;
}

// Queue a parameter change for the next render call, at a sample offset into
// its buffer. Returns false when the unit does not render or the queue is
// full: the caller then sets the value right away.
static bool BeamerAuv2QueueParameter(BeamerAuv2Instance* inst, AudioUnitParameterID paramID,
    AudioUnitParameterValue value, UInt32 bufferOffset, UInt32 rampFrames) {

    if (!atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
        return false;
    }

    AURenderEvent event;
    memset(&event, 0, sizeof(event));
    event.parameter.eventType = (rampFrames > 0) ? AURenderEventParameterRamp : AURenderEventParameter;
    // Offset into the next buffer. Render turns it into a sample time.
    event.parameter.eventSampleTime = (AUEventSampleTime)bufferOffset;
    event.parameter.rampDurationSampleFrames = rampFrames;
    event.parameter.parameterAddress = paramID;
    event.parameter.value = value;
    if (!EventQueuePush(&inst->parameterQueue, &event)) {
        return false;
    }

    // Uninitialize may have emptied the queue between the check above and
    // the push, and the change would wait for the next Initialize: apply it
    // now. Without render calls, nothing holds the mutex for long. The
    // fence pairs with the one in Uninitialize: either this load sees the
    // unit uninitialized, or Uninitialize sees the event.
    atomic_thread_fence(memory_order_seq_cst);
    if (!atomic_load_explicit(&inst->initialized, memory_order_relaxed)) {
        pthread_mutex_lock(&inst->queueReaderMutex);
        BeamerAuv2ApplyPendingParameters(inst);
        pthread_mutex_unlock(&inst->queueReaderMutex);
    }
    return true;
}

// Apply the queued parameter changes now and empty the queue. For a render
// call that does not process (bypass), for a reset and when the unit
// stops rendering. The caller holds queueReaderMutex.
static void BeamerAuv2ApplyPendingParameters(BeamerAuv2Instance* inst) {
    UInt32 count = EventQueueReadyCount(&inst->parameterQueue);
    for (UInt32 i = 0; i < count; i++) {
        const AURenderEvent* event = EventQueueAt(&inst->parameterQueue, i);
        beamer_au_set_parameter_value_au(inst->rustInstance,
            (uint32_t)event->parameter.parameterAddress, event->parameter.value);
    }
    EventQueueRelease(&inst->parameterQueue, count);
}

// Drop the queued MIDI input. The caller holds queueReaderMutex.
static void BeamerAuv2DiscardMidi(BeamerAuv2Instance* inst) {
    EventQueueRelease(&inst->midiQueue, EventQueueReadyCount(&inst->midiQueue));
}

static OSStatus BeamerAuv2SetParameter(void* self, AudioUnitParameterID paramID,
    AudioUnitScope scope, AudioUnitElement element, AudioUnitParameterValue value, UInt32 bufferOffset) {

    (void)element;

    if (scope != kAudioUnitScope_Global) {
        return kAudioUnitErr_InvalidScope;
    }

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    // A host passes an offset only right before a render call, for a change
    // within its buffer. Without one, the value applies now.
    if (bufferOffset > 0 && BeamerAuv2QueueParameter(inst, paramID, value, bufferOffset, 0)) {
        return noErr;
    }

    // Use AU-format setter which handles indexed parameter conversion internally
    beamer_au_set_parameter_value_au(inst->rustInstance, paramID, value);
    return noErr;
}

static OSStatus BeamerAuv2ScheduleParameters(void* self, const AudioUnitParameterEvent* events, UInt32 numEvents) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    for (UInt32 i = 0; i < numEvents; i++) {
        const AudioUnitParameterEvent* event = &events[i];
        if (event->scope != kAudioUnitScope_Global) {
            continue;
        }

        AudioUnitParameterValue value;
        UInt32 offset;
        UInt32 rampFrames;
        if (event->eventType == kParameterEvent_Ramped) {
            // The parameter's smoother interpolates: the ramp becomes a change
            // to its end value at its start, as in the AUv3 render path.
            value = event->eventValues.ramp.endValue;
            offset = (UInt32)(event->eventValues.ramp.startBufferOffset > 0
                ? event->eventValues.ramp.startBufferOffset : 0);
            rampFrames = event->eventValues.ramp.durationInFrames;
            if (rampFrames == 0) {
                rampFrames = 1;
            }
        } else {
            value = event->eventValues.immediate.value;
            offset = event->eventValues.immediate.bufferOffset;
            rampFrames = 0;
        }

        if (!BeamerAuv2QueueParameter(inst, event->parameter, value, offset, rampFrames)) {
            // Use AU-format setter which handles indexed parameter conversion internally
            beamer_au_set_parameter_value_au(inst->rustInstance, event->parameter, value);
        }
    }

    return noErr;
}

// =============================================================================
// MARK: - MIDI Output
// =============================================================================

// Start collecting the MIDI output of a render call. Returns false if the
// plugin produces no MIDI or the host does not listen.
static bool BeamerAuv2BeginMidiOutput(BeamerAuv2Instance* inst, const AudioTimeStamp* timeStamp,
    BeamerAuv2MidiOutput* output) {

    output->callback = inst->renderCallbacks.midiOutput;
    if (!inst->midiOutputList || !output->callback.midiOutputCallback) {
        return false;
    }
    output->timeStamp = timeStamp;
    output->list = inst->midiOutputList;
    output->packet = MIDIPacketListInit(output->list);
    return true;
}

// Hand the collected packets to the host and start a new list.
static void BeamerAuv2FlushMidiOutput(BeamerAuv2MidiOutput* output) {
    if (output->list->numPackets > 0) {
        output->callback.midiOutputCallback(output->callback.userData,
            output->timeStamp, 0, output->list);
    }
    output->packet = MIDIPacketListInit(output->list);
}

// Add one MIDI message from the plugin to the list (BeamerAuMidiOutputFn).
// The time stamp of a packet is its sample offset in the buffer. A full list
// goes to the host at once, during the render call and with the event
// queues taken.
static void BeamerAuv2SendMidi(void* context, uint32_t sampleOffset, const uint8_t* bytes, uint32_t length) {
    BeamerAuv2MidiOutput* output = (BeamerAuv2MidiOutput*)context;

    MIDIPacket* next = MIDIPacketListAdd(output->list, MIDI_OUTPUT_LIST_BYTES,
        output->packet, sampleOffset, length, bytes);
    if (next == NULL) {
        // The list is full: pass it on and retry in an empty one. A message
        // that does not fit there either is dropped.
        BeamerAuv2FlushMidiOutput(output);
        next = MIDIPacketListAdd(output->list, MIDI_OUTPUT_LIST_BYTES,
            output->packet, sampleOffset, length, bytes);
        if (next == NULL) {
            return;
        }
    }
    output->packet = next;
}

// Number of bytes of a channel message, from its status byte.
static uint32_t BeamerAuv2MidiMessageLength(uint8_t status) {
    uint8_t type = status & 0xF0;
    return (type == 0xC0 || type == 0xD0) ? 2 : 3;
}

// =============================================================================
// MARK: - Host Transport
// =============================================================================

// Ask the host for its musical context and transport state through the
// callbacks it registered. Called on the render thread.
static void BeamerAuv2ReadTransport(const HostCallbackInfo* host, BeamerAuTransport* transport) {
    memset(transport, 0, sizeof(*transport));

    if (host->beatAndTempoProc) {
        Float64 beatPosition = 0.0;
        Float64 tempo = 0.0;
        if (host->beatAndTempoProc(host->hostUserData, &beatPosition, &tempo) == noErr) {
            transport->flags |= BeamerAuTransportHasBeatAndTempo;
            transport->tempo = tempo;
            transport->beat_position = beatPosition;
        }
    }

    if (host->musicalTimeLocationProc) {
        UInt32 samplesToNextBeat = 0;
        Float32 numerator = 0.0f;
        UInt32 denominator = 0;
        Float64 downbeatPosition = 0.0;
        if (host->musicalTimeLocationProc(host->hostUserData, &samplesToNextBeat,
                &numerator, &denominator, &downbeatPosition) == noErr) {
            transport->flags |= BeamerAuTransportHasTimeSignature;
            transport->time_sig_numerator = numerator;
            transport->time_sig_denominator = (int32_t)denominator;
            transport->measure_downbeat_position = downbeatPosition;
        }
    }

    Boolean isPlaying = false;
    Boolean isRecording = false;
    Boolean stateChanged = false;
    Boolean isCycling = false;
    Float64 samplePosition = 0.0;
    Float64 cycleStart = 0.0;
    Float64 cycleEnd = 0.0;
    OSStatus stateStatus = kAudioUnitErr_CannotDoInCurrentContext;
    if (host->transportStateProc2) {
        stateStatus = host->transportStateProc2(host->hostUserData, &isPlaying, &isRecording,
            &stateChanged, &samplePosition, &isCycling, &cycleStart, &cycleEnd);
    } else if (host->transportStateProc) {
        // The older callback does not report recording.
        stateStatus = host->transportStateProc(host->hostUserData, &isPlaying,
            &stateChanged, &samplePosition, &isCycling, &cycleStart, &cycleEnd);
    }
    if (stateStatus == noErr) {
        transport->flags |= BeamerAuTransportHasState;
        if (isPlaying) transport->flags |= BeamerAuTransportPlaying;
        if (isRecording) transport->flags |= BeamerAuTransportRecording;
        if (isCycling) transport->flags |= BeamerAuTransportCycling;
        transport->sample_position = samplePosition;
        transport->cycle_start_beat = cycleStart;
        transport->cycle_end_beat = cycleEnd;
    }
}

// =============================================================================
// MARK: - Render
// =============================================================================

// Pull the main input through the callback or the connection the host set.
// Returns NULL when there is no input source or the pull failed.
static AudioBufferList* BeamerAuv2PullInput(BeamerAuv2Instance* inst, const AudioTimeStamp* timeStamp,
    UInt32 frames) {

    const CallbackSet* callbacks = &inst->renderCallbacks;
    if (!callbacks->input.inputProc && !callbacks->inputConnection.sourceAU) {
        return NULL;
    }

    EnsureInputBufferList(inst, inst->inputFormats[0].mChannelsPerFrame, frames);
    AudioUnitRenderActionFlags pullFlags = 0;
    OSStatus pullStatus;
    if (callbacks->input.inputProc) {
        pullStatus = callbacks->input.inputProc(callbacks->input.inputProcRefCon,
            &pullFlags, timeStamp, 0, frames, inst->inputBufferList);
    } else {
        pullStatus = AudioUnitRender(callbacks->inputConnection.sourceAU,
            &pullFlags, timeStamp, callbacks->inputConnection.sourceOutputNumber,
            frames, inst->inputBufferList);
    }
    return (pullStatus == noErr) ? inst->inputBufferList : NULL;
}

static OSStatus BeamerAuv2Render(void* self, AudioUnitRenderActionFlags* ioActionFlags,
    const AudioTimeStamp* inTimeStamp, UInt32 inOutputBusNumber, UInt32 inNumberFrames, AudioBufferList* ioData) {

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    if (!atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
        return kAudioUnitErr_Uninitialized;
    }

    if (inNumberFrames > inst->maxFramesPerSlice) {
        return kAudioUnitErr_TooManyFramesToProcess;
    }

    // The render notifications of this call. The render thread does not
    // wait for the lock: when a host thread is changing the list right now,
    // this call goes without notifications. Holding the lock until the
    // post-render notifications keeps each pre-render one paired with its
    // post-render one, and a removed notification is not called after
    // AudioUnitRemoveRenderNotify returned. The lock is held across the
    // input pull and the render, which take no lock of their own.
    bool notifies = (pthread_mutex_trylock(&inst->renderNotifyMutex) == 0);
    if (notifies) {
        for (UInt32 i = 0; i < inst->renderNotifyCount; i++) {
            AudioUnitRenderActionFlags preFlags = kAudioUnitRenderAction_PreRender;
            inst->renderNotify[i].proc(inst->renderNotify[i].userData,
                &preFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData);
        }
    }

    // Copy the callbacks after the pre-render notifications, in which a
    // host may still change them. When a host thread is changing them
    // right now, this call keeps the copy of the previous call.
    if (pthread_mutex_trylock(&inst->callbacksMutex) == 0) {
        inst->renderCallbacks = inst->callbacks;
        pthread_mutex_unlock(&inst->callbacksMutex);
    }

    // Bypass: the input goes to the output unchanged.
    if (atomic_load_explicit(&inst->bypassed, memory_order_relaxed)) {
        AudioBufferList* inputData = BeamerAuv2PullInput(inst, inTimeStamp, inNumberFrames);

        // Copy input to output for bypass
        if (inputData) {
            UInt32 buffersToCopy = (inputData->mNumberBuffers < ioData->mNumberBuffers) ?
                inputData->mNumberBuffers : ioData->mNumberBuffers;
            for (UInt32 i = 0; i < buffersToCopy; i++) {
                UInt32 bytesToCopy = (inputData->mBuffers[i].mDataByteSize < ioData->mBuffers[i].mDataByteSize) ?
                    inputData->mBuffers[i].mDataByteSize : ioData->mBuffers[i].mDataByteSize;
                memcpy(ioData->mBuffers[i].mData, inputData->mBuffers[i].mData, bytesToCopy);
            }
        } else {
            // No input, silence output
            for (UInt32 i = 0; i < ioData->mNumberBuffers; i++) {
                memset(ioData->mBuffers[i].mData, 0, ioData->mBuffers[i].mDataByteSize);
            }
        }

        // Consume the pending MIDI input, so that it does not play when bypass
        // ends. A bypassed MIDI effect passes it on unchanged. When a reset
        // holds the queues right now, the events wait for the next call.
        if (inOutputBusNumber == 0 && pthread_mutex_trylock(&inst->queueReaderMutex) == 0) {
            UInt32 midiCount = EventQueueReadyCount(&inst->midiQueue);
            BeamerAuv2MidiOutput thru;
            bool passesMidi = midiCount > 0 && inst->isMidiEffect
                && BeamerAuv2BeginMidiOutput(inst, inTimeStamp, &thru);
            if (passesMidi) {
                for (UInt32 i = 0; i < midiCount; i++) {
                    const AURenderEvent* ev = EventQueueAt(&inst->midiQueue, i);
                    // Only channel messages, as in normal processing.
                    uint8_t status = ev->MIDI.data[0];
                    if (status < 0x80 || status >= 0xF0) {
                        continue;
                    }
                    BeamerAuv2SendMidi(&thru, (uint32_t)ev->MIDI.eventSampleTime, ev->MIDI.data,
                        BeamerAuv2MidiMessageLength(status));
                }
            }
            EventQueueRelease(&inst->midiQueue, midiCount);

            // The plugin does not process now, so the queued parameter
            // changes apply at once.
            BeamerAuv2ApplyPendingParameters(inst);
            pthread_mutex_unlock(&inst->queueReaderMutex);

            if (passesMidi) {
                BeamerAuv2FlushMidiOutput(&thru);
            }
        }

        // Call post-render notifications
        if (notifies) {
            for (UInt32 i = 0; i < inst->renderNotifyCount; i++) {
                AudioUnitRenderActionFlags postFlags = kAudioUnitRenderAction_PostRender;
                inst->renderNotify[i].proc(inst->renderNotify[i].userData,
                    &postFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData);
            }
            pthread_mutex_unlock(&inst->renderNotifyMutex);
        }

        return noErr;
    }

    // Pull input audio
    AudioBufferList* inputData = NULL;
    if (beamer_au_get_input_bus_count(inst->rustInstance) > 0) {
        inputData = BeamerAuv2PullInput(inst, inTimeStamp, inNumberFrames);
    }

    // Only the render of the main bus processes audio and needs the
    // transport. It is read before the queues are taken, so that the host
    // callbacks do not run while a reset waits for them.
    BeamerAuTransport transport;
    bool hasTransport = (inOutputBusNumber == 0);
    if (hasTransport) {
        BeamerAuv2ReadTransport(&inst->renderCallbacks.host, &transport);
    }

    // Take the queued events and link them into one list: the MIDI events,
    // then the parameter changes the host scheduled for this buffer. The
    // queues hold offsets into this buffer. Render events carry a sample time
    // on the timeline of the render timestamp, so add the buffer start. Only
    // the render of the main bus takes events: a call for another output bus
    // leaves them for it, so that none are lost. When a reset holds the
    // queues right now, the events wait for the next call.
    const AURenderEvent* eventList = NULL;
    AUEventSampleTime bufferStart = (AUEventSampleTime)inTimeStamp->mSampleTime;
    UInt32 midiCount = 0;
    UInt32 parameterCount = 0;
    bool readsQueues = (inOutputBusNumber == 0)
        && pthread_mutex_trylock(&inst->queueReaderMutex) == 0;
    if (readsQueues) {
        midiCount = EventQueueReadyCount(&inst->midiQueue);
        parameterCount = EventQueueReadyCount(&inst->parameterQueue);
        AURenderEvent* prev = NULL;
        for (UInt32 i = 0; i < midiCount + parameterCount; i++) {
            AURenderEvent* ev = (i < midiCount)
                ? EventQueueAt(&inst->midiQueue, i)
                : EventQueueAt(&inst->parameterQueue, i - midiCount);
            ev->head.next = NULL;
            ev->head.eventSampleTime += bufferStart;
            if (prev) prev->head.next = ev;
            else eventList = ev;
            prev = ev;
        }
    }

    // The plugin's MIDI output is collected in a packet list during the call.
    BeamerAuv2MidiOutput midiOutput;
    bool sendsMidi = (inOutputBusNumber == 0)
        && BeamerAuv2BeginMidiOutput(inst, inTimeStamp, &midiOutput);

    // Call Rust render function
    OSStatus status = beamer_au_render(
        inst->rustInstance,
        ioActionFlags,
        inTimeStamp,
        inNumberFrames,
        inOutputBusNumber,
        ioData,
        eventList,
        NULL,  // pull_input_block (we pre-pulled via callback/connection)
        inputData,
        hasTransport ? &transport : NULL,
        sendsMidi ? BeamerAuv2SendMidi : NULL,
        sendsMidi ? &midiOutput : NULL
    );

    // Give the events back to the queues. A render call that was refused
    // before the plugin processed (resources being prepared again, not
    // prepared, too many frames) leaves them for the next call, at their
    // offsets into its buffer, so that no note-off or parameter change is
    // lost.
    if (readsQueues) {
        bool refused = (status == kAudioUnitErr_CannotDoInCurrentContext
            || status == kAudioUnitErr_Uninitialized
            || status == kAudioUnitErr_TooManyFramesToProcess);
        if (refused) {
            for (UInt32 i = 0; i < midiCount; i++) {
                EventQueueAt(&inst->midiQueue, i)->head.eventSampleTime -= bufferStart;
            }
            for (UInt32 i = 0; i < parameterCount; i++) {
                EventQueueAt(&inst->parameterQueue, i)->head.eventSampleTime -= bufferStart;
            }
        } else {
            EventQueueRelease(&inst->midiQueue, midiCount);
            EventQueueRelease(&inst->parameterQueue, parameterCount);
        }
        pthread_mutex_unlock(&inst->queueReaderMutex);
    }

    // Pass the collected MIDI output to the host.
    if (sendsMidi) {
        BeamerAuv2FlushMidiOutput(&midiOutput);
    }

    // Call post-render notifications. A failed render call says so in the
    // flags.
    if (notifies) {
        for (UInt32 i = 0; i < inst->renderNotifyCount; i++) {
            AudioUnitRenderActionFlags postFlags = kAudioUnitRenderAction_PostRender;
            if (status != noErr) {
                postFlags |= kAudioUnitRenderAction_PostRenderError;
            }
            inst->renderNotify[i].proc(inst->renderNotify[i].userData,
                &postFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData);
        }
        pthread_mutex_unlock(&inst->renderNotifyMutex);
    }

    return status;
}

// =============================================================================
// MARK: - MIDI (MusicDevice)
// =============================================================================

static OSStatus BeamerAuv2MIDIEvent(void* self, UInt32 inStatus, UInt32 inData1, UInt32 inData2, UInt32 inOffsetSampleFrame) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    AURenderEvent event;
    memset(&event, 0, sizeof(event));
    event.MIDI.eventType = AURenderEventMIDI;
    // Offset into the next buffer. Render turns it into a sample time.
    event.MIDI.eventSampleTime = (AUEventSampleTime)inOffsetSampleFrame;
    event.MIDI.cable = 0;
    event.MIDI.length = 3;
    event.MIDI.data[0] = (uint8_t)(inStatus & 0xFF);
    event.MIDI.data[1] = (uint8_t)(inData1 & 0xFF);
    event.MIDI.data[2] = (uint8_t)(inData2 & 0xFF);

    // A full queue drops the event.
    EventQueuePush(&inst->midiQueue, &event);
    return noErr;
}

// =============================================================================
// MARK: - Reset
// =============================================================================

static OSStatus BeamerAuv2Reset(void* self, AudioUnitScope scope, AudioUnitElement element) {
    (void)scope;
    (void)element;

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    beamer_au_reset(inst->rustInstance);

    // Flush any pending MIDI events. Pending parameter changes are applied,
    // so that the values the host set stay set. A render call that runs
    // right now holds the mutex until it is done with its events.
    pthread_mutex_lock(&inst->queueReaderMutex);
    BeamerAuv2DiscardMidi(inst);
    BeamerAuv2ApplyPendingParameters(inst);
    pthread_mutex_unlock(&inst->queueReaderMutex);

    return noErr;
}

// =============================================================================
// MARK: - Render Notifications
// =============================================================================

static OSStatus BeamerAuv2AddRenderNotify(void* self, AURenderCallback proc, void* userData) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->renderNotifyMutex);
    if (inst->renderNotifyCount >= MAX_RENDER_NOTIFY) {
        pthread_mutex_unlock(&inst->renderNotifyMutex);
        return kAudio_TooManyFilesOpenError;
    }

    inst->renderNotify[inst->renderNotifyCount].proc = proc;
    inst->renderNotify[inst->renderNotifyCount].userData = userData;
    inst->renderNotifyCount++;

    pthread_mutex_unlock(&inst->renderNotifyMutex);
    return noErr;
}

static OSStatus BeamerAuv2RemoveRenderNotify(void* self, AURenderCallback proc, void* userData) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->renderNotifyMutex);
    for (UInt32 i = 0; i < inst->renderNotifyCount; i++) {
        if (inst->renderNotify[i].proc == proc && inst->renderNotify[i].userData == userData) {
            // Shift remaining entries down
            for (UInt32 j = i; j < inst->renderNotifyCount - 1; j++) {
                inst->renderNotify[j] = inst->renderNotify[j + 1];
            }
            inst->renderNotifyCount--;
            pthread_mutex_unlock(&inst->renderNotifyMutex);
            return noErr;
        }
    }
    pthread_mutex_unlock(&inst->renderNotifyMutex);
    return noErr;
}

// =============================================================================
// MARK: - CocoaUI GUI View
// =============================================================================

// =============================================================================
// MARK: - AUv2 WebView IPC Callbacks
// =============================================================================

static void beamer_auv2_on_message(void* context, const uint8_t* json, size_t len) {
    {{COCOA_GUI_VIEW_CLASS}}* self = (__bridge {{COCOA_GUI_VIEW_CLASS}}*)context;
    NSString* jsonStr = [[NSString alloc] initWithBytes:json length:len encoding:NSUTF8StringEncoding];
    if (!jsonStr) return;

    NSData* data = [jsonStr dataUsingEncoding:NSUTF8StringEncoding];
    NSDictionary* msg = [NSJSONSerialization JSONObjectWithData:data options:0 error:nil];
    if (!msg) return;

    NSString* type = msg[@"type"];
    if ([type isEqualToString:@"param:set"]) {
        uint32_t paramId = [msg[@"id"] unsignedIntValue];
        double value = [msg[@"value"] doubleValue];
        beamer_au_param_set_from_ui(self->_rustInstance, paramId, value);
        // Notify host via AudioUnitSetParameter (convert normalized to AU value)
        float auValue = beamer_au_get_parameter_value_au(self->_rustInstance, paramId);
        AudioUnitSetParameter(self->_audioUnit, paramId,
            kAudioUnitScope_Global, 0, auValue, 0);
        // Answer with the value in effect, which the host rounded.
        beamer_au_ipc_answer_param(self->_sync, self->_webviewHandle, paramId);
    } else if ([type isEqualToString:@"state:set"]) {
        beamer_au_ipc_handle_state_set(self->_sync, self->_webviewHandle, json, len);
    } else if ([type isEqualToString:@"param:begin"]) {
        uint32_t paramId = [msg[@"id"] unsignedIntValue];
        AudioUnitEvent event;
        memset(&event, 0, sizeof(event));
        event.mEventType = kAudioUnitEvent_BeginParameterChangeGesture;
        event.mArgument.mParameter.mAudioUnit = self->_audioUnit;
        event.mArgument.mParameter.mParameterID = paramId;
        event.mArgument.mParameter.mScope = kAudioUnitScope_Global;
        AUEventListenerNotify(NULL, NULL, &event);
    } else if ([type isEqualToString:@"param:end"]) {
        uint32_t paramId = [msg[@"id"] unsignedIntValue];
        AudioUnitEvent event;
        memset(&event, 0, sizeof(event));
        event.mEventType = kAudioUnitEvent_EndParameterChangeGesture;
        event.mArgument.mParameter.mAudioUnit = self->_audioUnit;
        event.mArgument.mParameter.mParameterID = paramId;
        event.mArgument.mParameter.mScope = kAudioUnitScope_Global;
        AUEventListenerNotify(NULL, NULL, &event);
    } else if ([type isEqualToString:@"invoke"]) {
        if (!beamer_au_ipc_handle_builtin_invoke(self->_rustInstance, self->_webviewHandle, msg)) {
            beamer_au_ipc_handle_invoke(self->_rustInstance, self->_webviewHandle, msg);
        }
    } else if ([type isEqualToString:@"event"]) {
        beamer_au_ipc_handle_event(self->_rustInstance, msg);
    }
}

static void beamer_auv2_on_loaded(void* context) {
    {{COCOA_GUI_VIEW_CLASS}}* self = (__bridge {{COCOA_GUI_VIEW_CLASS}}*)context;
    beamer_au_ipc_send_init_dump(self->_sync, self->_webviewHandle, "auv2");
}

@implementation {{COCOA_GUI_VIEW_CLASS}}
- (instancetype)initWithFrame:(NSRect)frame
                 webviewHandle:(void*)handle
                  rustInstance:(BeamerAuInstanceHandle)rustInstance
                     audioUnit:(AudioUnit)audioUnit {
    self = [super initWithFrame:frame];
    if (self) {
        _webviewHandle = handle;
        _rustInstance = rustInstance;
        _audioUnit = audioUnit;
        [[NSNotificationCenter defaultCenter] addObserver:self
                                                 selector:@selector(applicationWillTerminate:)
                                                     name:NSApplicationWillTerminateNotification
                                                   object:nil];
        // The first frame sends every parameter, and events the plugin
        // emits from now on are kept for this editor.
        _sync = beamer_au_editor_sync_open(_rustInstance);

        // Start 60 Hz parameter sync timer.
        // Use __weak to avoid a retain cycle (self -> _syncTimer -> block -> self).
        __weak typeof(self) weakSelf = self;
        _syncTimer = [NSTimer timerWithTimeInterval:1.0/60.0 repeats:YES block:^(NSTimer* t) {
            (void)t;
            [weakSelf _sendFrame];
        }];
        // Schedule in the common modes, so that the UI keeps following the host
        // while it runs a tracking loop (a drag, a resize or an open menu).
        [[NSRunLoop currentRunLoop] addTimer:_syncTimer forMode:NSRunLoopCommonModes];
    }
    return self;
}
- (void)dealloc {
    [_syncTimer invalidate];
    _syncTimer = nil;
    beamer_au_ipc_close_sync(&_sync);
    [[NSNotificationCenter defaultCenter] removeObserver:self];
    if (_webviewHandle != NULL) {
        beamer_webview_destroy(_webviewHandle);
    }
}
- (void)_sendFrame {
    beamer_au_ipc_send_frame(_sync, _webviewHandle);
}
- (BOOL)mouseDownCanMoveWindow {
    return NO;
}
- (void)viewDidMoveToWindow {
    NSWindow* window = [self window];
    if (window != nil) {
        [window setAcceptsMouseMovedEvents:YES];
    }
}
- (void)applicationWillTerminate:(NSNotification*)notification {
    [_syncTimer invalidate];
    _syncTimer = nil;
    beamer_au_ipc_close_sync(&_sync);
    if (_webviewHandle != NULL) {
        beamer_webview_destroy(_webviewHandle);
        _webviewHandle = NULL;
    }
}
@end

// =============================================================================
// MARK: - CocoaUI View Factory
// =============================================================================

@implementation {{COCOA_VIEW_FACTORY_CLASS}}
- (unsigned)interfaceVersion {
    return 0;
}

- (NSView*)uiViewForAudioUnit:(AudioUnit)audioUnit withSize:(NSSize)preferredSize {
    // Retrieve the Rust instance handle via custom property
    BeamerAuInstanceHandle rustInstance = NULL;
    UInt32 dataSize = sizeof(BeamerAuInstanceHandle);
    OSStatus status = AudioUnitGetProperty(audioUnit, kBeamerAuPropertyRustInstance,
        kAudioUnitScope_Global, 0, &rustInstance, &dataSize);
    if (status != noErr || rustInstance == NULL) {
        return nil;
    }

    if (!beamer_au_has_gui(rustInstance)) {
        return nil;
    }

    uint32_t width = 0, height = 0;
    beamer_au_get_gui_size(rustInstance, &width, &height);
    NSSize viewSize = NSMakeSize(width, height);

    // Create a temporary container to attach the WebView to
    NSView* container = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, viewSize.width, viewSize.height)];
    if (container == nil) {
        return nil;
    }

    uint8_t bgColor[4];
    beamer_au_get_gui_background_color(bgColor);

    const char* devUrl = beamer_au_get_gui_url(rustInstance);
    void* webviewHandle;
    // Create the GUI view that will be returned to the host.
    // __bridge (no retain) is safe for the callback context because the host
    // retains this view for the editor lifetime, and -dealloc destroys the
    // WebView before the view is freed - so the context cannot dangle.
    {{COCOA_GUI_VIEW_CLASS}}* guiView = [[{{COCOA_GUI_VIEW_CLASS}} alloc]
        initWithFrame:NSMakeRect(0, 0, viewSize.width, viewSize.height)
        webviewHandle:NULL
         rustInstance:rustInstance
            audioUnit:audioUnit];

    // Create WebView via beamer-webview C-ABI (shared platform layer).
    if (devUrl != NULL) {
        webviewHandle = beamer_webview_create_url_with_ipc(
            (__bridge void*)container, devUrl, bgColor,
            beamer_auv2_on_message, beamer_auv2_on_loaded,
            (__bridge void*)guiView);
    } else {
        const void* assets = beamer_au_get_gui_assets();
        webviewHandle = beamer_webview_create_with_ipc(
            (__bridge void*)container, assets, bgColor,
            beamer_auv2_on_message, beamer_auv2_on_loaded,
            (__bridge void*)guiView);
    }
    if (webviewHandle == NULL) {
        return nil;
    }

    // Store the webview handle in the GUI view
    guiView->_webviewHandle = webviewHandle;

    // Re-parent the WebView's container into the GUI view
    [container setFrame:guiView.bounds];
    [container setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
    [guiView addSubview:container];

    return guiView;
}
@end
