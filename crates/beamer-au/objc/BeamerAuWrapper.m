// BeamerAuWrapper.m
// Native Objective-C AUAudioUnit subclass for the Beamer AU framework.
//
// This file implements a thin Objective-C wrapper around the Rust audio processing
// core. All actual audio processing, parameter handling, and state management
// are delegated to Rust via C-ABI bridge functions defined in BeamerAuBridge.h.
//
// Design Philosophy:
// - This wrapper does minimal work; all heavy lifting happens in Rust
// - Memory management uses ARC (compile with -fobjc-arc)
// - Real-time safety: render block never allocates or locks
// - KVO compliance: bus arrays and parameter tree are cached
//
// Compile with: -fobjc-arc
// Link frameworks: AudioToolbox, AVFoundation, Foundation

#import <AudioToolbox/AudioToolbox.h>
#import <AVFoundation/AVFoundation.h>
#import <Foundation/Foundation.h>
#include <stdio.h>
#include <unistd.h>

#include "BeamerAuBridge.h"

// =============================================================================
// MARK: - Constants
// =============================================================================

/// Default sample rate when not specified by host
static const double kDefaultSampleRate = 44100.0;

/// Default maximum frames per render call
static const AUAudioFrameCount kDefaultMaxFrames = 4096;

static FILE* BeamerObjCLogFile(void) {
    static dispatch_once_t onceToken;
    static FILE* file = NULL;
    dispatch_once(&onceToken, ^{
        const char* path = getenv("BEAMER_AU_LOG_FILE");
        if (path == NULL || path[0] == '\0') {
            path = "/tmp/beamer_au.log";
        }

        FILE* f = fopen(path, "a");
        if (f) {
            file = f;
        }
    });
    return file;
}

static void BeamerObjCLog(const char* msg) {
    FILE* f = BeamerObjCLogFile();
    if (f) {
        fprintf(f, "%s\n", msg);
        fflush(f);
    }
}

static BOOL BeamerLoggingEnabled(void) {
    static dispatch_once_t onceToken;
    static BOOL enabled = NO;
    dispatch_once(&onceToken, ^{
        NSDictionary* env = [[NSProcessInfo processInfo] environment];
        NSString* flag = env[@"BEAMER_AU_LOG"];
        if (flag != nil) {
            enabled = ![flag isEqualToString:@"0"];
        } else {
            NSUserDefaults* defaults = [NSUserDefaults standardUserDefaults];
            enabled = [defaults boolForKey:@"BEAMER_AU_LOG"];
            if (!enabled) {
                CFPropertyListRef val = CFPreferencesCopyAppValue(CFSTR("BEAMER_AU_LOG"), CFSTR("com.helpermedia.beamer"));
                if (val && CFGetTypeID(val) == CFBooleanGetTypeID()) {
                    enabled = CFBooleanGetValue((CFBooleanRef)val);
                }
                if (val) CFRelease(val);
            }
        }

        // Propagate to process env so Rust side sees it via std::env
        if (enabled) {
            setenv("BEAMER_AU_LOG", "1", 1);
        }
    });
    return enabled;
}

// =============================================================================
// MARK: - BeamerAuWrapper Interface
// =============================================================================

@interface BeamerAuWrapper : AUAudioUnit {
    /// Opaque pointer to the Rust plugin instance.
    /// Managed via beamer_au_create_instance() / beamer_au_destroy_instance().
    BeamerAuInstanceHandle _rustInstance;

    /// Lock to protect _rustInstance access during parameter callbacks.
    /// This prevents use-after-free when dealloc races with callbacks.
    NSLock* _instanceLock;

    /// Flag indicating whether the instance is still valid for use.
    /// Set to NO during dealloc before destroying _rustInstance.
    BOOL _instanceValid;

    /// Cached input bus array for KVO compliance.
    /// AUAudioUnit requires returning the same instance each time.
    AUAudioUnitBusArray* _inputBusArray;

    /// Cached output bus array for KVO compliance.
    AUAudioUnitBusArray* _outputBusArray;

    /// Cached parameter tree for KVO compliance.
    AUParameterTree* _parameterTree;

    /// Sample format (f32 or f64) determined during allocateRenderResources.
    BeamerAuSampleFormat _sampleFormat;

    /// Current sample rate (set during allocateRenderResources).
    double _sampleRate;

    /// Maximum frames per render call (set during allocateRenderResources).
    AUAudioFrameCount _maxFrames;

    /// Whether render resources have been allocated.
    BOOL _resourcesAllocated;

    /// Cached bus configuration (used for render resource allocation).
    BeamerAuBusConfig _busConfig;
}

@end

// =============================================================================
// MARK: - BeamerAuWrapper Implementation
// =============================================================================

@implementation BeamerAuWrapper

// -----------------------------------------------------------------------------
// MARK: Initialization
// -----------------------------------------------------------------------------

