// =============================================================================
// AUv2 Plugin Implementation for {{PLUGIN_NAME}}
// =============================================================================
// Auto-generated proper AUv2 implementation that returns AudioComponentPlugInInterface*.
// All plugin logic is handled by the beamer_au_* bridge functions.

#include <AudioToolbox/AudioToolbox.h>
#include <AudioUnit/AudioUnit.h>
#include <CoreFoundation/CoreFoundation.h>
#include <CoreMIDI/CoreMIDI.h>
#include <stddef.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>
#include <stdatomic.h>

#import <Cocoa/Cocoa.h>
#import <AudioUnit/AUCocoaUIView.h>

// Include the bridge header for Rust plugin access
#include "BeamerAuBridge.h"
#include "au_ipc_helpers.h"
#include "au_layout_helpers.h"

// CocoaUI class interfaces (implementations at end of file)
@interface {{COCOA_EDITOR_VIEW_CLASS}} : NSView {
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

// Size of the packet list that collects the channel messages of one render
// call. Enough for a full buffer of them. The list also gets room for the
// SysEx output of a render call (sysex_bytes_per_block). When more is
// produced, the list goes to the host in parts.
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

// What the host sets for the render thread to call: the input source of
// each input element (a callback or a connection), the MIDI output
// callback and the host callbacks.
typedef struct {
    AURenderCallbackStruct input[BEAMER_AU_MAX_BUSES];
    InputConnection inputConnection[BEAMER_AU_MAX_BUSES];
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

// The header of a record in a SysEx queue. A message record continues with
// a SysEx render event: an AUMIDIEvent whose message runs from its data
// field on, past the end of the struct.
typedef struct {
    // The low 32 bits of the record's position plus one once its producer
    // is done with it. Released records are zeroed, so a record still being
    // written reads 0 (a position is a multiple of 16, so the value is
    // never 0).
    _Atomic UInt32 sequence;
    // Bytes of the record, header included, a multiple of 16.
    UInt32 size;
    // Padding up to the end of the ring instead of a message.
    UInt32 isPadding;
    UInt32 unused;
} SysExRecordHeader;

// SysEx messages for the next render call, in a ring of bytes that several
// threads may add to at once and one thread at a time takes from, without
// locks. Producers claim a range with compare-and-swap; the consumer is
// whoever holds queueReaderMutex. Render links the message records into the
// event list as they are, so a record never wraps around the end of the
// ring: one that would is preceded by a padding record up to the end.
// Positions only grow (64 bits, they never wrap); their remainder by the
// capacity is the offset.
typedef struct {
    // NULL for a plugin without MIDI input.
    uint8_t* bytes;
    // A multiple of 16, room for two of the largest records.
    UInt32 capacity;
    // The largest message accepted, in bytes.
    UInt32 largestMessage;
    // Next position to fill, shared by the producers.
    _Atomic uint64_t writePosition;
    // Next position to take. The consumer moves it after zeroing what it
    // took; producers read it to see the free room.
    _Atomic uint64_t readPosition;
    // Messages dropped since the last render call: the ring was full or
    // the message too large.
    _Atomic UInt32 dropped;
} SysExQueue;

typedef struct BeamerAuv2Instance {
    // AudioComponentPlugInInterface MUST be first (ABI requirement)
    AudioComponentPlugInInterface interface;
    AudioComponentInstance componentInstance;
    BeamerAuInstanceHandle rustInstance;

    // Audio configuration
    Float64 sampleRate;
    UInt32 maxFramesPerSlice;
    _Atomic bool initialized;
    // The host's bypass for a plugin without a bypass parameter: the
    // render call passes the input on.
    _Atomic bool bypassed;
    // The plugin's bypass parameter, which kAudioUnitProperty_BypassEffect
    // sets and reads instead of `bypassed`.
    bool hasBypassParameter;
    uint32_t bypassParameterId;
    // kAudioUnitProperty_InPlaceProcessing: on for a plugin with a main
    // input unless the host turns it off. Without an input (an instrument)
    // there is nothing to process in place, so it stays off.
    _Atomic bool processesInPlace;
    bool offlineRender;

    // Stream formats per bus (indexed by element number). A format with 0
    // channels switches an aux bus off.
    AudioStreamBasicDescription inputFormats[BEAMER_AU_MAX_BUSES];
    AudioStreamBasicDescription outputFormats[BEAMER_AU_MAX_BUSES];

    // Channel layouts per bus as the host set them
    // (kAudioUnitProperty_AudioChannelLayout), none when it set none. A new
    // channel count in the stream format drops the layout.
    BeamerAuLayout inputLayouts[BEAMER_AU_MAX_BUSES];
    BeamerAuLayout outputLayouts[BEAMER_AU_MAX_BUSES];

    // Elements per scope (kAudioUnitProperty_ElementCount): the declared
    // buses. The count is not writable; a host switches an aux bus off with
    // a stream format without channels.
    UInt32 inputElementCount;
    UInt32 outputElementCount;

    // Element names (kAudioUnitProperty_ElementName): the name the plugin
    // gives each bus and the current one, which a host may change. The
    // state saves the names that differ. Host threads read and set the
    // current names under namesMutex.
    CFStringRef declaredInputNames[BEAMER_AU_MAX_BUSES];
    CFStringRef declaredOutputNames[BEAMER_AU_MAX_BUSES];
    CFStringRef inputNames[BEAMER_AU_MAX_BUSES];
    CFStringRef outputNames[BEAMER_AU_MAX_BUSES];
    pthread_mutex_t namesMutex;

    // Memory for the output buffers without memory of a bypassed render
    // call, bypassSilenceFrames samples of 64 bits. Zeroed before each use,
    // since the host may write into a buffer it got.
    void* bypassSilence;
    UInt32 bypassSilenceFrames;

    // Allocated input buffer for pulling
    AudioBufferList* inputBufferList;
    // Samples each owned input buffer holds.
    UInt32 inputBufferFrames;
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
    // renderCallbacks, which only it uses: it never waits and a copy is
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

    // SysEx input for the next render call. MusicDeviceSysEx adds to it,
    // from the render thread or from another thread.
    SysExQueue sysexQueue;

    // The MIDI protocol the plugin asks for (a MIDIProtocolID), 0 for a
    // plugin without MIDI input, which reports none.
    SInt32 midiProtocol;

    // Parameter changes with a sample offset, as render events.
    // ScheduleParameters and SetParameter add to it.
    EventSlot parameterSlots[PARAMETER_QUEUE_SIZE];
    EventQueue parameterQueue;

    // Held while taking events from the two queues. The render thread only
    // tries it and leaves the events for its next call when it is held.
    // Reset, Initialize and Uninitialize take it.
    pthread_mutex_t queueReaderMutex;

    // MIDI output. The list is only allocated for a plugin with MIDI
    // output. The host sets the callback when it listens.
    bool isMidiEffect;
    // An instrument or generator with several output buses: it reports no
    // channel counts and its buses list no layouts (see
    // kAudioUnitProperty_SupportedNumChannels).
    bool isMultiOutputSource;
    MIDIPacketList* midiOutputList;
    UInt32 midiOutputListBytes;
#ifdef BEAMER_DIAGNOSTICS
    // The failing property queries logged so far, so that each is logged
    // once: a host may repeat a query, also on its render thread. Written
    // without a lock; a race only logs a line twice.
    struct {
        AudioUnitPropertyID propID;
        AudioUnitScope scope;
        AudioUnitElement element;
        OSStatus status;
    } loggedQueries[32];
    _Atomic UInt32 loggedQueryCount;
#endif
} BeamerAuv2Instance;

// MIDI output of one render call.
typedef struct {
    AUMIDIOutputCallbackStruct callback;
    const AudioTimeStamp* timeStamp;
    MIDIPacketList* list;
    // Size of the list in bytes.
    UInt32 listBytes;
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
#ifdef BEAMER_DIAGNOSTICS
static OSStatus BeamerAuv2GetPropertyInfoNoted(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, UInt32* outDataSize, Boolean* outWritable);
static OSStatus BeamerAuv2GetPropertyNoted(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, void* outData, UInt32* ioDataSize);
static OSStatus BeamerAuv2InitializeNoted(void* self);
static OSStatus BeamerAuv2SetPropertyNoted(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, const void* inData, UInt32 inDataSize);
#endif
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
static OSStatus BeamerAuv2SysEx(void* self, const UInt8* inData, UInt32 inLength);
static OSStatus BeamerAuv2MIDIEventList(void* self, UInt32 inOffsetSampleFrame, const MIDIEventList* evtList);
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

// Whether `element` is an input element of the plugin.
static bool BeamerAuv2IsInputElement(BeamerAuv2Instance* inst, AudioUnitElement element) {
    return element < inst->inputElementCount;
}

// The elements of an input or output scope, 0 for another scope.
static UInt32 BeamerAuv2ElementCount(BeamerAuv2Instance* inst, AudioUnitScope scope) {
    switch (scope) {
        case kAudioUnitScope_Input: return inst->inputElementCount;
        case kAudioUnitScope_Output: return inst->outputElementCount;
        default: return 0;
    }
}

// The buses the plugin declares in an input or output scope, 0 for another
// scope.
static UInt32 BeamerAuv2DeclaredCount(BeamerAuv2Instance* inst, AudioUnitScope scope) {
    uint32_t count = 0;
    switch (scope) {
        case kAudioUnitScope_Input: count = beamer_au_get_input_bus_count(inst->rustInstance); break;
        case kAudioUnitScope_Output: count = beamer_au_get_output_bus_count(inst->rustInstance); break;
        default: break;
    }
    return (count < BEAMER_AU_MAX_BUSES) ? count : BEAMER_AU_MAX_BUSES;
}

// Whether the unit processes in place as the host sees it: when it is on
// and the main buses have the same channel count. With other counts the
// input is pulled into buffers of its own (see BeamerAuv2PullInput).
static bool BeamerAuv2ReportsInPlace(BeamerAuv2Instance* inst) {
    return atomic_load_explicit(&inst->processesInPlace, memory_order_relaxed)
        && inst->inputElementCount > 0 && inst->outputElementCount > 0
        && inst->inputFormats[0].mChannelsPerFrame == inst->outputFormats[0].mChannelsPerFrame;
}

// The stream format of an element of an input or output scope, NULL for
// another scope or an element past the count.
static AudioStreamBasicDescription* BeamerAuv2Format(BeamerAuv2Instance* inst,
    AudioUnitScope scope, AudioUnitElement element) {
    if (element >= BeamerAuv2ElementCount(inst, scope)) {
        return NULL;
    }
    return (scope == kAudioUnitScope_Input) ? &inst->inputFormats[element] : &inst->outputFormats[element];
}

// The channel layout the host set on an element, as for BeamerAuv2Format.
static BeamerAuLayout* BeamerAuv2HostLayout(BeamerAuv2Instance* inst,
    AudioUnitScope scope, AudioUnitElement element) {
    if (element >= BeamerAuv2ElementCount(inst, scope)) {
        return NULL;
    }
    return (scope == kAudioUnitScope_Input) ? &inst->inputLayouts[element] : &inst->outputLayouts[element];
}

// The channel layout of an element: the host's, or the tag of the set its
// channels have. An element that is off has none.
static OSStatus BeamerAuv2CurrentLayout(BeamerAuv2Instance* inst, AudioUnitScope scope,
    AudioUnitElement element, BeamerAuLayout* out) {
    if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
        return kAudioUnitErr_InvalidScope;
    }
    const AudioStreamBasicDescription* format = BeamerAuv2Format(inst, scope, element);
    const BeamerAuLayout* host = BeamerAuv2HostLayout(inst, scope, element);
    if (format == NULL || host == NULL) {
        return kAudioUnitErr_InvalidElement;
    }
    if (format->mChannelsPerFrame == 0) {
        return kAudioUnitErr_PropertyNotInUse;
    }
    if (host->count == format->mChannelsPerFrame) {
        *out = *host;
        return noErr;
    }
    uint32_t tag = beamer_au_get_layout_tag(inst->rustInstance,
        scope == kAudioUnitScope_Input, element, format->mChannelsPerFrame);
    if (tag == 0) {
        return kAudioUnitErr_InvalidProperty;
    }
    *out = beamer_au_layout_of_tag(tag);
    return noErr;
}

// Give a bus that is on the labels of the layout the unit reports for it:
// the host's, or the one of its set's tag, whose channel order the plugin
// then takes (Apple's 7.x tags have the side pair first).
static void BeamerAuv2LayoutToBus(BeamerAuv2Instance* inst, AudioUnitScope scope,
    AudioUnitElement element, BeamerAuBusInfo* bus) {
    BeamerAuLayout layout;
    if (bus->channel_count == 0 || BeamerAuv2CurrentLayout(inst, scope, element, &layout) != noErr) {
        return;
    }
    if (layout.count == 0) {
        AudioChannelLayout tagOnly;
        memset(&tagOnly, 0, sizeof(tagOnly));
        tagOnly.mChannelLayoutTag = layout.tag;
        if (!beamer_au_layout_read(&tagOnly, beamer_au_layout_bytes(0), &layout)) {
            return;
        }
    }
    beamer_au_layout_to_bus(&layout, bus);
}

// Whether two channel layouts name the same channels in the same order.
static bool BeamerAuv2SameLayout(const BeamerAuLayout* a, const BeamerAuLayout* b) {
    BeamerAuLayout expandedA = *a;
    BeamerAuLayout expandedB = *b;
    // A layout of only a tag has its labels from the system.
    AudioChannelLayout tagOnly;
    memset(&tagOnly, 0, sizeof(tagOnly));
    if (a->count == 0) {
        tagOnly.mChannelLayoutTag = a->tag;
        if (!beamer_au_layout_read(&tagOnly, beamer_au_layout_bytes(0), &expandedA)) return false;
    }
    if (b->count == 0) {
        tagOnly.mChannelLayoutTag = b->tag;
        if (!beamer_au_layout_read(&tagOnly, beamer_au_layout_bytes(0), &expandedB)) return false;
    }
    return expandedA.count == expandedB.count
        && memcmp(expandedA.labels, expandedB.labels, expandedA.count * sizeof(UInt32)) == 0;
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

// Tell the host on the main thread that the latency, the tail or the
// bypass parameter changed after a render call (BeamerAuHostChangeFn): its
// property listeners ask for the new values.
static void BeamerAuv2HostChanged(void* context, uint32_t changes) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)context;
    if (changes & BEAMER_AU_HOST_CHANGE_LATENCY) {
        NotifyPropertyListeners(inst, kAudioUnitProperty_Latency, kAudioUnitScope_Global, 0);
    }
    if (changes & BEAMER_AU_HOST_CHANGE_TAIL) {
        NotifyPropertyListeners(inst, kAudioUnitProperty_TailTime, kAudioUnitScope_Global, 0);
    }
    if (changes & BEAMER_AU_HOST_CHANGE_BYPASS) {
        NotifyPropertyListeners(inst, kAudioUnitProperty_BypassEffect, kAudioUnitScope_Global, 0);
    }
}

// Tell parameter listeners (a generic view, an automation lane) that a
// parameter changed, for a change that did not come through
// AudioUnitSetParameter.
static void BeamerAuv2NotifyParameterChange(BeamerAuv2Instance* inst, AudioUnitParameterID paramID) {
    AudioUnitEvent event;
    memset(&event, 0, sizeof(event));
    event.mEventType = kAudioUnitEvent_ParameterValueChange;
    event.mArgument.mParameter.mAudioUnit = inst->componentInstance;
    event.mArgument.mParameter.mParameterID = paramID;
    event.mArgument.mParameter.mScope = kAudioUnitScope_Global;
    event.mArgument.mParameter.mElement = 0;
    AUEventListenerNotify(NULL, NULL, &event);
}

// The current name of an element, as the slot that holds it, as for
// BeamerAuv2Format. Read and written under namesMutex.
static CFStringRef* BeamerAuv2NameSlot(BeamerAuv2Instance* inst,
    AudioUnitScope scope, AudioUnitElement element) {
    if (element >= BeamerAuv2ElementCount(inst, scope)) {
        return NULL;
    }
    return (scope == kAudioUnitScope_Input) ? &inst->inputNames[element] : &inst->outputNames[element];
}

// The name the plugin gives an element, for an element that exists.
static CFStringRef BeamerAuv2DeclaredName(BeamerAuv2Instance* inst,
    AudioUnitScope scope, AudioUnitElement element) {
    return (scope == kAudioUnitScope_Input) ? inst->declaredInputNames[element] : inst->declaredOutputNames[element];
}

// Give an element a name (the declared one for NULL or an empty name). The
// listeners hear of it when it changes. Hosts restoring a state may set
// every name again; notifying only on a change keeps them from reacting to
// names that stay the same.
static void BeamerAuv2SetElementName(BeamerAuv2Instance* inst,
    AudioUnitScope scope, AudioUnitElement element, CFStringRef name) {
    CFStringRef* slot = BeamerAuv2NameSlot(inst, scope, element);
    if (slot == NULL) {
        return;
    }
    CFStringRef next = (name != NULL && CFStringGetLength(name) > 0)
        ? CFStringCreateCopy(kCFAllocatorDefault, name)
        : (CFStringRef)CFRetain(BeamerAuv2DeclaredName(inst, scope, element));
    pthread_mutex_lock(&inst->namesMutex);
    CFStringRef previous = *slot;
    *slot = next;
    pthread_mutex_unlock(&inst->namesMutex);
    bool changed = CFStringCompare(previous, next, 0) != kCFCompareEqualTo;
    CFRelease(previous);
    if (changed) {
        NotifyPropertyListeners(inst, kAudioUnitProperty_ElementName, scope, element);
    }
}

// The names a host changed, for the state: under "element-name", as the
// AU SDK saves them, a dictionary of scope numbers to dictionaries of
// element numbers to names. NULL when every element has its declared name.
static CFDictionaryRef BeamerAuv2CopyChangedNames(BeamerAuv2Instance* inst) {
    static const AudioUnitScope scopes[] = { kAudioUnitScope_Input, kAudioUnitScope_Output };
    CFMutableDictionaryRef names = NULL;
    pthread_mutex_lock(&inst->namesMutex);
    for (size_t s = 0; s < sizeof(scopes) / sizeof(scopes[0]); s++) {
        CFMutableDictionaryRef elements = NULL;
        for (UInt32 element = 0; element < BeamerAuv2ElementCount(inst, scopes[s]); element++) {
            CFStringRef name = *BeamerAuv2NameSlot(inst, scopes[s], element);
            if (CFStringCompare(name, BeamerAuv2DeclaredName(inst, scopes[s], element), 0) == kCFCompareEqualTo) {
                continue;
            }
            if (elements == NULL) {
                elements = CFDictionaryCreateMutable(kCFAllocatorDefault, 0,
                    &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
            }
            CFStringRef key = CFStringCreateWithFormat(kCFAllocatorDefault, NULL, CFSTR("%u"), (unsigned)element);
            CFDictionarySetValue(elements, key, name);
            CFRelease(key);
        }
        if (elements != NULL) {
            if (names == NULL) {
                names = CFDictionaryCreateMutable(kCFAllocatorDefault, 0,
                    &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
            }
            CFStringRef key = CFStringCreateWithFormat(kCFAllocatorDefault, NULL, CFSTR("%u"), (unsigned)scopes[s]);
            CFDictionarySetValue(names, key, elements);
            CFRelease(key);
            CFRelease(elements);
        }
    }
    pthread_mutex_unlock(&inst->namesMutex);
    return names;
}

// The value under a number key of a state dictionary, if it has the type
// typeID.
static CFTypeRef BeamerAuv2NumberedValue(CFDictionaryRef dict, UInt32 number, CFTypeID typeID) {
    if (dict == NULL) {
        return NULL;
    }
    CFStringRef key = CFStringCreateWithFormat(kCFAllocatorDefault, NULL, CFSTR("%u"), (unsigned)number);
    CFTypeRef value = CFDictionaryGetValue(dict, key);
    CFRelease(key);
    return (value != NULL && CFGetTypeID(value) == typeID) ? value : NULL;
}

// Give each element the name a state saved for it (see
// BeamerAuv2CopyChangedNames), the declared name where it saved none.
static void BeamerAuv2RestoreNames(BeamerAuv2Instance* inst, CFDictionaryRef names) {
    static const AudioUnitScope scopes[] = { kAudioUnitScope_Input, kAudioUnitScope_Output };
    for (size_t s = 0; s < sizeof(scopes) / sizeof(scopes[0]); s++) {
        CFDictionaryRef elements = (CFDictionaryRef)BeamerAuv2NumberedValue(names, scopes[s],
            CFDictionaryGetTypeID());
        for (UInt32 element = 0; element < BeamerAuv2ElementCount(inst, scopes[s]); element++) {
            CFStringRef name = (CFStringRef)BeamerAuv2NumberedValue(elements, element, CFStringGetTypeID());
            BeamerAuv2SetElementName(inst, scopes[s], element, name);
        }
    }
}

// Bytes per sample of a float format: 4 or 8.
static UInt32 BeamerAuv2SampleBytes(const AudioStreamBasicDescription* format) {
    return (format->mBitsPerChannel == 64) ? sizeof(Float64) : sizeof(Float32);
}

// The input buffers hold 64-bit samples, so that they fit either format.
static OSStatus EnsureInputBufferList(BeamerAuv2Instance* inst, UInt32 channels, UInt32 frames) {
    if (channels > BEAMER_AU_MAX_CHANNELS) return kAudio_ParamError;
    UInt32 byteSize = frames * BeamerAuv2SampleBytes(&inst->inputFormats[0]);
    if (inst->inputBufferList && inst->inputBufferList->mNumberBuffers == channels
        && inst->inputBufferFrames >= frames) {
        // Existing buffer is large enough. Restore our owned mData pointers
        // because the host may have replaced them during a previous render.
        for (UInt32 i = 0; i < inst->inputBufferList->mNumberBuffers; i++) {
            inst->inputBufferList->mBuffers[i].mDataByteSize = byteSize;
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
        inst->inputBufferList->mBuffers[i].mDataByteSize = byteSize;
        inst->ownedInputBuffers[i] = calloc(frames, sizeof(Float64));
        inst->inputBufferList->mBuffers[i].mData = inst->ownedInputBuffers[i];
        if (!inst->ownedInputBuffers[i]) {
            FreeInputBufferList(inst);
            return kAudio_MemFullError;
        }
    }

    inst->inputBufferFrames = frames;
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
        inst->inputBufferFrames = 0;
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

static UInt32 SysExRoundUp(UInt32 bytes) {
    return (bytes + 15) & ~(UInt32)15;
}

// Bytes of the record of a message of `length` bytes.
static UInt32 SysExRecordSize(UInt32 length) {
    UInt32 eventBytes = (UInt32)offsetof(AUMIDIEvent, data) + length;
    // At least a parameter event, the largest member of the render event
    // union as Rust reads it.
    if (eventBytes < sizeof(AUParameterEvent)) {
        eventBytes = sizeof(AUParameterEvent);
    }
    return SysExRoundUp((UInt32)sizeof(SysExRecordHeader) + eventBytes);
}

// Allocate the ring for messages of up to `largestMessage` bytes, at most
// 65,535 (the length of a render event has 16 bits). Not for the render
// thread. Without memory the queue stays off.
static void SysExQueueInit(SysExQueue* queue, UInt32 largestMessage) {
    queue->bytes = NULL;
    queue->capacity = 0;
    queue->largestMessage = 0;
    atomic_init(&queue->writePosition, 0);
    atomic_init(&queue->readPosition, 0);
    atomic_init(&queue->dropped, 0);
    if (largestMessage == 0) {
        return;
    }
    // The length of a SysEx render event has 16 bits.
    if (largestMessage > UINT16_MAX) {
        largestMessage = UINT16_MAX;
    }
    UInt32 capacity = 2 * SysExRecordSize(largestMessage);
    queue->bytes = (uint8_t*)calloc(1, capacity);
    if (queue->bytes) {
        queue->capacity = capacity;
        queue->largestMessage = largestMessage;
    }
}

static void SysExQueueFree(SysExQueue* queue) {
    free(queue->bytes);
    queue->bytes = NULL;
    queue->capacity = 0;
    queue->largestMessage = 0;
}

static SysExRecordHeader* SysExRecordAt(SysExQueue* queue, uint64_t position) {
    return (SysExRecordHeader*)(queue->bytes + position % queue->capacity);
}

// The render event of a message record.
static AUMIDIEvent* SysExRecordEvent(SysExRecordHeader* record) {
    return (AUMIDIEvent*)((uint8_t*)record + sizeof(SysExRecordHeader));
}

// Add a message. Any thread may call it, also several at once. Returns
// false when the queue is off or full, or the message empty or too large.
static bool SysExQueuePush(SysExQueue* queue, const UInt8* message, UInt32 length) {
    if (!queue->bytes || length == 0 || length > queue->largestMessage) {
        return false;
    }
    UInt32 recordSize = SysExRecordSize(length);
    uint64_t position = atomic_load_explicit(&queue->writePosition, memory_order_relaxed);
    UInt32 padding;
    for (;;) {
        // The consumer may have taken records past a position that is out
        // of date by now: load it again.
        uint64_t read = atomic_load_explicit(&queue->readPosition, memory_order_acquire);
        if (position < read) {
            position = atomic_load_explicit(&queue->writePosition, memory_order_relaxed);
            continue;
        }
        // A read position older than the one that made this position
        // possible gives more than the capacity: load it again.
        uint64_t used = position - read;
        if (used > queue->capacity) {
            continue;
        }
        UInt32 offset = (UInt32)(position % queue->capacity);
        padding = (offset + recordSize > queue->capacity) ? queue->capacity - offset : 0;
        if (used + padding + recordSize > queue->capacity) {
            return false;
        }
        // On failure position holds the one another producer moved on to.
        if (atomic_compare_exchange_weak_explicit(&queue->writePosition, &position,
                position + padding + recordSize, memory_order_relaxed, memory_order_relaxed)) {
            break;
        }
    }

    if (padding > 0) {
        SysExRecordHeader* pad = SysExRecordAt(queue, position);
        pad->size = padding;
        pad->isPadding = 1;
        atomic_store_explicit(&pad->sequence, (UInt32)(position + 1), memory_order_release);
        position += padding;
    }

    SysExRecordHeader* record = SysExRecordAt(queue, position);
    record->size = recordSize;
    record->isPadding = 0;
    AUMIDIEvent* event = SysExRecordEvent(record);
    event->next = NULL;
    // Offset 0 of the next buffer: the call has no sample offset.
    event->eventSampleTime = 0;
    event->eventType = AURenderEventMIDISysEx;
    event->reserved = 0;
    event->length = (uint16_t)length;
    event->cable = 0;
    memcpy((uint8_t*)event + offsetof(AUMIDIEvent, data), message, length);
    atomic_store_explicit(&record->sequence, (UInt32)(position + 1), memory_order_release);
    return true;
}

// The position after the records ready for the consumer, in order from
// the read position. A record whose producer is still writing it ends them.
// For each message record `visit` gets its event, if it is not NULL.
static uint64_t SysExQueueReadyEnd(SysExQueue* queue, void (*visit)(void* context, AUMIDIEvent* event), void* context) {
    uint64_t position = atomic_load_explicit(&queue->readPosition, memory_order_relaxed);
    if (!queue->bytes) {
        return position;
    }
    uint64_t end = atomic_load_explicit(&queue->writePosition, memory_order_acquire);
    while (position != end) {
        SysExRecordHeader* record = SysExRecordAt(queue, position);
        if (atomic_load_explicit(&record->sequence, memory_order_acquire) != (UInt32)(position + 1)) {
            break;
        }
        if (!record->isPadding && visit) {
            visit(context, SysExRecordEvent(record));
        }
        position += record->size;
    }
    return position;
}

// Give the records before `end` back to the producers, zeroed.
static void SysExQueueRelease(SysExQueue* queue, uint64_t end) {
    uint64_t position = atomic_load_explicit(&queue->readPosition, memory_order_relaxed);
    if (!queue->bytes || position == end) {
        return;
    }
    UInt32 start = (UInt32)(position % queue->capacity);
    UInt32 count = (UInt32)(end - position);
    UInt32 first = (start + count <= queue->capacity) ? count : queue->capacity - start;
    memset(queue->bytes + start, 0, first);
    memset(queue->bytes, 0, count - first);
    atomic_store_explicit(&queue->readPosition, end, memory_order_release);
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
    atomic_init(&inst->processesInPlace, false);

    // Initialize mutexes
    // Recursive: a listener may add or remove listeners while it is told
    // about a change.
    pthread_mutexattr_t listenerMutexAttributes;
    pthread_mutexattr_init(&listenerMutexAttributes);
    pthread_mutexattr_settype(&listenerMutexAttributes, PTHREAD_MUTEX_RECURSIVE);
    pthread_mutex_init(&inst->listenerMutex, &listenerMutexAttributes);
    pthread_mutexattr_destroy(&listenerMutexAttributes);
    pthread_mutex_init(&inst->renderNotifyMutex, NULL);
    pthread_mutex_init(&inst->callbacksMutex, NULL);
    pthread_mutex_init(&inst->queueReaderMutex, NULL);
    pthread_mutex_init(&inst->namesMutex, NULL);

    EventQueueInit(&inst->midiQueue, inst->midiSlots, BEAMER_AU_MAX_MIDI_EVENTS);
    EventQueueInit(&inst->parameterQueue, inst->parameterSlots, PARAMETER_QUEUE_SIZE);

    return &inst->interface;
}

// =============================================================================
// MARK: - Open/Close/Lookup
// =============================================================================

// The name the plugin gives a bus, as a new string. An empty name if the
// bridge has none, so that every element has a name.
static CFStringRef BeamerAuv2CopyDeclaredName(BeamerAuv2Instance* inst, bool isInput, UInt32 index) {
    char name[BEAMER_AU_MAX_PARAM_NAME_LENGTH];
    CFStringRef string = NULL;
    if (beamer_au_get_bus_name(inst->rustInstance, isInput, index, name, sizeof(name))) {
        string = CFStringCreateWithCString(kCFAllocatorDefault, name, kCFStringEncodingUTF8);
    }
    return string ? string : (CFStringRef)CFRetain(CFSTR(""));
}

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
    // Destroying the Rust instance in Close clears the callback before the
    // instance goes.
    beamer_au_set_host_change_callback(inst->rustInstance, BeamerAuv2HostChanged, inst);
    inst->hasBypassParameter = beamer_au_get_bypass_parameter(inst->rustInstance, &inst->bypassParameterId);

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

    // Every declared bus is an element and starts on.
    inst->inputElementCount = BeamerAuv2DeclaredCount(inst, kAudioUnitScope_Input);
    inst->outputElementCount = BeamerAuv2DeclaredCount(inst, kAudioUnitScope_Output);

    // Each element starts with the name the plugin gives its bus.
    for (UInt32 i = 0; i < inst->inputElementCount; i++) {
        inst->declaredInputNames[i] = BeamerAuv2CopyDeclaredName(inst, true, i);
        inst->inputNames[i] = (CFStringRef)CFRetain(inst->declaredInputNames[i]);
    }
    for (UInt32 i = 0; i < inst->outputElementCount; i++) {
        inst->declaredOutputNames[i] = BeamerAuv2CopyDeclaredName(inst, false, i);
        inst->outputNames[i] = (CFStringRef)CFRetain(inst->declaredOutputNames[i]);
    }

    // In place with a main input.
    atomic_store_explicit(&inst->processesInPlace, inst->inputElementCount > 0, memory_order_relaxed);

    // Prepare MIDI output for a plugin with MIDI output.
    AudioComponentDescription desc;
    beamer_au_get_component_description(&desc);
    inst->isMidiEffect = (desc.componentType == kAudioUnitType_MIDIProcessor);
    inst->isMultiOutputSource = (desc.componentType == kAudioUnitType_MusicDevice
        || desc.componentType == kAudioUnitType_Generator) && inst->outputElementCount > 1;
    if (beamer_au_produces_midi(inst->rustInstance)) {
        // Room for the channel messages and the SysEx output of a render call.
        UInt32 sysexBytes = beamer_au_get_sysex_output_bytes(inst->rustInstance);
        UInt32 listBytes = (sysexBytes > UINT32_MAX - MIDI_OUTPUT_LIST_BYTES)
            ? UINT32_MAX : MIDI_OUTPUT_LIST_BYTES + sysexBytes;
        inst->midiOutputList = (MIDIPacketList*)calloc(1, listBytes);
        inst->midiOutputListBytes = inst->midiOutputList ? listBytes : 0;
    }

    // SysEx input for a plugin with MIDI input: a message may be as large
    // as what a render call holds.
    SysExQueueInit(&inst->sysexQueue, beamer_au_get_sysex_input_bytes(inst->rustInstance));
    inst->midiProtocol = beamer_au_get_midi_protocol(inst->rustInstance);

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

    free(inst->bypassSilence);
    inst->bypassSilence = NULL;
    inst->bypassSilenceFrames = 0;

    free(inst->midiOutputList);
    inst->midiOutputList = NULL;
    inst->midiOutputListBytes = 0;

    SysExQueueFree(&inst->sysexQueue);

    for (UInt32 i = 0; i < BEAMER_AU_MAX_BUSES; i++) {
        if (inst->declaredInputNames[i]) CFRelease(inst->declaredInputNames[i]);
        if (inst->declaredOutputNames[i]) CFRelease(inst->declaredOutputNames[i]);
        if (inst->inputNames[i]) CFRelease(inst->inputNames[i]);
        if (inst->outputNames[i]) CFRelease(inst->outputNames[i]);
    }

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
    pthread_mutex_destroy(&inst->namesMutex);

    free(inst);
    return noErr;
}

static AudioComponentMethod BeamerAuv2Lookup(SInt16 selector) {
    switch (selector) {
        case kAudioUnitInitializeSelect:
#ifdef BEAMER_DIAGNOSTICS
            return (AudioComponentMethod)BeamerAuv2InitializeNoted;
#else
            return (AudioComponentMethod)BeamerAuv2Initialize;
#endif
        case kAudioUnitUninitializeSelect:
            return (AudioComponentMethod)BeamerAuv2Uninitialize;
#ifdef BEAMER_DIAGNOSTICS
        case kAudioUnitGetPropertyInfoSelect:
            return (AudioComponentMethod)BeamerAuv2GetPropertyInfoNoted;
        case kAudioUnitGetPropertySelect:
            return (AudioComponentMethod)BeamerAuv2GetPropertyNoted;
#else
        case kAudioUnitGetPropertyInfoSelect:
            return (AudioComponentMethod)BeamerAuv2GetPropertyInfo;
        case kAudioUnitGetPropertySelect:
            return (AudioComponentMethod)BeamerAuv2GetProperty;
#endif
        case kAudioUnitSetPropertySelect:
#ifdef BEAMER_DIAGNOSTICS
            return (AudioComponentMethod)BeamerAuv2SetPropertyNoted;
#else
            return (AudioComponentMethod)BeamerAuv2SetProperty;
#endif
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

    // Every declared bus with the channels of its format: 0 for one the
    // host switched off.
    for (uint32_t i = 0; i < inputBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
        busConfig.input_buses[i].channel_count = inst->inputFormats[i].mChannelsPerFrame;
        busConfig.input_buses[i].bus_type = (i == 0) ? BeamerAuBusTypeMain : BeamerAuBusTypeAuxiliary;
        BeamerAuv2LayoutToBus(inst, kAudioUnitScope_Input, i, &busConfig.input_buses[i]);
    }
    if (inputBusCount > 0) {
        inputChannels = inst->inputFormats[0].mChannelsPerFrame;
    }

    for (uint32_t i = 0; i < outputBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
        busConfig.output_buses[i].channel_count = inst->outputFormats[i].mChannelsPerFrame;
        busConfig.output_buses[i].bus_type = (i == 0) ? BeamerAuBusTypeMain : BeamerAuBusTypeAuxiliary;
        BeamerAuv2LayoutToBus(inst, kAudioUnitScope_Output, i, &busConfig.output_buses[i]);
    }
    if (outputBusCount > 0) {
        outputChannels = inst->outputFormats[0].mChannelsPerFrame;
    }

    // Validate channel configuration before proceeding
    bool configValid = beamer_au_is_channel_config_valid(inst->rustInstance, inputChannels, outputChannels);
    if (!configValid) {
        return kAudioUnitErr_FormatNotSupported;
    }

    // Determine sample format. The plugin renders every bus in one sample
    // type, so all buses that are on must have the same depth.
    UInt32 bits = 32;
    if (outputBusCount > 0) {
        bits = inst->outputFormats[0].mBitsPerChannel;
    } else if (inputBusCount > 0) {
        bits = inst->inputFormats[0].mBitsPerChannel;
    }
    for (uint32_t i = 0; i < inputBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
        if (busConfig.input_buses[i].channel_count > 0 && inst->inputFormats[i].mBitsPerChannel != bits) {
            return kAudioUnitErr_FormatNotSupported;
        }
    }
    for (uint32_t i = 0; i < outputBusCount && i < BEAMER_AU_MAX_BUSES; i++) {
        if (busConfig.output_buses[i].channel_count > 0 && inst->outputFormats[i].mBitsPerChannel != bits) {
            return kAudioUnitErr_FormatNotSupported;
        }
    }
    BeamerAuSampleFormat format = (bits == 64) ? BeamerAuSampleFormatFloat64 : BeamerAuSampleFormatFloat32;

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

        // Silence for the bypassed render calls, sized for 64-bit samples.
        if (inst->bypassSilenceFrames < inst->maxFramesPerSlice) {
            void* silence = calloc(inst->maxFramesPerSlice, sizeof(Float64));
            if (silence) {
                free(inst->bypassSilence);
                inst->bypassSilence = silence;
                inst->bypassSilenceFrames = inst->maxFramesPerSlice;
            }
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
        // for a render call that will not come still apply. MIDI queued for
        // it is dropped.
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
            if (scope == kAudioUnitScope_Input || scope == kAudioUnitScope_Output) {
                if (element >= BeamerAuv2ElementCount(inst, scope))
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
            if (scope != kAudioUnitScope_Input) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!BeamerAuv2IsInputElement(inst, element)) {
                return kAudioUnitErr_InvalidElement;
            }
            if (outDataSize) *outDataSize = sizeof(AURenderCallbackStruct);
            if (outWritable) *outWritable = true;
            return noErr;

        // Audio unit connection
        case kAudioUnitProperty_MakeConnection:
            if (scope != kAudioUnitScope_Input) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!BeamerAuv2IsInputElement(inst, element)) {
                return kAudioUnitErr_InvalidElement;
            }
            if (outDataSize) *outDataSize = sizeof(AudioUnitConnection);
            if (outWritable) *outWritable = true;
            return noErr;

        // Supported channel layouts
        // An instrument or generator with several output buses reports no
        // channel counts: without them it supports its initial buses only,
        // which hosts then offer as they are. The AUv3 does the same: hosts keep one record
        // for both formats.
        case kAudioUnitProperty_SupportedNumChannels:
            if (scope == kAudioUnitScope_Global && !inst->isMultiOutputSource) {
                BeamerAuChannelCapabilities caps;
                if (beamer_au_get_channel_capabilities(inst->rustInstance, &caps)) {
                    if (outDataSize) *outDataSize = caps.count * sizeof(AUChannelInfo);
                    if (outWritable) *outWritable = false;
                    return noErr;
                }
            }
            return kAudioUnitErr_InvalidProperty;

        // The channel layout of a bus. Writable, as hosts expect of a unit
        // that takes layouts; the plugin is prepared for it, so while
        // initialized only the same channels are accepted.
        case kAudioUnitProperty_AudioChannelLayout: {
            BeamerAuLayout layout;
            OSStatus status = BeamerAuv2CurrentLayout(inst, scope, element, &layout);
            if (status != noErr) {
                return status;
            }
            if (outDataSize) *outDataSize = beamer_au_layout_size(&layout);
            if (outWritable) *outWritable = true;
            return noErr;
        }

        // The channel layouts a bus takes.
        case kAudioUnitProperty_SupportedChannelLayoutTags: {
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
                return kAudioUnitErr_InvalidScope;
            }
            if (element >= BeamerAuv2ElementCount(inst, scope)) {
                return kAudioUnitErr_InvalidElement;
            }
            // The buses of a multi-output instrument or generator list no
            // other layouts.
            if (inst->isMultiOutputSource) {
                return kAudioUnitErr_InvalidProperty;
            }
            uint32_t count = beamer_au_get_supported_layout_tags(inst->rustInstance,
                scope == kAudioUnitScope_Input, element, NULL, 0);
            if (count == 0) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (outDataSize) *outDataSize = count * sizeof(AudioChannelLayoutTag);
            if (outWritable) *outWritable = false;
            return noErr;
        }

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

        // Element count: the declared buses. It is not writable, so that a
        // host does not offer bus counts of its own (Logic lists them as
        // multi-output variants); an aux bus goes off with a format without
        // channels.
        case kAudioUnitProperty_ElementCount:
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = false;
            return noErr;

        // The name of a bus. A host may rename it; the state keeps the name.
        case kAudioUnitProperty_ElementName:
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
                return kAudioUnitErr_InvalidScope;
            }
            if (element >= BeamerAuv2ElementCount(inst, scope)) {
                return kAudioUnitErr_InvalidElement;
            }
            if (outDataSize) *outDataSize = sizeof(CFStringRef);
            if (outWritable) *outWritable = true;
            return noErr;

        // In-place processing.
        case kAudioUnitProperty_InPlaceProcessing:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = BeamerAuv2DeclaredCount(inst, kAudioUnitScope_Input) > 0;
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

        // The MIDI protocol of the event lists the plugin takes, for a
        // plugin with MIDI input. A host that gets it sends event lists
        // through MusicDeviceMIDIEventList.
        case kAudioUnitProperty_AudioUnitMIDIProtocol:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (inst->midiProtocol == 0) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (outDataSize) *outDataSize = sizeof(SInt32);
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

        // CocoaUI - only supported when the plugin has an editor
        case kAudioUnitProperty_CocoaUI:
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!beamer_au_has_editor(inst->rustInstance)) {
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
                if (element >= inst->inputElementCount)
                    return kAudioUnitErr_InvalidElement;
                *desc = inst->inputFormats[element];
            } else if (scope == kAudioUnitScope_Output) {
                if (element >= inst->outputElementCount)
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
            if (inst->hasBypassParameter) {
                *(UInt32*)outData = beamer_au_get_parameter_value(inst->rustInstance, inst->bypassParameterId) >= 0.5f ? 1 : 0;
            } else {
                *(UInt32*)outData = atomic_load_explicit(&inst->bypassed, memory_order_relaxed) ? 1 : 0;
            }
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }

        case kAudioUnitProperty_AudioChannelLayout: {
            BeamerAuLayout layout;
            OSStatus status = BeamerAuv2CurrentLayout(inst, scope, element, &layout);
            if (status != noErr) {
                return status;
            }
            UInt32 needed = beamer_au_layout_size(&layout);
            if (!outData || !ioDataSize || *ioDataSize < needed) {
                if (ioDataSize) *ioDataSize = needed;
                return outData ? kAudioUnitErr_InvalidPropertyValue : noErr;
            }
            beamer_au_layout_write(&layout, (AudioChannelLayout*)outData);
            *ioDataSize = needed;
            return noErr;
        }

        case kAudioUnitProperty_SupportedChannelLayoutTags: {
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
                return kAudioUnitErr_InvalidScope;
            }
            if (element >= BeamerAuv2ElementCount(inst, scope)) {
                return kAudioUnitErr_InvalidElement;
            }
            if (inst->isMultiOutputSource) {
                return kAudioUnitErr_InvalidProperty;
            }
            bool isInput = (scope == kAudioUnitScope_Input);
            uint32_t count = beamer_au_get_supported_layout_tags(inst->rustInstance, isInput, element, NULL, 0);
            if (count == 0) {
                return kAudioUnitErr_InvalidProperty;
            }
            UInt32 needed = count * sizeof(AudioChannelLayoutTag);
            if (!outData || !ioDataSize || *ioDataSize < needed) {
                if (ioDataSize) *ioDataSize = needed;
                return outData ? kAudioUnitErr_InvalidPropertyValue : noErr;
            }
            *ioDataSize = beamer_au_get_supported_layout_tags(inst->rustInstance, isInput, element,
                (uint32_t*)outData, count) * sizeof(AudioChannelLayoutTag);
            return noErr;
        }

        case kAudioUnitProperty_SupportedNumChannels: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (inst->isMultiOutputSource) {
                return kAudioUnitErr_InvalidProperty;
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

            // The bus names a host changed.
            CFDictionaryRef elementNames = BeamerAuv2CopyChangedNames(inst);
            if (elementNames) {
                CFDictionarySetValue(dict, CFSTR(kAUPresetElementNameKey), elementNames);
                CFRelease(elementNames);
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

        // The host owns the string it gets.
        case kAudioUnitProperty_ElementName: {
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
                return kAudioUnitErr_InvalidScope;
            }
            CFStringRef* slot = BeamerAuv2NameSlot(inst, scope, element);
            if (slot == NULL) {
                return kAudioUnitErr_InvalidElement;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(CFStringRef)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            pthread_mutex_lock(&inst->namesMutex);
            CFStringRef name = (CFStringRef)CFRetain(*slot);
            pthread_mutex_unlock(&inst->namesMutex);
            *(CFStringRef*)outData = name;
            *ioDataSize = sizeof(CFStringRef);
            return noErr;
        }

        case kAudioUnitProperty_ElementCount: {
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            if (scope == kAudioUnitScope_Input || scope == kAudioUnitScope_Output) {
                *(UInt32*)outData = BeamerAuv2ElementCount(inst, scope);
            } else if (scope == kAudioUnitScope_Global) {
                *(UInt32*)outData = 1;
            } else {
                return kAudioUnitErr_InvalidScope;
            }
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }

        case kAudioUnitProperty_InPlaceProcessing: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(UInt32*)outData = BeamerAuv2ReportsInPlace(inst) ? 1 : 0;
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

        case kAudioUnitProperty_AudioUnitMIDIProtocol: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (inst->midiProtocol == 0) {
                return kAudioUnitErr_InvalidProperty;
            }
            if (!outData || !ioDataSize || *ioDataSize < sizeof(SInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            *(SInt32*)outData = inst->midiProtocol;
            *ioDataSize = sizeof(SInt32);
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
            if (!beamer_au_has_editor(inst->rustInstance)) {
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

            // Validate scope and element (bus must exist)
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
                return kAudioUnitErr_InvalidScope;
            }
            if (element >= BeamerAuv2ElementCount(inst, scope)) {
                return kAudioUnitErr_InvalidElement;
            }

            // A format without channels switches an aux bus off. It carries
            // no samples, so its sample type does not matter.
            UInt32 proposedChannels = desc->mChannelsPerFrame;
            bool switchesOff = (proposedChannels == 0 && element > 0);

            // Validate format (must be float, non-interleaved)
            if (!switchesOff && desc->mFormatID != kAudioFormatLinearPCM) {
                return kAudioUnitErr_FormatNotSupported;
            }
            if (!switchesOff && !(desc->mFormatFlags & kAudioFormatFlagIsFloat)) {
                return kAudioUnitErr_FormatNotSupported;
            }

            // A bus takes the channel counts the plugin's layouts give it.
            // Whether the main buses go together is checked at Initialize,
            // which the capabilities in SupportedNumChannels describe.
            if (!switchesOff && !beamer_au_bus_supports_channel_count(inst->rustInstance,
                    scope == kAudioUnitScope_Input, element, proposedChannels)) {
                return kAudioUnitErr_FormatNotSupported;
            }

            // The plugin is prepared for the current format: an initialized
            // unit only accepts it again.
            const AudioStreamBasicDescription* current = (scope == kAudioUnitScope_Input)
                ? &inst->inputFormats[element] : &inst->outputFormats[element];
            if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
                return StreamFormatsEqual(desc, current) ? noErr : kAudioUnitErr_PropertyNotWritable;
            }

            // Apply the format change. A channel layout of another count
            // no longer describes the bus.
            bool newChannelCount = (current->mChannelsPerFrame != proposedChannels);
            if (scope == kAudioUnitScope_Input) {
                inst->inputFormats[element] = *desc;
            } else {
                inst->outputFormats[element] = *desc;
            }
            if (!switchesOff) {
                inst->sampleRate = desc->mSampleRate;
            }
            BeamerAuLayout* layout = BeamerAuv2HostLayout(inst, scope, element);
            if (newChannelCount && layout != NULL) {
                memset(layout, 0, sizeof(*layout));
            }

            NotifyPropertyListeners(inst, propID, scope, element);
            if (newChannelCount) {
                NotifyPropertyListeners(inst, kAudioUnitProperty_AudioChannelLayout, scope, element);
            }
            // Whether it processes in place follows the main buses' counts.
            if (newChannelCount && element == 0) {
                NotifyPropertyListeners(inst, kAudioUnitProperty_InPlaceProcessing, kAudioUnitScope_Global, 0);
            }
            return noErr;
        }

        case kAudioUnitProperty_AudioChannelLayout: {
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
                return kAudioUnitErr_InvalidScope;
            }
            const AudioStreamBasicDescription* format = BeamerAuv2Format(inst, scope, element);
            BeamerAuLayout* stored = BeamerAuv2HostLayout(inst, scope, element);
            if (format == NULL || stored == NULL) {
                return kAudioUnitErr_InvalidElement;
            }
            if (format->mChannelsPerFrame == 0) {
                return kAudioUnitErr_PropertyNotInUse;
            }
            bool initialized = atomic_load_explicit(&inst->initialized, memory_order_acquire);

            // No data removes the host's layout: the bus has the set of
            // its channel count again.
            if (inData == NULL) {
                if (initialized && stored->count > 0) {
                    return kAudioUnitErr_PropertyNotWritable;
                }
                memset(stored, 0, sizeof(*stored));
                NotifyPropertyListeners(inst, propID, scope, element);
                return noErr;
            }

            BeamerAuLayout layout;
            if (!beamer_au_layout_read((const AudioChannelLayout*)inData, inDataSize, &layout)
                || layout.count != format->mChannelsPerFrame) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            beamer_au_layout_keep(&layout);
            if (!beamer_au_bus_supports_layout(inst->rustInstance, scope == kAudioUnitScope_Input,
                    element, layout.labels, layout.count)) {
                return kAudioUnitErr_FormatNotSupported;
            }

            // The plugin is prepared for the current layout: an initialized
            // unit only accepts the same channels again.
            if (initialized) {
                BeamerAuLayout current;
                OSStatus status = BeamerAuv2CurrentLayout(inst, scope, element, &current);
                if (status != noErr) {
                    return status;
                }
                return BeamerAuv2SameLayout(&layout, &current) ? noErr : kAudioUnitErr_PropertyNotWritable;
            }

            *stored = layout;
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
            // A bus that is off (without channels) has no rate the plugin
            // uses.
            if (atomic_load_explicit(&inst->initialized, memory_order_acquire)) {
                bool changes = (inst->sampleRate != sampleRate);
                for (uint32_t i = 0; i < inst->inputElementCount; i++) {
                    const AudioStreamBasicDescription* f = &inst->inputFormats[i];
                    changes |= (f->mChannelsPerFrame > 0 && f->mSampleRate != sampleRate);
                }
                for (uint32_t i = 0; i < inst->outputElementCount; i++) {
                    const AudioStreamBasicDescription* f = &inst->outputFormats[i];
                    changes |= (f->mChannelsPerFrame > 0 && f->mSampleRate != sampleRate);
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

        case kAudioUnitProperty_ElementCount: {
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inData || inDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            // The count stays at the declared buses; setting it to that is
            // accepted.
            UInt32 count = *(const UInt32*)inData;
            return (count == BeamerAuv2ElementCount(inst, scope)) ? noErr : kAudioUnitErr_PropertyNotWritable;
        }

        // A host renames a bus, also while initialized. No data or an empty
        // name gives the bus its declared name again.
        case kAudioUnitProperty_ElementName: {
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {
                return kAudioUnitErr_InvalidScope;
            }
            if (element >= BeamerAuv2ElementCount(inst, scope)) {
                return kAudioUnitErr_InvalidElement;
            }
            CFStringRef name = NULL;
            if (inData != NULL) {
                if (inDataSize < sizeof(CFStringRef)) {
                    return kAudioUnitErr_InvalidPropertyValue;
                }
                name = *(const CFStringRef*)inData;
                if (name != NULL && CFGetTypeID(name) != CFStringGetTypeID()) {
                    return kAudioUnitErr_InvalidPropertyValue;
                }
            }
            BeamerAuv2SetElementName(inst, scope, element, name);
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
            bool bypassed = *(UInt32*)inData != 0;
            if (inst->hasBypassParameter) {
                // A host that answers a change with the same value sets
                // nothing, so that no notification goes back and forth.
                bool current = beamer_au_get_parameter_value(inst->rustInstance, inst->bypassParameterId) >= 0.5f;
                if (current == bypassed) {
                    return noErr;
                }
                // The plugin bypasses itself: it keeps processing and ramps
                // (and delays) its bypass. Its parameter listeners follow.
                // No reset: it would clear the plugin's bypass mid-ramp. A
                // render call between the note and the write reports the
                // bypass once more, which the host reads back unchanged.
                beamer_au_note_host_bypass(inst->rustInstance, bypassed);
                beamer_au_set_parameter_value(inst->rustInstance, inst->bypassParameterId, bypassed ? 1.0f : 0.0f);
                BeamerAuv2NotifyParameterChange(inst, inst->bypassParameterId);
            } else {
                beamer_au_note_host_bypass(inst->rustInstance, bypassed);
                atomic_store_explicit(&inst->bypassed, bypassed, memory_order_relaxed);
            }
            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }

        case kAudioUnitProperty_SetRenderCallback: {
            if (scope != kAudioUnitScope_Input) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!BeamerAuv2IsInputElement(inst, element)) {
                return kAudioUnitErr_InvalidElement;
            }
            if (!inData || inDataSize < sizeof(AURenderCallbackStruct)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            pthread_mutex_lock(&inst->callbacksMutex);
            inst->callbacks.input[element] = *(AURenderCallbackStruct*)inData;
            // Clear connection when callback is set.
            inst->callbacks.inputConnection[element].sourceAU = NULL;
            pthread_mutex_unlock(&inst->callbacksMutex);
            return noErr;
        }

        case kAudioUnitProperty_MakeConnection: {
            if (scope != kAudioUnitScope_Input) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!BeamerAuv2IsInputElement(inst, element)) {
                return kAudioUnitErr_InvalidElement;
            }
            if (!inData || inDataSize < sizeof(AudioUnitConnection)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            const AudioUnitConnection* conn = (const AudioUnitConnection*)inData;
            pthread_mutex_lock(&inst->callbacksMutex);
            inst->callbacks.inputConnection[element].sourceAU = conn->sourceAudioUnit;
            inst->callbacks.inputConnection[element].sourceOutputNumber = conn->sourceOutputNumber;
            // Clear callback when connection is set.
            inst->callbacks.input[element].inputProc = NULL;
            inst->callbacks.input[element].inputProcRefCon = NULL;
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

            // The bus names it saved, the declared ones for the others.
            CFDictionaryRef elementNames = (CFDictionaryRef)CFDictionaryGetValue(dict,
                CFSTR(kAUPresetElementNameKey));
            if (elementNames && CFGetTypeID(elementNames) != CFDictionaryGetTypeID()) {
                elementNames = NULL;
            }
            BeamerAuv2RestoreNames(inst, elementNames);

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

        case kAudioUnitProperty_InPlaceProcessing: {
            if (scope != kAudioUnitScope_Global) {
                return kAudioUnitErr_InvalidScope;
            }
            if (!inData || inDataSize < sizeof(UInt32)) {
                return kAudioUnitErr_InvalidPropertyValue;
            }
            // Without a main input it stays off. Listeners hear of a change
            // only.
            bool inPlace = *(UInt32*)inData != 0 && BeamerAuv2DeclaredCount(inst, kAudioUnitScope_Input) > 0;
            bool before = atomic_exchange_explicit(&inst->processesInPlace, inPlace, memory_order_relaxed);
            if (before != inPlace) {
                NotifyPropertyListeners(inst, propID, scope, element);
            }
            return noErr;
        }

        case kAudioUnitProperty_ShouldAllocateBuffer:
            // Accept but ignore this
            return noErr;

        default:
            return kAudioUnitErr_InvalidProperty;
    }
}

// =============================================================================
// MARK: - Property Listeners
// =============================================================================

#ifdef BEAMER_DIAGNOSTICS
// The name of a scope for the diagnostics lines, written into `name`.
static const char* BeamerAuv2ScopeName(AudioUnitScope scope, char name[16]) {
    switch (scope) {
        case kAudioUnitScope_Global: return "global";
        case kAudioUnitScope_Input: return "input";
        case kAudioUnitScope_Output: return "output";
        default:
            snprintf(name, 16, "scope %u", (unsigned)scope);
            return name;
    }
}

// Whether a failing query is new for the instance, which then remembers
// it. Once the list is full, nothing more is new.
static bool BeamerAuv2NewFailingQuery(BeamerAuv2Instance* inst, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, OSStatus status) {
    UInt32 count = atomic_load_explicit(&inst->loggedQueryCount, memory_order_acquire);
    UInt32 capacity = sizeof(inst->loggedQueries) / sizeof(inst->loggedQueries[0]);
    for (UInt32 i = 0; i < count && i < capacity; i++) {
        if (inst->loggedQueries[i].propID == propID && inst->loggedQueries[i].scope == scope
            && inst->loggedQueries[i].element == element && inst->loggedQueries[i].status == status) {
            return false;
        }
    }
    if (count >= capacity) {
        return false;
    }
    inst->loggedQueries[count].propID = propID;
    inst->loggedQueries[count].scope = scope;
    inst->loggedQueries[count].element = element;
    inst->loggedQueries[count].status = status;
    atomic_store_explicit(&inst->loggedQueryCount, count + 1, memory_order_release);
    return true;
}

// Diagnostics builds log each property query that fails once, with the
// property id: a host may give up on a unit over one of them.
static OSStatus BeamerAuv2GetPropertyInfoNoted(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, UInt32* outDataSize, Boolean* outWritable) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    OSStatus status = BeamerAuv2GetPropertyInfo(self, propID, scope, element, outDataSize, outWritable);
    if (status != noErr && BeamerAuv2NewFailingQuery(inst, propID, scope, element, status)) {
        char text[128];
        char scopeText[16];
        snprintf(text, sizeof(text), "host asks for property %u on %s %u: status %d",
            (unsigned)propID, BeamerAuv2ScopeName(scope, scopeText), (unsigned)element, (int)status);
        beamer_au_diagnostics_note(inst->rustInstance, text);
    }
    return status;
}

static OSStatus BeamerAuv2GetPropertyNoted(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, void* outData, UInt32* ioDataSize) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    OSStatus status = BeamerAuv2GetProperty(self, propID, scope, element, outData, ioDataSize);
    if (status != noErr && BeamerAuv2NewFailingQuery(inst, propID, scope, element, status)) {
        char text[128];
        char scopeText[16];
        snprintf(text, sizeof(text), "host asks for property %u on %s %u: status %d",
            (unsigned)propID, BeamerAuv2ScopeName(scope, scopeText), (unsigned)element, (int)status);
        beamer_au_diagnostics_note(inst->rustInstance, text);
    }
    return status;
}

// Diagnostics builds log Initialize and the host's bus setup calls with
// their results.
static OSStatus BeamerAuv2InitializeNoted(void* self) {
    OSStatus status = BeamerAuv2Initialize(self);
    char text[64];
    snprintf(text, sizeof(text), "host initializes (status %d)", (int)status);
    beamer_au_diagnostics_note(((BeamerAuv2Instance*)self)->rustInstance, text);
    return status;
}

static OSStatus BeamerAuv2SetPropertyNoted(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, const void* inData, UInt32 inDataSize) {
    OSStatus status = BeamerAuv2SetProperty(self, propID, scope, element, inData, inDataSize);
    char scopeText[16];
    const char* scopeName = BeamerAuv2ScopeName(scope, scopeText);
    bool hasCount = (inData != NULL && inDataSize >= sizeof(UInt32));
    char text[192];
    switch (propID) {
        case kAudioUnitProperty_StreamFormat: {
            if (inData == NULL || inDataSize < sizeof(AudioStreamBasicDescription)) {
                return status;
            }
            const AudioStreamBasicDescription* format = (const AudioStreamBasicDescription*)inData;
            snprintf(text, sizeof(text), "host sets the format of %s %u: %u ch, %.0f Hz (status %d)",
                scopeName, (unsigned)element, (unsigned)format->mChannelsPerFrame, format->mSampleRate, (int)status);
            break;
        }
        case kAudioUnitProperty_ElementCount:
            snprintf(text, sizeof(text), "host sets the %s element count to %u (status %d)",
                scopeName, hasCount ? (unsigned)*(const UInt32*)inData : 0, (int)status);
            break;
        case kAudioUnitProperty_AudioChannelLayout:
            snprintf(text, sizeof(text), "host sets the channel layout of %s %u (status %d)",
                scopeName, (unsigned)element, (int)status);
            break;
        case kAudioUnitProperty_InPlaceProcessing:
            snprintf(text, sizeof(text), "host sets in-place processing %s (status %d)",
                (hasCount && *(const UInt32*)inData != 0) ? "on" : "off", (int)status);
            break;
        case kAudioUnitProperty_MakeConnection:
            snprintf(text, sizeof(text), "host sets a connection on %s %u (status %d)",
                scopeName, (unsigned)element, (int)status);
            break;
        case kAudioUnitProperty_SetRenderCallback:
            snprintf(text, sizeof(text), "host sets the render callback of %s %u (status %d)",
                scopeName, (unsigned)element, (int)status);
            break;
        case kAudioUnitProperty_MaximumFramesPerSlice:
            snprintf(text, sizeof(text), "host sets max frames to %u (status %d)",
                hasCount ? (unsigned)*(const UInt32*)inData : 0, (int)status);
            break;
        case kAudioUnitProperty_ElementName: {
            char name[64] = "";
            CFStringRef string = (inData != NULL && inDataSize >= sizeof(CFStringRef))
                ? *(const CFStringRef*)inData : NULL;
            if (string != NULL && CFGetTypeID(string) == CFStringGetTypeID()) {
                CFStringGetCString(string, name, sizeof(name), kCFStringEncodingUTF8);
            }
            snprintf(text, sizeof(text), "host names %s %u \"%s\" (status %d)",
                scopeName, (unsigned)element, name, (int)status);
            break;
        }
        default:
            return status;
    }
    beamer_au_diagnostics_note(((BeamerAuv2Instance*)self)->rustInstance, text);
    return status;
}
#endif

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
    // the push. The change would then wait for the next Initialize: apply
    // it now. Without render calls, nothing holds the mutex for long. The
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

// Drop the queued MIDI input, SysEx included. The caller holds
// queueReaderMutex.
static void BeamerAuv2DiscardMidi(BeamerAuv2Instance* inst) {
    EventQueueRelease(&inst->midiQueue, EventQueueReadyCount(&inst->midiQueue));
    SysExQueueRelease(&inst->sysexQueue, SysExQueueReadyEnd(&inst->sysexQueue, NULL, NULL));
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
    output->listBytes = inst->midiOutputListBytes;
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

    MIDIPacket* next = MIDIPacketListAdd(output->list, output->listBytes,
        output->packet, sampleOffset, length, bytes);
    if (next == NULL) {
        // The list is full: pass it on and retry in an empty one. A message
        // that does not fit there either is dropped.
        BeamerAuv2FlushMidiOutput(output);
        next = MIDIPacketListAdd(output->list, output->listBytes,
            output->packet, sampleOffset, length, bytes);
        if (next == NULL) {
            return;
        }
    }
    output->packet = next;
}

// The SysEx events of a render call, linked in order.
typedef struct {
    // Sample time of the start of the buffer, where the messages play.
    AUEventSampleTime start;
    AURenderEvent* first;
    AURenderEvent* last;
} BeamerAuv2SysExLink;

// Link a queued SysEx message into the event list of a render call.
// `context` is a BeamerAuv2SysExLink.
static void BeamerAuv2LinkSysEx(void* context, AUMIDIEvent* event) {
    BeamerAuv2SysExLink* link = (BeamerAuv2SysExLink*)context;
    AURenderEvent* renderEvent = (AURenderEvent*)event;
    event->next = NULL;
    event->eventSampleTime = link->start;
    if (link->last) {
        link->last->head.next = renderEvent;
    } else {
        link->first = renderEvent;
    }
    link->last = renderEvent;
}

// The queued events of a render call, linked into one list.
typedef struct {
    const AURenderEvent* list;
    UInt32 midiCount;
    UInt32 parameterCount;
    uint64_t sysexEnd;
} BeamerAuv2QueuedEvents;

// Take the queued events and link them into one list: the SysEx messages,
// the MIDI events, then (with `parameters`) the parameter changes the host
// scheduled for this buffer. The queues hold offsets into this buffer (a
// SysEx message plays at its start). Render events carry a sample time on
// the timeline of the render timestamp, so the buffer start is added. The
// caller holds queueReaderMutex and releases what it took.
static BeamerAuv2QueuedEvents BeamerAuv2LinkQueuedEvents(BeamerAuv2Instance* inst,
    AUEventSampleTime bufferStart, bool parameters) {
    BeamerAuv2QueuedEvents queued = { .list = NULL, .midiCount = 0, .parameterCount = 0, .sysexEnd = 0 };
    BeamerAuv2SysExLink sysexLink = { .start = bufferStart, .last = NULL, .first = NULL };
    queued.sysexEnd = SysExQueueReadyEnd(&inst->sysexQueue, BeamerAuv2LinkSysEx, &sysexLink);
    queued.midiCount = EventQueueReadyCount(&inst->midiQueue);
    queued.parameterCount = parameters ? EventQueueReadyCount(&inst->parameterQueue) : 0;
    queued.list = sysexLink.first;
    AURenderEvent* prev = sysexLink.last;
    for (UInt32 i = 0; i < queued.midiCount + queued.parameterCount; i++) {
        AURenderEvent* ev = (i < queued.midiCount)
            ? EventQueueAt(&inst->midiQueue, i)
            : EventQueueAt(&inst->parameterQueue, i - queued.midiCount);
        ev->head.next = NULL;
        ev->head.eventSampleTime += bufferStart;
        if (prev) prev->head.next = ev;
        else queued.list = ev;
        prev = ev;
    }
    return queued;
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

// Pull input element `element` into `list` from the callback or connection
// the host set for it. Returns kAudioUnitErr_NoConnection without one.
static OSStatus BeamerAuv2PullElement(BeamerAuv2Instance* inst, AudioUnitRenderActionFlags* flags,
    const AudioTimeStamp* timeStamp, UInt32 frames, UInt32 element, AudioBufferList* list) {

    if (element >= BEAMER_AU_MAX_BUSES) {
        return kAudioUnitErr_InvalidElement;
    }
    const CallbackSet* callbacks = &inst->renderCallbacks;
    if (callbacks->input[element].inputProc) {
        return callbacks->input[element].inputProc(callbacks->input[element].inputProcRefCon,
            flags, timeStamp, element, frames, list);
    }
    if (callbacks->inputConnection[element].sourceAU) {
        return AudioUnitRender(callbacks->inputConnection[element].sourceAU,
            flags, timeStamp, callbacks->inputConnection[element].sourceOutputNumber,
            frames, list);
    }
    return kAudioUnitErr_NoConnection;
}

// Whether every buffer of `list` is one channel with memory for `byteSize`
// bytes.
static bool BeamerAuv2HasMemory(const AudioBufferList* list, UInt32 byteSize) {
    for (UInt32 i = 0; i < list->mNumberBuffers; i++) {
        const AudioBuffer* buffer = &list->mBuffers[i];
        if (buffer->mData == NULL || buffer->mNumberChannels != 1 || buffer->mDataByteSize < byteSize) {
            return false;
        }
    }
    return true;
}

// Pull the main input through the callback or the connection the host set,
// where the plugin processes it. Returns NULL when there is no input source
// or the pull failed.
//
// In place (kAudioUnitProperty_InPlaceProcessing on) and with matching
// channel counts, the input goes straight into the host's output buffers
// when they have memory. Otherwise it goes into our own buffers; in place
// an output buffer without memory gets our input buffer's. When the
// input callback replaced our buffers with its own, the samples are copied
// into ours before an output points at them: the plugin never writes into
// memory the host gave as input.
static AudioBufferList* BeamerAuv2PullInput(BeamerAuv2Instance* inst, const AudioTimeStamp* timeStamp,
    UInt32 frames, AudioBufferList* ioData) {

    const CallbackSet* callbacks = &inst->renderCallbacks;
    if (!callbacks->input[0].inputProc && !callbacks->inputConnection[0].sourceAU) {
        return NULL;
    }

    if (EnsureInputBufferList(inst, inst->inputFormats[0].mChannelsPerFrame, frames) != noErr) {
        return NULL;
    }
    AudioBufferList* input = inst->inputBufferList;
    UInt32 byteSize = frames * BeamerAuv2SampleBytes(&inst->inputFormats[0]);
    bool inPlace = atomic_load_explicit(&inst->processesInPlace, memory_order_relaxed)
        && ioData != NULL && ioData->mNumberBuffers == input->mNumberBuffers;
    bool intoOutput = inPlace && BeamerAuv2HasMemory(ioData, byteSize);
    if (intoOutput) {
        for (UInt32 i = 0; i < input->mNumberBuffers; i++) {
            input->mBuffers[i].mData = ioData->mBuffers[i].mData;
        }
    }

    AudioUnitRenderActionFlags pullFlags = 0;
    if (BeamerAuv2PullElement(inst, &pullFlags, timeStamp, frames, 0, input) != noErr) {
        return NULL;
    }

    if (inPlace && !intoOutput) {
        for (UInt32 i = 0; i < input->mNumberBuffers; i++) {
            AudioBuffer* out = &ioData->mBuffers[i];
            if (out->mData != NULL) {
                continue;
            }
            AudioBuffer* in = &input->mBuffers[i];
            void* owned = inst->ownedInputBuffers[i];
            if (in->mData != owned) {
                if (in->mData != NULL) {
                    memcpy(owned, in->mData, (in->mDataByteSize < byteSize) ? in->mDataByteSize : byteSize);
                } else {
                    memset(owned, 0, byteSize);
                }
                in->mData = owned;
                in->mDataByteSize = byteSize;
            }
            out->mData = owned;
            out->mDataByteSize = byteSize;
        }
    }
    return input;
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

    if (inOutputBusNumber >= inst->outputElementCount) {
        return kAudioUnitErr_InvalidElement;
    }

    // The render notifications of this call. The render thread does not
    // wait for the lock: when a host thread is changing the list right now,
    // this call goes without notifications. Holding the lock until the
    // post-render notifications keeps each pre-render one paired with its
    // post-render one. A removed notification is not called after
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

#ifdef BEAMER_DIAGNOSTICS
    // Whether the host gave output memory, before the pull points any
    // output at ours.
    if (inOutputBusNumber == 0) {
        beamer_au_diagnostics_host_output_memory(inst->rustInstance, beamer_au_diagnostics_has_memory(ioData));
    }
#endif

    // Bypass (a plugin without a bypass parameter): the input goes to the
    // output, delayed by the plugin's latency.
    if (atomic_load_explicit(&inst->bypassed, memory_order_relaxed)) {
        // Only the main bus passes its input on; another output bus is
        // silent. In place the input may already be in the output buffers.
        AudioBufferList* inputData = (inOutputBusNumber == 0)
            ? BeamerAuv2PullInput(inst, inTimeStamp, inNumberFrames, ioData)
            : NULL;

        // Copy input to output for bypass
        if (inputData) {
            UInt32 buffersToCopy = (inputData->mNumberBuffers < ioData->mNumberBuffers) ?
                inputData->mNumberBuffers : ioData->mNumberBuffers;
            for (UInt32 i = 0; i < buffersToCopy; i++) {
                void* out = ioData->mBuffers[i].mData;
                void* in = inputData->mBuffers[i].mData;
                // An output buffer without memory (left so with in-place
                // processing off) gets the input's.
                if (out == NULL && in != NULL) {
                    ioData->mBuffers[i].mData = in;
                    ioData->mBuffers[i].mDataByteSize = inputData->mBuffers[i].mDataByteSize;
                    continue;
                }
                if (out == NULL || in == NULL || out == in) {
                    continue;
                }
                UInt32 bytesToCopy = (inputData->mBuffers[i].mDataByteSize < ioData->mBuffers[i].mDataByteSize) ?
                    inputData->mBuffers[i].mDataByteSize : ioData->mBuffers[i].mDataByteSize;
                memcpy(out, in, bytesToCopy);
            }
            // Output buffers beyond the input's channels are silent.
            for (UInt32 i = buffersToCopy; i < ioData->mNumberBuffers; i++) {
                if (ioData->mBuffers[i].mData != NULL) {
                    memset(ioData->mBuffers[i].mData, 0, ioData->mBuffers[i].mDataByteSize);
                }
            }
        } else {
            // No input, silence output
            for (UInt32 i = 0; i < ioData->mNumberBuffers; i++) {
                if (ioData->mBuffers[i].mData != NULL) {
                    memset(ioData->mBuffers[i].mData, 0, ioData->mBuffers[i].mDataByteSize);
                }
            }
        }

        // An output buffer still without memory (no input for it) gets
        // silence of our own.
        UInt32 formatBus = (inOutputBusNumber < BEAMER_AU_MAX_BUSES) ? inOutputBusNumber : 0;
        UInt32 silenceBytes = inNumberFrames * BeamerAuv2SampleBytes(&inst->outputFormats[formatBus]);
        bool silenceZeroed = false;
        for (UInt32 i = 0; i < ioData->mNumberBuffers; i++) {
            if (ioData->mBuffers[i].mData == NULL && inst->bypassSilence != NULL
                && inNumberFrames <= inst->bypassSilenceFrames) {
                if (!silenceZeroed) {
                    memset(inst->bypassSilence, 0, silenceBytes);
                    silenceZeroed = true;
                }
                ioData->mBuffers[i].mData = inst->bypassSilence;
                ioData->mBuffers[i].mDataByteSize = silenceBytes;
            }
        }

        // The input comes out as late as the plugin's output would.
        if (inOutputBusNumber == 0) {
            beamer_au_delay_bypassed_output(inst->rustInstance, inputData, ioData, inNumberFrames);
        }

#ifdef BEAMER_DIAGNOSTICS
        if (inOutputBusNumber == 0) {
            beamer_au_diagnostics_bypassed_render(inst->rustInstance, ioData, inNumberFrames);
        }
#endif

        // Consume the pending MIDI input, so that it does not play when bypass
        // ends. Rust reads it as in normal processing, so that a message
        // spanning the bypass stays whole; a bypassed MIDI effect passes it
        // on. When a reset holds the queues right now, the events wait for
        // the next call.
        if (inOutputBusNumber == 0 && pthread_mutex_trylock(&inst->queueReaderMutex) == 0) {
            BeamerAuv2MidiOutput thru;
            bool passesMidi = inst->isMidiEffect && BeamerAuv2BeginMidiOutput(inst, inTimeStamp, &thru);
            BeamerAuv2QueuedEvents queued = BeamerAuv2LinkQueuedEvents(inst,
                (AUEventSampleTime)inTimeStamp->mSampleTime, false);
            bool read = beamer_au_pass_midi_through(inst->rustInstance, inTimeStamp, inNumberFrames,
                queued.list, passesMidi ? BeamerAuv2SendMidi : NULL, passesMidi ? &thru : NULL);
            if (read) {
                EventQueueRelease(&inst->midiQueue, queued.midiCount);
                SysExQueueRelease(&inst->sysexQueue, queued.sysexEnd);
            } else {
                // Unread, the events wait for the next call at their offsets
                // into its buffer, as for a refused render call.
                for (UInt32 i = 0; i < queued.midiCount; i++) {
                    EventQueueAt(&inst->midiQueue, i)->head.eventSampleTime -= (AUEventSampleTime)inTimeStamp->mSampleTime;
                }
            }
            beamer_au_note_sysex_input_dropped(inst->rustInstance,
                atomic_exchange_explicit(&inst->sysexQueue.dropped, 0, memory_order_relaxed));

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

    // Pull input audio. Only the render of the main bus processes it; a
    // call for another output bus copies what that render produced.
    AudioBufferList* inputData = NULL;
    if (inOutputBusNumber == 0 && beamer_au_get_input_bus_count(inst->rustInstance) > 0) {
        inputData = BeamerAuv2PullInput(inst, inTimeStamp, inNumberFrames, ioData);
    }

    // Only the render of the main bus processes audio and needs the
    // transport. It is read before the queues are taken, so that the host
    // callbacks do not run while a reset waits for them.
    BeamerAuTransport transport;
    bool hasTransport = (inOutputBusNumber == 0);
    if (hasTransport) {
        BeamerAuv2ReadTransport(&inst->renderCallbacks.host, &transport);
    }

    // Take the queued events, linked into one list with the parameter
    // changes. Only the render of the main bus takes events: a call for
    // another output bus leaves them for it, so that none are lost. When a
    // reset holds the queues right now, the events wait for the next call.
    AUEventSampleTime bufferStart = (AUEventSampleTime)inTimeStamp->mSampleTime;
    BeamerAuv2QueuedEvents queued = { .list = NULL, .midiCount = 0, .parameterCount = 0, .sysexEnd = 0 };
    bool readsQueues = (inOutputBusNumber == 0)
        && pthread_mutex_trylock(&inst->queueReaderMutex) == 0;
    if (readsQueues) {
        queued = BeamerAuv2LinkQueuedEvents(inst, bufferStart, true);
    }
    const AURenderEvent* eventList = queued.list;
    UInt32 midiCount = queued.midiCount;
    UInt32 parameterCount = queued.parameterCount;
    uint64_t sysexEnd = queued.sysexEnd;

    // The plugin's MIDI output is collected in a packet list during the call.
    BeamerAuv2MidiOutput midiOutput;
    bool sendsMidi = (inOutputBusNumber == 0)
        && BeamerAuv2BeginMidiOutput(inst, inTimeStamp, &midiOutput);

    // The aux inputs (sidechains) are pulled by the plugin's render through
    // this block, from the callback or connection the host set for each
    // element. It lives on the stack for this call only; unretained, so that
    // it is not copied to the heap on the render thread.
    __unsafe_unretained AURenderPullInputBlock pullAuxInput = ^AUAudioUnitStatus(AudioUnitRenderActionFlags* flags,
        const AudioTimeStamp* timestamp, AUAudioFrameCount frameCount, NSInteger bus,
        AudioBufferList* list) {
        if (bus < 1) {
            return kAudioUnitErr_InvalidElement;
        }
        return BeamerAuv2PullElement(inst, flags, timestamp, frameCount, (UInt32)bus, list);
    };

    // Call Rust render function
    OSStatus status = beamer_au_render(
        inst->rustInstance,
        ioActionFlags,
        inTimeStamp,
        inNumberFrames,
        inOutputBusNumber,
        ioData,
        eventList,
        pullAuxInput,
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
            SysExQueueRelease(&inst->sysexQueue, sysexEnd);
        }
        pthread_mutex_unlock(&inst->queueReaderMutex);
        beamer_au_note_sysex_input_dropped(inst->rustInstance,
            atomic_exchange_explicit(&inst->sysexQueue.dropped, 0, memory_order_relaxed));
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

// A SysEx message, F0 to F7, for the start of the next render call: the
// call has no sample offset. Any thread may call it, the render thread
// included. A message that does not fit (also one of more than 65,535
// bytes) or an empty one is dropped and counted; the call still succeeds,
// so that the host keeps sending. A plugin without MIDI input has no queue
// and ignores the call, as Rust ignores its other MIDI.
static OSStatus BeamerAuv2SysEx(void* self, const UInt8* inData, UInt32 inLength) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    if (inData == NULL && inLength > 0) {
        return kAudio_ParamError;
    }
    if (!inst->sysexQueue.bytes) {
        return noErr;
    }
    if (!SysExQueuePush(&inst->sysexQueue, inData, inLength)) {
        atomic_fetch_add_explicit(&inst->sysexQueue.dropped, 1, memory_order_relaxed);
    }
    return noErr;
}

// Put the packed words of one offset into the MIDI queue as an event list
// render event of one packet. A full queue drops it, as MIDIEvent does.
static void BeamerAuv2QueueEventListSlot(BeamerAuv2Instance* inst, AURenderEvent* slot) {
    if (slot->MIDIEventsList.eventList.packet[0].wordCount > 0) {
        EventQueuePush(&inst->midiQueue, slot);
    }
}

// A list of Universal MIDI Packets for the next render call. Any thread may
// call it, the render thread included. A packet plays at
// inOffsetSampleFrame plus its time stamp, which go into the event's sample
// time; the packet's own time stamp is 0, so that render does not add it
// again (render takes the time of an event list event plus the time stamp
// of each packet, as an AUv3 gets them). The messages of packets at one
// offset share a queue slot, as many as its packet holds (64 words); a
// message is never split. The protocol of the list goes with each slot. A
// plugin without MIDI input ignores the call, as Rust ignores its other
// MIDI. AudioToolbox passes a host's list on one packet per call, with
// packets of more than 64 words split and messages cut short dropped, so
// the checks here are for a call that does not pass through it.
static OSStatus BeamerAuv2MIDIEventList(void* self, UInt32 inOffsetSampleFrame, const MIDIEventList* evtList) {
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    if (evtList == NULL) {
        return kAudio_ParamError;
    }
    if (inst->midiProtocol == 0) {
        return noErr;
    }

    AURenderEvent slot;
    memset(&slot, 0, sizeof(slot));
    slot.MIDIEventsList.eventType = AURenderEventMIDIEventList;
    slot.MIDIEventsList.eventList.protocol = evtList->protocol;
    slot.MIDIEventsList.eventList.numPackets = 1;
    MIDIEventPacket* slotPacket = &slot.MIDIEventsList.eventList.packet[0];
    const UInt32 slotWords = sizeof(slotPacket->words) / sizeof(slotPacket->words[0]);

    const MIDIEventPacket* packet = &evtList->packet[0];
    for (UInt32 p = 0; p < evtList->numPackets; p++) {
        // A packet holds at most 64 words: a larger count is a damaged
        // list, whose next packet cannot be found.
        if (packet->wordCount > slotWords) {
            break;
        }
#ifdef BEAMER_DIAGNOSTICS
        beamer_au_diagnostics_host_packet_time_stamp(inst->rustInstance, packet->timeStamp);
#endif
        // A time stamp that would put the packet past any buffer (a host
        // that stamps packets with another clock) is not used.
        UInt64 offset = (UInt64)inOffsetSampleFrame + packet->timeStamp;
        AUEventSampleTime sampleTime = (packet->timeStamp < inst->maxFramesPerSlice && offset < inst->maxFramesPerSlice)
            ? (AUEventSampleTime)offset : (AUEventSampleTime)inOffsetSampleFrame;
        if (slotPacket->wordCount > 0 && slot.MIDIEventsList.eventSampleTime != sampleTime) {
            BeamerAuv2QueueEventListSlot(inst, &slot);
            slotPacket->wordCount = 0;
        }
        slot.MIDIEventsList.eventSampleTime = sampleTime;

        // Copy whole messages; a message that does not fit starts a new
        // slot, one cut short by the end of the packet is left out.
        UInt32 i = 0;
        while (i < packet->wordCount) {
            UInt32 type = packet->words[i] >> 28;
            UInt32 size = (type <= 2 || type == 6 || type == 7) ? 1
                : (type <= 4 || (type >= 8 && type <= 10)) ? 2
                : (type <= 12) ? 3 : 4;
            if (i + size > packet->wordCount) {
                break;
            }
            if (slotPacket->wordCount + size > slotWords) {
                BeamerAuv2QueueEventListSlot(inst, &slot);
                slotPacket->wordCount = 0;
            }
            memcpy(&slotPacket->words[slotPacket->wordCount], &packet->words[i], size * sizeof(UInt32));
            slotPacket->wordCount += size;
            i += size;
        }
        packet = MIDIEventPacketNext(packet);
    }
    BeamerAuv2QueueEventListSlot(inst, &slot);
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
// MARK: - CocoaUI Editor View
// =============================================================================

// =============================================================================
// MARK: - AUv2 WebView IPC Callbacks
// =============================================================================

static void beamer_auv2_on_message(void* context, const uint8_t* json, size_t len) {
    {{COCOA_EDITOR_VIEW_CLASS}}* self = (__bridge {{COCOA_EDITOR_VIEW_CLASS}}*)context;
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
    {{COCOA_EDITOR_VIEW_CLASS}}* self = (__bridge {{COCOA_EDITOR_VIEW_CLASS}}*)context;
    beamer_au_ipc_send_init_dump(self->_sync, self->_webviewHandle, "auv2");
}

@implementation {{COCOA_EDITOR_VIEW_CLASS}}
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
        // The first frame sends every parameter. Events the plugin emits
        // from now on are kept for this editor.
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

    if (!beamer_au_has_editor(rustInstance)) {
        return nil;
    }

    uint32_t width = 0, height = 0;
    beamer_au_get_editor_size(rustInstance, &width, &height);
    NSSize viewSize = NSMakeSize(width, height);

    // Create a temporary container to attach the WebView to
    NSView* container = [[NSView alloc] initWithFrame:NSMakeRect(0, 0, viewSize.width, viewSize.height)];
    if (container == nil) {
        return nil;
    }

    uint8_t bgColor[4];
    beamer_au_get_editor_background_color(bgColor);

    const char* devUrl = beamer_au_get_webview_url(rustInstance);
    void* webviewHandle;
    // Create the editor view that will be returned to the host.
    // __bridge (no retain) is safe for the callback context because the host
    // retains this view for the editor lifetime. -dealloc destroys the
    // WebView before the view is freed, so the context cannot dangle.
    {{COCOA_EDITOR_VIEW_CLASS}}* editorView = [[{{COCOA_EDITOR_VIEW_CLASS}} alloc]
        initWithFrame:NSMakeRect(0, 0, viewSize.width, viewSize.height)
        webviewHandle:NULL
         rustInstance:rustInstance
            audioUnit:audioUnit];

    // Create WebView via beamer-webview C-ABI (shared platform layer).
    if (devUrl != NULL) {
        webviewHandle = beamer_webview_create_url_with_ipc(
            (__bridge void*)container, devUrl, bgColor,
            beamer_auv2_on_message, beamer_auv2_on_loaded,
            (__bridge void*)editorView);
    } else {
        const void* assets = beamer_au_get_webview_assets();
        webviewHandle = beamer_webview_create_with_ipc(
            (__bridge void*)container, assets, bgColor,
            beamer_auv2_on_message, beamer_auv2_on_loaded,
            (__bridge void*)editorView);
    }
    if (webviewHandle == NULL) {
        return nil;
    }

    // Store the webview handle in the editor view
    editorView->_webviewHandle = webviewHandle;

    // Re-parent the WebView's container into the editor view
    [container setFrame:editorView.bounds];
    [container setAutoresizingMask:NSViewWidthSizable | NSViewHeightSizable];
    [editorView addSubview:container];

    return editorView;
}
@end
