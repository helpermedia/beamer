//! Build tooling for BEAMR plugins.
//!
//! Usage: cargo xtask bundle <package> [--release] [--install]

use std::fs;
use std::path::PathBuf;
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

    if let Err(e) = bundle(package, release, install) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage: cargo xtask bundle <package> [--release] [--install]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  bundle    Build and bundle a plugin as VST3");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --release    Build in release mode");
    eprintln!("  --install    Install to ~/Library/Audio/Plug-Ins/VST3/");
}

fn bundle(package: &str, release: bool, install: bool) -> Result<(), String> {
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

    // Create bundle name (convert to CamelCase and add .vst3)
    let bundle_name = to_bundle_name(package);
    let bundle_dir = target_dir.join(&bundle_name);

    // Create bundle directory structure
    let contents_dir = bundle_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");

    println!("Creating bundle at {}...", bundle_dir.display());

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
    fs::copy(&dylib_path, &plugin_binary)
        .map_err(|e| format!("Failed to copy dylib: {}", e))?;

    // Create Info.plist
    let info_plist = create_info_plist(package, &bundle_name);
    fs::write(contents_dir.join("Info.plist"), info_plist)
        .map_err(|e| format!("Failed to write Info.plist: {}", e))?;

    // Create PkgInfo
    fs::write(contents_dir.join("PkgInfo"), "BNDL????")
        .map_err(|e| format!("Failed to write PkgInfo: {}", e))?;

    println!("Bundle created: {}", bundle_dir.display());

    // Install if requested
    if install {
        install_vst3(&bundle_dir, &bundle_name)?;
    }

    Ok(())
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

fn to_bundle_name(package: &str) -> String {
    // Convert package name to CamelCase bundle name
    // e.g., "beamr-gain" -> "BeamrGain.vst3"
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
    format!("{}.vst3", name)
}

fn create_info_plist(package: &str, bundle_name: &str) -> String {
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
    <string>com.beamr.{}</string>
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

fn install_vst3(bundle_dir: &PathBuf, bundle_name: &str) -> Result<(), String> {
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

    println!("Installed to: {}", dest.display());
    Ok(())
}

fn copy_dir_all(src: &PathBuf, dst: &PathBuf) -> Result<(), String> {
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
