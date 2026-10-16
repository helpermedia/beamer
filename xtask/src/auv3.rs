//! AUv3 app extension bundling.
//!
//! This module handles creating and installing AUv3 plugin bundles on macOS.
//! AUv3 plugins are packaged as app extensions (.appex) within a container app (.app).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::build::{clang_arches, get_version_info};
use crate::util::{au_tags, codesign_bundle, combine_or_rename_binaries, detect_au_component_info, detect_subcategories, install_bundle, plist_strings, refresh_au_registry, shorten_path, to_au_bundle_name, uninstall_bundle, Arch, AuCodes, AuNames, PathExt, MACOS_DEPLOYMENT_TARGET, MACOS_MIN_VERSION_FLAG};
use crate::AppexPlistConfig;

/// Where an installed app lives, relative to the home directory.
const INSTALL_DIR: &[&str] = &["Applications"];

/// Creates an AUv3 app extension bundle from a compiled dylib.
///
/// This creates the standard macOS AUv3 bundle structure:
/// ```text
/// PluginName.app/
/// └── Contents/
///     ├── Info.plist
///     ├── PkgInfo
///     ├── MacOS/
///     │   └── PluginName (host app binary)
///     ├── Frameworks/
///     │   └── PluginNameAU.framework/
///     ├── PlugIns/
///     │   └── PluginName.appex/
///     │       └── Contents/
///     │           ├── Info.plist
///     │           ├── MacOS/
///     │           │   └── PluginName (appex binary)
///     │           └── Resources/
///     └── Resources/
/// ```
pub fn bundle_auv3(
    package: &str,
    target_dir: &Path,
    dylib_path: &Path,
    install: bool,
    workspace_root: &Path,
    arch: Arch,
    verbose: bool,
) -> Result<(), String> {
    // Get the version of the plugin package.
    let package_info = crate::util::find_package(package, workspace_root)?;
    let (version_string, version_int) = get_version_info(&package_info.version)?;

    let bundle_name = to_au_bundle_name(package);
    let bundle_dir = target_dir.join(&bundle_name);
    let contents_dir = bundle_dir.join("Contents");
    let app_resources_dir = contents_dir.join("Resources");
    let plugins_dir = contents_dir.join("PlugIns");

    crate::status!("  Creating AUv3 app extension...");
    crate::verbose!(verbose, "    Path: {}", bundle_dir.display());

    // Clean up existing bundle
    if bundle_dir.exists() {
        fs::remove_dir_all(&bundle_dir).map_err(|e| format!("Failed to remove old bundle: {}", e))?;
    }

    // Create app directories
    let app_macos_dir = contents_dir.join("MacOS");
    let frameworks_dir = contents_dir.join("Frameworks");
    fs::create_dir_all(&app_macos_dir).map_err(|e| format!("Failed to create app MacOS dir: {}", e))?;
    fs::create_dir_all(&app_resources_dir).map_err(|e| format!("Failed to create app Resources dir: {}", e))?;
    fs::create_dir_all(&plugins_dir).map_err(|e| format!("Failed to create PlugIns dir: {}", e))?;
    fs::create_dir_all(&frameworks_dir).map_err(|e| format!("Failed to create Frameworks dir: {}", e))?;

    // Create the .appex bundle structure
    let executable_name = bundle_name.trim_end_matches(".app");
    let appex_name = format!("{}.appex", executable_name);
    let appex_dir = plugins_dir.join(&appex_name);
    let appex_contents_dir = appex_dir.join("Contents");
    let appex_macos_dir = appex_contents_dir.join("MacOS");
    let appex_resources_dir = appex_contents_dir.join("Resources");

    fs::create_dir_all(&appex_macos_dir).map_err(|e| format!("Failed to create appex MacOS dir: {}", e))?;
    fs::create_dir_all(&appex_resources_dir).map_err(|e| format!("Failed to create appex Resources dir: {}", e))?;

    // Create framework bundle for in-process AU loading on macOS.
    // Use versioned framework structure (standard macOS framework layout):
    // Framework.framework/
    // ├── Framework -> Versions/Current/Framework  (symlink)
    // ├── Resources -> Versions/Current/Resources  (symlink)
    // └── Versions/
    //     ├── A/
    //     │   ├── Framework (binary)
    //     │   ├── Resources/
    //     │   │   └── Info.plist
    //     │   └── _CodeSignature/
    //     └── Current -> A  (symlink)
    let framework_name = format!("{}AU", executable_name);
    let framework_bundle_id = format!("com.beamer.{}.framework", package);
    let framework_dir = frameworks_dir.join(format!("{}.framework", framework_name));

    // Create versioned directory structure
    let versions_dir = framework_dir.join("Versions");
    let version_a_dir = versions_dir.join("A");
    let version_a_resources = version_a_dir.join("Resources");
    fs::create_dir_all(&version_a_resources)
        .map_err(|e| format!("Failed to create framework Versions/A/Resources dir: {}", e))?;

    // Copy dylib to Versions/A/
    let framework_binary = version_a_dir.join(&framework_name);
    fs::copy(dylib_path, &framework_binary)
        .map_err(|e| format!("Failed to copy dylib to framework: {}", e))?;

    // Fix dylib install name to use @rpath with versioned path
    let install_name_status = Command::new("install_name_tool")
        .args(["-id", &format!("@rpath/{}.framework/Versions/A/{}", framework_name, framework_name),
               &framework_binary.to_string_lossy()])
        .status()
        .map_err(|e| format!("Failed to run install_name_tool: {}", e))?;

    if !install_name_status.success() {
        return Err("Failed to set the install name of the framework binary".to_string());
    }

    // Create framework Info.plist in Versions/A/Resources/
    let framework_plist = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>{framework_name}</string>
    <key>CFBundleIdentifier</key>
    <string>{bundle_id}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{framework_name}</string>
    <key>CFBundlePackageType</key>
    <string>FMWK</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
</dict>
</plist>
"#, framework_name = framework_name, bundle_id = framework_bundle_id, version = version_string);
    fs::write(version_a_resources.join("Info.plist"), framework_plist)
        .map_err(|e| format!("Failed to write framework Info.plist: {}", e))?;

    // Create symlinks for versioned framework structure
    // Versions/Current -> A
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let current_link = versions_dir.join("Current");
        let _ = fs::remove_file(&current_link); // Remove if exists
        symlink("A", &current_link)
            .map_err(|e| format!("Failed to create Versions/Current symlink: {}", e))?;

        // Framework root symlinks
        // Framework -> Versions/Current/Framework
        let binary_link = framework_dir.join(&framework_name);
        let _ = fs::remove_file(&binary_link);
        symlink(format!("Versions/Current/{}", framework_name), &binary_link)
            .map_err(|e| format!("Failed to create framework binary symlink: {}", e))?;

        // Resources -> Versions/Current/Resources
        let resources_link = framework_dir.join("Resources");
        let _ = fs::remove_file(&resources_link);
        symlink("Versions/Current/Resources", &resources_link)
            .map_err(|e| format!("Failed to create Resources symlink: {}", e))?;
    }

    crate::verbose!(verbose, "    Created framework: {}.framework", framework_name);

    // Build appex executable - thin wrapper that links the framework
    let appex_binary_path = appex_macos_dir.join(executable_name);

    // Generate plugin-specific appex_stub.m (no main function - uses NSExtensionMain)
    let appex_stub_source = generate_appex_stub_source(package);
    let appex_gen_dir = workspace_root.join("target/au-gen").join(package);
    fs::create_dir_all(&appex_gen_dir)
        .map_err(|e| format!("Failed to create appex gen dir: {}", e))?;
    let appex_stub_path = appex_gen_dir.join("appex_stub.m");
    fs::write(&appex_stub_path, appex_stub_source)
        .map_err(|e| format!("Failed to write appex_stub.m: {}", e))?;

    // Build appex executable with appropriate architecture(s)
    let arches = clang_arches(arch)?;

    let arch_str = if arches.len() > 1 { "universal" } else { arches[0] };
    crate::verbose!(verbose, "    Building appex executable ({})...", arch_str);

    let mut built_paths: Vec<PathBuf> = Vec::new();

    for target_arch in &arches {
        let appex_arch_path = bundle_dir.join(format!("{}_{}", executable_name, target_arch));
        let clang_status = Command::new("clang")
            .args([
                "-arch", target_arch,
                MACOS_MIN_VERSION_FLAG,
                "-fobjc-arc",
                "-fmodules",
                "-framework", "Foundation",
                "-framework", "AudioToolbox",
                "-framework", "AVFoundation",
                "-framework", "CoreAudio",
                "-framework", "CoreAudioKit",
                "-framework", "WebKit",
                "-F", frameworks_dir.to_str_safe()?,
                "-framework", &framework_name,
                "-Wl,-rpath,@loader_path/../../../../Frameworks",
                "-Wl,-e,_NSExtensionMain",  // Use Apple's standard extension entry point
                "-o", appex_arch_path.to_str_safe()?,
                appex_stub_path.to_str_safe()?,
            ])
            .status()
            .map_err(|e| format!("Failed to run clang for {}: {}", target_arch, e))?;

        if !clang_status.success() {
            return Err(format!("Failed to build appex for {}", target_arch));
        }
        built_paths.push(appex_arch_path);
    }

    combine_or_rename_binaries(&built_paths, &appex_binary_path, true)?;

    crate::verbose!(verbose, "    Appex executable built ({})", arch_str);

    // Auto-detect component type, manufacturer and subtype from plugin source
    let codes = AuCodes::detect(package, &package_info.dir);
    let names = AuNames::new(package, &codes);
    let (_, _, _, detected_plugin_name, detected_vendor_name, has_gui) = detect_au_component_info(&package_info.dir);
    crate::verbose!(
        verbose,
        "    Detected: {} (manufacturer: {}, subtype: {})",
        codes.component_type,
        codes.manufacturer,
        codes.subtype
    );
    if let Some(ref name) = detected_plugin_name {
        crate::verbose!(verbose, "    Plugin name: {}", name);
    }
    if let Some(ref vendor) = detected_vendor_name {
        crate::verbose!(verbose, "    Vendor: {}", vendor);
    }

    // Create appex Info.plist with NSExtension (out-of-process/XPC mode)
    let appex_info_plist = create_appex_info_plist(&AppexPlistConfig {
        package,
        executable_name,
        codes: &codes,
        subcategories: &detect_subcategories(&package_info.dir),
        extension_class: &names.extension_class,
        framework_bundle_id: &framework_bundle_id,
        version_string: &version_string,
        version_int,
        plugin_name: detected_plugin_name.as_deref(),
        vendor_name: detected_vendor_name.as_deref(),
        has_gui,
    });
    fs::write(appex_contents_dir.join("Info.plist"), appex_info_plist)
        .map_err(|e| format!("Failed to write appex Info.plist: {}", e))?;

    // Create container app Info.plist
    let app_info_plist = create_app_info_plist(package, executable_name, &version_string);
    fs::write(contents_dir.join("Info.plist"), app_info_plist)
        .map_err(|e| format!("Failed to write app Info.plist: {}", e))?;

    // Create PkgInfo for app
    fs::write(contents_dir.join("PkgInfo"), "APPL????")
        .map_err(|e| format!("Failed to write PkgInfo: {}", e))?;

    // Build host app executable from C stub.
    // This is a minimal stub that triggers pluginkit registration when launched.
    // The app is marked LSBackgroundOnly so it exits immediately after registration.
    crate::verbose!(verbose, "    Building host app executable ({})...", arch_str);

    let stub_main_path = workspace_root.join("crates/beamer-au/objc/stub_main.c");
    let host_binary_dst = app_macos_dir.join(executable_name);

    let mut host_built_paths: Vec<PathBuf> = Vec::new();

    for target_arch in &arches {
        let host_arch_path = bundle_dir.join(format!("{}_{}", executable_name, target_arch));
        let clang_status = Command::new("clang")
            .args([
                "-arch", target_arch,
                MACOS_MIN_VERSION_FLAG,
                "-framework", "Foundation",
                "-o", host_arch_path.to_str_safe()?,
                stub_main_path.to_str_safe()?,
            ])
            .status()
            .map_err(|e| format!("Failed to run clang for {}: {}", target_arch, e))?;

        if !clang_status.success() {
            return Err(format!("Failed to build host app for {}", target_arch));
        }
        host_built_paths.push(host_arch_path);
    }

    combine_or_rename_binaries(&host_built_paths, &host_binary_dst, true)?;

    crate::verbose!(verbose, "    Host app built ({})", arch_str);

    // Code sign framework first, then appex, then container app
    crate::verbose!(verbose, "    Signing...");
    codesign_bundle(&framework_dir, None, "Framework", verbose)?;

    let entitlements_path = workspace_root.join("crates/beamer-au/resources/appex.entitlements");
    codesign_bundle(&appex_dir, Some(&entitlements_path), "Appex", verbose)?;

    codesign_bundle(&bundle_dir, None, "Container app", verbose)?;

    // Install if requested
    if install {
        install_auv3(&bundle_dir, &bundle_name, verbose)?;
    } else {
        crate::status!("  {}", bundle_name);
    }

    Ok(())
}