/// Initialize the Audio Unit with the given component description.
///
/// This is called by the AU host when instantiating the plugin.
/// We create the Rust plugin instance and set up the bus arrays.
- (instancetype)initWithComponentDescription:(AudioComponentDescription)componentDescription
                                     options:(AudioComponentInstantiationOptions)options
                                       error:(NSError**)outError {
        BeamerObjCLog("objc init entry");
        NSLog(@"BeamerAU initWithComponentDescription options=%lu", (unsigned long)options);
    // Ensure the Rust factory is registered before creating instances
    if (!beamer_au_ensure_factory_registered()) {
        if (outError != NULL) {
            *outError = [NSError errorWithDomain:NSOSStatusErrorDomain
                                            code:kAudioUnitErr_FailedInitialization
                                        userInfo:@{NSLocalizedDescriptionKey: @"Failed to register Rust plugin factory"}];
        }
        return nil;
    }

    self = [super initWithComponentDescription:componentDescription
                                       options:options
                                         error:outError];
    if (self == nil) {
        return nil;
    }

    // Initialize instance variables
    _rustInstance = NULL;
    _instanceLock = [[NSLock alloc] init];
    _instanceValid = NO;
    _inputBusArray = nil;
    _outputBusArray = nil;
    _parameterTree = nil;
    _sampleFormat = BeamerAuSampleFormatFloat32;
    _sampleRate = kDefaultSampleRate;
    _maxFrames = kDefaultMaxFrames;
    _resourcesAllocated = NO;
    memset(&_busConfig, 0, sizeof(_busConfig));

    // Create the Rust plugin instance
    _rustInstance = beamer_au_create_instance();
    if (_rustInstance == NULL) {
        if (outError != NULL) {
            *outError = [NSError errorWithDomain:NSOSStatusErrorDomain
                                            code:kAudioUnitErr_FailedInitialization
                                        userInfo:@{NSLocalizedDescriptionKey: @"Failed to create Rust plugin instance"}];
        }
        BeamerObjCLog("objc init rust instance null");
        NSLog(@"BeamerAU init failed to create Rust instance");
        return nil;
    }

    BeamerObjCLog("objc init rust instance ok");

    // Mark instance as valid now that it's created
    _instanceValid = YES;

    // Query bus configuration from Rust and set up bus arrays
    if (![self setupBusArraysWithError:outError]) {
        _instanceValid = NO;
        beamer_au_destroy_instance(_rustInstance);
        _rustInstance = NULL;
        NSLog(@"BeamerAU init failed bus setup");
        BeamerObjCLog("objc init bus setup failed");
        return nil;
    }

    // Build the parameter tree from Rust parameter info
    [self buildParameterTree];
    BeamerObjCLog("objc init parameter tree built");

    // Set default maximum frames
    self.maximumFramesToRender = kDefaultMaxFrames;

    NSLog(@"BeamerAU init completed");
    BeamerObjCLog("objc init completed");
    return self;
}

// -----------------------------------------------------------------------------
// MARK: AUAudioUnitFactory Protocol
// -----------------------------------------------------------------------------

/// Create a new Audio Unit instance (AUAudioUnitFactory protocol).
///
/// This method is called by the AU host to create instances of the audio unit.
/// It returns a new BeamerAuWrapper configured with the given component description.
///
/// @param desc The component description specifying type, subtype, manufacturer.
/// @param error Output parameter for error information if creation fails.
/// @return A new AUAudioUnit instance, or nil on failure.
- (nullable AUAudioUnit *)createAudioUnitWithComponentDescription:(AudioComponentDescription)desc
                                                            error:(NSError **)error {
    // Ensure the Rust factory is registered before creating instances
    if (!beamer_au_ensure_factory_registered()) {
        if (error) {
            *error = [NSError errorWithDomain:NSOSStatusErrorDomain
                                         code:kAudioUnitErr_FailedInitialization
                                     userInfo:@{NSLocalizedDescriptionKey: @"Failed to register plugin factory"}];
        }
        return nil;
    }

    // Create and return a new instance
    return [[BeamerAuWrapper alloc] initWithComponentDescription:desc
                                                         options:0
                                                           error:error];
}

/// Clean up the Rust plugin instance.
- (void)dealloc {
    // Acquire lock to ensure no callbacks are in-flight before destroying instance.
    // This prevents use-after-free where a callback could be executing Rust code
    // while we destroy the Rust instance.
    [_instanceLock lock];

    // Mark instance as invalid first, so any callbacks that acquire the lock
    // after we release it will see the invalid state and bail out.
    _instanceValid = NO;

    if (_rustInstance != NULL) {
        // Deallocate render resources if still allocated
        if (_resourcesAllocated) {
            beamer_au_deallocate_render_resources(_rustInstance);
            _resourcesAllocated = NO;
        }

        // Destroy the Rust instance
        beamer_au_destroy_instance(_rustInstance);
        _rustInstance = NULL;
    }

    [_instanceLock unlock];
}

// -----------------------------------------------------------------------------
// MARK: Bus Configuration
// -----------------------------------------------------------------------------

