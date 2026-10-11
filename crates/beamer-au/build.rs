//! Build script for beamer-au.
//!
//! NOTE: ObjC compilation is now handled by xtask with plugin-specific class names.
//! This build script only links required frameworks.

fn main() {
    // Nothing else in the crate is an input of this script.
    println!("cargo:rerun-if-changed=build.rs");

    // Build scripts run on the host, where `cfg` checks the host OS, so decide
    // by the compile target instead.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    // Link required frameworks.
    println!("cargo:rustc-link-lib=framework=AudioToolbox");
    println!("cargo:rustc-link-lib=framework=AVFoundation");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=CoreAudio");
    println!("cargo:rustc-link-lib=framework=CoreAudioKit");
}
