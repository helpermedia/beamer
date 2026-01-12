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

        // Compile APPEX principal class
        cc::Build::new()
            .file("objc/BeamerAuExtension.m")
            .flag("-fobjc-arc")
            .flag("-fmodules")
            .compile("beamer_au_extension");

        // Link required frameworks
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreAudio");

        // Export the output directory so dependent crates can find the static libraries.
        // Due to `links = "beamer_au_native"` in Cargo.toml, this becomes
        // DEP_BEAMER_AU_NATIVE_OUT_DIR in dependent build scripts.
        let out_dir = std::env::var("OUT_DIR").unwrap();
        println!("cargo:out_dir={}", out_dir);

        // Rerun if ObjC files change
        println!("cargo:rerun-if-changed=objc/BeamerAuWrapper.m");
        println!("cargo:rerun-if-changed=objc/BeamerAuExtension.m");
        println!("cargo:rerun-if-changed=objc/BeamerAuBridge.h");
    }
}