/// Create a single audio bus with the specified configuration.
///
/// This helper method extracts the common logic for creating both input and output buses.
/// It queries the channel count from Rust, clamps it to valid bounds, creates the format,
/// and initializes the bus object.
///
/// @param index The bus index (0-based).
/// @param isInput YES for input buses, NO for output buses.
/// @param defaultFormat Fallback format if channel-specific format creation fails.
/// @param outError On failure, contains the error description.
/// @return The created bus, or nil on failure.
- (AUAudioUnitBus*)createBusAtIndex:(uint32_t)index
                            isInput:(BOOL)isInput
                      defaultFormat:(AVAudioFormat*)defaultFormat
                              error:(NSError**)outError {
    // Query channel count from Rust based on bus direction
    uint32_t channelCount = isInput
        ? beamer_au_get_input_bus_channel_count(_rustInstance, index)
        : beamer_au_get_output_bus_channel_count(_rustInstance, index);

    // Clamp channel count to valid range
    if (channelCount == 0) channelCount = 2; // Default to stereo
    if (channelCount > BEAMER_AU_MAX_CHANNELS) channelCount = BEAMER_AU_MAX_CHANNELS;

    // Create format for this bus
    AVAudioFormat* format = [[AVAudioFormat alloc]
        initStandardFormatWithSampleRate:kDefaultSampleRate
                                channels:(AVAudioChannelCount)channelCount];
    if (format == nil) {
        format = defaultFormat;
    }

    // Create the bus
    NSError* busError = nil;
    AUAudioUnitBus* bus = [[AUAudioUnitBus alloc] initWithFormat:format error:&busError];
    if (bus == nil) {
        if (outError != NULL) {
            *outError = busError;
        }
        return nil;
    }

    // Set bus name based on direction and index
    if (isInput) {
        bus.name = (index == 0) ? @"Main Input" : [NSString stringWithFormat:@"Aux Input %u", index];
    } else {
        bus.name = (index == 0) ? @"Main Output" : [NSString stringWithFormat:@"Aux Output %u", index];
    }

    return bus;
}

/// Set up the input and output bus arrays based on Rust plugin configuration.
- (BOOL)setupBusArraysWithError:(NSError**)outError {
    if (_rustInstance == NULL) {
        if (outError != NULL) {
            *outError = [NSError errorWithDomain:NSOSStatusErrorDomain
                                            code:kAudioUnitErr_Uninitialized
                                        userInfo:@{NSLocalizedDescriptionKey: @"Rust instance not initialized"}];
        }
        return NO;
    }

    // Query bus counts from Rust
    uint32_t inputBusCount = beamer_au_get_input_bus_count(_rustInstance);
    uint32_t outputBusCount = beamer_au_get_output_bus_count(_rustInstance);

    // Clamp to maximum
    if (inputBusCount > BEAMER_AU_MAX_BUSES) inputBusCount = BEAMER_AU_MAX_BUSES;
    if (outputBusCount > BEAMER_AU_MAX_BUSES) outputBusCount = BEAMER_AU_MAX_BUSES;

    // Ensure at least one output bus for instruments/generators
    if (inputBusCount == 0 && outputBusCount == 0) {
        outputBusCount = 1;
    }

    // Create default format (44.1kHz, float32, non-interleaved) as fallback
    AVAudioFormat* stereoFormat = [[AVAudioFormat alloc]
        initStandardFormatWithSampleRate:kDefaultSampleRate
                                channels:2];

    // Create input buses using helper method
    NSMutableArray<AUAudioUnitBus*>* inputBuses = [[NSMutableArray alloc] initWithCapacity:inputBusCount];
    for (uint32_t i = 0; i < inputBusCount; i++) {
        AUAudioUnitBus* bus = [self createBusAtIndex:i
                                             isInput:YES
                                       defaultFormat:stereoFormat
                                               error:outError];
        if (bus == nil) {
            return NO;
        }
        [inputBuses addObject:bus];
    }

    // Create output buses using helper method
    NSMutableArray<AUAudioUnitBus*>* outputBuses = [[NSMutableArray alloc] initWithCapacity:outputBusCount];
    for (uint32_t i = 0; i < outputBusCount; i++) {
        AUAudioUnitBus* bus = [self createBusAtIndex:i
                                             isInput:NO
                                       defaultFormat:stereoFormat
                                               error:outError];
        if (bus == nil) {
            return NO;
        }
        [outputBuses addObject:bus];
    }

    // Create bus arrays (must return same instance for KVO compliance)
    _inputBusArray = [[AUAudioUnitBusArray alloc] initWithAudioUnit:self
                                                           busType:AUAudioUnitBusTypeInput
                                                            busses:inputBuses];
    _outputBusArray = [[AUAudioUnitBusArray alloc] initWithAudioUnit:self
                                                            busType:AUAudioUnitBusTypeOutput
                                                             busses:outputBuses];

    return YES;
}

/// Build the bus configuration structure from current bus arrays.
- (void)buildBusConfig {
    memset(&_busConfig, 0, sizeof(_busConfig));

    // Input buses
    _busConfig.input_bus_count = (uint32_t)_inputBusArray.count;
    for (uint32_t i = 0; i < _busConfig.input_bus_count && i < BEAMER_AU_MAX_BUSES; i++) {
        _busConfig.input_buses[i].channel_count = (uint32_t)_inputBusArray[i].format.channelCount;
        _busConfig.input_buses[i].bus_type = (i == 0) ? BeamerAuBusTypeMain : BeamerAuBusTypeAuxiliary;
    }

    // Output buses
    _busConfig.output_bus_count = (uint32_t)_outputBusArray.count;
    for (uint32_t i = 0; i < _busConfig.output_bus_count && i < BEAMER_AU_MAX_BUSES; i++) {
        _busConfig.output_buses[i].channel_count = (uint32_t)_outputBusArray[i].format.channelCount;
        _busConfig.output_buses[i].bus_type = (i == 0) ? BeamerAuBusTypeMain : BeamerAuBusTypeAuxiliary;
    }
}

