// BeamerAuExtension.m - AUv3 App Extension Principal Class
// For NON-UI Audio Units (headless processing only)

#import <AudioToolbox/AudioToolbox.h>
#import <Foundation/Foundation.h>
#include "BeamerAuBridge.h"

// Forward declare the wrapper class
@class BeamerAuWrapper;

// External declaration for BeamerAuWrapper
@interface BeamerAuWrapper : AUAudioUnit
- (instancetype)initWithComponentDescription:(AudioComponentDescription)componentDescription
                                     options:(AudioComponentInstantiationOptions)options
                                       error:(NSError**)outError;
@end

@interface BeamerAuExtension : NSObject <AUAudioUnitFactory>
@end

@implementation BeamerAuExtension

// Required by NSExtensionRequestHandling protocol (inherited by AUAudioUnitFactory)
// Audio Units don't use this method - it's for other extension types
- (void)beginRequestWithExtensionContext:(NSExtensionContext *)context {
    // Not used for Audio Unit extensions - do nothing
    (void)context;
}

- (nullable AUAudioUnit *)createAudioUnitWithComponentDescription:(AudioComponentDescription)desc
                                                            error:(NSError **)error {
    // Ensure Rust factory is registered
    if (!beamer_au_ensure_factory_registered()) {
        if (error) {
            *error = [NSError errorWithDomain:NSOSStatusErrorDomain
                                         code:kAudioUnitErr_FailedInitialization
                                     userInfo:@{NSLocalizedDescriptionKey: @"Failed to register plugin factory"}];
        }
        return nil;
    }

    // Create BeamerAuWrapper instance
    // APPEX instantiation - options parameter is 0 since we handle it directly
    return [[BeamerAuWrapper alloc] initWithComponentDescription:desc
                                                         options:0
                                                           error:error];
}

@end

// This function is called from Rust to force the linker to include this file.
// Without it, the ObjC classes would be stripped since nothing references them.
void beamer_au_appex_force_link(void) {
    // Reference the class to ensure it's not stripped
    (void)[BeamerAuExtension class];
}

// Factory function for AudioComponent in-process loading.
// This is called by the AudioComponent system when the host requests in-process instantiation.
// The function name must match the 'factoryFunction' key in Info.plist.
void* BeamerAuExtensionFactory(const AudioComponentDescription* desc) {
    // Ensure Rust factory is registered
    if (!beamer_au_ensure_factory_registered()) {
        return NULL;
    }

    // Create and return BeamerAuWrapper instance
    NSError* error = nil;
    BeamerAuWrapper* wrapper = [[BeamerAuWrapper alloc] initWithComponentDescription:*desc
                                                                             options:0
                                                                               error:&error];
    if (error) {
        NSLog(@"BeamerAuExtensionFactory: Error creating AU: %@", error);
        return NULL;
    }

    // Return retained pointer (caller is responsible for releasing)
    return (__bridge_retained void*)wrapper;
}
