// AUv2 C code generation template
// This file is included by auv2.rs via include!()

fn generate_auv2_wrapper_source(plugin_name: &str) -> String {
    let pascal_name = to_pascal_case(plugin_name);
    let factory_name = format!("Beamer{}Factory", pascal_name);

    format!(
        r#"
// =============================================================================
// AUv2 Plugin Implementation for {plugin_name}
// =============================================================================
// Auto-generated proper AUv2 implementation that returns AudioComponentPlugInInterface*.
// All plugin logic is handled by the beamer_au_* bridge functions.

#include <AudioToolbox/AudioToolbox.h>
#include <AudioUnit/AudioUnit.h>
#include <CoreFoundation/CoreFoundation.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>

// Include the bridge header for Rust plugin access
#include "BeamerAuBridge.h"

// =============================================================================
// MARK: - Constants
// =============================================================================

#define MAX_PROPERTY_LISTENERS 64
#define MAX_RENDER_NOTIFY 32

// =============================================================================
// MARK: - Data Structures
// =============================================================================

typedef struct {{
    AudioUnitPropertyID propID;
    AudioUnitPropertyListenerProc proc;
    void* userData;
}} PropertyListener;

typedef struct {{
    AURenderCallback proc;
    void* userData;
}} RenderNotify;

typedef struct {{
    AudioUnit sourceAU;
    UInt32 sourceOutputNumber;
}} InputConnection;

typedef struct BeamerAuv2Instance {{
    // AudioComponentPlugInInterface MUST be first (ABI requirement)
    AudioComponentPlugInInterface interface;
    AudioComponentInstance componentInstance;
    BeamerAuInstanceHandle rustInstance;

    // Audio configuration
    Float64 sampleRate;
    UInt32 maxFramesPerSlice;
    bool initialized;
    bool bypassed;

    // Stream formats for input/output scope, element 0
    AudioStreamBasicDescription inputFormat;
    AudioStreamBasicDescription outputFormat;

    // Input handling - either callback or connection
    AURenderCallbackStruct inputCallback;
    InputConnection inputConnection;

    // Allocated input buffer for pulling
    AudioBufferList* inputBufferList;
    UInt32 inputBufferCapacity;

    // Property listeners
    PropertyListener propertyListeners[MAX_PROPERTY_LISTENERS];
    UInt32 propertyListenerCount;
    pthread_mutex_t listenerMutex;

    // Render notifications
    RenderNotify renderNotify[MAX_RENDER_NOTIFY];
    UInt32 renderNotifyCount;
    pthread_mutex_t renderNotifyMutex;

    // Host callbacks (for tempo, transport, etc.)
    HostCallbackInfo hostCallbacks;

    // Factory presets
    CFArrayRef factoryPresets;         // CFArray of AUPreset pointers (NULL callbacks)
    AUPreset* presetStorage;           // Backing storage for preset structs
    uint32_t presetCount;              // Number of factory presets
    int32_t currentPresetIndex;        // -1 = no preset, >=0 = factory preset index
}} BeamerAuv2Instance;

// =============================================================================
// MARK: - Forward Declarations
// =============================================================================

static OSStatus BeamerAuv2Open(void* self, AudioComponentInstance ci);
static OSStatus BeamerAuv2Close(void* self);
static AudioComponentMethod BeamerAuv2Lookup(SInt16 selector);

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

// =============================================================================
// MARK: - Helper Functions
// =============================================================================

static void InitDefaultFormat(AudioStreamBasicDescription* format, Float64 sampleRate, UInt32 channels) {{
    memset(format, 0, sizeof(AudioStreamBasicDescription));
    format->mSampleRate = sampleRate;
    format->mFormatID = kAudioFormatLinearPCM;
    format->mFormatFlags = kAudioFormatFlagsNativeFloatPacked | kAudioFormatFlagIsNonInterleaved;
    format->mBytesPerPacket = sizeof(Float32);
    format->mFramesPerPacket = 1;
    format->mBytesPerFrame = sizeof(Float32);
    format->mChannelsPerFrame = channels;
    format->mBitsPerChannel = 32;
}}

static void NotifyPropertyListeners(BeamerAuv2Instance* inst, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element) {{
    pthread_mutex_lock(&inst->listenerMutex);
    for (UInt32 i = 0; i < inst->propertyListenerCount; i++) {{
        if (inst->propertyListeners[i].propID == propID) {{
            inst->propertyListeners[i].proc(
                inst->propertyListeners[i].userData,
                inst->componentInstance,
                propID, scope, element);
        }}
    }}
    pthread_mutex_unlock(&inst->listenerMutex);
}}

static OSStatus EnsureInputBufferList(BeamerAuv2Instance* inst, UInt32 channels, UInt32 frames) {{
    UInt32 neededCapacity = frames * channels;
    if (inst->inputBufferList && inst->inputBufferCapacity >= neededCapacity) {{
        // Existing buffer is large enough, just update frame counts
        for (UInt32 i = 0; i < inst->inputBufferList->mNumberBuffers; i++) {{
            inst->inputBufferList->mBuffers[i].mDataByteSize = frames * sizeof(Float32);
        }}
        return noErr;
    }}

    // Free old buffer if it exists
    if (inst->inputBufferList) {{
        for (UInt32 i = 0; i < inst->inputBufferList->mNumberBuffers; i++) {{
            if (inst->inputBufferList->mBuffers[i].mData) {{
                free(inst->inputBufferList->mBuffers[i].mData);
            }}
        }}
        free(inst->inputBufferList);
    }}

    // Allocate new buffer list (non-interleaved: one buffer per channel)
    size_t listSize = sizeof(AudioBufferList) + (channels > 0 ? (channels - 1) * sizeof(AudioBuffer) : 0);
    inst->inputBufferList = (AudioBufferList*)calloc(1, listSize);
    if (!inst->inputBufferList) return kAudio_MemFullError;

    inst->inputBufferList->mNumberBuffers = channels;
    for (UInt32 i = 0; i < channels; i++) {{
        inst->inputBufferList->mBuffers[i].mNumberChannels = 1;
        inst->inputBufferList->mBuffers[i].mDataByteSize = frames * sizeof(Float32);
        inst->inputBufferList->mBuffers[i].mData = calloc(frames, sizeof(Float32));
        if (!inst->inputBufferList->mBuffers[i].mData) return kAudio_MemFullError;
    }}

    inst->inputBufferCapacity = neededCapacity;
    return noErr;
}}

static void FreeInputBufferList(BeamerAuv2Instance* inst) {{
    if (inst->inputBufferList) {{
        for (UInt32 i = 0; i < inst->inputBufferList->mNumberBuffers; i++) {{
            if (inst->inputBufferList->mBuffers[i].mData) {{
                free(inst->inputBufferList->mBuffers[i].mData);
            }}
        }}
        free(inst->inputBufferList);
        inst->inputBufferList = NULL;
        inst->inputBufferCapacity = 0;
    }}
}}

// =============================================================================
// MARK: - Factory Function
// =============================================================================

__attribute__((visibility("default")))
void* {factory_name}(const AudioComponentDescription* inDesc) {{
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
    inst->initialized = false;
    inst->bypassed = false;

    // Initialize mutexes
    pthread_mutex_init(&inst->listenerMutex, NULL);
    pthread_mutex_init(&inst->renderNotifyMutex, NULL);

    return &inst->interface;
}}

// =============================================================================
// MARK: - Open/Close/Lookup
// =============================================================================

static OSStatus BeamerAuv2Open(void* self, AudioComponentInstance ci) {{
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    inst->componentInstance = ci;

    // Ensure Rust factory is registered
    if (!beamer_au_ensure_factory_registered()) {{
        return kAudioUnitErr_FailedInitialization;
    }}

    // Create Rust plugin instance
    inst->rustInstance = beamer_au_create_instance();
    if (!inst->rustInstance) {{
        return kAudioUnitErr_FailedInitialization;
    }}

    // Query bus configuration from Rust and set up default formats
    uint32_t inputChannels = beamer_au_get_input_bus_channel_count(inst->rustInstance, 0);
    uint32_t outputChannels = beamer_au_get_output_bus_channel_count(inst->rustInstance, 0);

    // Default to stereo if plugin reports 0 (shouldn't happen)
    if (inputChannels == 0 && beamer_au_get_input_bus_count(inst->rustInstance) > 0) inputChannels = 2;
    if (outputChannels == 0 && beamer_au_get_output_bus_count(inst->rustInstance) > 0) outputChannels = 2;

    InitDefaultFormat(&inst->inputFormat, inst->sampleRate, inputChannels);
    InitDefaultFormat(&inst->outputFormat, inst->sampleRate, outputChannels);

    // Build factory presets cache
    uint32_t presetCount = beamer_au_get_preset_count(inst->rustInstance);
    inst->presetCount = presetCount;
    inst->currentPresetIndex = -1;

    if (presetCount > 0) {{
        // Allocate backing storage for AUPreset structs
        inst->presetStorage = (AUPreset*)calloc(presetCount, sizeof(AUPreset));
        if (inst->presetStorage) {{
            // Initialize each preset from Rust
            for (uint32_t i = 0; i < presetCount; i++) {{
                BeamerAuPresetInfo info;
                memset(&info, 0, sizeof(info));
                if (beamer_au_get_preset_info(inst->rustInstance, i, &info)) {{
                    inst->presetStorage[i].presetNumber = (SInt32)info.number;
                    inst->presetStorage[i].presetName = CFStringCreateWithCString(
                        kCFAllocatorDefault, info.name, kCFStringEncodingUTF8);
                }}
            }}

            // Build CFArray with NULL callbacks (stores raw pointers to AUPreset)
            CFMutableArrayRef presets = CFArrayCreateMutable(kCFAllocatorDefault, presetCount, NULL);
            if (presets) {{
                for (uint32_t i = 0; i < presetCount; i++) {{
                    CFArrayAppendValue(presets, &inst->presetStorage[i]);
                }}
                inst->factoryPresets = presets;
            }} else {{
                inst->factoryPresets = NULL;
            }}
        }} else {{
            inst->factoryPresets = NULL;
            inst->presetCount = 0;
        }}
    }} else {{
        inst->factoryPresets = NULL;
        inst->presetStorage = NULL;
    }}

    return noErr;
}}

static OSStatus BeamerAuv2Close(void* self) {{
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    if (inst->initialized) {{
        beamer_au_deallocate_render_resources(inst->rustInstance);
        inst->initialized = false;
    }}

    if (inst->rustInstance) {{
        beamer_au_destroy_instance(inst->rustInstance);
        inst->rustInstance = NULL;
    }}

    FreeInputBufferList(inst);

    // Release factory presets
    if (inst->factoryPresets) {{
        CFRelease(inst->factoryPresets);
        inst->factoryPresets = NULL;
    }}

    // Free preset storage and release dynamically created CFStrings
    if (inst->presetStorage) {{
        for (uint32_t i = 0; i < inst->presetCount; i++) {{
            if (inst->presetStorage[i].presetName) {{
                CFRelease(inst->presetStorage[i].presetName);
            }}
        }}
        free(inst->presetStorage);
        inst->presetStorage = NULL;
    }}

    pthread_mutex_destroy(&inst->listenerMutex);
    pthread_mutex_destroy(&inst->renderNotifyMutex);

    free(inst);
    return noErr;
}}

static AudioComponentMethod BeamerAuv2Lookup(SInt16 selector) {{
    switch (selector) {{
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
        default:
            return NULL;
    }}
}}

// =============================================================================
// MARK: - Initialize/Uninitialize
// =============================================================================

static OSStatus BeamerAuv2Initialize(void* self) {{
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    if (inst->initialized) {{
        return noErr; // Already initialized
    }}

    // Build bus config from current stream formats
    BeamerAuBusConfig busConfig;
    memset(&busConfig, 0, sizeof(busConfig));

    uint32_t inputBusCount = beamer_au_get_input_bus_count(inst->rustInstance);
    uint32_t outputBusCount = beamer_au_get_output_bus_count(inst->rustInstance);

    busConfig.input_bus_count = inputBusCount;
    busConfig.output_bus_count = outputBusCount;

    uint32_t inputChannels = 0;
    uint32_t outputChannels = 0;

    if (inputBusCount > 0) {{
        inputChannels = inst->inputFormat.mChannelsPerFrame;
        busConfig.input_buses[0].channel_count = inputChannels;
        busConfig.input_buses[0].bus_type = BeamerAuBusTypeMain;
    }}
    if (outputBusCount > 0) {{
        outputChannels = inst->outputFormat.mChannelsPerFrame;
        busConfig.output_buses[0].channel_count = outputChannels;
        busConfig.output_buses[0].bus_type = BeamerAuBusTypeMain;
    }}

    // Validate channel configuration before proceeding
    bool configValid = beamer_au_is_channel_config_valid(inst->rustInstance, inputChannels, outputChannels);
    NSLog(@"AUv2 Initialize: input=%u output=%u valid=%d", inputChannels, outputChannels, configValid);
    if (!configValid) {{
        NSLog(@"AUv2 Initialize: Rejecting invalid channel config");
        return kAudioUnitErr_FormatNotSupported;
    }}

    // Determine sample format
    BeamerAuSampleFormat format = BeamerAuSampleFormatFloat32;
    if (inst->outputFormat.mBitsPerChannel == 64) {{
        format = BeamerAuSampleFormatFloat64;
    }}

    // Allocate render resources in Rust
    OSStatus status = beamer_au_allocate_render_resources(
        inst->rustInstance,
        inst->sampleRate,
        inst->maxFramesPerSlice,
        format,
        &busConfig
    );

    if (status == noErr) {{
        inst->initialized = true;

        // Pre-allocate input buffer if we have input buses
        if (inputBusCount > 0) {{
            EnsureInputBufferList(inst, inst->inputFormat.mChannelsPerFrame, inst->maxFramesPerSlice);
        }}
    }}

    return status;
}}

static OSStatus BeamerAuv2Uninitialize(void* self) {{
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    if (inst->initialized) {{
        beamer_au_deallocate_render_resources(inst->rustInstance);
        inst->initialized = false;
    }}

    return noErr;
}}

// =============================================================================
// MARK: - Property Handling
// =============================================================================

static OSStatus BeamerAuv2GetPropertyInfo(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, UInt32* outDataSize, Boolean* outWritable) {{

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    // Default to not writable
    if (outWritable) *outWritable = false;

    switch (propID) {{
        // Stream format
        case kAudioUnitProperty_StreamFormat:
            if (outDataSize) *outDataSize = sizeof(AudioStreamBasicDescription);
            if (outWritable) *outWritable = true;
            return noErr;

        // Sample rate
        case kAudioUnitProperty_SampleRate:
            if (outDataSize) *outDataSize = sizeof(Float64);
            if (outWritable) *outWritable = true;
            return noErr;

        // Maximum frames per slice
        case kAudioUnitProperty_MaximumFramesPerSlice:
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = true;
            return noErr;

        // Parameter list
        case kAudioUnitProperty_ParameterList:
            if (scope == kAudioUnitScope_Global && element == 0) {{
                uint32_t count = beamer_au_get_parameter_count(inst->rustInstance);
                if (outDataSize) *outDataSize = count * sizeof(AudioUnitParameterID);
                if (outWritable) *outWritable = false;
                return noErr;
            }}
            return kAudioUnitErr_InvalidScope;

        // Parameter info (element is param ID)
        case kAudioUnitProperty_ParameterInfo:
            if (scope == kAudioUnitScope_Global) {{
                if (outDataSize) *outDataSize = sizeof(AudioUnitParameterInfo);
                if (outWritable) *outWritable = false;
                return noErr;
            }}
            return kAudioUnitErr_InvalidScope;

        // Parameter value strings (for indexed params)
        case kAudioUnitProperty_ParameterValueStrings:
            if (scope == kAudioUnitScope_Global) {{
                uint32_t count = beamer_au_get_parameter_value_count(inst->rustInstance, element);
                if (count > 0) {{
                    if (outDataSize) *outDataSize = sizeof(CFArrayRef);
                    if (outWritable) *outWritable = false;
                    return noErr;
                }}
            }}
            return kAudioUnitErr_InvalidProperty;

        // Parameter string from value (convert value to display string)
        case kAudioUnitProperty_ParameterStringFromValue:
            if (scope == kAudioUnitScope_Global) {{
                if (outDataSize) *outDataSize = sizeof(AudioUnitParameterStringFromValue);
                if (outWritable) *outWritable = false;
                return noErr;
            }}
            return kAudioUnitErr_InvalidScope;

        // Parameter value from string (convert display string to value)
        case kAudioUnitProperty_ParameterValueFromString:
            if (scope == kAudioUnitScope_Global) {{
                if (outDataSize) *outDataSize = sizeof(AudioUnitParameterValueFromString);
                if (outWritable) *outWritable = true;
                return noErr;
            }}
            return kAudioUnitErr_InvalidScope;

        // Latency (Global scope only)
        case kAudioUnitProperty_Latency:
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (outDataSize) *outDataSize = sizeof(Float64);
            if (outWritable) *outWritable = false;
            return noErr;

        // Tail time (Global scope only)
        case kAudioUnitProperty_TailTime:
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (outDataSize) *outDataSize = sizeof(Float64);
            if (outWritable) *outWritable = false;
            return noErr;

        // Bypass (Global scope only)
        case kAudioUnitProperty_BypassEffect:
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (outDataSize) *outDataSize = sizeof(UInt32);
            if (outWritable) *outWritable = true;
            return noErr;

        // Present preset
        case kAudioUnitProperty_PresentPreset:
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (outDataSize) *outDataSize = sizeof(AUPreset);
            if (outWritable) *outWritable = true;
            return noErr;

        // Factory presets
        case kAudioUnitProperty_FactoryPresets:
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (outDataSize) *outDataSize = sizeof(CFArrayRef);
            if (outWritable) *outWritable = false;  // Factory presets are read-only
            return noErr;

        // Render callback (for setting input source)
        case kAudioUnitProperty_SetRenderCallback:
            if (scope == kAudioUnitScope_Input && element == 0) {{
                if (outDataSize) *outDataSize = sizeof(AURenderCallbackStruct);
                if (outWritable) *outWritable = true;
                return noErr;
            }}
            return kAudioUnitErr_InvalidScope;

        // Audio unit connection
        case kAudioUnitProperty_MakeConnection:
            if (scope == kAudioUnitScope_Input && element == 0) {{
                if (outDataSize) *outDataSize = sizeof(AudioUnitConnection);
                if (outWritable) *outWritable = true;
                return noErr;
            }}
            return kAudioUnitErr_InvalidScope;

        // Supported channel layouts
        case kAudioUnitProperty_SupportedNumChannels:
            if (scope == kAudioUnitScope_Global) {{
                BeamerAuChannelCapabilities caps;
                if (beamer_au_get_channel_capabilities(inst->rustInstance, &caps)) {{
                    if (outDataSize) *outDataSize = caps.count * sizeof(AUChannelInfo);
                    if (outWritable) *outWritable = false;
                    return noErr;
                }}
            }}
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

        // Offline render
        case kAudioUnitProperty_OfflineRender:
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

        default:
            return kAudioUnitErr_InvalidProperty;
    }}
}}

static OSStatus BeamerAuv2GetProperty(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, void* outData, UInt32* ioDataSize) {{

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    switch (propID) {{
        case kAudioUnitProperty_StreamFormat: {{
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioStreamBasicDescription)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            AudioStreamBasicDescription* desc = (AudioStreamBasicDescription*)outData;
            if (scope == kAudioUnitScope_Input) {{
                *desc = inst->inputFormat;
            }} else if (scope == kAudioUnitScope_Output) {{
                *desc = inst->outputFormat;
            }} else {{
                return kAudioUnitErr_InvalidScope;
            }}
            *ioDataSize = sizeof(AudioStreamBasicDescription);
            return noErr;
        }}

        case kAudioUnitProperty_SampleRate: {{
            if (!outData || !ioDataSize || *ioDataSize < sizeof(Float64)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            *(Float64*)outData = inst->sampleRate;
            *ioDataSize = sizeof(Float64);
            return noErr;
        }}

        case kAudioUnitProperty_MaximumFramesPerSlice: {{
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            *(UInt32*)outData = inst->maxFramesPerSlice;
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }}

        case kAudioUnitProperty_ParameterList: {{
            if (scope != kAudioUnitScope_Global || element != 0) {{
                return kAudioUnitErr_InvalidScope;
            }}
            uint32_t count = beamer_au_get_parameter_count(inst->rustInstance);
            UInt32 needed = count * sizeof(AudioUnitParameterID);
            if (!outData || !ioDataSize || *ioDataSize < needed) {{
                if (ioDataSize) *ioDataSize = needed;
                return outData ? kAudioUnitErr_InvalidPropertyValue : noErr;
            }}
            AudioUnitParameterID* ids = (AudioUnitParameterID*)outData;
            for (uint32_t i = 0; i < count; i++) {{
                BeamerAuParameterInfo info;
                if (beamer_au_get_parameter_info(inst->rustInstance, i, &info)) {{
                    ids[i] = info.id;
                }} else {{
                    ids[i] = 0;
                }}
            }}
            *ioDataSize = needed;
            return noErr;
        }}

        case kAudioUnitProperty_ParameterInfo: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioUnitParameterInfo)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            // element is the parameter ID - find it by iterating
            uint32_t count = beamer_au_get_parameter_count(inst->rustInstance);
            for (uint32_t i = 0; i < count; i++) {{
                BeamerAuParameterInfo bInfo;
                if (beamer_au_get_parameter_info(inst->rustInstance, i, &bInfo) && bInfo.id == element) {{
                    AudioUnitParameterInfo* auInfo = (AudioUnitParameterInfo*)outData;
                    memset(auInfo, 0, sizeof(AudioUnitParameterInfo));

                    // Copy name (CFString)
                    auInfo->cfNameString = CFStringCreateWithCString(NULL, bInfo.name, kCFStringEncodingUTF8);
                    auInfo->flags = kAudioUnitParameterFlag_HasCFNameString |
                                    kAudioUnitParameterFlag_IsReadable |
                                    kAudioUnitParameterFlag_IsWritable;

                    if (bInfo.flags & BeamerAuParameterFlagAutomatable) {{
                        auInfo->flags |= kAudioUnitParameterFlag_IsHighResolution;
                    }}

                    // Map unit type
                    auInfo->unit = bInfo.unit_type;

                    // Use actual value range from Rust
                    auInfo->minValue = bInfo.min_value;
                    auInfo->maxValue = bInfo.max_value;
                    auInfo->defaultValue = bInfo.default_value;

                    // Check if indexed parameter (for value strings)
                    // AUv2 indexed params use integer values 0..step_count
                    if (bInfo.unit_type == kAudioUnitParameterUnit_Indexed && bInfo.step_count > 0) {{
                        auInfo->flags |= kAudioUnitParameterFlag_ValuesHaveStrings;
                        auInfo->maxValue = (float)bInfo.step_count;
                        // Convert default from normalized to index
                        auInfo->defaultValue = roundf(bInfo.default_value * (float)bInfo.step_count);
                    }}

                    // Copy unit label if present
                    if (bInfo.units[0] != '\0') {{
                        auInfo->unitName = CFStringCreateWithCString(NULL, bInfo.units, kCFStringEncodingUTF8);
                    }}

                    *ioDataSize = sizeof(AudioUnitParameterInfo);
                    return noErr;
                }}
            }}
            return kAudioUnitErr_InvalidParameter;
        }}

        case kAudioUnitProperty_ParameterValueStrings: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            uint32_t count = beamer_au_get_parameter_value_count(inst->rustInstance, element);
            if (count == 0) {{
                return kAudioUnitErr_InvalidProperty;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(CFArrayRef)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            CFMutableArrayRef array = CFArrayCreateMutable(NULL, count, &kCFTypeArrayCallBacks);
            char buffer[256];
            for (uint32_t i = 0; i < count; i++) {{
                if (beamer_au_get_parameter_value_string(inst->rustInstance, element, i, buffer, sizeof(buffer))) {{
                    CFStringRef str = CFStringCreateWithCString(NULL, buffer, kCFStringEncodingUTF8);
                    CFArrayAppendValue(array, str);
                    CFRelease(str);
                }}
            }}
            *(CFArrayRef*)outData = array;
            *ioDataSize = sizeof(CFArrayRef);
            return noErr;
        }}

        case kAudioUnitProperty_ParameterStringFromValue: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioUnitParameterStringFromValue)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            AudioUnitParameterStringFromValue* params = (AudioUnitParameterStringFromValue*)outData;
            AudioUnitParameterID paramID = params->inParamID;

            // Get the value to convert (either provided or current)
            float value;
            if (params->inValue != NULL) {{
                value = *(params->inValue);
            }} else {{
                value = beamer_au_get_parameter_value_au(inst->rustInstance, paramID);
            }}

            // For indexed parameters, convert index to normalized for formatting
            float formatValue = value;
            uint32_t count = beamer_au_get_parameter_count(inst->rustInstance);
            for (uint32_t i = 0; i < count; i++) {{
                BeamerAuParameterInfo info;
                if (beamer_au_get_parameter_info(inst->rustInstance, i, &info) && info.id == paramID) {{
                    if (info.unit_type == kAudioUnitParameterUnit_Indexed && info.step_count > 0) {{
                        formatValue = value / (float)info.step_count;
                    }}
                    break;
                }}
            }}

            char buffer[256];
            uint32_t written = beamer_au_format_parameter_value(inst->rustInstance, paramID, formatValue, buffer, sizeof(buffer));
            if (written > 0) {{
                params->outString = CFStringCreateWithCString(NULL, buffer, kCFStringEncodingUTF8);
            }} else {{
                // Fallback: format as number
                char fallback[64];
                snprintf(fallback, sizeof(fallback), "%.2f", value);
                params->outString = CFStringCreateWithCString(NULL, fallback, kCFStringEncodingUTF8);
            }}

            *ioDataSize = sizeof(AudioUnitParameterStringFromValue);
            return noErr;
        }}

        case kAudioUnitProperty_ParameterValueFromString: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AudioUnitParameterValueFromString)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            AudioUnitParameterValueFromString* params = (AudioUnitParameterValueFromString*)outData;
            AudioUnitParameterID paramID = params->inParamID;
            CFStringRef inputString = params->inString;

            if (inputString == NULL) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            char buffer[256];
            if (!CFStringGetCString(inputString, buffer, sizeof(buffer), kCFStringEncodingUTF8)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            float parsedValue = 0.0f;
            if (beamer_au_parse_parameter_value(inst->rustInstance, paramID, buffer, &parsedValue)) {{
                // For indexed parameters, convert normalized to index
                uint32_t count = beamer_au_get_parameter_count(inst->rustInstance);
                for (uint32_t i = 0; i < count; i++) {{
                    BeamerAuParameterInfo info;
                    if (beamer_au_get_parameter_info(inst->rustInstance, i, &info) && info.id == paramID) {{
                        if (info.unit_type == kAudioUnitParameterUnit_Indexed && info.step_count > 0) {{
                            parsedValue = roundf(parsedValue * (float)info.step_count);
                        }}
                        break;
                    }}
                }}
                params->outValue = parsedValue;
            }} else {{
                // Parsing failed, try to interpret as a number directly
                params->outValue = (float)atof(buffer);
            }}

            *ioDataSize = sizeof(AudioUnitParameterValueFromString);
            return noErr;
        }}

        case kAudioUnitProperty_Latency: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(Float64)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            uint32_t samples = beamer_au_get_latency_samples(inst->rustInstance);
            *(Float64*)outData = (inst->sampleRate > 0) ? (Float64)samples / inst->sampleRate : 0.0;
            *ioDataSize = sizeof(Float64);
            return noErr;
        }}

        case kAudioUnitProperty_TailTime: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(Float64)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            uint32_t samples = beamer_au_get_tail_samples(inst->rustInstance);
            if (samples == UINT32_MAX) {{
                *(Float64*)outData = INFINITY;
            }} else {{
                *(Float64*)outData = (inst->sampleRate > 0) ? (Float64)samples / inst->sampleRate : 0.0;
            }}
            *ioDataSize = sizeof(Float64);
            return noErr;
        }}

        case kAudioUnitProperty_BypassEffect: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            *(UInt32*)outData = inst->bypassed ? 1 : 0;
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }}

        case kAudioUnitProperty_SupportedNumChannels: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            BeamerAuChannelCapabilities caps;
            if (!beamer_au_get_channel_capabilities(inst->rustInstance, &caps)) {{
                return kAudioUnitErr_InvalidProperty;
            }}
            UInt32 needed = caps.count * sizeof(AUChannelInfo);
            if (!outData || !ioDataSize || *ioDataSize < needed) {{
                if (ioDataSize) *ioDataSize = needed;
                return outData ? kAudioUnitErr_InvalidPropertyValue : noErr;
            }}
            AUChannelInfo* info = (AUChannelInfo*)outData;
            for (uint32_t i = 0; i < caps.count; i++) {{
                info[i].inChannels = (SInt16)caps.capabilities[i].input_channels;
                info[i].outChannels = (SInt16)caps.capabilities[i].output_channels;
            }}
            *ioDataSize = needed;
            return noErr;
        }}

        case kAudioUnitProperty_ClassInfo: {{
            if (!outData || !ioDataSize || *ioDataSize < sizeof(CFPropertyListRef)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

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

            // Add plugin name
            char nameBuffer[256];
            uint32_t nameLen = beamer_au_get_name(inst->rustInstance, nameBuffer, sizeof(nameBuffer));
            if (nameLen > 0) {{
                CFStringRef nameStr = CFStringCreateWithCString(NULL, nameBuffer, kCFStringEncodingUTF8);
                CFDictionarySetValue(dict, CFSTR("name"), nameStr);
                CFRelease(nameStr);
            }}

            // Store format version
            SInt32 version = 0;
            CFNumberRef versionNum = CFNumberCreate(NULL, kCFNumberSInt32Type, &version);
            CFDictionarySetValue(dict, CFSTR("version"), versionNum);
            CFRelease(versionNum);

            // Get state from Rust (save as "data" key which is the standard AU key)
            uint32_t stateSize = beamer_au_get_state_size(inst->rustInstance);
            if (stateSize > 0) {{
                uint8_t* stateBuffer = (uint8_t*)malloc(stateSize);
                if (stateBuffer) {{
                    uint32_t written = beamer_au_get_state(inst->rustInstance, stateBuffer, stateSize);
                    if (written > 0) {{
                        CFDataRef data = CFDataCreate(NULL, stateBuffer, written);
                        CFDictionarySetValue(dict, CFSTR("data"), data);
                        CFRelease(data);
                    }}
                    free(stateBuffer);
                }}
            }}

            *(CFPropertyListRef*)outData = dict;
            *ioDataSize = sizeof(CFPropertyListRef);
            return noErr;
        }}

        case kAudioUnitProperty_ElementCount: {{
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            if (scope == kAudioUnitScope_Input) {{
                *(UInt32*)outData = beamer_au_get_input_bus_count(inst->rustInstance);
            }} else if (scope == kAudioUnitScope_Output) {{
                *(UInt32*)outData = beamer_au_get_output_bus_count(inst->rustInstance);
            }} else if (scope == kAudioUnitScope_Global) {{
                *(UInt32*)outData = 1;
            }} else {{
                return kAudioUnitErr_InvalidScope;
            }}
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }}

        case kAudioUnitProperty_InPlaceProcessing: {{
            if (!outData || !ioDataSize || *ioDataSize < sizeof(UInt32)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            *(UInt32*)outData = 0; // Not using in-place processing
            *ioDataSize = sizeof(UInt32);
            return noErr;
        }}

        case kAudioUnitProperty_PresentPreset: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(AUPreset)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            AUPreset* preset = (AUPreset*)outData;
            if (inst->currentPresetIndex >= 0 && (uint32_t)inst->currentPresetIndex < inst->presetCount && inst->presetStorage) {{
                preset->presetNumber = inst->presetStorage[inst->currentPresetIndex].presetNumber;
                preset->presetName = inst->presetStorage[inst->currentPresetIndex].presetName;
            }} else {{
                preset->presetNumber = -1;
                preset->presetName = CFSTR("Current Settings");
            }}
            *ioDataSize = sizeof(AUPreset);
            return noErr;
        }}

        case kAudioUnitProperty_FactoryPresets: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!outData || !ioDataSize || *ioDataSize < sizeof(CFArrayRef)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            if (inst->factoryPresets) {{
                CFRetain(inst->factoryPresets);  // Caller owns reference
                *(CFArrayRef*)outData = inst->factoryPresets;
            }} else {{
                *(CFArrayRef*)outData = NULL;
            }}
            *ioDataSize = sizeof(CFArrayRef);
            return noErr;
        }}

        case kAudioUnitProperty_LastRenderError: {{
            if (!outData || !ioDataSize || *ioDataSize < sizeof(OSStatus)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            *(OSStatus*)outData = noErr;
            *ioDataSize = sizeof(OSStatus);
            return noErr;
        }}

        default:
            return kAudioUnitErr_InvalidProperty;
    }}
}}

