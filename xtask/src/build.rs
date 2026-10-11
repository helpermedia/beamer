//! Build orchestration and AU ObjC code generation.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{combine_or_rename_binaries, to_au_bundle_name, to_auv2_component_name, to_pascal_case, to_vst3_bundle_name, Arch, PathExt};
use crate::BundleConfig;

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

/// Extra rustc arguments for the plugin crate of an AU build.
///
/// They link the AU static libraries into the plugin. The `objc_lib_dir`
/// parameter points to where plugin-specific ObjC static libraries were
/// compiled (contains libplugin_au_objc.a and libplugin_au_extension.a).
fn au_rustc_args(
    plugin_name: &str,
    objc_lib_dir: &Path,
) -> Result<Vec<String>, String> {
    let objc_lib_dir_str = objc_lib_dir.to_str()
        .ok_or_else(|| "Invalid ObjC library directory path".to_string())?;

    // Generate plugin-specific class and function names
    let pascal_name = to_pascal_case(plugin_name);
    let wrapper_class = format!("Beamer{}AuWrapper", pascal_name);
    let extension_class = format!("Beamer{}AuExtension", pascal_name);
    let factory_func = format!("Beamer{}AuExtensionFactory", pascal_name);

    // Library names based on plugin
    let lib_name = plugin_name.replace('-', "_");

    // Every linker argument the plugin needs, as flag and value pairs.
    // Note: beamer_au_appex_force_link is internal (not exported) to avoid symbol collisions.
    let args = [
        ("-L", format!("native={}", objc_lib_dir_str)),
        ("-C", format!("link-arg=-Wl,-force_load,{}/lib{}_au_objc.a", objc_lib_dir_str, lib_name)),
        ("-C", format!("link-arg=-Wl,-force_load,{}/lib{}_au_extension.a", objc_lib_dir_str, lib_name)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_{}", wrapper_class)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_{}", extension_class)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_OBJC_METACLASS_$_{}", wrapper_class)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_OBJC_METACLASS_$_{}", extension_class)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_{}", factory_func)),
    ];

    // Every flag and its value is a separate argument for rustc.
    Ok(args
        .into_iter()
        .flat_map(|(flag, value)| [flag.to_string(), value])
        .collect())
}

/// Find the commit hash in the output of `rustc -vV`.
fn rustc_commit_hash(version_output: &str) -> Option<&str> {
    version_output
        .lines()
        .find_map(|line| line.strip_prefix("commit-hash: "))
        .map(str::trim)
}

/// Flags that replace the local path prefixes rustc would embed.
///
/// The workspace becomes `.`, the Cargo home (registry and git dependencies)
/// `/cargo` and the standard library sources `/rustc/<commit>`, which is the
/// path the prebuilt standard library already uses for itself.
fn remap_path_flags_for(
    workspace_root: &str,
    cargo_home: &str,
    rust_src: &str,
    commit_hash: &str,
) -> Vec<String> {
    vec![
        format!("--remap-path-prefix={}=.", workspace_root),
        format!("--remap-path-prefix={}=/cargo", cargo_home),
        format!("--remap-path-prefix={}=/rustc/{}", rust_src, commit_hash),
    ]
}