// -----------------------------------------------------------------------------
// MARK: Bus Properties (KVO-compliant)
// -----------------------------------------------------------------------------

/// Return the cached input bus array.
/// Must return the same instance each time for KVO compliance.
- (AUAudioUnitBusArray*)inputBusses {
    return _inputBusArray;
}

/// Return the cached output bus array.
/// Must return the same instance each time for KVO compliance.
- (AUAudioUnitBusArray*)outputBusses {
    return _outputBusArray;
}

// -----------------------------------------------------------------------------
// MARK: Render Resources
// -----------------------------------------------------------------------------

/// Allocate render resources for audio processing.
///
/// Called by the host before audio processing begins.
/// Extracts format info from buses and notifies Rust.
- (BOOL)allocateRenderResourcesAndReturnError:(NSError**)outError {
    BeamerObjCLog("objc allocate entry");
    NSLog(@"BeamerAU allocateRenderResources entry");
    if (_rustInstance == NULL) {
        if (outError != NULL) {
            *outError = [NSError errorWithDomain:NSOSStatusErrorDomain
                                            code:kAudioUnitErr_Uninitialized
                                        userInfo:@{NSLocalizedDescriptionKey: @"Rust instance not initialized"}];
        }
        return NO;
    }

    // Build bus configuration from current bus arrays BEFORE calling super
    // This allows us to validate the configuration and reject early
    [self buildBusConfig];

    NSLog(@"BeamerAU allocateRenderResources bus_config inputs=%u outputs=%u", _busConfig.input_bus_count, _busConfig.output_bus_count);
    {
        char buf[128];
        snprintf(buf, sizeof(buf), "objc allocate bus_config in=%u out=%u", _busConfig.input_bus_count, _busConfig.output_bus_count);
        BeamerObjCLog(buf);
    }

    (void)BeamerLoggingEnabled();

    // Validate channel configuration before allocating any resources
    // For effect plugins (aufx), input channels must equal output channels
    uint32_t mainInputChannels = (_busConfig.input_bus_count > 0) ? _busConfig.input_buses[0].channel_count : 0;
    uint32_t mainOutputChannels = (_busConfig.output_bus_count > 0) ? _busConfig.output_buses[0].channel_count : 0;

    if (!beamer_au_is_channel_config_valid(_rustInstance, mainInputChannels, mainOutputChannels)) {
        if (outError != NULL) {
            *outError = [NSError errorWithDomain:NSOSStatusErrorDomain
                                            code:kAudioUnitErr_FormatNotSupported
                                        userInfo:@{NSLocalizedDescriptionKey: @"Channel configuration not supported"}];
        }
        return NO;
    }

    // Call super after validation passes (required by Apple)
    if (![super allocateRenderResourcesAndReturnError:outError]) {
        BeamerObjCLog("objc allocate super failed");
        return NO;
    }

    BeamerObjCLog("objc allocate super ok");

    // Get format info from the first output bus (or input if no outputs)
    AVAudioFormat* format = nil;
    if (_outputBusArray.count > 0) {
        format = _outputBusArray[0].format;
    } else if (_inputBusArray.count > 0) {
        format = _inputBusArray[0].format;
    }

    if (format != nil) {
        _sampleRate = format.sampleRate;
        _sampleFormat = (format.commonFormat == AVAudioPCMFormatFloat64)
            ? BeamerAuSampleFormatFloat64
            : BeamerAuSampleFormatFloat32;
    } else {
        _sampleRate = kDefaultSampleRate;
        _sampleFormat = BeamerAuSampleFormatFloat32;
    }

    _maxFrames = self.maximumFramesToRender;

    // Allocate render resources in Rust
    OSStatus result = beamer_au_allocate_render_resources(
        _rustInstance,
        _sampleRate,
        _maxFrames,
        _sampleFormat,
        &_busConfig
    );

    if (result != noErr) {
        if (outError != NULL) {
            *outError = [NSError errorWithDomain:NSOSStatusErrorDomain
                                            code:result
                                        userInfo:@{NSLocalizedDescriptionKey: @"Failed to allocate Rust render resources"}];
        }
        [super deallocateRenderResources];
        NSLog(@"BeamerAU allocateRenderResources rust allocation failed: %d", (int)result);
        BeamerObjCLog("objc allocate rust failed");
        return NO;
    }

    _resourcesAllocated = YES;
    NSLog(@"BeamerAU allocateRenderResources success sr=%.2f maxFrames=%u format=%d", _sampleRate, _maxFrames, _sampleFormat);
    {
        char buf[160];
        snprintf(buf, sizeof(buf), "objc allocate success sr=%.2f max=%u fmt=%d", _sampleRate, _maxFrames, _sampleFormat);
        BeamerObjCLog(buf);
    }
    return YES;
}

