//! AUv3 app extension bundling.
//!
//! This module handles creating and installing AUv3 plugin bundles on macOS.
//! AUv3 plugins are packaged as app extensions (.appex) within a container app (.app).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::auv2::detect_au_component_info;
use crate::build::get_version_info;
use crate::util::{copy_dir_all, shorten_path, to_pascal_case, Arch};
use crate::AppexPlistConfig;

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
    // Get version from Cargo.toml
    let (version_string, version_int) = get_version_info(workspace_root)?;

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
    let _ = Command::new("install_name_tool")
        .args(["-id", &format!("@rpath/{}.framework/Versions/A/{}", framework_name, framework_name),
               framework_binary.to_str().unwrap()])
        .status();

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
    let arches = match arch {
        Arch::Universal => vec!["x86_64", "arm64"],
        Arch::Native => vec![if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" }],
        Arch::Arm64 => vec!["arm64"],
        Arch::X86_64 => vec!["x86_64"],
    };

    let arch_str = if arches.len() > 1 { "universal" } else { arches[0] };
    crate::verbose!(verbose, "    Building appex executable ({})...", arch_str);

    let mut built_paths: Vec<PathBuf> = Vec::new();

    for target_arch in &arches {
        let appex_arch_path = bundle_dir.join(format!("{}_{}", executable_name, target_arch));
        let clang_status = Command::new("clang")
            .args([
                "-arch", target_arch,
                "-fobjc-arc",
                "-fmodules",
                "-framework", "Foundation",
                "-framework", "AudioToolbox",
                "-framework", "AVFoundation",
                "-framework", "CoreAudio",
                "-F", frameworks_dir.to_str().unwrap(),
                "-framework", &framework_name,
                "-Wl,-rpath,@loader_path/../../../../Frameworks",
                "-Wl,-e,_NSExtensionMain",  // Use Apple's standard extension entry point
                "-o", appex_arch_path.to_str().unwrap(),
                appex_stub_path.to_str().unwrap(),
            ])
            .status()
            .map_err(|e| format!("Failed to run clang for {}: {}", target_arch, e))?;

        if !clang_status.success() {
            return Err(format!("Failed to build appex for {}", target_arch));
        }
        built_paths.push(appex_arch_path);
    }

    if built_paths.len() == 1 {
        // Single architecture - just rename
        fs::rename(&built_paths[0], &appex_binary_path)
            .map_err(|e| format!("Failed to rename appex binary: {}", e))?;
    } else {
        // Multiple architectures - combine with lipo
        let mut lipo_args: Vec<&str> = vec!["-create"];
        for path in &built_paths {
            lipo_args.push(path.to_str().unwrap());
        }
        lipo_args.push("-output");
        lipo_args.push(appex_binary_path.to_str().unwrap());

        let lipo_status = Command::new("lipo")
            .args(&lipo_args)
            .status()
            .map_err(|e| format!("Failed to run lipo for appex: {}", e))?;

        if !lipo_status.success() {
            return Err("Failed to create universal appex binary".to_string());
        }

        // Clean up intermediate binaries
        for path in &built_paths {
            let _ = fs::remove_file(path);
        }
    }

    crate::verbose!(verbose, "    Appex executable built ({})", arch_str);

    // Auto-detect component type, manufacturer, and subtype from plugin source
    let (component_type, detected_manufacturer, detected_subtype, detected_plugin_name, detected_vendor_name) = detect_au_component_info(package, workspace_root);
    crate::verbose!(
        verbose,
        "    Detected: {} (manufacturer: {}, subtype: {})",
        component_type,
        detected_manufacturer.as_deref().unwrap_or("Bemr"),
        detected_subtype.as_deref().unwrap_or("auto")
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
        component_type: &component_type,
        manufacturer: detected_manufacturer.as_deref(),
        subtype: detected_subtype.as_deref(),
        framework_bundle_id: &framework_bundle_id,
        version_string: &version_string,
        version_int,
        plugin_name: detected_plugin_name.as_deref(),
        vendor_name: detected_vendor_name.as_deref(),
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
                "-framework", "Foundation",
                "-o", host_arch_path.to_str().unwrap(),
                stub_main_path.to_str().unwrap(),
            ])
            .status()
            .map_err(|e| format!("Failed to run clang for {}: {}", target_arch, e))?;

        if !clang_status.success() {
            return Err(format!("Failed to build host app for {}", target_arch));
        }
        host_built_paths.push(host_arch_path);
    }

    if host_built_paths.len() == 1 {
        // Single architecture - just rename
        fs::rename(&host_built_paths[0], &host_binary_dst)
            .map_err(|e| format!("Failed to rename host binary: {}", e))?;
    } else {
        // Multiple architectures - combine with lipo
        let mut lipo_args: Vec<&str> = vec!["-create"];
        for path in &host_built_paths {
            lipo_args.push(path.to_str().unwrap());
        }
        lipo_args.push("-output");
        lipo_args.push(host_binary_dst.to_str().unwrap());

        let lipo_status = Command::new("lipo")
            .args(&lipo_args)
            .status()
            .map_err(|e| format!("Failed to run lipo for host app: {}", e))?;

        if !lipo_status.success() {
            return Err("Failed to create universal host app binary".to_string());
        }

        // Clean up intermediate binaries
        for path in &host_built_paths {
            let _ = fs::remove_file(path);
        }
    }

    crate::verbose!(verbose, "    Host app built ({})", arch_str);

    // Code sign framework first, then appex, then container app
    crate::verbose!(verbose, "    Signing...");
    let framework_sign_result = Command::new("codesign")
        .args(["--force", "--sign", "-", framework_dir.to_str().unwrap()])
        .output();

    match framework_sign_result {
        Ok(output) if output.status.success() => {
            if verbose {
                let stderr = String::from_utf8_lossy(&output.stderr);
                for line in stderr.lines() {
                    crate::verbose!(verbose, "    {}", line);
                }
            }
            crate::verbose!(verbose, "    Framework code signing successful")
        }
        Ok(_) => crate::status!("  Warning: Framework code signing failed"),
        Err(e) => crate::status!("  Warning: Could not run codesign on framework: {}", e),
    }

    let entitlements_path = workspace_root.join("crates/beamer-au/resources/appex.entitlements");
    let appex_sign_result = Command::new("codesign")
        .args([
            "--force",
            "--sign", "-",
            "--entitlements", entitlements_path.to_str().unwrap(),
            appex_dir.to_str().unwrap()
        ])
        .output();

    match appex_sign_result {
        Ok(output) if output.status.success() => {
            if verbose {
                let stderr = String::from_utf8_lossy(&output.stderr);
                for line in stderr.lines() {
                    crate::verbose!(verbose, "    {}", line);
                }
            }
            crate::verbose!(verbose, "    Appex code signing successful")
        }
        Ok(_) => crate::status!("  Warning: Appex code signing failed"),
        Err(e) => crate::status!("  Warning: Could not run codesign on appex: {}", e),
    }

    let app_sign_result = Command::new("codesign")
        .args(["--force", "--sign", "-", bundle_dir.to_str().unwrap()])
        .output();

    match app_sign_result {
        Ok(output) if output.status.success() => {
            if verbose {
                let stderr = String::from_utf8_lossy(&output.stderr);
                for line in stderr.lines() {
                    crate::verbose!(verbose, "    {}", line);
                }
            }
            crate::verbose!(verbose, "    Container app code signing successful")
        }
        Ok(_) => crate::status!("  Warning: Container app code signing failed"),
        Err(e) => crate::status!("  Warning: Could not run codesign on app: {}", e),
    }

    // Install if requested
    if install {
        install_au(&bundle_dir, &bundle_name, verbose)?;
    } else {
        crate::status!("✓ {}", bundle_name);
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

/// Returns app bundle name for AUv3.
///
/// Examples:
/// - "gain" -> "BeamerGain.app"
/// - "midi-transform" -> "BeamerMidiTransform.app"
fn to_au_bundle_name(package: &str) -> String {
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
    format!("Beamer{}.app", name)
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
    <string>10.13</string>
    <key>LSBackgroundOnly</key>
    <true/>
</dict>
</plist>
"#,
        executable = executable_name,
        package = package,
        version = version
    )
}

