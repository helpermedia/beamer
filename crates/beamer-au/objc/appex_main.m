// appex_main.m - Entry point for AUv3 app extension executable
//
// App extensions require a Mach-O executable (not a dylib). This provides
// the main() entry point and runs an NSRunLoop for XPC communication.
//
// The actual AU implementation (BeamerAuExtension, BeamerAuWrapper) is linked
// from the plugin's static/dynamic library.

#import <Foundation/Foundation.h>

// Forward declaration - this is defined in BeamerAuExtension.m
extern void beamer_au_appex_force_link(void);

int main(int argc, char *argv[]) {
    @autoreleasepool {
        // Ensure ObjC classes are linked and available to the runtime
        beamer_au_appex_force_link();

        // App extensions communicate via XPC. The system handles loading
        // our NSExtensionPrincipalClass (BeamerAuExtension) which implements
        // the AUAudioUnitFactory protocol to create AU instances.
        //
        // We run the main run loop to keep the process alive and handle
        // XPC messages from host applications.
        [[NSRunLoop mainRunLoop] run];
    }
    return 0;
}