/// Deallocate render resources.
///
/// Called by the host when audio processing is stopping.
- (void)deallocateRenderResources {
    BeamerObjCLog("objc deallocate entry");
    if (_rustInstance != NULL && _resourcesAllocated) {
        (void)BeamerLoggingEnabled();
        beamer_au_deallocate_render_resources(_rustInstance);
        _resourcesAllocated = NO;
        BeamerObjCLog("objc deallocate rust resources");
    }

    [super deallocateRenderResources];
    BeamerObjCLog("objc deallocate super done");
}

// -----------------------------------------------------------------------------
// MARK: Render Block
// -----------------------------------------------------------------------------

/// Return the internal render block for audio processing.
///
/// This block is called by the host for each render cycle.
/// All real work is delegated to Rust via beamer_au_render().
///
/// IMPORTANT: This block must be real-time safe:
/// - No memory allocation
/// - No locks (only try_lock in Rust)
/// - No Objective-C messaging
/// - Fast execution (sub-millisecond)
- (AUInternalRenderBlock)internalRenderBlock {
    (void)BeamerLoggingEnabled();

    NSLog(@"BeamerAU internalRenderBlock created");
    BeamerObjCLog("objc internalRenderBlock created");

    // Capture all needed values at block creation time.
    // This avoids accessing self from inside the render block, which would:
    // 1. Cause a race condition (host blocks can change while rendering)
    // 2. Violate real-time safety (ObjC messaging in audio thread)
    BeamerAuInstanceHandle rustInstance = _rustInstance;
    AUHostMusicalContextBlock musicalContext = self.musicalContextBlock;
    AUHostTransportStateBlock transportState = self.transportStateBlock;
    AUScheduleMIDIEventBlock scheduleMIDI = self.scheduleMIDIEventBlock;

    // Return the render block
    return ^AUAudioUnitStatus(
        AudioUnitRenderActionFlags* actionFlags,
        const AudioTimeStamp* timestamp,
        AUAudioFrameCount frameCount,
        NSInteger outputBusNumber,
        AudioBufferList* outputData,
        const AURenderEvent* realtimeEventListHead,
        AURenderPullInputBlock __unsafe_unretained pullInputBlock
    ) {
        // Safety check (should never happen in normal operation)
        if (rustInstance == NULL) {
            return kAudioUnitErr_Uninitialized;
        }

        // Delegate to Rust render function.
        // All parameters are passed directly without ObjC object creation.
        // Host callback blocks were captured at block creation time above.
        return beamer_au_render(
            rustInstance,
            actionFlags,
            timestamp,
            frameCount,
            outputBusNumber,
            outputData,
            realtimeEventListHead,
            pullInputBlock,
            musicalContext,
            transportState,
            scheduleMIDI
        );
    };
}

// -----------------------------------------------------------------------------
// MARK: Parameters
// -----------------------------------------------------------------------------

/// Return the cached parameter tree.
/// Must return the same instance each time for KVO compliance.
- (AUParameterTree*)parameterTree {
    return _parameterTree;
}

/// Build the parameter tree from Rust parameter info.
- (void)buildParameterTree {
    if (_rustInstance == NULL) {
        _parameterTree = nil;
        return;
    }

    // Query parameter count from Rust
    uint32_t paramCount = beamer_au_get_parameter_count(_rustInstance);
    if (paramCount == 0) {
        // Create empty parameter tree
        _parameterTree = [AUParameterTree createTreeWithChildren:@[]];
        return;
    }

    // Build parameters array
    NSMutableArray<AUParameterNode*>* parameters = [[NSMutableArray alloc] initWithCapacity:paramCount];

    for (uint32_t i = 0; i < paramCount; i++) {
        // Query parameter info from Rust
        BeamerAuParameterInfo info;
        if (!beamer_au_get_parameter_info(_rustInstance, i, &info)) {
            continue;
        }

        // Create identifier and name strings
        NSString* identifier = [NSString stringWithFormat:@"param_%u", info.id];
        NSString* name = [NSString stringWithUTF8String:info.name];

        // Determine AU parameter unit from units string
        AudioUnitParameterUnit auUnit = [self auUnitFromUnitsString:info.units];

        // Determine flags
        AudioUnitParameterOptions flags = kAudioUnitParameterFlag_IsReadable;
        if (!(info.flags & BeamerAuParameterFlagReadOnly)) {
            flags |= kAudioUnitParameterFlag_IsWritable;
        }

        // Create the parameter (normalized 0.0-1.0 range)
        AUParameter* param = [AUParameterTree createParameterWithIdentifier:identifier
                                                                       name:name
                                                                    address:(AUParameterAddress)info.id
                                                                        min:0.0f
                                                                        max:1.0f
                                                                       unit:auUnit
                                                                   unitName:nil
                                                                      flags:flags
                                                               valueStrings:nil
                                                        dependentParameters:nil];

        // Set default value
        param.value = info.default_value;

        [parameters addObject:param];
    }

    // Create the parameter tree
    _parameterTree = [AUParameterTree createTreeWithChildren:parameters];

    // Set up parameter callbacks
    [self setupParameterCallbacks];
}