/// Maps AU component type code to appropriate tags for Info.plist.
///
/// DAWs use these tags for plugin categorization.
fn get_au_tags(component_type: &str) -> &'static str {
    match component_type {
        "aufx" => "Effects",           // Audio effect
        "aumu" => "Synth",             // Music device/instrument
        "aumi" => "MIDI",              // MIDI processor
        "aumf" => "Effects",           // Music effect
        _ => "Effects",                // Default fallback
    }
}

/// Creates the Info.plist content for the appex with NSExtension.
fn create_appex_info_plist(config: &AppexPlistConfig) -> String {
    let manufacturer = config.manufacturer.unwrap_or("Bemr");
    let subtype = config.subtype.map(|s| s.to_string()).unwrap_or_else(|| {
        let gen: String = config.package.chars().filter(|c| c.is_alphanumeric()).take(4).collect::<String>().to_lowercase();
        if gen.len() < 4 { format!("{:_<4}", gen) } else { gen }
    });

    // Get appropriate tags based on component type
    let tags = get_au_tags(config.component_type);

    // Generate plugin-specific extension class name (implements AUAudioUnitFactory)
    let pascal_name = to_pascal_case(config.package);
    let extension_class = format!("Beamer{}AuExtension", pascal_name);

    // Create the plugin display name from vendor and plugin name
    // Format: "Vendor: Plugin Name" (e.g., "Beamer Framework: Beamer Synth")
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
    <string>10.13</string>
    <key>NSExtension</key>
    <dict>
        <key>NSExtensionPointIdentifier</key>
        <string>com.apple.AudioUnit</string>
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
                        <string>{tags}</string>
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
        executable = config.executable_name,
        package = config.package,
        extension_class = extension_class,
        manufacturer = manufacturer,
        component_type = config.component_type,
        subtype = subtype,
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
fn install_au(bundle_dir: &Path, bundle_name: &str, verbose: bool) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;

    // AUv3 app extensions must be installed as apps (not in Components folder).
    // The system discovers them when the containing app is launched.
    let au_dir = PathBuf::from(&home).join("Applications");

    // Create Applications directory if needed
    fs::create_dir_all(&au_dir).map_err(|e| format!("Failed to create Applications dir: {}", e))?;

    let dest = au_dir.join(bundle_name);

    // Remove existing installation
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("Failed to remove old installation: {}", e))?;
    }

    // Copy bundle
    copy_dir_all(bundle_dir, &dest)?;

    crate::verbose!(verbose, "    Installed to: {}", dest.display());

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
    let killall_au = Command::new("killall")
        .arg("-9")
        .arg("AudioComponentRegistrar")
        .output();

    if verbose {
        if let Ok(output) = killall_au {
            let stderr = String::from_utf8_lossy(&output.stderr);
            for line in stderr.lines() {
                crate::verbose!(verbose, "    {}", line);
            }
        }
    }
    crate::verbose!(verbose, "    Audio Unit registered");
    crate::status!("✓ {} → {}", bundle_name, shorten_path(&dest));

    Ok(())
}