/// Generates the appex stub source code.
///
/// This is a minimal ObjC file that gets compiled into the appex executable.
/// The actual entry point is NSExtensionMain, set via linker flag.
fn generate_appex_stub_source(plugin_name: &str) -> String {
    format!(r#"// Auto-generated appex stub for {plugin_name}
// DO NOT EDIT - Generated by xtask
//
// This file does NOT contain main(). The entry point is NSExtensionMain,
// which is set via the -e _NSExtensionMain linker flag. NSExtensionMain
// handles all XPC setup for app extensions automatically.
//
// The framework is linked via -framework flag, which ensures it's loaded.
// NSExtensionMain reads NSExtensionPrincipalClass from Info.plist and
// instantiates the extension class from the framework.

#import <Foundation/Foundation.h>

// Minimal stub - just needs to compile to create an object file.
// The -framework link ensures our AU framework is loaded at runtime.
"#, plugin_name = plugin_name)
}

/// Creates the Info.plist content for the container app.
///
/// The container app is a minimal stub that triggers pluginkit registration.
/// It's marked as LSBackgroundOnly so it doesn't show in the Dock.
fn create_app_info_plist(package: &str, executable_name: &str, version: &str) -> String {
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
    <string>APPL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>{min_os}</string>
    <key>LSBackgroundOnly</key>
    <true/>
</dict>
</plist>
"#,
        min_os = MACOS_DEPLOYMENT_TARGET,
        executable = executable_name,
        package = package,
        version = version
    )
}