static OSStatus BeamerAuv2SetProperty(void* self, AudioUnitPropertyID propID,
    AudioUnitScope scope, AudioUnitElement element, const void* inData, UInt32 inDataSize) {{

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    switch (propID) {{
        case kAudioUnitProperty_StreamFormat: {{
            if (!inData || inDataSize < sizeof(AudioStreamBasicDescription)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            const AudioStreamBasicDescription* desc = (const AudioStreamBasicDescription*)inData;

            // Validate format (must be float, non-interleaved)
            if (desc->mFormatID != kAudioFormatLinearPCM) {{
                return kAudioUnitErr_FormatNotSupported;
            }}
            if (!(desc->mFormatFlags & kAudioFormatFlagIsFloat)) {{
                return kAudioUnitErr_FormatNotSupported;
            }}

            // Validate scope first
            if (scope != kAudioUnitScope_Input && scope != kAudioUnitScope_Output) {{
                return kAudioUnitErr_InvalidScope;
            }}

            // Validate channel count is reasonable (1-64 channels)
            UInt32 proposedChannels = desc->mChannelsPerFrame;
            if (proposedChannels == 0 || proposedChannels > 64) {{
                return kAudioUnitErr_FormatNotSupported;
            }}

            // Validate channel count against declared capability for MAIN bus (element 0).
            // This enforces the [N, M] capability we report in SupportedNumChannels.
            // Auxiliary buses (sidechain, etc.) can have any reasonable channel count.
            if (element == 0) {{
                uint32_t declaredChannels;
                if (scope == kAudioUnitScope_Input) {{
                    declaredChannels = beamer_au_get_input_bus_channel_count(inst->rustInstance, 0);
                }} else {{
                    declaredChannels = beamer_au_get_output_bus_channel_count(inst->rustInstance, 0);
                }}
                if (declaredChannels > 0 && proposedChannels != declaredChannels) {{
                    return kAudioUnitErr_FormatNotSupported;
                }}
            }}

            // Apply the format change
            if (scope == kAudioUnitScope_Input) {{
                inst->inputFormat = *desc;
            }} else {{
                inst->outputFormat = *desc;
            }}
            inst->sampleRate = desc->mSampleRate;

            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }}

        case kAudioUnitProperty_SampleRate: {{
            if (!inData || inDataSize < sizeof(Float64)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            inst->sampleRate = *(Float64*)inData;
            inst->inputFormat.mSampleRate = inst->sampleRate;
            inst->outputFormat.mSampleRate = inst->sampleRate;
            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }}

        case kAudioUnitProperty_MaximumFramesPerSlice: {{
            if (!inData || inDataSize < sizeof(UInt32)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            inst->maxFramesPerSlice = *(UInt32*)inData;
            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }}

        case kAudioUnitProperty_BypassEffect: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!inData || inDataSize < sizeof(UInt32)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            inst->bypassed = (*(UInt32*)inData != 0);
            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }}

        case kAudioUnitProperty_SetRenderCallback: {{
            if (scope != kAudioUnitScope_Input || element != 0) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!inData || inDataSize < sizeof(AURenderCallbackStruct)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            inst->inputCallback = *(AURenderCallbackStruct*)inData;
            // Clear connection when callback is set
            inst->inputConnection.sourceAU = NULL;
            return noErr;
        }}

        case kAudioUnitProperty_MakeConnection: {{
            if (scope != kAudioUnitScope_Input || element != 0) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!inData || inDataSize < sizeof(AudioUnitConnection)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            const AudioUnitConnection* conn = (const AudioUnitConnection*)inData;
            inst->inputConnection.sourceAU = conn->sourceAudioUnit;
            inst->inputConnection.sourceOutputNumber = conn->sourceOutputNumber;
            // Clear callback when connection is set
            inst->inputCallback.inputProc = NULL;
            inst->inputCallback.inputProcRefCon = NULL;
            return noErr;
        }}

        case kAudioUnitProperty_HostCallbacks: {{
            if (!inData || inDataSize < sizeof(HostCallbackInfo)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            inst->hostCallbacks = *(HostCallbackInfo*)inData;
            return noErr;
        }}

        case kAudioUnitProperty_ClassInfo: {{
            if (!inData || inDataSize < sizeof(CFPropertyListRef)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}
            CFDictionaryRef dict = *(CFDictionaryRef*)inData;
            if (!dict || CFGetTypeID(dict) != CFDictionaryGetTypeID()) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            // Try "data" key (standard AU) first, then fallback to "beamer-state"
            CFDataRef stateData = (CFDataRef)CFDictionaryGetValue(dict, CFSTR("data"));
            if (!stateData) {{
                stateData = (CFDataRef)CFDictionaryGetValue(dict, CFSTR("beamer-state"));
            }}
            if (stateData && CFGetTypeID(stateData) == CFDataGetTypeID()) {{
                const uint8_t* bytes = CFDataGetBytePtr(stateData);
                CFIndex length = CFDataGetLength(stateData);
                beamer_au_set_state(inst->rustInstance, bytes, (uint32_t)length);
            }}

            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }}

        case kAudioUnitProperty_PresentPreset: {{
            if (scope != kAudioUnitScope_Global) {{
                return kAudioUnitErr_InvalidScope;
            }}
            if (!inData || inDataSize < sizeof(AUPreset)) {{
                return kAudioUnitErr_InvalidPropertyValue;
            }}

            const AUPreset* newPreset = (const AUPreset*)inData;
            if (newPreset->presetNumber >= 0 && (uint32_t)newPreset->presetNumber < inst->presetCount) {{
                inst->currentPresetIndex = newPreset->presetNumber;
                beamer_au_apply_preset(inst->rustInstance, (uint32_t)newPreset->presetNumber);
            }} else {{
                // User preset (negative number) - just track the index
                inst->currentPresetIndex = -1;
            }}

            NotifyPropertyListeners(inst, propID, scope, element);
            return noErr;
        }}

        case kAudioUnitProperty_OfflineRender:
        case kAudioUnitProperty_InPlaceProcessing:
        case kAudioUnitProperty_ShouldAllocateBuffer:
            // Accept but ignore these
            return noErr;

        default:
            return kAudioUnitErr_InvalidProperty;
    }}
}}

// =============================================================================
// MARK: - Property Listeners
// =============================================================================

static OSStatus BeamerAuv2AddPropertyListener(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc, void* userData) {{

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->listenerMutex);
    if (inst->propertyListenerCount >= MAX_PROPERTY_LISTENERS) {{
        pthread_mutex_unlock(&inst->listenerMutex);
        return kAudio_TooManyFilesOpenError;
    }}

    PropertyListener* listener = &inst->propertyListeners[inst->propertyListenerCount++];
    listener->propID = propID;
    listener->proc = proc;
    listener->userData = userData;

    pthread_mutex_unlock(&inst->listenerMutex);
    return noErr;
}}

static OSStatus BeamerAuv2RemovePropertyListener(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc) {{

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->listenerMutex);
    for (UInt32 i = 0; i < inst->propertyListenerCount; i++) {{
        if (inst->propertyListeners[i].propID == propID && inst->propertyListeners[i].proc == proc) {{
            // Shift remaining listeners down
            for (UInt32 j = i; j < inst->propertyListenerCount - 1; j++) {{
                inst->propertyListeners[j] = inst->propertyListeners[j + 1];
            }}
            inst->propertyListenerCount--;
            pthread_mutex_unlock(&inst->listenerMutex);
            return noErr;
        }}
    }}
    pthread_mutex_unlock(&inst->listenerMutex);
    return noErr;
}}

