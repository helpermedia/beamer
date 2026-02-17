//! Build orchestration and AU ObjC code generation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{combine_or_rename_binaries, to_au_bundle_name, to_auv2_component_name, to_pascal_case, to_vst3_bundle_name, Arch, PathExt};

/// Read version from workspace Cargo.toml and convert to Apple's version integer format
pub fn get_version_info(workspace_root: &Path) -> Result<(String, u32), String> {
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

fn find_beamer_au_out_dir(workspace_root: &Path, target: &str, profile: &str) -> Option<PathBuf> {
    let build_dir = workspace_root
        .join("target")
        .join(target)
        .join(profile)
        .join("build");

    if !build_dir.exists() {
        return None;
    }

    // Find beamer-au-* directory
    for entry in fs::read_dir(&build_dir).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("beamer-au-") {
            let out_dir = entry.path().join("out");
            if out_dir.exists() {
                return Some(out_dir);
            }
        }
    }

    None
}

/// Generate RUSTFLAGS for linking AU static libraries.
///
/// The `objc_lib_dir` parameter points to where plugin-specific ObjC static libraries
/// were compiled (contains libplugin_au_objc.a and libplugin_au_extension.a).
fn get_au_rustflags(
    plugin_name: &str,
    objc_lib_dir: &Path,
) -> Result<String, String> {
    let objc_lib_dir_str = objc_lib_dir.to_str()
        .ok_or_else(|| "Invalid ObjC library directory path".to_string())?;

    // Generate plugin-specific class and function names
    let pascal_name = to_pascal_case(plugin_name);
    let wrapper_class = format!("Beamer{}AuWrapper", pascal_name);
    let extension_class = format!("Beamer{}AuExtension", pascal_name);
    let factory_func = format!("Beamer{}AuExtensionFactory", pascal_name);

    // Library names based on plugin
    let lib_name = plugin_name.replace('-', "_");

    // Build RUSTFLAGS with all necessary linker arguments
    // Note: beamer_au_appex_force_link is internal (not exported) to avoid symbol collisions
    let flags = [
        format!("-L native={}", objc_lib_dir_str),
        format!("-C link-arg=-Wl,-force_load,{}/lib{}_au_objc.a", objc_lib_dir_str, lib_name),
        format!("-C link-arg=-Wl,-force_load,{}/lib{}_au_extension.a", objc_lib_dir_str, lib_name),
        format!("-C link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_{}", wrapper_class),
        format!("-C link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_{}", extension_class),
        format!("-C link-arg=-Wl,-exported_symbol,_OBJC_METACLASS_$_{}", wrapper_class),
        format!("-C link-arg=-Wl,-exported_symbol,_OBJC_METACLASS_$_{}", extension_class),
        format!("-C link-arg=-Wl,-exported_symbol,_{}", factory_func),
    ];

    Ok(flags.join(" "))
}

/// Build beamer-au to ensure static libraries exist before building plugins.
fn ensure_beamer_au_built(workspace_root: &Path, target: &str, release: bool) -> Result<(), String> {
    let profile = if release { "release" } else { "debug" };

    // Check if already built
    if find_beamer_au_out_dir(workspace_root, target, profile).is_some() {
        return Ok(());
    }

    println!("Building beamer-au for {}...", target);
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg("beamer-au")
        .arg("--target")
        .arg(target)
        .current_dir(workspace_root);

    if release {
        cmd.arg("--release");
    }

    let status = cmd.status()
        .map_err(|e| format!("Failed to build beamer-au: {}", e))?;

    if !status.success() {
        return Err("Failed to build beamer-au".to_string());
    }

    Ok(())
}

/// Get the current host target triple.
pub fn current_target() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin";

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin";

    #[cfg(not(target_os = "macos"))]
    compile_error!("Unsupported platform");
}

/// Build for a single architecture (native, arm64, or x86_64).
pub fn build_native(
    package: &str,
    release: bool,
    workspace_root: &Path,
    format: &str,
    arch: Arch,
    verbose: bool,
) -> Result<PathBuf, String> {
    // Always use explicit target to prevent RUSTFLAGS leaking into build scripts
    let target = match arch {
        Arch::Native => current_target(),
        Arch::Arm64 => "aarch64-apple-darwin",
        Arch::X86_64 => "x86_64-apple-darwin",
        Arch::Universal => unreachable!("Universal should use build_universal"),
    };

    let arch_name = if target.starts_with("aarch64") { "arm64" } else { "x86_64" };
    crate::status!("  Building {} ({})...", format.to_uppercase(), arch_name);

    let profile = if release { "release" } else { "debug" };
    let lib_name = package.replace('-', "_");
    let dylib_name = format!("lib{}.dylib", lib_name);

    // AU requires additional setup (beamer-au and ObjC code)
    let rustflags = if format == "au" {
        ensure_beamer_au_built(workspace_root, target, release)?;
        crate::verbose!(verbose, "    Generating plugin-specific ObjC for {}...", package);
        let objc_lib_dir = compile_plugin_objc(package, workspace_root, target)?;
        Some(get_au_rustflags(package, &objc_lib_dir)?)
    } else {
        None
    };

    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg(package)
        .arg("--target")
        .arg(target)
        .arg("--features")
        .arg(format)
        .current_dir(workspace_root);

    if release {
        cmd.arg("--release");
    }

    if let Some(flags) = &rustflags {
        cmd.env("RUSTFLAGS", flags);
    }

    let status = cmd.status().map_err(|e| format!("Failed to run cargo: {}", e))?;
    if !status.success() {
        return Err(format!("{} build failed", format.to_uppercase()));
    }

    // Output is always in target/<target>/<profile>/
    let dylib_path = workspace_root.join("target").join(target).join(profile).join(&dylib_name);

    if !dylib_path.exists() {
        return Err(format!("Built library not found: {}", dylib_path.display()));
    }

    crate::verbose!(verbose, "    Binary: {}", dylib_path.display());
    Ok(dylib_path)
}

