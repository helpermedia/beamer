//! Build script for beamer-core: the bridge to the macOS system log.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/os_log.c");

    // Build scripts run on the host, where `cfg` checks the host OS, so decide
    // by the compile target instead.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }

    cc::Build::new().file("src/os_log.c").compile("beamer_os_log");
}