static OSStatus BeamerAuv2RemovePropertyListenerWithUserData(void* self, AudioUnitPropertyID propID,
    AudioUnitPropertyListenerProc proc, void* userData) {{

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->listenerMutex);
    for (UInt32 i = 0; i < inst->propertyListenerCount; i++) {{
        if (inst->propertyListeners[i].propID == propID &&
            inst->propertyListeners[i].proc == proc &&
            inst->propertyListeners[i].userData == userData) {{
            // Shift remaining listeners down
            for (UInt32 j = i; j < inst->propertyListenerCount - 1; j++) {{
                inst->propertyListeners[j] = inst->propertyListeners[j + 1];
            }}
            inst->propertyListenerCount--;
            pthread_mutex_unlock(&inst->listenerMutex);
            return noErr;
        }}
    }}
    pthread_mutex_unlock(&inst->listenerMutex);
    return noErr;
}}

// =============================================================================
// MARK: - Parameters
// =============================================================================

static OSStatus BeamerAuv2GetParameter(void* self, AudioUnitParameterID paramID,
    AudioUnitScope scope, AudioUnitElement element, AudioUnitParameterValue* outValue) {{

    (void)element;

    if (scope != kAudioUnitScope_Global) {{
        return kAudioUnitErr_InvalidScope;
    }}

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    // Use AU-format getter which handles indexed parameter conversion internally
    *outValue = beamer_au_get_parameter_value_au(inst->rustInstance, paramID);
    return noErr;
}}