/// Set up parameter value provider and observer callbacks.
///
/// Uses weak/strong self pattern combined with a lock to prevent use-after-free.
/// The lock ensures that callbacks cannot execute Rust code while dealloc is
/// destroying the Rust instance. The _instanceValid flag is checked under the
/// lock to detect when dealloc is in progress.
///
/// Thread Safety Pattern:
/// 1. Callback promotes weakSelf to strongSelf (keeps self alive during callback)
/// 2. Callback acquires _instanceLock
/// 3. Callback checks _instanceValid flag
/// 4. If valid, callback executes Rust code while holding lock
/// 5. Callback releases lock
///
/// Dealloc Pattern:
/// 1. Dealloc acquires _instanceLock (waits for any in-flight callbacks)
/// 2. Dealloc sets _instanceValid = NO
/// 3. Dealloc destroys Rust instance
/// 4. Dealloc releases lock
- (void)setupParameterCallbacks {
    if (_parameterTree == nil || _rustInstance == NULL) {
        return;
    }

    // Use weak/strong self pattern to avoid retain cycles
    __weak typeof(self) weakSelf = self;

    // Value provider: called when AU needs to read parameter value from plugin
    _parameterTree.implementorValueProvider = ^AUValue(AUParameter* param) {
        __strong typeof(self) strongSelf = weakSelf;
        if (strongSelf == nil) {
            return 0.0f;
        }

        // Acquire lock to prevent dealloc from destroying instance while we use it
        [strongSelf->_instanceLock lock];

        AUValue result = 0.0f;
        if (strongSelf->_instanceValid && strongSelf->_rustInstance != NULL) {
            result = beamer_au_get_parameter_value(strongSelf->_rustInstance, (uint32_t)param.address);
        }

        [strongSelf->_instanceLock unlock];
        return result;
    };

    // Value observer: called when AU sets parameter value (from host automation or UI)
    _parameterTree.implementorValueObserver = ^(AUParameter* param, AUValue value) {
        __strong typeof(self) strongSelf = weakSelf;
        if (strongSelf == nil) {
            return;
        }

        // Acquire lock to prevent dealloc from destroying instance while we use it
        [strongSelf->_instanceLock lock];

        if (strongSelf->_instanceValid && strongSelf->_rustInstance != NULL) {
            beamer_au_set_parameter_value(strongSelf->_rustInstance, (uint32_t)param.address, value);
        }

        [strongSelf->_instanceLock unlock];
    };

    // String from value: format parameter value for display
    _parameterTree.implementorStringFromValueCallback = ^NSString* _Nonnull(AUParameter* param, const AUValue* value) {
        __strong typeof(self) strongSelf = weakSelf;
        AUValue displayValue = (value != NULL) ? *value : param.value;

        if (strongSelf == nil) {
            return [NSString stringWithFormat:@"%.2f", displayValue];
        }

        // Acquire lock to prevent dealloc from destroying instance while we use it
        [strongSelf->_instanceLock lock];

        NSString* result = nil;
        if (strongSelf->_instanceValid && strongSelf->_rustInstance != NULL) {
            char buffer[128];
            uint32_t written = beamer_au_format_parameter_value(
                strongSelf->_rustInstance,
                (uint32_t)param.address,
                displayValue,
                buffer,
                sizeof(buffer)
            );

            if (written > 0) {
                result = [NSString stringWithUTF8String:buffer];
            }
        }

        [strongSelf->_instanceLock unlock];

        // Return formatted result or fallback
        return (result != nil) ? result : [NSString stringWithFormat:@"%.2f", displayValue];
    };

    // Value from string: parse display string to parameter value
    _parameterTree.implementorValueFromStringCallback = ^AUValue(AUParameter* param, NSString* string) {
        __strong typeof(self) strongSelf = weakSelf;
        if (strongSelf == nil || string == nil) {
            return param.value;
        }

        // Acquire lock to prevent dealloc from destroying instance while we use it
        [strongSelf->_instanceLock lock];

        AUValue result = param.value;
        if (strongSelf->_instanceValid && strongSelf->_rustInstance != NULL) {
            float parsedValue = 0.0f;
            if (beamer_au_parse_parameter_value(strongSelf->_rustInstance, (uint32_t)param.address, string.UTF8String, &parsedValue)) {
                result = parsedValue;
            }
        }

        [strongSelf->_instanceLock unlock];
        return result;
    };
}

