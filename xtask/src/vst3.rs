//! VST3 plugin bundling support.
//!
//! This module handles creating and installing VST3 plugin bundles on macOS.

use std::fs;
use std::path::Path;

use crate::build::get_version_info;
use crate::util::{install_bundle, shorten_path, to_vst3_bundle_name};

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
    workspace_root: &Path,
    verbose: bool,
) -> Result<(), String> {
    // Get version from Cargo.toml
    let (version_string, _version_int) = get_version_info(workspace_root)?;

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
    let info_plist = create_vst3_info_plist(package, &bundle_name, &version_string);
    fs::write(contents_dir.join("Info.plist"), info_plist)
        .map_err(|e| format!("Failed to write Info.plist: {}", e))?;

    // Create PkgInfo
    fs::write(contents_dir.join("PkgInfo"), "BNDL????")
        .map_err(|e| format!("Failed to write PkgInfo: {}", e))?;

    // Install if requested
    if install {
        install_vst3(&bundle_dir, &bundle_name, verbose)?;
    } else {
        crate::status!("  {}", bundle_name);
    }

    Ok(())
}

/// Creates the Info.plist content for a VST3 bundle.
fn create_vst3_info_plist(package: &str, bundle_name: &str, version: &str) -> String {
    let executable_name = bundle_name.trim_end_matches(".vst3");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>English</string>
    <key>CFBundleExecutable</key>
    <string>{executable}</string>
    <key>CFBundleIdentifier</key>
    <string>com.beamer.{package}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{executable}</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
</dict>
</plist>
"#,
        executable = executable_name,
        package = package,
        version = version
    )
}

/// Installs a VST3 bundle to the user's plugin directory.
///
/// The bundle is copied to `~/Library/Audio/Plug-Ins/VST3/`.
fn install_vst3(bundle_dir: &Path, bundle_name: &str, verbose: bool) -> Result<(), String> {
    let dest = install_bundle(
        bundle_dir,
        bundle_name,
        &["Library", "Audio", "Plug-Ins", "VST3"],
        verbose,
    )?;
    crate::status!("  {} -> {}", bundle_name, shorten_path(&dest));
    Ok(())
}
