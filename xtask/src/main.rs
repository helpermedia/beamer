//! Build tooling for Beamer plugins.
//!
//! Usage: cargo xtask bundle <package> [--vst3] [--au] [--release] [--install] [--clean]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Configuration for creating appex Info.plist
struct AppexPlistConfig<'a> {
    package: &'a str,
    executable_name: &'a str,
    component_type: &'a str,
    manufacturer: Option<&'a str>,
    subtype: Option<&'a str>,
    framework_bundle_id: &'a str,
    version_string: &'a str,
    version_int: u32,
}

/// Read version from workspace Cargo.toml and convert to Apple's version integer format
fn get_version_info(workspace_root: &Path) -> Result<(String, u32), String> {
    let cargo_toml_path = workspace_root.join("Cargo.toml");
    let cargo_toml = fs::read_to_string(&cargo_toml_path)
        .map_err(|e| format!("Failed to read Cargo.toml: {}", e))?;

    // Parse version from workspace.package.version
    let version = cargo_toml
        .lines()
        .skip_while(|line| !line.contains("[workspace.package]"))
        .skip(1)
        .find(|line| line.trim().starts_with("version"))
        .and_then(|line| line.split('=').nth(1))
        .map(|v| v.trim().trim_matches('"').to_string())
        .ok_or("Could not find version in Cargo.toml")?;

    // Parse version into major.minor.patch
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() < 3 {
        return Err(format!("Invalid version format: {}", version));
    }

    let major: u32 = parts[0].parse().map_err(|_| "Invalid major version")?;
    let minor: u32 = parts[1].parse().map_err(|_| "Invalid minor version")?;
    let patch: u32 = parts[2].parse().map_err(|_| "Invalid patch version")?;

    // Convert to Apple's version format: (major << 16) | (minor << 8) | patch
    let version_int = (major << 16) | (minor << 8) | patch;

    Ok((version, version_int))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 3 || args[1] != "bundle" {
        print_usage();
        std::process::exit(1);
    }

    let package = &args[2];
    let release = args.iter().any(|a| a == "--release");
    let install = args.iter().any(|a| a == "--install");
    let clean = args.iter().any(|a| a == "--clean");
    let build_vst3 = args.iter().any(|a| a == "--vst3");
    let build_au = args.iter().any(|a| a == "--au");

    // Default to VST3 if no format specified
    let (build_vst3, build_au) = if !build_vst3 && !build_au {
        (true, false)
    } else {
        (build_vst3, build_au)
    };

    if let Err(e) = bundle(package, release, install, clean, build_vst3, build_au) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage: cargo xtask bundle <package> [--vst3] [--au] [--release] [--install] [--clean]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  bundle    Build and bundle a plugin");
    eprintln!();
    eprintln!("Formats:");
    eprintln!("  --vst3    Build VST3 bundle (default if no format specified)");
    eprintln!("  --au      Build Audio Unit bundle (AUv3 App Extension: .app with .appex)");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --release    Build in release mode");
    eprintln!("  --install    Install to system plugin directories");
    eprintln!("  --clean      Clean build caches before building (forces full rebuild)");
    eprintln!("               Removes beamer-au cc cache and previous app bundle.");
    eprintln!("               Use when ObjC/header changes aren't being picked up.");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  cargo xtask bundle gain --vst3 --release --install");
    eprintln!("  cargo xtask bundle gain --au --release --install");
    eprintln!("  cargo xtask bundle gain --au --release --clean");
}

