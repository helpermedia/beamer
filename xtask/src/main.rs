//! Build tooling for Beamer plugins.
//!
//! Usage: cargo xtask bundle <package> [--vst3] [--au] [--release] [--install]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 || args[1] != "bundle" {
        print_usage();
        std::process::exit(1);
    }

    let package = &args[2];
    let release = args.iter().any(|a| a == "--release");
    let install = args.iter().any(|a| a == "--install");
    let build_vst3 = args.iter().any(|a| a == "--vst3");
    let build_au = args.iter().any(|a| a == "--au");

    // Default to VST3 if no format specified
    let (build_vst3, build_au) = if !build_vst3 && !build_au {
        (true, false)
    } else {
        (build_vst3, build_au)
    };

    if let Err(e) = bundle(package, release, install, build_vst3, build_au) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage: cargo xtask bundle <package> [--vst3] [--au] [--release] [--install]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  bundle    Build and bundle a plugin");
    eprintln!();
    eprintln!("Formats:");
    eprintln!("  --vst3    Build VST3 bundle (default if no format specified)");
    eprintln!("  --au      Build Audio Unit bundle (.component)");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --release    Build in release mode");
    eprintln!("  --install    Install to system plugin directories");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  cargo xtask bundle gain --vst3 --release --install");
    eprintln!("  cargo xtask bundle gain --au --release --install");
    eprintln!("  cargo xtask bundle gain --vst3 --au --release --install");
}

fn bundle(
    package: &str,
    release: bool,
    install: bool,
    build_vst3: bool,
    build_au: bool,
) -> Result<(), String> {
    println!("Bundling {} (release: {})...", package, release);

    // Get workspace root
    let workspace_root = get_workspace_root()?;

    // Build the plugin
    println!("Building...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg(package)
        .current_dir(&workspace_root);

    if release {
        cmd.arg("--release");
    }

    let status = cmd.status().map_err(|e| format!("Failed to run cargo: {}", e))?;
    if !status.success() {
        return Err("Build failed".to_string());
    }

    // Determine paths
    let profile = if release { "release" } else { "debug" };
    let target_dir = workspace_root.join("target").join(profile);

    // Convert package name to library name (replace hyphens with underscores)
    let lib_name = package.replace('-', "_");

    // Find the dylib
    let dylib_name = format!("lib{}.dylib", lib_name);
    let dylib_path = target_dir.join(&dylib_name);

    if !dylib_path.exists() {
        return Err(format!("Built library not found: {}", dylib_path.display()));
    }

    // Build requested formats
    if build_vst3 {
        bundle_vst3(package, &target_dir, &dylib_path, install)?;
    }

    if build_au {
        bundle_au(package, &target_dir, &dylib_path, install, &workspace_root)?;
    }

    Ok(())
}

fn bundle_vst3(
    package: &str,
    target_dir: &Path,
    dylib_path: &Path,
    install: bool,
) -> Result<(), String> {
    // Create bundle name (convert to CamelCase and add .vst3)
    let bundle_name = to_vst3_bundle_name(package);
    let bundle_dir = target_dir.join(&bundle_name);

    // Create bundle directory structure
    let contents_dir = bundle_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");

    println!("Creating VST3 bundle at {}...", bundle_dir.display());

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

    println!("VST3 bundle created: {}", bundle_dir.display());

    // Install if requested
    if install {
        install_vst3(&bundle_dir, &bundle_name)?;
    }

    Ok(())
}

fn bundle_au(
    package: &str,
    target_dir: &Path,
    dylib_path: &Path,
    install: bool,
    workspace_root: &Path,
) -> Result<(), String> {
    // Create bundle name (convert to CamelCase and add .component)
    let bundle_name = to_au_bundle_name(package);
    let bundle_dir = target_dir.join(&bundle_name);

    // Create bundle directory structure
    let contents_dir = bundle_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");

    println!("Creating AU bundle at {}...", bundle_dir.display());

    // Clean up existing bundle
    if bundle_dir.exists() {
        fs::remove_dir_all(&bundle_dir).map_err(|e| format!("Failed to remove old bundle: {}", e))?;
    }

    // Create directories
    fs::create_dir_all(&macos_dir).map_err(|e| format!("Failed to create MacOS dir: {}", e))?;
    fs::create_dir_all(&resources_dir)
        .map_err(|e| format!("Failed to create Resources dir: {}", e))?;

    // Copy dylib
    let plugin_binary = macos_dir.join(bundle_name.trim_end_matches(".component"));
    fs::copy(dylib_path, &plugin_binary)
        .map_err(|e| format!("Failed to copy dylib: {}", e))?;

    // Auto-detect component type and subtype from plugin source
    let (component_type, detected_subtype) = detect_au_component_info(package, workspace_root);
    println!(
        "Detected AU component type: {} (subtype: {})",
        component_type,
        detected_subtype.as_deref().unwrap_or("auto-generated")
    );

    // Create Info.plist with AudioComponents
    let info_plist = create_au_info_plist(package, &bundle_name, &component_type, detected_subtype.as_deref());
    fs::write(contents_dir.join("Info.plist"), info_plist)
        .map_err(|e| format!("Failed to write Info.plist: {}", e))?;

    // Create PkgInfo
    fs::write(contents_dir.join("PkgInfo"), "BNDL????")
        .map_err(|e| format!("Failed to write PkgInfo: {}", e))?;

    println!("AU bundle created: {}", bundle_dir.display());

    // Ad-hoc code sign (required for modern macOS)
    println!("Code signing...");
    let sign_status = Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-", bundle_dir.to_str().unwrap()])
        .status();

    match sign_status {
        Ok(status) if status.success() => println!("Code signing successful"),
        Ok(_) => println!("Warning: Code signing failed (plugin may not load)"),
        Err(e) => println!("Warning: Could not run codesign: {}", e),
    }

    // Install if requested
    if install {
        install_au(&bundle_dir, &bundle_name)?;
    }

    Ok(())
}

/// Detect AU component type and subtype from plugin source code.
///
/// Parses the plugin's lib.rs file looking for the `AuConfig::new()` declaration
/// to extract the ComponentType and fourcc codes.
///
/// Returns (component_type_code, subtype_code_option)
fn detect_au_component_info(package: &str, workspace_root: &Path) -> (String, Option<String>) {
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

        // Try to detect subtype from fourcc!(b"xxxx") pattern
        // Look for the second fourcc! call in AuConfig::new (subtype is third argument)
        // Pattern: AuConfig::new(..., fourcc!(b"manu"), fourcc!(b"subt"))
        let subtype = detect_au_subtype(&content);

        (component_type, subtype)
    } else {
        // Default to effect if we can't read the file
        ("aufx".to_string(), None)
    }
}