/// Creates the Info.plist content for the appex with NSExtension.
fn create_appex_info_plist(config: &AppexPlistConfig) -> String {
    // Get appropriate tags based on component type
    let tags = plist_strings(&au_tags(&config.codes.component_type, config.subcategories), "                        ");

    // Use AudioUnit-UI extension point for plugins with a GUI
    let extension_point = if config.has_gui {
        "com.apple.AudioUnit-UI"
    } else {
        "com.apple.AudioUnit"
    };

    // Create the plugin display name from vendor and plugin name
    // Format: "Vendor: Plugin Name" (e.g., "Beamer Framework: Beamer Synthesizer")
    let plugin_display_name = match (config.vendor_name, config.plugin_name) {
        (Some(vendor), Some(name)) => format!("{}: {}", vendor, name),
        (None, Some(name)) => format!("Beamer: {}", name),
        (Some(vendor), None) => format!("{}: {}", vendor, config.executable_name),
        (None, None) => format!("Beamer: {}", config.executable_name),
    };

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
    <string>com.beamer.{package}.audiounit</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{executable}</string>
    <key>CFBundleDisplayName</key>
    <string>{display_name}</string>
    <key>CFBundlePackageType</key>
    <string>XPC!</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleSupportedPlatforms</key>
    <array>
        <string>MacOSX</string>
    </array>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>{min_os}</string>
    <key>NSExtension</key>
    <dict>
        <key>NSExtensionPointIdentifier</key>
        <string>{extension_point}</string>
        <key>NSExtensionPrincipalClass</key>
        <string>{extension_class}</string>
        <key>NSExtensionAttributes</key>
        <dict>
            <key>AudioComponents</key>
            <array>
                <dict>
                    <key>type</key>
                    <string>{component_type}</string>
                    <key>subtype</key>
                    <string>{subtype}</string>
                    <key>manufacturer</key>
                    <string>{manufacturer}</string>
                    <key>name</key>
                    <string>{plugin_display_name}</string>
                    <key>sandboxSafe</key>
                    <true/>
                    <key>tags</key>
                    <array>
                        {tags}
                    </array>
                    <key>version</key>
                    <integer>{version_int}</integer>
                    <key>description</key>
                    <string>{executable} Audio Unit</string>
                </dict>
            </array>
            <key>AudioComponentBundle</key>
            <string>{framework_bundle_id}</string>
        </dict>
    </dict>
</dict>
</plist>
"#,
        min_os = MACOS_DEPLOYMENT_TARGET,
        executable = config.executable_name,
        package = config.package,
        extension_point = extension_point,
        extension_class = config.extension_class,
        manufacturer = config.codes.manufacturer,
        component_type = config.codes.component_type,
        subtype = config.codes.subtype,
        tags = tags,
        framework_bundle_id = config.framework_bundle_id,
        version = config.version_string,
        version_int = config.version_int,
        plugin_display_name = plugin_display_name,
        display_name = config.plugin_name.unwrap_or(config.executable_name),
    )
}