static OSStatus BeamerAuv2SetParameter(void* self, AudioUnitParameterID paramID,
    AudioUnitScope scope, AudioUnitElement element, AudioUnitParameterValue value, UInt32 bufferOffset) {{

    (void)element;
    (void)bufferOffset; // TODO: Support sample-accurate automation

    if (scope != kAudioUnitScope_Global) {{
        return kAudioUnitErr_InvalidScope;
    }}

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    // Use AU-format setter which handles indexed parameter conversion internally
    beamer_au_set_parameter_value_au(inst->rustInstance, paramID, value);
    return noErr;
}}

static OSStatus BeamerAuv2ScheduleParameters(void* self, const AudioUnitParameterEvent* events, UInt32 numEvents) {{
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    for (UInt32 i = 0; i < numEvents; i++) {{
        const AudioUnitParameterEvent* event = &events[i];
        if (event->eventType == kParameterEvent_Immediate) {{
            // Use AU-format setter which handles indexed parameter conversion internally
            beamer_au_set_parameter_value_au(inst->rustInstance, event->parameter,
                event->eventValues.immediate.value);
        }}
        // TODO: Handle ramped parameter changes
    }}

    return noErr;
}}

// =============================================================================
// MARK: - Render
// =============================================================================

static OSStatus BeamerAuv2Render(void* self, AudioUnitRenderActionFlags* ioActionFlags,
    const AudioTimeStamp* inTimeStamp, UInt32 inOutputBusNumber, UInt32 inNumberFrames, AudioBufferList* ioData) {{

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    if (!inst->initialized) {{
        return kAudioUnitErr_Uninitialized;
    }}

    if (inNumberFrames > inst->maxFramesPerSlice) {{
        return kAudioUnitErr_TooManyFramesToProcess;
    }}

    // Call pre-render notifications
    pthread_mutex_lock(&inst->renderNotifyMutex);
    for (UInt32 i = 0; i < inst->renderNotifyCount; i++) {{
        AudioUnitRenderActionFlags preFlags = kAudioUnitRenderAction_PreRender;
        inst->renderNotify[i].proc(inst->renderNotify[i].userData,
            &preFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData);
    }}
    pthread_mutex_unlock(&inst->renderNotifyMutex);

    // Handle bypass
    if (inst->bypassed) {{
        // For bypass, we need to copy input to output
        // Pull input first
        AudioBufferList* inputData = NULL;
        if (inst->inputCallback.inputProc) {{
            EnsureInputBufferList(inst, inst->inputFormat.mChannelsPerFrame, inNumberFrames);
            AudioUnitRenderActionFlags pullFlags = 0;
            OSStatus pullStatus = inst->inputCallback.inputProc(
                inst->inputCallback.inputProcRefCon,
                &pullFlags, inTimeStamp, 0, inNumberFrames, inst->inputBufferList);
            if (pullStatus == noErr) {{
                inputData = inst->inputBufferList;
            }}
        }} else if (inst->inputConnection.sourceAU) {{
            EnsureInputBufferList(inst, inst->inputFormat.mChannelsPerFrame, inNumberFrames);
            AudioUnitRenderActionFlags pullFlags = 0;
            OSStatus pullStatus = AudioUnitRender(inst->inputConnection.sourceAU,
                &pullFlags, inTimeStamp, inst->inputConnection.sourceOutputNumber,
                inNumberFrames, inst->inputBufferList);
            if (pullStatus == noErr) {{
                inputData = inst->inputBufferList;
            }}
        }}

        // Copy input to output for bypass
        if (inputData) {{
            UInt32 buffersToCopy = (inputData->mNumberBuffers < ioData->mNumberBuffers) ?
                inputData->mNumberBuffers : ioData->mNumberBuffers;
            for (UInt32 i = 0; i < buffersToCopy; i++) {{
                UInt32 bytesToCopy = (inputData->mBuffers[i].mDataByteSize < ioData->mBuffers[i].mDataByteSize) ?
                    inputData->mBuffers[i].mDataByteSize : ioData->mBuffers[i].mDataByteSize;
                memcpy(ioData->mBuffers[i].mData, inputData->mBuffers[i].mData, bytesToCopy);
            }}
        }} else {{
            // No input, silence output
            for (UInt32 i = 0; i < ioData->mNumberBuffers; i++) {{
                memset(ioData->mBuffers[i].mData, 0, ioData->mBuffers[i].mDataByteSize);
            }}
        }}

        // Call post-render notifications
        pthread_mutex_lock(&inst->renderNotifyMutex);
        for (UInt32 i = 0; i < inst->renderNotifyCount; i++) {{
            AudioUnitRenderActionFlags postFlags = kAudioUnitRenderAction_PostRender;
            inst->renderNotify[i].proc(inst->renderNotify[i].userData,
                &postFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData);
        }}
        pthread_mutex_unlock(&inst->renderNotifyMutex);

        return noErr;
    }}

    // Pull input audio
    AudioBufferList* inputData = NULL;
    uint32_t inputBusCount = beamer_au_get_input_bus_count(inst->rustInstance);

    if (inputBusCount > 0) {{
        if (inst->inputCallback.inputProc) {{
            EnsureInputBufferList(inst, inst->inputFormat.mChannelsPerFrame, inNumberFrames);
            AudioUnitRenderActionFlags pullFlags = 0;
            OSStatus pullStatus = inst->inputCallback.inputProc(
                inst->inputCallback.inputProcRefCon,
                &pullFlags, inTimeStamp, 0, inNumberFrames, inst->inputBufferList);
            if (pullStatus == noErr) {{
                inputData = inst->inputBufferList;
            }}
        }} else if (inst->inputConnection.sourceAU) {{
            EnsureInputBufferList(inst, inst->inputFormat.mChannelsPerFrame, inNumberFrames);
            AudioUnitRenderActionFlags pullFlags = 0;
            OSStatus pullStatus = AudioUnitRender(inst->inputConnection.sourceAU,
                &pullFlags, inTimeStamp, inst->inputConnection.sourceOutputNumber,
                inNumberFrames, inst->inputBufferList);
            if (pullStatus == noErr) {{
                inputData = inst->inputBufferList;
            }}
        }}
    }}

    // Call Rust render function
    // Note: AUv2 doesn't have AURenderEvent, so we pass NULL for events and blocks
    OSStatus status = beamer_au_render(
        inst->rustInstance,
        ioActionFlags,
        inTimeStamp,
        inNumberFrames,
        inOutputBusNumber,
        ioData,
        NULL,  // events (AUv2 doesn't use AURenderEvent linked list)
        NULL,  // pull_input_block (we pre-pulled via callback/connection)
        inputData,
        NULL,  // musical_context_block (TODO: wrap host callbacks)
        NULL,  // transport_state_block (TODO: wrap host callbacks)
        NULL   // schedule_midi_block
    );

    // Call post-render notifications
    pthread_mutex_lock(&inst->renderNotifyMutex);
    for (UInt32 i = 0; i < inst->renderNotifyCount; i++) {{
        AudioUnitRenderActionFlags postFlags = kAudioUnitRenderAction_PostRender;
        inst->renderNotify[i].proc(inst->renderNotify[i].userData,
            &postFlags, inTimeStamp, inOutputBusNumber, inNumberFrames, ioData);
    }}
    pthread_mutex_unlock(&inst->renderNotifyMutex);

    return status;
}}

