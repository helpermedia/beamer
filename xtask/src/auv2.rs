//! AUv2 component bundling and code generation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::build::get_version_info;
use crate::util::{codesign_bundle, combine_or_rename_binaries, detect_au_component_info, generate_au_subtype, get_au_tags, install_bundle, shorten_path, to_auv2_component_name, to_pascal_case, Arch, PathExt};
use crate::ComponentPlistConfig;

// AUv2 C code generation template (large embedded C implementation)
include!("au_codegen/auv2_c.rs");

pub fn bundle_auv2(
    package: &str,
    target_dir: &Path,
    dylib_path: &Path,
    install: bool,
    workspace_root: &Path,
    arch: Arch,
    verbose: bool,
) -> Result<(), String> {
    // Create AUv2 .component bundle structure:
    // BeamerGain.component/
    // ├── Contents/
    // │   ├── Info.plist       ← AudioComponents + factoryFunction
    // │   ├── MacOS/
    // │   │   └── BeamerGain   ← Plugin dylib with factory symbol
    // │   ├── Resources/
    // │   └── PkgInfo

    // Get version from Cargo.toml
    let (version_string, version_int) = get_version_info(workspace_root)?;

    let bundle_name = to_auv2_component_name(package);
    let bundle_dir = target_dir.join(&bundle_name);
    let contents_dir = bundle_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");

    crate::status!("  Creating AUv2 component...");
    crate::verbose!(verbose, "    Path: {}", bundle_dir.display());

    // Clean up existing bundle
    if bundle_dir.exists() {
        fs::remove_dir_all(&bundle_dir).map_err(|e| format!("Failed to remove old bundle: {}", e))?;
    }

    // Create directories
    fs::create_dir_all(&macos_dir).map_err(|e| format!("Failed to create MacOS dir: {}", e))?;
    fs::create_dir_all(&resources_dir).map_err(|e| format!("Failed to create Resources dir: {}", e))?;

    // Auto-detect component type, manufacturer and subtype from plugin source
    let (component_type, detected_manufacturer, detected_subtype, detected_plugin_name, detected_vendor_name, _) =
        detect_au_component_info(package, workspace_root);
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

    // Generate ObjC wrapper with factory function
    let wrapper_source = generate_auv2_wrapper_source(package, &component_type);
    let gen_dir = workspace_root.join("target/au-gen").join(package);
    fs::create_dir_all(&gen_dir).map_err(|e| format!("Failed to create gen dir: {}", e))?;
    let wrapper_path = gen_dir.join("auv2_wrapper.m");
    fs::write(&wrapper_path, wrapper_source).map_err(|e| format!("Failed to write wrapper: {}", e))?;

    // Build for each architecture
    let arches = match arch {
        Arch::Universal => vec!["x86_64", "arm64"],
        Arch::Native => vec![if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" }],
        Arch::Arm64 => vec!["arm64"],
        Arch::X86_64 => vec!["x86_64"],
    };

    let arch_str = if arches.len() > 1 { "universal" } else { arches[0] };
    crate::verbose!(verbose, "    Building component ({})...", arch_str);

    // Get bridge header path
    let bridge_header_dir = workspace_root.join("crates/beamer-au/objc");

    let executable_name = bundle_name.trim_end_matches(".component");
    let binary_dest = macos_dir.join(executable_name);

    let mut built_paths: Vec<PathBuf> = Vec::new();

    for target_arch in &arches {
        let arch_output = gen_dir.join(format!("{}_{}", executable_name, target_arch));

        // Compile ObjC wrapper and link with Rust dylib as a bundle
        let clang_status = Command::new("clang")
            .args([
                "-arch", target_arch,
                "-bundle",  // Create a bundle (loadable module), not an executable
                "-fobjc-arc",
                "-fmodules",
                "-framework", "Foundation",
                "-framework", "AudioToolbox",
                "-framework", "AVFoundation",
                "-framework", "CoreAudio",
                "-framework", "CoreAudioKit",
                "-framework", "WebKit",
                "-I", bridge_header_dir.to_str_safe()?,
                dylib_path.to_str_safe()?,  // Link directly with the dylib
                "-Wl,-rpath,@loader_path",
                "-o", arch_output.to_str_safe()?,
                wrapper_path.to_str_safe()?,
            ])
            .status()
            .map_err(|e| format!("Failed to run clang for {}: {}", target_arch, e))?;

        if !clang_status.success() {
            return Err(format!("Failed to build AUv2 component for {}", target_arch));
        }
        built_paths.push(arch_output);
    }

    combine_or_rename_binaries(&built_paths, &binary_dest, true)?;

    // Copy dylib next to the binary (for @rpath resolution)
    let dylib_name = dylib_path.file_name().unwrap();
    let dylib_dest = macos_dir.join(dylib_name);
    fs::copy(dylib_path, &dylib_dest).map_err(|e| format!("Failed to copy dylib: {}", e))?;

    // Create Info.plist with factoryFunction
    let info_plist = create_component_info_plist(&ComponentPlistConfig {
        package,
        executable_name,
        component_type: &component_type,
        manufacturer: detected_manufacturer.as_deref(),
        subtype: detected_subtype.as_deref(),
        version_string: &version_string,
        version_int,
        plugin_name: detected_plugin_name.as_deref(),
        vendor_name: detected_vendor_name.as_deref(),
    });
    fs::write(contents_dir.join("Info.plist"), info_plist)
        .map_err(|e| format!("Failed to write Info.plist: {}", e))?;

    // Create PkgInfo
    fs::write(contents_dir.join("PkgInfo"), "BNDL????")
        .map_err(|e| format!("Failed to write PkgInfo: {}", e))?;

    // Code sign with ad-hoc signature
    crate::verbose!(verbose, "    Signing...");
    codesign_bundle(&bundle_dir, None, "Component", verbose);

    // Install if requested
    if install {
        install_auv2(&bundle_dir, &bundle_name, verbose)?;
    } else {
        crate::status!("  {}", bundle_name);
    }

    Ok(())
}

fn create_component_info_plist(config: &ComponentPlistConfig) -> String {
    let manufacturer = config.manufacturer.unwrap_or("Bmer");
    let subtype = config
        .subtype
        .map(|s| s.to_string())
        .unwrap_or_else(|| generate_au_subtype(config.package));

    // Get appropriate tags based on component type
    let tags = get_au_tags(config.component_type);

    // Generate factory function name
    let pascal_name = to_pascal_case(config.package);
    let factory_name = format!("Beamer{}Factory", pascal_name);

    // Create the plugin display name from vendor and plugin name
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
    <string>com.beamer.{package}.component</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{executable}</string>
    <key>CFBundleDisplayName</key>
    <string>{display_name}</string>
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
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
            <key>description</key>
            <string>{executable} Audio Unit</string>
            <key>factoryFunction</key>
            <string>{factory_name}</string>
            <key>sandboxSafe</key>
            <true/>
            <key>tags</key>
            <array>
                <string>{tags}</string>
            </array>
            <key>version</key>
            <integer>{version_int}</integer>
        </dict>
    </array>
</dict>
</plist>
"#,
        executable = config.executable_name,
        package = config.package,
        manufacturer = manufacturer,
        component_type = config.component_type,
        subtype = subtype,
        tags = tags,
        factory_name = factory_name,
        version = config.version_string,
        version_int = config.version_int,
        plugin_display_name = plugin_display_name,
        display_name = config.plugin_name.unwrap_or(config.executable_name),
    )
}

fn install_auv2(bundle_dir: &Path, bundle_name: &str, verbose: bool) -> Result<(), String> {
    let dest = install_bundle(
        bundle_dir,
        bundle_name,
        &["Library", "Audio", "Plug-Ins", "Components"],
        verbose,
    )?;

    // Refresh AU cache to pick up the new component
    let killall_result = Command::new("killall")
        .arg("-9")
        .arg("AudioComponentRegistrar")
        .output();

    if verbose {
        if let Ok(output) = killall_result {
            let stderr = String::from_utf8_lossy(&output.stderr);
            for line in stderr.lines() {
                crate::verbose!(verbose, "    {}", line);
            }
        }
    }
    crate::verbose!(verbose, "    Audio Unit cache refreshed");
    crate::status!("  {} -> {}", bundle_name, shorten_path(&dest));

    Ok(())
}