/// Map a units string (e.g., "dB", "Hz") to an AudioUnitParameterUnit.
- (AudioUnitParameterUnit)auUnitFromUnitsString:(const char*)units {
    if (units == NULL || units[0] == '\0') {
        return kAudioUnitParameterUnit_Generic;
    }

    NSString* unitStr = [[NSString stringWithUTF8String:units] lowercaseString];

    if ([unitStr isEqualToString:@"db"] || [unitStr isEqualToString:@"decibels"]) {
        return kAudioUnitParameterUnit_Decibels;
    } else if ([unitStr isEqualToString:@"hz"] || [unitStr isEqualToString:@"hertz"]) {
        return kAudioUnitParameterUnit_Hertz;
    } else if ([unitStr isEqualToString:@"ms"] || [unitStr isEqualToString:@"milliseconds"]) {
        return kAudioUnitParameterUnit_Milliseconds;
    } else if ([unitStr isEqualToString:@"s"] || [unitStr isEqualToString:@"seconds"]) {
        return kAudioUnitParameterUnit_Seconds;
    } else if ([unitStr isEqualToString:@"%"] || [unitStr isEqualToString:@"percent"]) {
        return kAudioUnitParameterUnit_Percent;
    } else if ([unitStr isEqualToString:@"pan"]) {
        return kAudioUnitParameterUnit_Pan;
    } else if ([unitStr isEqualToString:@"ratio"]) {
        return kAudioUnitParameterUnit_Ratio;
    } else if ([unitStr isEqualToString:@"bpm"]) {
        return kAudioUnitParameterUnit_BPM;
    } else if ([unitStr isEqualToString:@"semitones"]) {
        return kAudioUnitParameterUnit_RelativeSemiTones;
    } else if ([unitStr isEqualToString:@"cents"]) {
        return kAudioUnitParameterUnit_Cents;
    } else if ([unitStr isEqualToString:@"octaves"]) {
        return kAudioUnitParameterUnit_Octaves;
    } else if ([unitStr isEqualToString:@"degrees"]) {
        return kAudioUnitParameterUnit_Degrees;
    }

    return kAudioUnitParameterUnit_Generic;
}

// -----------------------------------------------------------------------------
// MARK: State Persistence
// -----------------------------------------------------------------------------

/// Return the full state dictionary for preset saving.
- (NSDictionary<NSString*, id>*)fullState {
    NSMutableDictionary* state = [[super fullState] mutableCopy];
    if (state == nil) {
        state = [[NSMutableDictionary alloc] init];
    }

    if (_rustInstance != NULL) {
        // Get state size from Rust
        uint32_t stateSize = beamer_au_get_state_size(_rustInstance);
        if (stateSize > 0) {
            // Allocate buffer and get state data
            uint8_t* buffer = (uint8_t*)malloc(stateSize);
            if (buffer != NULL) {
                uint32_t written = beamer_au_get_state(_rustInstance, buffer, stateSize);
                if (written > 0) {
                    // Convert to NSData and add to state dictionary
                    NSData* stateData = [NSData dataWithBytes:buffer length:written];
                    state[@"beamerState"] = stateData;
                }
                free(buffer);
            }
        }
    }

    return state;
}

/// Set the full state dictionary for preset loading.
- (void)setFullState:(NSDictionary<NSString*, id>*)fullState {
    // Call super first to handle standard AU state
    [super setFullState:fullState];

    if (_rustInstance != NULL && fullState != nil) {
        // Get beamer state data from dictionary
        NSData* stateData = fullState[@"beamerState"];
        if (stateData != nil && stateData.length > 0) {
            // Pass state data to Rust for deserialization
            OSStatus status = beamer_au_set_state(_rustInstance, stateData.bytes, (uint32_t)stateData.length);

            if (status != noErr) {
                NSLog(@"[BeamerAU] Warning: Failed to restore plugin state (OSStatus: %d)", (int)status);
                return;
            }

            // Notify parameter tree observers that values may have changed
            if (_parameterTree != nil) {
                // Re-read all parameter values after state load
                for (AUParameter* param in _parameterTree.allParameters) {
                    AUValue newValue = beamer_au_get_parameter_value(_rustInstance, (uint32_t)param.address);
                    // Use setValue:originator: to notify observers
                    [param setValue:newValue originator:nil];
                }
            }
        }
    }
}

// -----------------------------------------------------------------------------
// MARK: Processing Properties
// -----------------------------------------------------------------------------

/// Return the processing latency in seconds.
- (NSTimeInterval)latency {
    if (_rustInstance == NULL || _sampleRate <= 0.0) {
        return 0.0;
    }

    uint32_t latencySamples = beamer_au_get_latency_samples(_rustInstance);
    return (NSTimeInterval)latencySamples / _sampleRate;
}

/// Return the tail time in seconds.
- (NSTimeInterval)tailTime {
    if (_rustInstance == NULL || _sampleRate <= 0.0) {
        return 0.0;
    }

    uint32_t tailSamples = beamer_au_get_tail_samples(_rustInstance);
    if (tailSamples == UINT32_MAX) {
        return INFINITY; // Infinite tail (e.g., reverb that never decays)
    }
    return (NSTimeInterval)tailSamples / _sampleRate;
}

/// Return whether the plugin supports MPE (MIDI Polyphonic Expression).
- (BOOL)supportsMPE {
    // TODO: Query from Rust plugin config
    return NO;
}

/// Return whether the AU provides user presets.
- (BOOL)supportsUserPresets {
    return YES;
}

/// Return the supported channel configurations.
///
/// For effect plugins with [-1,-1] capability (any matching channels),
/// we return explicit pairs to help hosts understand valid configurations.
/// Each pair is [inputChannels, outputChannels] as NSNumber objects.
- (NSArray<NSNumber*>*)channelCapabilities {
    // Return explicit supported configurations
    // For effect plugins: pairs where input == output
    // Common channel counts: 1 (mono), 2 (stereo), 4 (quad), 5 (5.0), 6 (5.1), 7 (6.1), 8 (7.1)
    return @[
        @1, @1,     // Mono
        @2, @2,     // Stereo
        @4, @4,     // Quad
        @5, @5,     // 5.0
        @6, @6,     // 5.1
        @7, @7,     // 6.1
        @8, @8      // 7.1
    ];
}

