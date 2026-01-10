// BeamerAuWrapper.h
// Native Objective-C AUAudioUnit subclass header for the Beamer AU framework.
//
// Copyright (c) 2026 Helpermedia. All rights reserved.

#ifndef BEAMER_AU_WRAPPER_H
#define BEAMER_AU_WRAPPER_H

#import <AudioToolbox/AudioToolbox.h>
#import <AVFoundation/AVFoundation.h>

// =============================================================================
// MARK: - BeamerAuWrapper Class
// =============================================================================

/**
 * Native Objective-C AUAudioUnit subclass for Beamer plugins.
 *
 * This class provides a thin wrapper around the Rust plugin implementation.
 * All audio processing, parameter handling, and state management are delegated
 * to Rust via the C-ABI bridge functions declared in BeamerAuBridge.h.
 *
 * ## Architecture
 *
 * Beamer uses AUv2 .component bundles with v3 AUAudioUnit internally:
 * - BeamerAudioUnitFactory() is the v2 entry point (in Info.plist)
 * - registerSubclass() registers this class with the AU framework
 * - Lookup() returns NULL, so the framework uses AUAudioUnit API
 *
 * ## Design Philosophy
 *
 * - **Minimal Objective-C**: The wrapper does minimal work; all heavy lifting
 *   happens in Rust for consistency across plugin formats (AU, VST3).
 *
 * - **Real-time Safety**: The render block never allocates memory or acquires
 *   locks. All buffers are pre-allocated in Rust during allocateRenderResources.
 *
 * - **KVO Compliance**: Bus arrays and parameter tree are cached and return
 *   the same instance each time, as required by Apple's AU documentation.
 *
 * ## Lifecycle
 *
 * 1. **Factory**: Host calls BeamerAudioUnitFactory (registers subclass once)
 * 2. **Open**: Framework creates BeamerAuWrapper via registered subclass
 * 3. **Bus Setup**: Input/output bus arrays configured from Rust plugin config
 * 4. **Parameter Tree**: Built from Rust parameter definitions
 * 5. **Render Resources**: Host calls allocateRenderResourcesAndReturnError:
 * 6. **Processing**: Host calls internalRenderBlock for each audio buffer
 * 7. **Cleanup**: Host calls deallocateRenderResources, then dealloc
 *
 * ## Thread Safety
 *
 * - Initialization/deallocation: Main thread only
 * - allocate/deallocateRenderResources: Main thread only
 * - internalRenderBlock: Real-time audio thread only
 * - Parameter get/set: Any thread (uses atomics in Rust)
 * - State save/load: Main thread only
 */
@interface BeamerAuWrapper : AUAudioUnit <AUAudioUnitFactory>

// No public methods beyond what AUAudioUnit provides.
// All customization happens through the standard AUAudioUnit interface.

// AUAudioUnitFactory protocol method (for future AUv3 App Extension support)
- (nullable AUAudioUnit *)createAudioUnitWithComponentDescription:(AudioComponentDescription)desc
                                                            error:(NSError **)error;

@end

// =============================================================================
// MARK: - AUv2 Factory Function
// =============================================================================

/**
 * AUv2 factory function - entry point for .component bundles.
 *
 * This function is specified in Info.plist's factoryFunction key.
 * It registers the AUAudioUnit subclass on first call, then returns
 * an AudioComponentPlugInInterface with Lookup() returning NULL to
 * delegate all operations to the AUAudioUnit API.
 *
 * @param desc The component description from Info.plist.
 * @return AudioComponentPlugInInterface* for the AU framework.
 */
__attribute__((visibility("default")))
void* BeamerAudioUnitFactoryImpl(const AudioComponentDescription* desc);

#endif // BEAMER_AU_WRAPPER_H
