//! AUv2 component bundling and code generation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::build::{clang_arches, get_version_info};
use crate::util::{au_tags, codesign_bundle, combine_or_rename_binaries, detect_au_component_info, detect_subcategories, install_bundle, plist_strings, refresh_au_registry, shorten_path, to_auv2_component_name, uninstall_bundle, Arch, AuCodes, AuNames, PathExt, MACOS_DEPLOYMENT_TARGET, MACOS_MIN_VERSION_FLAG};
use crate::ComponentPlistConfig;

/// Where an installed component lives, relative to the home directory.
const INSTALL_DIR: &[&str] = &["Library", "Audio", "Plug-Ins", "Components"];

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
    // │   │   ├── BeamerGain     ← Component binary with factory symbol
    // │   │   └── libgain.dylib  ← Plugin's Rust library
    // │   ├── Resources/
    // │   └── PkgInfo

    // Get the version of the plugin package.
    let package_info = crate::util::find_package(package, workspace_root)?;
    let (version_string, version_int) = get_version_info(&package_info.version)?;

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
    let codes = AuCodes::detect(package, &package_info.dir);
    let names = AuNames::new(package, &codes);
    let (_, _, _, detected_plugin_name, detected_vendor_name, _) = detect_au_component_info(&package_info.dir);
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

    // Generate the AUv2 wrapper with its factory function.
    let wrapper_source = generate_auv2_wrapper_source(package, &codes, &names);
    let gen_dir = workspace_root.join("target/au-gen").join(package);
    fs::create_dir_all(&gen_dir).map_err(|e| format!("Failed to create gen dir: {}", e))?;
    let wrapper_path = gen_dir.join("auv2_wrapper.m");
    fs::write(&wrapper_path, wrapper_source).map_err(|e| format!("Failed to write wrapper: {}", e))?;

    // Build for each architecture
    let arches = clang_arches(arch)?;

    let arch_str = if arches.len() > 1 { "universal" } else { arches[0] };
    crate::verbose!(verbose, "    Building component ({})...", arch_str);

    // Header search paths for clang
    let bridge_header_dir = workspace_root.join("crates/beamer-au/objc");
    let ipc_header_dir = workspace_root.join("xtask/src/au_codegen");

    let executable_name = bundle_name.trim_end_matches(".component");
    let binary_dest = macos_dir.join(executable_name);

    // Copy the dylib next to the component binary and give the copy an install
    // name relative to it. The linker records that name in the component
    // binary, which then loads the bundled copy. The dylib that cargo builds
    // has the absolute path of the build directory as its install name.
    //
    // The name has to be @loader_path based. dyld compares a requested name
    // with the install names of the images that are already loaded before it
    // looks on disk, and it expands @loader_path to the directory of the
    // requesting binary first. An @rpath name can therefore be satisfied by a
    // library with the same name that another bundle loaded earlier. The
    // absolute build path was satisfied by the VST3 build of the same plugin,
    // which keeps that path as its install name and has no AU symbols.
    let dylib_name = dylib_path.file_name().unwrap();
    let dylib_dest = macos_dir.join(dylib_name);
    fs::copy(dylib_path, &dylib_dest).map_err(|e| format!("Failed to copy dylib: {}", e))?;

    let install_name = format!("@loader_path/{}", dylib_name.to_string_lossy());
    let install_name_status = Command::new("install_name_tool")
        .args(["-id", &install_name, dylib_dest.to_str_safe()?])
        .status()
        .map_err(|e| format!("Failed to run install_name_tool: {}", e))?;

    if !install_name_status.success() {
        return Err("Failed to set the install name of the plugin dylib".to_string());
    }

    let mut built_paths: Vec<PathBuf> = Vec::new();

    for target_arch in &arches {
        let arch_output = gen_dir.join(format!("{}_{}", executable_name, target_arch));

        // Compile the AUv2 wrapper and link it with the Rust dylib as a bundle.
        let clang_status = Command::new("clang")
            .args([
                "-arch", target_arch,
                MACOS_MIN_VERSION_FLAG,
                "-bundle",  // Create a bundle (loadable module), not an executable
                "-fobjc-arc",
                "-fmodules",
                "-framework", "Foundation",
                "-framework", "AudioToolbox",
                "-framework", "AVFoundation",
                "-framework", "CoreAudio",
                "-framework", "CoreAudioKit",
                "-framework", "CoreMIDI",
                "-framework", "WebKit",
                "-I", bridge_header_dir.to_str_safe()?,
                "-I", ipc_header_dir.to_str_safe()?,
                // Link with the bundled dylib, which has the @loader_path install name.
                dylib_dest.to_str_safe()?,
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

    // Create Info.plist with factoryFunction
    let info_plist = create_component_info_plist(&ComponentPlistConfig {
        package,
        executable_name,
        codes: &codes,
        subcategories: &detect_subcategories(&package_info.dir),
        factory_name: &names.auv2_factory,
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

    // Code sign with ad-hoc signature: the dylib first, then the component.
    // codesign does not sign nested code and refuses a bundle that contains an
    // unsigned dylib. Only arm64 code is signed by the linker.
    crate::verbose!(verbose, "    Signing...");
    codesign_bundle(&dylib_dest, None, "Plugin library", verbose)?;
    codesign_bundle(&bundle_dir, None, "Component", verbose)?;

    // Install if requested
    if install {
        install_auv2(&bundle_dir, &bundle_name, verbose)?;
    } else {
        crate::status!("  {}", bundle_name);
    }

    Ok(())
}

fn create_component_info_plist(config: &ComponentPlistConfig) -> String {
    // Get appropriate tags based on component type
    let tags = plist_strings(&au_tags(&config.codes.component_type, config.subcategories), "                ");

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
    <string>{min_os}</string>
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
                {tags}
            </array>
            <key>version</key>
            <integer>{version_int}</integer>
        </dict>
    </array>
</dict>
</plist>
"#,
        min_os = MACOS_DEPLOYMENT_TARGET,
        executable = config.executable_name,
        package = config.package,
        manufacturer = config.codes.manufacturer,
        component_type = config.codes.component_type,
        subtype = config.codes.subtype,
        tags = tags,
        factory_name = config.factory_name,
        version = config.version_string,
        version_int = config.version_int,
        plugin_display_name = plugin_display_name,
        display_name = config.plugin_name.unwrap_or(config.executable_name),
    )
}

fn install_auv2(bundle_dir: &Path, bundle_name: &str, verbose: bool) -> Result<(), String> {
    let dest = install_bundle(bundle_dir, bundle_name, INSTALL_DIR, verbose)?;

    // Refresh AU cache to pick up the new component
    refresh_au_registry(verbose);
    crate::verbose!(verbose, "    Audio Unit cache refreshed");
    crate::status!("  {} -> {}", bundle_name, shorten_path(&dest));

    Ok(())
}

/// Removes an installed AUv2 component and returns whether there was one.
///
/// The caller refreshes the Audio Unit registry afterwards.
pub fn uninstall_auv2(package: &str) -> Result<bool, String> {
    uninstall_bundle(&to_auv2_component_name(package), INSTALL_DIR)
}