/// Called when the host wants to change the format for a bus.
/// Return YES to accept the format change.
///
/// For format changes, we accept if the proposed channel count exists in any
/// of our supported configurations for that bus direction. The final validation
/// of the complete configuration happens during allocateRenderResources.
- (BOOL)shouldChangeToFormat:(AVAudioFormat*)format forBus:(AUAudioUnitBus*)bus {
    // Reject formats with too many channels
    if (format.channelCount > BEAMER_AU_MAX_CHANNELS) {
        return NO;
    }

    // Reject non-floating-point formats
    if (format.commonFormat != AVAudioPCMFormatFloat32 &&
        format.commonFormat != AVAudioPCMFormatFloat64) {
        return NO;
    }

    // Check which bus this is
    BOOL isMainInputBus = (_inputBusArray.count > 0 && _inputBusArray[0] == bus);
    BOOL isMainOutputBus = (_outputBusArray.count > 0 && _outputBusArray[0] == bus);

    if (!isMainInputBus && !isMainOutputBus) {
        // Auxiliary bus - accept any valid format
        return YES;
    }

    // Check if the proposed channel count exists in any supported configuration
    uint32_t proposedChannels = (uint32_t)format.channelCount;
    NSArray<NSNumber*>* caps = [self channelCapabilities];

    for (NSUInteger i = 0; i + 1 < caps.count; i += 2) {
        if (isMainInputBus) {
            // Check if this input channel count is supported
            if (caps[i].unsignedIntValue == proposedChannels) {
                return YES;
            }
        } else {
            // Check if this output channel count is supported
            if (caps[i + 1].unsignedIntValue == proposedChannels) {
                return YES;
            }
        }
    }

    // Channel count not in any supported configuration
    return NO;
}

/// Reset the plugin's DSP state (clear delay lines, filter states, etc.).
- (void)reset {
    if (_rustInstance != NULL) {
        beamer_au_reset(_rustInstance);
    }
}

@end

// =============================================================================
// MARK: - AUv2 Factory Interface
// =============================================================================

/// Instance data for the v2 plugin interface.
/// Stores the v2 interface vtable and the AUAudioUnit instance.
typedef struct BeamerAuPlugInInstance {
    AudioComponentPlugInInterface interface;
    AudioComponentDescription desc;
    BeamerAuWrapper* auInstance;
} BeamerAuPlugInInstance;

/// Open callback - creates a new BeamerAuWrapper instance.
static OSStatus BeamerAuOpen(void* self, AudioComponentInstance component) {
    (void)component;
    BeamerAuPlugInInstance* instance = (BeamerAuPlugInInstance*)self;

    NSError* error = nil;
    instance->auInstance = [[BeamerAuWrapper alloc] initWithComponentDescription:instance->desc
                                                                         options:0
                                                                           error:&error];
    if (error || !instance->auInstance) {
        NSLog(@"BeamerAuOpen: Failed to create AU instance: %@", error);
        return kAudioUnitErr_FailedInitialization;
    }

    return noErr;
}

/// Close callback - releases the BeamerAuWrapper instance.
static OSStatus BeamerAuClose(void* self) {
    BeamerAuPlugInInstance* instance = (BeamerAuPlugInInstance*)self;
    instance->auInstance = nil;
    free(instance);
    return noErr;
}

/// Lookup callback - returns NULL to defer to AUAudioUnit v3 API.
/// When NULL is returned, the AU framework uses the modern AUAudioUnit
/// methods (parameterTree, internalRenderBlock, etc.) instead of v2 selectors.
static AudioComponentMethod BeamerAuLookup(SInt16 selector) {
    (void)selector;
    return NULL;
}

/// Register the AUAudioUnit subclass with the framework.
/// Called once on first factory invocation.
static void BeamerAuRegisterSubclass(const AudioComponentDescription* desc) {
    static dispatch_once_t onceToken;
    dispatch_once(&onceToken, ^{
        [BeamerAuWrapper registerSubclass:[BeamerAuWrapper class]
                    asComponentDescription:*desc
                                      name:@"BeamerAuWrapper"
                                   version:0x00020000];
    });
}

/// AUv2 factory function - entry point for .component bundles.
/// This function is specified in Info.plist's factoryFunction key.
/// Returns an AudioComponentPlugInInterface that wraps our AUAudioUnit subclass.
__attribute__((visibility("default")))
void* BeamerAudioUnitFactoryImpl(const AudioComponentDescription* desc) {
    // Register subclass on first call (Rust factory should be ready by now)
    BeamerAuRegisterSubclass(desc);

    BeamerAuPlugInInstance* instance = (BeamerAuPlugInInstance*)malloc(sizeof(BeamerAuPlugInInstance));
    if (!instance) return NULL;

    instance->interface.Open = BeamerAuOpen;
    instance->interface.Close = BeamerAuClose;
    instance->interface.Lookup = BeamerAuLookup;
    instance->interface.reserved = NULL;
    instance->desc = *desc;
    instance->auInstance = nil;

    return &instance->interface;
}