fn build_universal(package: &str, release: bool, workspace_root: &Path) -> Result<PathBuf, String> {
    println!("Building universal binary (x86_64 + arm64)...");

    let profile = if release { "release" } else { "debug" };
    let lib_name = package.replace('-', "_");
    let dylib_name = format!("lib{}.dylib", lib_name);

    // Build for x86_64
    println!("Building for x86_64...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg(package)
        .arg("--target")
        .arg("x86_64-apple-darwin")
        .current_dir(workspace_root);

    if release {
        cmd.arg("--release");
    }

    let status = cmd.status().map_err(|e| format!("Failed to build for x86_64: {}", e))?;
    if !status.success() {
        return Err("Build for x86_64 failed".to_string());
    }

    // Build for arm64
    println!("Building for arm64...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg(package)
        .arg("--target")
        .arg("aarch64-apple-darwin")
        .current_dir(workspace_root);

    if release {
        cmd.arg("--release");
    }

    let status = cmd.status().map_err(|e| format!("Failed to build for arm64: {}", e))?;
    if !status.success() {
        return Err("Build for arm64 failed".to_string());
    }

    // Paths to the built binaries
    let x86_64_path = workspace_root
        .join("target")
        .join("x86_64-apple-darwin")
        .join(profile)
        .join(&dylib_name);

    let arm64_path = workspace_root
        .join("target")
        .join("aarch64-apple-darwin")
        .join(profile)
        .join(&dylib_name);

    // Output path for universal binary
    let universal_path = workspace_root
        .join("target")
        .join(profile)
        .join(&dylib_name);

    // Create target directory if it doesn't exist
    if let Some(parent) = universal_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create target dir: {}", e))?;
    }

    // Combine using lipo
    println!("Creating universal binary with lipo...");
    let status = Command::new("lipo")
        .args([
            "-create",
            x86_64_path.to_str().unwrap(),
            arm64_path.to_str().unwrap(),
            "-output",
            universal_path.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to run lipo: {}", e))?;

    if !status.success() {
        return Err("lipo failed to create universal binary".to_string());
    }

    println!("Universal binary created at: {}", universal_path.display());

    Ok(universal_path)
}

/// Clean build caches to force full rebuild.
///
/// This is necessary because:
/// 1. `cc::Build` (used in build.rs) caches compiled .o files in target/build/beamer-au-*/
/// 2. Even if cargo triggers a rebuild, cc may use cached object files
/// 3. The final .app bundle may not get updated if only static libraries changed
///
/// Use --clean when ObjC or header file changes aren't being picked up.
fn clean_build_caches(workspace_root: &Path, package: &str, release: bool) -> Result<(), String> {
    println!("Cleaning build caches...");

    let profile = if release { "release" } else { "debug" };
    let target_dir = workspace_root.join("target").join(profile);

    // Clean beamer-au cc cache (compiled ObjC objects)
    let build_dir = workspace_root.join("target").join(profile).join("build");
    if build_dir.exists() {
        for entry in fs::read_dir(&build_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with("beamer-au-") {
                println!("  Removing: {}", entry.path().display());
                fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
            }
        }
    }

    // Clean beamer-au deps (compiled Rust library)
    let deps_dir = target_dir.join("deps");
    if deps_dir.exists() {
        for entry in fs::read_dir(&deps_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("libbeamer_au") {
                println!("  Removing: {}", entry.path().display());
                fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
            }
        }
    }

    // Clean previous app bundle
    let bundle_name = to_au_bundle_name(package);
    let app_path = target_dir.join(&bundle_name);
    if app_path.exists() {
        println!("  Removing: {}", app_path.display());
        fs::remove_dir_all(&app_path).map_err(|e| e.to_string())?;
    }

    println!("Clean complete.");
    Ok(())
}

fn bundle(
    package: &str,
    release: bool,
    install: bool,
    clean: bool,
    build_vst3: bool,
    build_au: bool,
) -> Result<(), String> {
    println!("Bundling {} (release: {})...", package, release);

    // Get workspace root
    let workspace_root = get_workspace_root()?;

    // Clean build caches if requested
    if clean {
        clean_build_caches(&workspace_root, package, release)?;
    }

    // For macOS AU, build universal binary (x86_64 + arm64)
    let universal = build_au && cfg!(target_os = "macos");

    // Determine paths
    let profile = if release { "release" } else { "debug" };
    let target_dir = workspace_root.join("target").join(profile);

    let dylib_path = if universal {
        build_universal(package, release, &workspace_root)?
    } else {
        // Build the plugin for current architecture
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

        // Convert package name to library name (replace hyphens with underscores)
        let lib_name = package.replace('-', "_");

        // Find the dylib
        let dylib_name = format!("lib{}.dylib", lib_name);
        let dylib_path = target_dir.join(&dylib_name);

        if !dylib_path.exists() {
            return Err(format!("Built library not found: {}", dylib_path.display()));
        }

        dylib_path
    };

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
    // Create AUv3 .app bundle structure with .appex extension:
    // BeamerGain.app/                         # Container app
    // ├── Contents/
    // │   ├── Info.plist                      # Minimal app plist (LSUIElement=true)
    // │   ├── MacOS/
    // │   │   └── BeamerGain                  # Symlink to appex binary
    // │   ├── PlugIns/
    // │   │   └── BeamerGain.appex/           # The actual AU extension
    // │   │       ├── Contents/
    // │   │       │   ├── Info.plist          # NSExtension + AudioComponents
    // │   │       │   ├── MacOS/
    // │   │       │   │   └── BeamerGain      # Plugin dylib
    // │   │       │   └── Resources/
    // │   ├── Resources/
    // │   └── PkgInfo                         # "APPL????"

    // Get version from Cargo.toml
    let (version_string, version_int) = get_version_info(workspace_root)?;

    let bundle_name = to_au_bundle_name(package);
    let bundle_dir = target_dir.join(&bundle_name);
    let contents_dir = bundle_dir.join("Contents");
    let app_resources_dir = contents_dir.join("Resources");
    let plugins_dir = contents_dir.join("PlugIns");

    println!("Creating AUv3 app extension bundle at {}...", bundle_dir.display());

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
    // Structure: Frameworks/PluginNameAU.framework/PluginNameAU (dylib)
    let framework_name = format!("{}AU", executable_name);
    let framework_bundle_id = format!("com.beamer.{}.framework", package);
    let framework_dir = frameworks_dir.join(format!("{}.framework", framework_name));
    fs::create_dir_all(&framework_dir).map_err(|e| format!("Failed to create framework dir: {}", e))?;

    // Copy dylib to framework
    let framework_binary = framework_dir.join(&framework_name);
    fs::copy(dylib_path, &framework_binary)
        .map_err(|e| format!("Failed to copy dylib to framework: {}", e))?;

    // Fix dylib install name to use @rpath
    let _ = Command::new("install_name_tool")
        .args(["-id", &format!("@rpath/{}.framework/{}", framework_name, framework_name),
               framework_binary.to_str().unwrap()])
        .status();

    // Create framework Info.plist
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
    fs::write(framework_dir.join("Info.plist"), framework_plist)
        .map_err(|e| format!("Failed to write framework Info.plist: {}", e))?;

    println!("Created framework: {}.framework", framework_name);

    // Build appex executable - thin wrapper that links the framework
    let appex_binary_path = appex_macos_dir.join(executable_name);
    let appex_main_path = workspace_root.join("crates/beamer-au/objc/appex_main.m");

    println!("Building appex executable (universal binary)...");

    // Build for x86_64
    let appex_x86_64_path = bundle_dir.join(format!("{}_x86_64", executable_name));
    let clang_status = Command::new("clang")
        .args([
            "-arch", "x86_64",
            "-fobjc-arc",
            "-fmodules",
            "-framework", "Foundation",
            "-framework", "AudioToolbox",
            "-framework", "AVFoundation",
            "-framework", "CoreAudio",
            "-F", frameworks_dir.to_str().unwrap(),
            "-framework", &framework_name,
            "-Wl,-rpath,@loader_path/../../../../Frameworks",
            "-o", appex_x86_64_path.to_str().unwrap(),
            appex_main_path.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to run clang for x86_64: {}", e))?;

    if !clang_status.success() {
        return Err("Failed to build appex for x86_64".to_string());
    }

    // Build for arm64
    let appex_arm64_path = bundle_dir.join(format!("{}_arm64", executable_name));
    let clang_status = Command::new("clang")
        .args([
            "-arch", "arm64",
            "-fobjc-arc",
            "-fmodules",
            "-framework", "Foundation",
            "-framework", "AudioToolbox",
            "-framework", "AVFoundation",
            "-framework", "CoreAudio",
            "-F", frameworks_dir.to_str().unwrap(),
            "-framework", &framework_name,
            "-Wl,-rpath,@loader_path/../../../../Frameworks",
            "-o", appex_arm64_path.to_str().unwrap(),
            appex_main_path.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to run clang for arm64: {}", e))?;

    if !clang_status.success() {
        return Err("Failed to build appex for arm64".to_string());
    }

    // Combine with lipo
    let lipo_status = Command::new("lipo")
        .args([
            "-create",
            appex_x86_64_path.to_str().unwrap(),
            appex_arm64_path.to_str().unwrap(),
            "-output",
            appex_binary_path.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to run lipo for appex: {}", e))?;

    if !lipo_status.success() {
        return Err("Failed to create universal appex binary".to_string());
    }

    // Clean up intermediate binaries
    let _ = fs::remove_file(&appex_x86_64_path);
    let _ = fs::remove_file(&appex_arm64_path);

    println!("Appex executable built (universal)");

    // Auto-detect component type, manufacturer, and subtype from plugin source
    let (component_type, detected_manufacturer, detected_subtype) = detect_au_component_info(package, workspace_root);
    println!(
        "Detected AU: {} (manufacturer: {}, subtype: {})",
        component_type,
        detected_manufacturer.as_deref().unwrap_or("Bemr"),
        detected_subtype.as_deref().unwrap_or("auto")
    );

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

    // Build host app executable from C stub (universal binary).
    // This is a minimal stub that triggers pluginkit registration when launched.
    // The app is marked LSBackgroundOnly so it exits immediately after registration.
    println!("Building host app executable (universal)...");

    let stub_main_path = workspace_root.join("crates/beamer-au/objc/stub_main.c");
    let host_x86_64_path = bundle_dir.join(format!("{}_x86_64", executable_name));
    let host_arm64_path = bundle_dir.join(format!("{}_arm64", executable_name));
    let host_binary_dst = app_macos_dir.join(executable_name);

    // Build for x86_64
    let clang_status = Command::new("clang")
        .args([
            "-arch", "x86_64",
            "-framework", "Foundation",
            "-o", host_x86_64_path.to_str().unwrap(),
            stub_main_path.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to run clang for x86_64: {}", e))?;

    if !clang_status.success() {
        return Err("Failed to build host app for x86_64".to_string());
    }

    // Build for arm64
    let clang_status = Command::new("clang")
        .args([
            "-arch", "arm64",
            "-framework", "Foundation",
            "-o", host_arm64_path.to_str().unwrap(),
            stub_main_path.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to run clang for arm64: {}", e))?;

    if !clang_status.success() {
        return Err("Failed to build host app for arm64".to_string());
    }

    // Combine with lipo
    let lipo_status = Command::new("lipo")
        .args([
            "-create",
            host_x86_64_path.to_str().unwrap(),
            host_arm64_path.to_str().unwrap(),
            "-output",
            host_binary_dst.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| format!("Failed to run lipo for host app: {}", e))?;

    if !lipo_status.success() {
        return Err("Failed to create universal host app binary".to_string());
    }

    // Clean up intermediate binaries
    let _ = fs::remove_file(&host_x86_64_path);
    let _ = fs::remove_file(&host_arm64_path);

    println!("Host app built (universal)");

    println!("AUv3 app extension bundle created: {}", bundle_dir.display());

    // Code sign framework first, then appex, then container app
    println!("Code signing framework...");
    let framework_sign_status = Command::new("codesign")
        .args(["--force", "--sign", "-", framework_dir.to_str().unwrap()])
        .status();

    match framework_sign_status {
        Ok(status) if status.success() => println!("Framework code signing successful"),
        Ok(_) => println!("Warning: Framework code signing failed"),
        Err(e) => println!("Warning: Could not run codesign on framework: {}", e),
    }

    println!("Code signing appex...");
    let entitlements_path = workspace_root.join("crates/beamer-au/resources/appex.entitlements");
    let appex_sign_status = Command::new("codesign")
        .args([
            "--force",
            "--sign", "-",
            "--entitlements", entitlements_path.to_str().unwrap(),
            appex_dir.to_str().unwrap()
        ])
        .status();

    match appex_sign_status {
        Ok(status) if status.success() => println!("Appex code signing successful"),
        Ok(_) => println!("Warning: Appex code signing failed"),
        Err(e) => println!("Warning: Could not run codesign on appex: {}", e),
    }

    println!("Code signing container app...");
    let app_sign_status = Command::new("codesign")
        .args(["--force", "--sign", "-", bundle_dir.to_str().unwrap()])
        .status();

    match app_sign_status {
        Ok(status) if status.success() => println!("Container app code signing successful"),
        Ok(_) => println!("Warning: Container app code signing failed"),
        Err(e) => println!("Warning: Could not run codesign on app: {}", e),
    }

    // Install if requested
    if install {
        install_au(&bundle_dir, &bundle_name)?;
    }

    Ok(())
}

/// Detect AU component type, manufacturer, and subtype from plugin source code.
///
/// Parses the plugin's lib.rs file looking for the `AuConfig::new()` declaration
/// to extract the ComponentType and fourcc codes.
///
/// Returns (component_type_code, manufacturer_option, subtype_option)
fn detect_au_component_info(package: &str, workspace_root: &Path) -> (String, Option<String>, Option<String>) {
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

        (component_type, manufacturer, subtype)
    } else {
        // Default to effect if we can't read the file
        ("aufx".to_string(), None, None)
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

/// Returns app bundle name for AUv3
/// e.g., "gain" -> "BeamerGain.app"
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

/// Create Info.plist for container app (stub executable that triggers pluginkit registration)
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

/// Create Info.plist for appex with NSExtension
fn create_appex_info_plist(config: &AppexPlistConfig) -> String {
    let manufacturer = config.manufacturer.unwrap_or("Bemr");
    let subtype = config.subtype.map(|s| s.to_string()).unwrap_or_else(|| {
        let gen: String = config.package.chars().filter(|c| c.is_alphanumeric()).take(4).collect::<String>().to_lowercase();
        if gen.len() < 4 { format!("{:_<4}", gen) } else { gen }
    });

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
    <string>XPC!</string>
    <key>CFBundleSignature</key>
    <string>????</string>
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
        <string>BeamerAuExtension</string>
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
                    <string>Beamer: {executable}</string>
                    <key>sandboxSafe</key>
                    <true/>
                    <key>tags</key>
                    <array>
                        <string>Effects</string>
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
        manufacturer = manufacturer,
        component_type = config.component_type,
        subtype = subtype,
        framework_bundle_id = config.framework_bundle_id,
        version = config.version_string,
        version_int = config.version_int
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

    println!("AUv3 app extension installed to: {}", dest.display());

    // Launch the app briefly to trigger pluginkit registration.
    // AUv3 extensions are registered when their containing app is first launched.
    println!("Registering Audio Unit extension...");
    let _ = Command::new("open")
        .arg(&dest)
        .status();

    // Give the system a moment to register the extension
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Terminate the background app (it has LSBackgroundOnly so it won't show UI)
    let executable_name = bundle_name.trim_end_matches(".app");
    let _ = Command::new("killall")
        .arg(executable_name)
        .status();

    // Also refresh AU cache
    let _ = Command::new("killall")
        .arg("-9")
        .arg("AudioComponentRegistrar")
        .status();

    println!("Audio Unit registered successfully");

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
        } else if ty.is_symlink() {
            // Preserve symlinks (important for AUv3 container app binary)
            #[cfg(unix)]
            {
                let target = fs::read_link(&src_path)
                    .map_err(|e| format!("Failed to read symlink: {}", e))?;
                std::os::unix::fs::symlink(&target, &dst_path)
                    .map_err(|e| format!("Failed to create symlink: {}", e))?;
            }
            #[cfg(not(unix))]
            {
                fs::copy(&src_path, &dst_path)
                    .map_err(|e| format!("Failed to copy file: {}", e))?;
            }
        } else {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| format!("Failed to copy file: {}", e))?;
        }
    }

    Ok(())
}