/// Extract the AU subtype (fourcc code) from plugin source code.
///
/// Looks for the pattern `fourcc!(b"xxxx")` which appears as the third
/// argument in `AuConfig::new(ComponentType::..., fourcc!(b"manu"), fourcc!(b"subt"))`.
fn detect_au_subtype(content: &str) -> Option<String> {
    // Find all fourcc!(b"xxxx") patterns
    let mut fourcc_codes: Vec<String> = Vec::new();

    let mut remaining = content;
    while let Some(start) = remaining.find("fourcc!(b\"") {
        let after_prefix = &remaining[start + 10..]; // Skip "fourcc!(b\""
        if let Some(end) = after_prefix.find("\"") {
            let code = &after_prefix[..end];
            if code.len() == 4 && code.is_ascii() {
                fourcc_codes.push(code.to_string());
            }
        }
        // Move past this match to find next
        remaining = &remaining[start + 10..];
    }

    // The subtype is typically the second fourcc! (first is manufacturer)
    // In AuConfig::new(type, manufacturer, subtype)
    fourcc_codes.get(1).cloned()
}

fn get_workspace_root() -> Result<PathBuf, String> {
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .map_err(|e| format!("Failed to locate workspace: {}", e))?;

    if !output.status.success() {
        return Err("Failed to locate workspace".to_string());
    }

    let cargo_toml = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(cargo_toml.trim());
    path.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "Invalid workspace path".to_string())
}

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

fn to_au_bundle_name(package: &str) -> String {
    // Convert package name to CamelCase bundle name with Beamer prefix
    // e.g., "gain" -> "BeamerGain.component"
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

fn create_au_info_plist(
    package: &str,
    bundle_name: &str,
    component_type: &str,
    detected_subtype: Option<&str>,
) -> String {
    let executable_name = bundle_name.trim_end_matches(".component");

    // Use detected subtype if available, otherwise generate from package name
    let subtype = if let Some(detected) = detected_subtype {
        detected.to_string()
    } else {
        // Generate 4-char codes from package name
        // subtype: first 4 chars of package, lowercase
        let generated: String = package
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(4)
            .collect::<String>()
            .to_lowercase();
        if generated.len() < 4 {
            format!("{:_<4}", generated)
        } else {
            generated
        }
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
    <key>CFBundlePackageType</key>
    <string>BNDL</string>
    <key>CFBundleSignature</key>
    <string>????</string>
    <key>CFBundleVersion</key>
    <string>0.2.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.2.0</string>
    <key>AudioComponents</key>
    <array>
        <dict>
            <key>name</key>
            <string>Beamer: {executable}</string>
            <key>description</key>
            <string>{executable} Audio Unit</string>
            <key>manufacturer</key>
            <string>Bemr</string>
            <key>type</key>
            <string>{component_type}</string>
            <key>subtype</key>
            <string>{subtype}</string>
            <key>version</key>
            <integer>131072</integer>
            <key>factoryFunction</key>
            <string>BeamerAudioUnitFactory</string>
            <key>sandboxSafe</key>
            <true/>
        </dict>
    </array>
</dict>
</plist>
"#,
        executable = executable_name,
        package = package,
        component_type = component_type,
        subtype = subtype
    )
}

fn install_vst3(bundle_dir: &Path, bundle_name: &str) -> Result<(), String> {
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

    println!("VST3 installed to: {}", dest.display());
    Ok(())
}

fn install_au(bundle_dir: &Path, bundle_name: &str) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;
    let au_dir = PathBuf::from(home)
        .join("Library")
        .join("Audio")
        .join("Plug-Ins")
        .join("Components");

    // Create Components directory if needed
    fs::create_dir_all(&au_dir).map_err(|e| format!("Failed to create Components dir: {}", e))?;

    let dest = au_dir.join(bundle_name);

    // Remove existing installation
    if dest.exists() {
        fs::remove_dir_all(&dest).map_err(|e| format!("Failed to remove old installation: {}", e))?;
    }

    // Copy bundle
    copy_dir_all(bundle_dir, &dest)?;

    println!("AU installed to: {}", dest.display());

    // Refresh AU cache
    println!("Refreshing Audio Unit cache...");
    let _ = Command::new("killall")
        .arg("-9")
        .arg("AudioComponentRegistrar")
        .status();

    Ok(())
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("Failed to create dir: {}", e))?;

    for entry in fs::read_dir(src).map_err(|e| format!("Failed to read dir: {}", e))? {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        let ty = entry
            .file_type()
            .map_err(|e| format!("Failed to get file type: {}", e))?;

        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Failed to copy file: {}", e))?;
        }
    }

    Ok(())
}
