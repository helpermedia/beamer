// BeamerAuWrapper.h
// Native Objective-C AUAudioUnit subclass header for the Beamer AU framework.
//
// This header declares the public interface for BeamerAuWrapper and the
// BeamerAudioUnitFactory function that macOS calls to instantiate the AU.
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
 * 1. **Instantiation**: Factory function creates the instance via alloc/init
 * 2. **Bus Setup**: Input/output bus arrays are configured from Rust plugin config
 * 3. **Parameter Tree**: Built from Rust parameter definitions
 * 4. **Render Resources**: Host calls allocateRenderResourcesAndReturnError:
 * 5. **Processing**: Host calls internalRenderBlock for each audio buffer
 * 6. **Cleanup**: Host calls deallocateRenderResources, then dealloc
 *
 * ## Thread Safety
 *
 * - Initialization/deallocation: Main thread only
 * - allocate/deallocateRenderResources: Main thread only
 * - internalRenderBlock: Real-time audio thread only
 * - Parameter get/set: Any thread (uses atomics in Rust)
 * - State save/load: Main thread only
 */
@interface BeamerAuWrapper : AUAudioUnit

// No public methods beyond what AUAudioUnit provides.
// All customization happens through the standard AUAudioUnit interface.

@end

// =============================================================================
// MARK: - Factory Function
// =============================================================================

/**
 * Audio Unit factory function called by macOS to create plugin instances.
 *
 * This is the entry point that macOS uses when instantiating the AU.
 * The function name must match the `factoryFunction` key in Info.plist.
 *
 * ## Info.plist Configuration
 *
 * ```xml
 * <key>AudioComponents</key>
 * <array>
 *     <dict>
 *         <key>factoryFunction</key>
 *         <string>BeamerAudioUnitFactory</string>
 *         <!-- Other AU configuration keys... -->
 *     </dict>
 * </array>
 * ```
 *
 * ## Thread Safety
 *
 * Called from the main thread by the Audio Unit framework.
 *
 * ## Memory Management
 *
 * - Returns a retained (+1) pointer using __bridge_retained
 * - The Audio Unit framework takes ownership and will release it
 * - On failure, returns NULL
 *
 * @param desc Pointer to AudioComponentDescription identifying which AU to create.
 *             This contains the type (aufx, aumu, aumi), subtype, and manufacturer.
 *
 * @return Retained pointer to the new AUAudioUnit instance, or NULL on failure.
 */
void* BeamerAudioUnitFactory(const AudioComponentDescription* desc);

#endif // BEAMER_AU_WRAPPER_H
