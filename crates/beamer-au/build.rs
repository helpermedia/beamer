//! Build script for beamer-au.
//!
//! Compiles the native Objective-C AUAudioUnit wrapper on macOS.

fn main() {
    #[cfg(target_os = "macos")]
    {
        // Compile Objective-C wrapper
        cc::Build::new()
            .file("objc/BeamerAuWrapper.m")
            .flag("-fobjc-arc") // Enable Automatic Reference Counting
            .flag("-fmodules") // Enable module imports
            .compile("beamer_au_objc");

        // Link required frameworks
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreAudio");

        // Export BeamerAudioUnitFactory symbol for AUv2 .component bundles.
        // This is required because macOS AU hosts look up this symbol by name
        // (specified in Info.plist's factoryFunction key).
        println!("cargo:rustc-cdylib-link-arg=-Wl,-exported_symbol,_BeamerAudioUnitFactory");

        // Rerun if ObjC files change
        println!("cargo:rerun-if-changed=objc/BeamerAuWrapper.m");
        println!("cargo:rerun-if-changed=objc/BeamerAuBridge.h");
    }
}