/// Build universal binary (x86_64 + arm64) for the given format.
pub fn build_universal(
    package: &str,
    release: bool,
    workspace_root: &Path,
    format: &str,
    verbose: bool,
) -> Result<PathBuf, String> {
    crate::status!("  Building {} (universal)...", format.to_uppercase());

    let profile = if release { "release" } else { "debug" };
    let lib_name = package.replace('-', "_");
    let dylib_name = format!("lib{}.dylib", lib_name);

    // AU requires additional setup (beamer-au and ObjC code)
    let (rustflags_x86, rustflags_arm) = if format == "au" {
        ensure_beamer_au_built(workspace_root, "x86_64-apple-darwin", release)?;
        ensure_beamer_au_built(workspace_root, "aarch64-apple-darwin", release)?;

        crate::verbose!(verbose, "    Generating plugin-specific ObjC for {}...", package);
        let objc_lib_dir_x86 = compile_plugin_objc(package, workspace_root, "x86_64-apple-darwin")?;
        let objc_lib_dir_arm = compile_plugin_objc(package, workspace_root, "aarch64-apple-darwin")?;

        (
            Some(get_au_rustflags(package, &objc_lib_dir_x86)?),
            Some(get_au_rustflags(package, &objc_lib_dir_arm)?),
        )
    } else {
        (None, None)
    };

    // Build for x86_64
    crate::verbose!(verbose, "    Building for x86_64...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg(package)
        .arg("--target")
        .arg("x86_64-apple-darwin")
        .arg("--features")
        .arg(format)
        .current_dir(workspace_root);

    if release {
        cmd.arg("--release");
    }

    if let Some(flags) = &rustflags_x86 {
        cmd.env("RUSTFLAGS", flags);
    }

    let status = cmd.status().map_err(|e| format!("Failed to build for x86_64: {}", e))?;
    if !status.success() {
        return Err("Build for x86_64 failed".to_string());
    }

    // Build for arm64
    crate::verbose!(verbose, "    Building for arm64...");
    let mut cmd = Command::new("cargo");
    cmd.arg("build")
        .arg("-p")
        .arg(package)
        .arg("--target")
        .arg("aarch64-apple-darwin")
        .arg("--features")
        .arg(format)
        .current_dir(workspace_root);

    if release {
        cmd.arg("--release");
    }

    if let Some(flags) = &rustflags_arm {
        cmd.env("RUSTFLAGS", flags);
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

    // Combine using lipo (don't clean up - these are standard cargo output directories)
    crate::verbose!(verbose, "    Creating universal binary with lipo...");
    combine_or_rename_binaries(&[x86_64_path, arm64_path], &universal_path, false)?;

    crate::verbose!(verbose, "    Binary: {}", universal_path.display());

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
pub fn clean_build_caches(
    workspace_root: &Path,
    package: &str,
    release: bool,
    verbose: bool,
    build_auv2: bool,
    build_auv3: bool,
    build_vst3: bool,
) -> Result<(), String> {
    let profile = if release { "release" } else { "debug" };
    let target_dir = workspace_root.join("target").join(profile);
    let clean_au = build_auv2 || build_auv3;

    // Build description of what we're cleaning
    let mut targets = Vec::new();
    if clean_au {
        targets.push("AU caches");
    }
    if build_auv2 {
        targets.push("AUv2");
    }
    if build_auv3 {
        targets.push("AUv3");
    }
    if build_vst3 {
        targets.push("VST3");
    }
    crate::status!("  Cleaning ({})...", targets.join(", "));

    // Clean beamer-au cc cache (compiled ObjC objects) - only for AU builds
    if clean_au {
        let build_dir = workspace_root.join("target").join(profile).join("build");
        if build_dir.exists() {
            for entry in fs::read_dir(&build_dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                if name.to_string_lossy().starts_with("beamer-au-") {
                    crate::verbose!(verbose, "    Removing: {}", entry.path().display());
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
                    crate::verbose!(verbose, "    Removing: {}", entry.path().display());
                    fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // Clean previous VST3 bundle
    if build_vst3 {
        let vst3_bundle_name = to_vst3_bundle_name(package);
        let vst3_path = target_dir.join(&vst3_bundle_name);
        if vst3_path.exists() {
            crate::verbose!(verbose, "    Removing: {}", vst3_path.display());
            fs::remove_dir_all(&vst3_path).map_err(|e| e.to_string())?;
        }
    }

    // Clean previous AUv2 component bundle
    if build_auv2 {
        let auv2_bundle_name = to_auv2_component_name(package);
        let component_path = target_dir.join(&auv2_bundle_name);
        if component_path.exists() {
            crate::verbose!(verbose, "    Removing: {}", component_path.display());
            fs::remove_dir_all(&component_path).map_err(|e| e.to_string())?;
        }
    }

    // Clean previous AUv3 app bundle
    if build_auv3 {
        let auv3_bundle_name = to_au_bundle_name(package);
        let app_path = target_dir.join(&auv3_bundle_name);
        if app_path.exists() {
            crate::verbose!(verbose, "    Removing: {}", app_path.display());
            fs::remove_dir_all(&app_path).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

/// Generate and compile plugin-specific ObjC code.
///
/// This creates uniquely named ObjC classes for each plugin to avoid symbol collisions
/// when multiple AU plugins are loaded in the same process (e.g., in Reaper).
///
/// Returns the path to the directory containing the compiled static libraries.
pub fn compile_plugin_objc(
    plugin_name: &str,
    workspace_root: &Path,
    target: &str,
) -> Result<PathBuf, String> {
    let lib_name = plugin_name.replace('-', "_");

    // Create output directory for generated sources and compiled libraries
    let gen_dir = workspace_root
        .join("target")
        .join("au-gen")
        .join(plugin_name)
        .join(target);

    fs::create_dir_all(&gen_dir)
        .map_err(|e| format!("Failed to create au-gen directory: {}", e))?;

    // Generate plugin-specific ObjC source files
    let wrapper_source = generate_au_wrapper_source(plugin_name);
    let has_editor = crate::util::detect_has_editor(plugin_name, workspace_root);
    let extension_source = generate_au_extension_source(plugin_name, has_editor);

    let wrapper_path = gen_dir.join("AuWrapper.m");
    let extension_path = gen_dir.join("AuExtension.m");

    fs::write(&wrapper_path, wrapper_source)
        .map_err(|e| format!("Failed to write AuWrapper.m: {}", e))?;
    fs::write(&extension_path, extension_source)
        .map_err(|e| format!("Failed to write AuExtension.m: {}", e))?;

    // Path to BeamerAuBridge.h header
    let bridge_header_dir = workspace_root.join("crates/beamer-au/objc");

    // Determine architecture for clang
    let arch = match target {
        "x86_64-apple-darwin" => "x86_64",
        "aarch64-apple-darwin" => "arm64",
        _ => return Err(format!("Unsupported target: {}", target)),
    };

    // Compile AuWrapper.m to static library
    let wrapper_obj = gen_dir.join("AuWrapper.o");
    let wrapper_lib = gen_dir.join(format!("lib{}_au_objc.a", lib_name));

    let status = Command::new("clang")
        .args([
            "-c",
            "-arch", arch,
            "-fobjc-arc",
            "-fmodules",
            "-I", bridge_header_dir.to_str_safe()?,
            "-o", wrapper_obj.to_str_safe()?,
            wrapper_path.to_str_safe()?,
        ])
        .status()
        .map_err(|e| format!("Failed to compile AuWrapper.m: {}", e))?;

    if !status.success() {
        return Err("Failed to compile AuWrapper.m".to_string());
    }

    // Create static library from object file
    let status = Command::new("ar")
        .args(["rcs", wrapper_lib.to_str_safe()?, wrapper_obj.to_str_safe()?])
        .status()
        .map_err(|e| format!("Failed to create static library: {}", e))?;

    if !status.success() {
        return Err("Failed to create AuWrapper static library".to_string());
    }

    // Compile AuExtension.m to static library
    let extension_obj = gen_dir.join("AuExtension.o");
    let extension_lib = gen_dir.join(format!("lib{}_au_extension.a", lib_name));

    let status = Command::new("clang")
        .args([
            "-c",
            "-arch", arch,
            "-fobjc-arc",
            "-fmodules",
            "-I", bridge_header_dir.to_str_safe()?,
            "-o", extension_obj.to_str_safe()?,
            extension_path.to_str_safe()?,
        ])
        .status()
        .map_err(|e| format!("Failed to compile AuExtension.m: {}", e))?;

    if !status.success() {
        return Err("Failed to compile AuExtension.m".to_string());
    }

    let status = Command::new("ar")
        .args(["rcs", extension_lib.to_str_safe()?, extension_obj.to_str_safe()?])
        .status()
        .map_err(|e| format!("Failed to create static library: {}", e))?;

    if !status.success() {
        return Err("Failed to create AuExtension static library".to_string());
    }

    // Clean up object files
    let _ = fs::remove_file(&wrapper_obj);
    let _ = fs::remove_file(&extension_obj);

    Ok(gen_dir)
}

// Include the AUv3 ObjC code generation functions
include!("au_codegen/auv3_objc.rs");