/// Installs an AUv3 bundle to the user's Applications directory.
///
/// AUv3 app extensions must be installed as apps (not in the Components folder).
/// The system discovers them when the containing app is launched.
fn install_auv3(bundle_dir: &Path, bundle_name: &str, verbose: bool) -> Result<(), String> {
    let dest = install_bundle(bundle_dir, bundle_name, INSTALL_DIR, verbose)?;

    // Launch the app briefly to trigger pluginkit registration.
    // AUv3 extensions are registered when their containing app is first launched.
    crate::verbose!(verbose, "    Registering Audio Unit extension...");
    let _ = Command::new("open")
        .arg(&dest)
        .status();

    // Give the system a moment to register the extension
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Terminate the background app (it has LSBackgroundOnly so it won't show UI)
    let executable_name = bundle_name.trim_end_matches(".app");
    let killall_app = Command::new("killall")
        .arg(executable_name)
        .output();

    if verbose {
        if let Ok(output) = killall_app {
            let stderr = String::from_utf8_lossy(&output.stderr);
            for line in stderr.lines() {
                crate::verbose!(verbose, "    {}", line);
            }
        }
    }

    // Also refresh AU cache
    refresh_au_registry(verbose);
    crate::verbose!(verbose, "    Audio Unit registered");
    crate::status!("  {} -> {}", bundle_name, shorten_path(&dest));

    Ok(())
}

/// Removes an installed AUv3 app and returns whether there was one.
///
/// The caller refreshes the Audio Unit registry afterwards.
pub fn uninstall_auv3(package: &str) -> Result<bool, String> {
    // The system drops the extension's registration by itself once the app is
    // gone, so removing the app is all it takes here.
    uninstall_bundle(&to_au_bundle_name(package), INSTALL_DIR)
}