// =============================================================================
// MARK: - Reset
// =============================================================================

static OSStatus BeamerAuv2Reset(void* self, AudioUnitScope scope, AudioUnitElement element) {{
    (void)scope;
    (void)element;

    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;
    beamer_au_reset(inst->rustInstance);
    return noErr;
}}

// =============================================================================
// MARK: - Render Notifications
// =============================================================================

static OSStatus BeamerAuv2AddRenderNotify(void* self, AURenderCallback proc, void* userData) {{
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->renderNotifyMutex);
    if (inst->renderNotifyCount >= MAX_RENDER_NOTIFY) {{
        pthread_mutex_unlock(&inst->renderNotifyMutex);
        return kAudio_TooManyFilesOpenError;
    }}

    inst->renderNotify[inst->renderNotifyCount].proc = proc;
    inst->renderNotify[inst->renderNotifyCount].userData = userData;
    inst->renderNotifyCount++;

    pthread_mutex_unlock(&inst->renderNotifyMutex);
    return noErr;
}}

static OSStatus BeamerAuv2RemoveRenderNotify(void* self, AURenderCallback proc, void* userData) {{
    BeamerAuv2Instance* inst = (BeamerAuv2Instance*)self;

    pthread_mutex_lock(&inst->renderNotifyMutex);
    for (UInt32 i = 0; i < inst->renderNotifyCount; i++) {{
        if (inst->renderNotify[i].proc == proc && inst->renderNotify[i].userData == userData) {{
            // Shift remaining entries down
            for (UInt32 j = i; j < inst->renderNotifyCount - 1; j++) {{
                inst->renderNotify[j] = inst->renderNotify[j + 1];
            }}
            inst->renderNotifyCount--;
            pthread_mutex_unlock(&inst->renderNotifyMutex);
            return noErr;
        }}
    }}
    pthread_mutex_unlock(&inst->renderNotifyMutex);
    return noErr;
}}
"#,
        plugin_name = plugin_name,
        factory_name = factory_name
    )
}
