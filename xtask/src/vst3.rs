//! VST3 plugin bundling support.
//!
//! This module handles creating and installing VST3 plugin bundles on macOS.

use std::fs;
use std::path::{Path, PathBuf};

use crate::util::{copy_dir_all, shorten_path};

/// Creates a VST3 bundle from a compiled dylib.
///
/// This creates the standard macOS VST3 bundle structure:
/// ```text
/// PluginName.vst3/
/// └── Contents/
///     ├── Info.plist
///     ├── PkgInfo
///     ├── MacOS/
///     │   └── PluginName (binary)
///     └── Resources/
/// ```
pub fn bundle_vst3(
    package: &str,
    target_dir: &Path,
    dylib_path: &Path,
    install: bool,
    verbose: bool,
) -> Result<(), String> {
    // Create bundle name (convert to CamelCase and add .vst3)
    let bundle_name = to_vst3_bundle_name(package);
    let bundle_dir = target_dir.join(&bundle_name);

    // Create bundle directory structure
    let contents_dir = bundle_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");

    crate::status!("  Creating VST3 bundle...");
    crate::verbose!(verbose, "    Path: {}", bundle_dir.display());

    // Clean up existing bundle
    if bundle_dir.exists() {
        fs::remove_dir_all(&bundle_dir).map_err(|e| format!("Failed to remove old bundle: {}", e))?;
    }

    // Create directories
    fs::create_dir_all(&macos_dir).map_err(|e| format!("Failed to create MacOS dir: {}", e))?;
    fs::create_dir_all(&resources_dir)
        .map_err(|e| format!("Failed to create Resources dir: {}", e))?;

    // Copy dylib
    let plugin_binary = macos_dir.join(bundle_name.trim_end_matches(".vst3"));
    fs::copy(dylib_path, &plugin_binary)
        .map_err(|e| format!("Failed to copy dylib: {}", e))?;

    // Create Info.plist
    let info_plist = create_vst3_info_plist(package, &bundle_name);
    fs::write(contents_dir.join("Info.plist"), info_plist)
        .map_err(|e| format!("Failed to write Info.plist: {}", e))?;

    // Create PkgInfo
    fs::write(contents_dir.join("PkgInfo"), "BNDL????")
        .map_err(|e| format!("Failed to write PkgInfo: {}", e))?;

    // Install if requested
    if install {
        install_vst3(&bundle_dir, &bundle_name, verbose)?;
    } else {
        crate::status!("✓ {}", bundle_name);
    }

    Ok(())
}

/// Converts a package name to a VST3 bundle name.
///
/// Examples:
/// - "gain" -> "BeamerGain.vst3"
/// - "midi-transform" -> "BeamerMidiTransform.vst3"
fn to_vst3_bundle_name(package: &str) -> String {
    // Convert package name to CamelCase bundle name with Beamer prefix
    // e.g., "gain" -> "BeamerGain.vst3", "midi-transform" -> "BeamerMidiTransform.vst3"
    let name: String = package
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().chain(chars).collect(),
            }
        })
        .collect();
    format!("Beamer{}.vst3", name)
}

/// Creates the Info.plist content for a VST3 bundle.
fn create_vst3_info_plist(package: &str, bundle_name: &str) -> String {
    let executable_name = bundle_name.trim_end_matches(".vst3");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>English</string>
    <key>CFBundleExecutable</key>
    <string>{}</string>
    <key>CFBundleIdentifier</key>
    <string>com.beamer.{}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{}</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleVersion</key>
    <string>0.2.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.2.0</string>
</dict>
</plist>
"#,
        executable_name, package, executable_name
    )
}

/// Installs a VST3 bundle to the user's plugin directory.
///
/// The bundle is copied to `~/Library/Audio/Plug-Ins/VST3/`.
fn install_vst3(bundle_dir: &Path, bundle_name: &str, verbose: bool) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let vst3_dir = PathBuf::from(home)
        .join("Library")
        .join("Audio")
        .join("Plug-Ins")
        .join("VST3");

    // Create VST3 directory if needed
    fs::create_dir_all(&vst3_dir).map_err(|e| format!("Failed to create VST3 dir: {}", e))?;

    let dest = vst3_dir.join(bundle_name);

    // Remove existing installation
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("Failed to remove old installation: {}", e))?;
    }

    // Copy bundle
    copy_dir_all(bundle_dir, &dest)?;

    crate::verbose!(verbose, "    Installed to: {}", dest.display());
    crate::status!("✓ {} → {}", bundle_name, shorten_path(&dest));
    Ok(())
}
