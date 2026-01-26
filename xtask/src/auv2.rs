//! AUv2 component bundling and code generation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::build::get_version_info;
use crate::util::{copy_dir_all, shorten_path, to_pascal_case, Arch};
use crate::ComponentPlistConfig;

// AUv2 C code generation template (large embedded C implementation)
include!("au_templates/auv2_c.rs");

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

    // Auto-detect component type, manufacturer, and subtype from plugin source
    let (component_type, detected_manufacturer, detected_subtype, detected_plugin_name, detected_vendor_name) =
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
    let wrapper_source = generate_auv2_wrapper_source(package);
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
                "-I", bridge_header_dir.to_str().unwrap(),
                dylib_path.to_str().unwrap(),  // Link directly with the dylib
                "-Wl,-rpath,@loader_path",
                "-o", arch_output.to_str().unwrap(),
                wrapper_path.to_str().unwrap(),
            ])
            .status()
            .map_err(|e| format!("Failed to run clang for {}: {}", target_arch, e))?;

        if !clang_status.success() {
            return Err(format!("Failed to build AUv2 component for {}", target_arch));
        }
        built_paths.push(arch_output);
    }

    if built_paths.len() == 1 {
        // Single architecture - just rename
        fs::rename(&built_paths[0], &binary_dest)
            .map_err(|e| format!("Failed to rename binary: {}", e))?;
    } else {
        // Multiple architectures - combine with lipo
        let mut lipo_args: Vec<&str> = vec!["-create"];
        for path in &built_paths {
            lipo_args.push(path.to_str().unwrap());
        }
        lipo_args.push("-output");
        lipo_args.push(binary_dest.to_str().unwrap());

        let lipo_status = Command::new("lipo")
            .args(&lipo_args)
            .status()
            .map_err(|e| format!("Failed to run lipo: {}", e))?;

        if !lipo_status.success() {
            return Err("Failed to create universal binary".to_string());
        }

        // Clean up intermediate binaries
        for path in &built_paths {
            let _ = fs::remove_file(path);
        }
    }

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
    let sign_result = Command::new("codesign")
        .args(["--force", "--sign", "-", bundle_dir.to_str().unwrap()])
        .output();

    match sign_result {
        Ok(output) if output.status.success() => {
            if verbose {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.is_empty() {
                    for line in stderr.lines() {
                        crate::verbose!(verbose, "    {}", line);
                    }
                }
                crate::verbose!(verbose, "    Code signing successful");
            }
        }
        Ok(_) => crate::status!("  Warning: Code signing failed"),
        Err(e) => crate::status!("  Warning: Could not run codesign: {}", e),
    }

    // Install if requested
    if install {
        install_auv2(&bundle_dir, &bundle_name, verbose)?;
    } else {
        crate::status!("  {}", bundle_name);
    }

    Ok(())
}

pub fn detect_au_component_info(package: &str, workspace_root: &Path) -> (String, Option<String>, Option<String>, Option<String>, Option<String>) {
    // Try to find the lib.rs for this package
    let lib_path = workspace_root.join("examples").join(package).join("src/lib.rs");

    if let Ok(content) = fs::read_to_string(&lib_path) {
        // Detect component type
        let component_type = if content.contains("ComponentType::MusicDevice")
            || content.contains("ComponentType::Generator")
        {
            "aumu".to_string()
        } else if content.contains("ComponentType::MidiProcessor") {
            "aumi".to_string()
        } else if content.contains("ComponentType::MusicEffect") {
            "aumf".to_string()
        } else {
            // Default to effect (aufx)
            "aufx".to_string()
        };

        // Detect manufacturer and subtype from fourcc!(b"xxxx") patterns
        // Pattern: AuConfig::new(type, fourcc!(b"manu"), fourcc!(b"subt"))
        let (manufacturer, subtype) = detect_au_fourcc_codes(&content);

        // Detect plugin name and vendor from PluginConfig
        let (plugin_name, vendor_name) = detect_plugin_metadata(&content);

        (component_type, manufacturer, subtype, plugin_name, vendor_name)
    } else {
        // Default to effect if we can't read the file
        ("aufx".to_string(), None, None, None, None)
    }
}