/// Flags that keep the paths of the build machine out of a release binary.
///
/// rustc embeds source paths for panic messages. They are absolute, so they
/// contain the home directory of whoever built the plugin.
fn remap_path_flags(workspace_root: &Path) -> Result<Vec<String>, String> {
    // rustup sets CARGO_HOME for the tools it runs. Fall back to the default
    // location for a toolchain that was installed in another way.
    let cargo_home = match std::env::var_os("CARGO_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|home| PathBuf::from(home).join(".cargo"))
            .ok_or("Could not find the Cargo home directory")?,
    };

    // Ask the compiler that cargo will use: the one from RUSTC if set, else
    // the `rustc` that rustup resolves for the workspace.
    let rustc_path = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let rustc = |args: &[&str]| -> Result<String, String> {
        let output = Command::new(&rustc_path)
            .args(args)
            .current_dir(workspace_root)
            .output()
            .map_err(|e| format!("Failed to run rustc: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = match stderr.trim() {
                "" => String::new(),
                message => format!(": {}", message),
            };
            return Err(format!("rustc {} failed{}", args.join(" "), reason));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    };

    // The same join rustc uses for the directory it looks for the sources in.
    let rust_src = PathBuf::from(rustc(&["--print", "sysroot"])?).join("lib/rustlib/src/rust");
    let version = rustc(&["-vV"])?;
    let commit_hash = rustc_commit_hash(&version).ok_or("Could not find the rustc commit hash")?;

    Ok(remap_path_flags_for(
        workspace_root.to_str_safe()?,
        cargo_home.to_str_safe()?,
        rust_src.to_str_safe()?,
        commit_hash,
    ))
}

/// Compiler flags the user set in the environment.
///
/// xtask passes its flags in `CARGO_ENCODED_RUSTFLAGS`, which makes cargo
/// ignore `RUSTFLAGS`, so the user's flags are carried over. `encoded` and
/// `plain` are the values of the two variables. As in cargo, the encoded one
/// wins when it is set, even if it is empty.
fn user_rustflags(encoded: Option<&str>, plain: Option<&str>) -> Vec<String> {
    let flags: Vec<&str> = match (encoded, plain) {
        (Some(encoded), _) => encoded.split('\x1f').collect(),
        (None, Some(plain)) => plain.split(' ').map(str::trim).collect(),
        (None, None) => Vec::new(),
    };

    flags.into_iter().filter(|flag| !flag.is_empty()).map(String::from).collect()
}

/// Join the compiler flags of a plugin build for `CARGO_ENCODED_RUSTFLAGS`.
///
/// The user's flags come first, so that xtask's flags win where rustc takes
/// the last of a repeated option. Returns `None` when there are no flags.
/// The variables then stay as they are and cargo applies its own rules.
fn encode_rustflags(user: &[String], remap: &[String]) -> Option<String> {
    let flags: Vec<&str> = user.iter().chain(remap).map(String::as_str).collect();

    if flags.is_empty() {
        None
    } else {
        Some(flags.join("\x1f"))
    }
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

/// Extra rustc arguments for the plugin crate of a VST3 build.
///
/// On macOS the VST3 bundle binary is a copy of the plugin dylib. Without an
/// explicit install name the linker uses the absolute path of the build
/// output, which then ships in the bundle and lets dyld bind a later request
/// for that path to the plugin. Name the dylib after the bundle binary,
/// relative to the loading image, instead.
fn vst3_rustc_args(package: &str, target: &str) -> Vec<String> {
    // The install name is a Mach-O concept, so only Apple targets get it.
    if !target.ends_with("-apple-darwin") {
        return Vec::new();
    }

    let bundle_name = to_vst3_bundle_name(package);
    let executable_name = bundle_name.trim_end_matches(".vst3");

    vec![
        "-C".to_string(),
        format!("link-arg=-Wl,-install_name,@loader_path/{}", executable_name),
    ]
}

/// Create the cargo command that builds the plugin crate for one target.
///
/// `plugin_args` are extra rustc arguments for the plugin crate alone, from
/// `au_rustc_args` or `vst3_rustc_args`. They go through `cargo rustc`, which
/// passes them to that crate only. Without them the command is a plain
/// `cargo build`.
///
/// The flags for every crate are the user's own and, for release builds, the
/// ones from `remap_path_flags`. They have to be the same for every format and
/// plugin: cargo leaves flags that contain `--remap-path-prefix` out of the
/// artifact file names, so all flag values share one set of artifacts and a
/// difference rebuilds every crate. The encoded form keeps paths with spaces
/// intact.
fn plugin_build_command(
    package: &str,
    target: &str,
    format: &str,
    release: bool,
    workspace_root: &Path,
    plugin_args: &[String],
) -> Result<Command, String> {
    let mut cmd = Command::new("cargo");
    cmd.arg(if plugin_args.is_empty() { "build" } else { "rustc" })
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

    if !plugin_args.is_empty() {
        cmd.arg("--lib").arg("--").args(plugin_args);
    }

    let user = user_rustflags(
        std::env::var("CARGO_ENCODED_RUSTFLAGS").ok().as_deref(),
        std::env::var("RUSTFLAGS").ok().as_deref(),
    );
    let remap = if release { remap_path_flags(workspace_root)? } else { Vec::new() };

    if let Some(encoded) = encode_rustflags(&user, &remap) {
        cmd.env("CARGO_ENCODED_RUSTFLAGS", encoded);
    }

    Ok(cmd)
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
    // Always use explicit target to prevent the rustc flags leaking into build scripts.
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

    // AU requires additional setup (ObjC code).
    let plugin_args = if format == "au" {
        crate::verbose!(verbose, "    Generating plugin-specific ObjC for {}...", package);
        let objc_lib_dir = compile_plugin_objc(package, workspace_root, target)?;
        au_rustc_args(package, &objc_lib_dir)?
    } else {
        vst3_rustc_args(package, target)
    };

    let mut cmd = plugin_build_command(package, target, format, release, workspace_root, &plugin_args)?;

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

    // AU requires additional setup (ObjC code).
    let (plugin_args_x86, plugin_args_arm) = if format == "au" {
        crate::verbose!(verbose, "    Generating plugin-specific ObjC for {}...", package);
        let objc_lib_dir_x86 = compile_plugin_objc(package, workspace_root, "x86_64-apple-darwin")?;
        let objc_lib_dir_arm = compile_plugin_objc(package, workspace_root, "aarch64-apple-darwin")?;

        (
            au_rustc_args(package, &objc_lib_dir_x86)?,
            au_rustc_args(package, &objc_lib_dir_arm)?,
        )
    } else {
        (
            vst3_rustc_args(package, "x86_64-apple-darwin"),
            vst3_rustc_args(package, "aarch64-apple-darwin"),
        )
    };

    // Build for x86_64
    crate::verbose!(verbose, "    Building for x86_64...");
    let mut cmd = plugin_build_command(package, "x86_64-apple-darwin", format, release, workspace_root, &plugin_args_x86)?;

    let status = cmd.status().map_err(|e| format!("Failed to build for x86_64: {}", e))?;
    if !status.success() {
        return Err("Build for x86_64 failed".to_string());
    }

    // Build for arm64
    crate::verbose!(verbose, "    Building for arm64...");
    let mut cmd = plugin_build_command(package, "aarch64-apple-darwin", format, release, workspace_root, &plugin_args_arm)?;

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

/// Target triples that are built for an architecture selection.
fn target_triples(arch: Arch) -> Vec<&'static str> {
    match arch {
        Arch::Native => vec![current_target()],
        Arch::Arm64 => vec!["aarch64-apple-darwin"],
        Arch::X86_64 => vec!["x86_64-apple-darwin"],
        Arch::Universal => vec!["x86_64-apple-darwin", "aarch64-apple-darwin"],
    }
}

/// Force a rebuild of the plugin and remove its previous bundles.
///
/// xtask compiles the generated AUv3 ObjC code into static libraries and
/// links them into the plugin through rustc arguments. Cargo does not track those
/// libraries, so a change to the ObjC templates or to `au_ipc_helpers.h`
/// alone does not relink the plugin. Cleaning the plugin package forces a
/// rebuild that picks the change up.
pub fn clean_build_caches(workspace_root: &Path, config: &BundleConfig) -> Result<(), String> {
    let package = config.package.as_str();
    let verbose = config.verbose;

    let profile = if config.release { "release" } else { "debug" };
    let target_dir = workspace_root.join("target").join(profile);

    // Build description of what we're cleaning
    let mut formats = Vec::new();
    if config.build_auv2 {
        formats.push("AUv2");
    }
    if config.build_auv3 {
        formats.push("AUv3");
    }
    if config.build_vst3 {
        formats.push("VST3");
    }
    crate::status!("  Cleaning ({})...", formats.join(", "));

    // Remove the plugin's build artifacts so that cargo rebuilds and relinks it.
    for target in target_triples(config.arch) {
        let mut cmd = Command::new("cargo");
        cmd.arg("clean")
            .arg("-p")
            .arg(package)
            .arg("--target")
            .arg(target)
            .current_dir(workspace_root);

        if config.release {
            cmd.arg("--release");
        }

        if !verbose {
            cmd.arg("--quiet");
        }

        let status = cmd.status().map_err(|e| format!("Failed to run cargo clean: {}", e))?;
        if !status.success() {
            return Err(format!("cargo clean failed for {}", target));
        }
    }

    // Remove the generated ObjC sources and static libraries (AU builds only).
    if config.build_auv2 || config.build_auv3 {
        let gen_dir = workspace_root.join("target").join("au-gen").join(package);
        if gen_dir.exists() {
            crate::verbose!(verbose, "    Removing: {}", gen_dir.display());
            fs::remove_dir_all(&gen_dir).map_err(|e| e.to_string())?;
        }
    }

    // Clean previous VST3 bundle
    if config.build_vst3 {
        let vst3_bundle_name = to_vst3_bundle_name(package);
        let vst3_path = target_dir.join(&vst3_bundle_name);
        if vst3_path.exists() {
            crate::verbose!(verbose, "    Removing: {}", vst3_path.display());
            fs::remove_dir_all(&vst3_path).map_err(|e| e.to_string())?;
        }
    }

    // Clean previous AUv2 component bundle
    if config.build_auv2 {
        let auv2_bundle_name = to_auv2_component_name(package);
        let component_path = target_dir.join(&auv2_bundle_name);
        if component_path.exists() {
            crate::verbose!(verbose, "    Removing: {}", component_path.display());
            fs::remove_dir_all(&component_path).map_err(|e| e.to_string())?;
        }
    }

    // Clean previous AUv3 app bundle
    if config.build_auv3 {
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
    let has_gui = crate::util::detect_has_gui(plugin_name, workspace_root);
    let extension_source = generate_au_extension_source(plugin_name, has_gui);

    let wrapper_path = gen_dir.join("AuWrapper.m");
    let extension_path = gen_dir.join("AuExtension.m");

    fs::write(&wrapper_path, wrapper_source)
        .map_err(|e| format!("Failed to write AuWrapper.m: {}", e))?;
    fs::write(&extension_path, extension_source)
        .map_err(|e| format!("Failed to write AuExtension.m: {}", e))?;

    // Header search paths for clang
    let bridge_header_dir = workspace_root.join("crates/beamer-au/objc");
    let ipc_header_dir = workspace_root.join("xtask/src/au_codegen");

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
            "-I", ipc_header_dir.to_str_safe()?,
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
            "-I", ipc_header_dir.to_str_safe()?,
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

/// Detect the package manager to use for a webview project.
///
/// Checks for lockfiles in order of specificity, defaulting to bun.
fn detect_package_manager(webview_dir: &Path) -> (&'static str, &'static [&'static str], &'static [&'static str]) {
    if webview_dir.join("bun.lock").exists() {
        ("bun", &["install"], &["run", "build"])
    } else if webview_dir.join("pnpm-lock.yaml").exists() {
        ("pnpm", &["install"], &["run", "build"])
    } else if webview_dir.join("yarn.lock").exists() {
        ("yarn", &["install"], &["run", "build"])
    } else if webview_dir.join("package-lock.json").exists() {
        ("npm", &["install"], &["run", "build"])
    } else {
        ("bun", &["install"], &["run", "build"])
    }
}

/// Build webview assets if the package has a framework-based webview.
///
/// Detects the package manager from lockfiles (defaulting to bun),
/// runs install + build and verifies that dist/index.html was produced.
/// Skips silently if there is no webview/package.json (plain HTML or no webview).
pub fn build_webview(package_dir: &Path, verbose: bool) -> Result<(), String> {
    let webview_dir = package_dir.join("webview");
    let package_json = webview_dir.join("package.json");

    if !package_json.exists() {
        return Ok(()); // Plain HTML or no webview
    }

    let (cmd, install_args, run_args) = detect_package_manager(&webview_dir);

    crate::status!("  Building webview ({})...", cmd);

    // Install dependencies
    crate::verbose!(verbose, "    Running {} install...", cmd);
    let status = Command::new(cmd)
        .args(install_args)
        .current_dir(&webview_dir)
        .status()
        .map_err(|e| format!("Failed to run {} install: {}", cmd, e))?;

    if !status.success() {
        return Err(format!("{} install failed", cmd));
    }

    // Run build
    crate::verbose!(verbose, "    Running {} run build...", cmd);
    let status = Command::new(cmd)
        .args(run_args)
        .current_dir(&webview_dir)
        .status()
        .map_err(|e| format!("Failed to run {} build: {}", cmd, e))?;

    if !status.success() {
        return Err(format!("{} build failed", cmd));
    }

    // Verify output
    let dist_dir = webview_dir.join("dist");
    if !dist_dir.join("index.html").exists() {
        return Err("webview build did not produce dist/index.html".to_string());
    }

    crate::verbose!(verbose, "    Webview build complete");
    Ok(())
}

// Include the AUv3 ObjC code generation functions
include!("au_codegen/auv3_objc.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rustc_commit_hash() {
        let output = "rustc 1.92.0 (ded5c06cf 2025-12-08)\nbinary: rustc\ncommit-hash: ded5c06cf21d2b93bffd5d884aa6e96934ee4234\ncommit-date: 2025-12-08\nhost: aarch64-apple-darwin\n";
        assert_eq!(
            rustc_commit_hash(output),
            Some("ded5c06cf21d2b93bffd5d884aa6e96934ee4234")
        );
        assert_eq!(rustc_commit_hash("rustc 1.92.0\nhost: aarch64-apple-darwin\n"), None);
    }

    #[test]
    fn test_user_rustflags() {
        // The encoded variable wins, also when it is empty.
        assert_eq!(user_rustflags(Some("-C\x1ftarget-cpu=native"), Some("--cfg a")), ["-C", "target-cpu=native"]);
        assert!(user_rustflags(Some(""), Some("--cfg a")).is_empty());

        // RUSTFLAGS is split on spaces, repeated spaces give no empty flags.
        assert_eq!(user_rustflags(None, Some(" --cfg  a ")), ["--cfg", "a"]);
        assert!(user_rustflags(None, None).is_empty());
    }

    #[test]
    fn test_encode_rustflags() {
        let user = ["--cfg".to_string(), "a".to_string()];
        let remap = remap_path_flags_for("/Users/me/My Projects/beamer", "/Users/me/.cargo", "/sysroot/src", "abc123");

        // User flags first, then the remap flags. A space inside a flag does
        // not end it.
        let encoded = encode_rustflags(&user, &remap).unwrap();
        assert_eq!(
            encoded.split('\x1f').collect::<Vec<_>>(),
            [
                "--cfg",
                "a",
                "--remap-path-prefix=/Users/me/My Projects/beamer=.",
                "--remap-path-prefix=/Users/me/.cargo=/cargo",
                "--remap-path-prefix=/sysroot/src=/rustc/abc123",
            ]
        );

        // Without flags the variable is not set at all.
        assert_eq!(encode_rustflags(&[], &[]), None);
    }

    #[test]
    fn test_vst3_rustc_args() {
        assert_eq!(
            vst3_rustc_args("midi-transform", "aarch64-apple-darwin"),
            ["-C", "link-arg=-Wl,-install_name,@loader_path/BeamerMidiTransform"]
        );

        // Only Mach-O has install names.
        assert!(vst3_rustc_args("midi-transform", "x86_64-pc-windows-msvc").is_empty());
    }

    #[test]
    fn test_au_rustc_args_are_separate_arguments() {
        let args = au_rustc_args("midi-transform", Path::new("/tmp/My Projects/au-gen")).unwrap();

        // Every flag is followed by its value, and a space in the path does
        // not split an argument.
        assert_eq!(args.len() % 2, 0);
        assert_eq!(&args[..2], ["-L", "native=/tmp/My Projects/au-gen"]);
        assert_eq!(args[2], "-C");
        assert_eq!(
            args[3],
            "link-arg=-Wl,-force_load,/tmp/My Projects/au-gen/libmidi_transform_au_objc.a"
        );
        assert!(args.chunks(2).all(|pair| pair[0] == "-L" || pair[0] == "-C"));
        assert!(args.contains(&"link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_BeamerMidiTransformAuWrapper".to_string()));
    }
}