/// Extract AU fourcc codes (manufacturer and subtype) from plugin source code.
///
/// Looks for the pattern `fourcc!(b"xxxx")` which appears in
/// `AuConfig::new(ComponentType::..., fourcc!(b"manu"), fourcc!(b"subt"))`.
///
/// Returns (manufacturer, subtype) as Options.
fn detect_au_fourcc_codes(content: &str) -> (Option<String>, Option<String>) {
    // Find all fourcc!(b"xxxx") patterns
    let mut fourcc_codes: Vec<String> = Vec::new();

    let mut remaining = content;
    while let Some(start) = remaining.find("fourcc!(b\"") {
        let after_prefix = &remaining[start + 10..]; // Skip "fourcc!(b\""
        if let Some(end) = after_prefix.find('"') {
            let code = &after_prefix[..end];
            if code.len() == 4 && code.is_ascii() {
                fourcc_codes.push(code.to_string());
            }
        }
        // Move past this match to find next
        remaining = &remaining[start + 10..];
    }

    // In AuConfig::new(type, manufacturer, subtype):
    // - First fourcc! is manufacturer
    // - Second fourcc! is subtype
    let manufacturer = fourcc_codes.first().cloned();
    let subtype = fourcc_codes.get(1).cloned();
    (manufacturer, subtype)
}

/// Extract plugin name and vendor from PluginConfig in source code.
///
/// Looks for patterns:
/// - `PluginConfig::new("Plugin Name")`
/// - `.with_vendor("Vendor Name")`
///
/// Returns (plugin_name, vendor_name) as Options.
fn detect_plugin_metadata(content: &str) -> (Option<String>, Option<String>) {
    let mut plugin_name = None;
    let mut vendor_name = None;

    // Find PluginConfig::new("name")
    if let Some(start) = content.find("PluginConfig::new(\"") {
        let after_prefix = &content[start + 19..]; // Skip "PluginConfig::new(\""
        if let Some(end) = after_prefix.find('"') {
            plugin_name = Some(after_prefix[..end].to_string());
        }
    }

    // Find .with_vendor("name")
    if let Some(start) = content.find(".with_vendor(\"") {
        let after_prefix = &content[start + 14..]; // Skip ".with_vendor(\""
        if let Some(end) = after_prefix.find('"') {
            vendor_name = Some(after_prefix[..end].to_string());
        }
    }

    (plugin_name, vendor_name)
}

fn to_auv2_component_name(package: &str) -> String {
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
    format!("Beamer{}.component", name)
}

fn get_au_tags(component_type: &str) -> &'static str {
    match component_type {
        "aufx" => "Effects",           // Audio effect
        "aumu" => "Synth",             // Music device/instrument
        "aumi" => "MIDI",              // MIDI processor
        "aumf" => "Effects",           // Music effect
        _ => "Effects",                // Default fallback
    }
}

fn create_component_info_plist(config: &ComponentPlistConfig) -> String {
    let manufacturer = config.manufacturer.unwrap_or("Bemr");
    let subtype = config.subtype.map(|s| s.to_string()).unwrap_or_else(|| {
        let gen: String = config.package.chars().filter(|c| c.is_alphanumeric()).take(4).collect::<String>().to_lowercase();
        if gen.len() < 4 { format!("{:_<4}", gen) } else { gen }
    });

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
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;

    // AUv2 components go to ~/Library/Audio/Plug-Ins/Components/
    let components_dir = PathBuf::from(&home)
        .join("Library")
        .join("Audio")
        .join("Plug-Ins")
        .join("Components");

    // Create Components directory if needed
    fs::create_dir_all(&components_dir)
        .map_err(|e| format!("Failed to create Components dir: {}", e))?;

    let dest = components_dir.join(bundle_name);

    // Remove existing installation
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("Failed to remove old installation: {}", e))?;
    }

    // Copy bundle
    copy_dir_all(bundle_dir, &dest)?;

    crate::verbose!(verbose, "    Installed to: {}", dest.display());

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
