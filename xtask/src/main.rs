//! Build tooling for Beamer plugins.
//!
//! Usage: cargo xtask bundle <package> [--vst3] [--auv2] [--auv3] [--arch <arch>] [--release] [--install] [--clean]

mod auv2;
mod auv3;
mod build;
mod util;
mod vst3;

use std::path::PathBuf;
use std::process::Command;

use util::{print_error, Arch};

// =============================================================================
// Configuration Structs
// =============================================================================

/// Configuration for creating appex Info.plist (AUv3)
pub struct AppexPlistConfig<'a> {
    pub package: &'a str,
    pub executable_name: &'a str,
    pub component_type: &'a str,
    pub manufacturer: Option<&'a str>,
    pub subtype: Option<&'a str>,
    pub framework_bundle_id: &'a str,
    pub version_string: &'a str,
    pub version_int: u32,
    pub plugin_name: Option<&'a str>,
    pub vendor_name: Option<&'a str>,
    pub has_editor: bool,
}

/// Configuration for creating AUv2 component Info.plist
pub struct ComponentPlistConfig<'a> {
    pub package: &'a str,
    pub executable_name: &'a str,
    pub component_type: &'a str,
    pub manufacturer: Option<&'a str>,
    pub subtype: Option<&'a str>,
    pub version_string: &'a str,
    pub version_int: u32,
    pub plugin_name: Option<&'a str>,
    pub vendor_name: Option<&'a str>,
}

/// Configuration for the bundle command
struct BundleConfig {
    package: String,
    release: bool,
    install: bool,
    clean: bool,
    build_vst3: bool,
    build_auv2: bool,
    build_auv3: bool,
    arch: Arch,
    verbose: bool,
}

// =============================================================================
// UUID Generation
// =============================================================================

/// Generate a new UUID for plugin identification.
///
/// Outputs a UUID in the standard format: XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX
/// This can be used directly with `Vst3Config::new()` in VST3 configs.
fn generate_uuid() {
    let uuid = uuid::Uuid::new_v4();
    // Format as uppercase without braces, matching uuidgen output
    println!("{}", uuid.as_hyphenated().to_string().to_uppercase());
}

// =============================================================================
// Entry Point
// =============================================================================

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let command = &args[1];

    match command.as_str() {
        "generate-uuid" => {
            generate_uuid();
            return;
        }
        "bundle" => {
            if args.len() < 3 {
                print_error("bundle command requires a package name");
                print_usage();
                std::process::exit(1);
            }
        }
        _ => {
            print_error(&format!("unknown command '{}'", command));
            print_usage();
            std::process::exit(1);
        }
    }

    let package = &args[2];
    let release = args.iter().any(|a| a == "--release");
    let install = args.iter().any(|a| a == "--install");
    let clean = args.iter().any(|a| a == "--clean");
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");
    let build_vst3 = args.iter().any(|a| a == "--vst3");
    let build_auv2 = args.iter().any(|a| a == "--auv2");
    let build_auv3 = args.iter().any(|a| a == "--auv3");

    // Parse --arch flag
    let arch = args.windows(2)
        .find(|w| w[0] == "--arch")
        .map(|w| {
            Arch::from_str(&w[1]).unwrap_or_else(|| {
                eprintln!("Warning: unrecognized arch '{}', using native", w[1]);
                Arch::Native
            })
        })
        .unwrap_or(Arch::Native);

    // Check for unknown flags
    let known_flags = ["--release", "--install", "--clean", "--verbose", "-v", "--vst3", "--auv2", "--auv3", "--arch"];
    let arch_values = ["native", "universal", "arm64", "x86_64"];
    for arg in args.iter().skip(3) {
        if arg.starts_with('-') && !known_flags.contains(&arg.as_str()) {
            print_error(&format!("unknown flag '{}'", arg));
            eprintln!("Known flags: {}", known_flags.join(", "));
            std::process::exit(1);
        } else if !arg.starts_with("--") && !arch_values.contains(&arg.as_str()) {
            print_error(&format!("unexpected argument '{}'", arg));
            print_usage();
            std::process::exit(1);
        }
    }

    // Require at least one format flag
    if !build_vst3 && !build_auv2 && !build_auv3 {
        print_error("at least one format flag is required (--auv2, --auv3, or --vst3)");
        print_usage();
        std::process::exit(1);
    }

    let config = BundleConfig {
        package: package.to_string(),
        release,
        install,
        clean,
        verbose,
        build_vst3,
        build_auv2,
        build_auv3,
        arch,
    };

    if let Err(e) = bundle(&config) {
        print_error(&e);
        std::process::exit(1);
    }
}

fn print_usage() {
    eprintln!("Usage: cargo xtask <command> [options]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  generate-uuid              Generate a new UUID for plugin identification");
    eprintln!("  bundle <package> [options] Build and bundle a plugin");
    eprintln!();
    eprintln!("Formats (at least one required):");
    eprintln!("  --auv2    Build AUv2 .component bundle (simple distribution, works with all DAWs)");
    eprintln!("  --auv3    Build AUv3 .app/.appex bundle (App Store distribution)");
    eprintln!("  --vst3    Build VST3 bundle");
    eprintln!();
    eprintln!("Architecture:");
    eprintln!("  --arch <arch>  Target architecture (default: native)");
    eprintln!("                 native    - Current machine's architecture only (fastest builds)");
    eprintln!("                 universal - x86_64 + arm64 (for distribution)");
    eprintln!("                 arm64     - Apple Silicon only");
    eprintln!("                 x86_64    - Intel only");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --release    Build in release mode");
    eprintln!("  --install    Install to system plugin directories");
    eprintln!("               AUv2: ~/Library/Audio/Plug-Ins/Components/");
    eprintln!("               AUv3: ~/Applications/");
    eprintln!("               VST3: ~/Library/Audio/Plug-Ins/VST3/");
    eprintln!("  --clean      Clean build caches before building (forces full rebuild)");
    eprintln!("               Removes beamer-au cc cache and previous bundles.");
    eprintln!("               Use when ObjC/header changes aren't being picked up.");
    eprintln!("  --verbose    Show detailed build output (default: quiet)");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  cargo xtask bundle gain --auv2 --release --install");
    eprintln!("  cargo xtask bundle gain --auv3 --release --install");
    eprintln!("  cargo xtask bundle gain --vst3 --release --install");
    eprintln!("  cargo xtask bundle gain --auv2 --auv3 --arch universal    # Both AU formats");
    eprintln!("  cargo xtask bundle gain --auv2 --vst3 --arch universal    # AUv2 + VST3");
}

// =============================================================================
// Bundle Orchestration
// =============================================================================

fn bundle(config: &BundleConfig) -> Result<(), String> {
    let arch_str = match config.arch {
        Arch::Native => "native",
        Arch::Universal => "universal",
        Arch::Arm64 => "arm64",
        Arch::X86_64 => "x86_64",
    };
    let profile_str = if config.release { "release" } else { "debug" };
    status!("Bundling {} ({}, {})...", config.package, profile_str, arch_str);

    // Get workspace root
    let workspace_root = get_workspace_root()?;

    // Clean build caches if requested
    if config.clean {
        build::clean_build_caches(
            &workspace_root,
            &config.package,
            config.release,
            config.verbose,
            config.build_auv2,
            config.build_auv3,
            config.build_vst3,
        )?;
    }

    // Determine paths
    let target_dir = workspace_root.join("target").join(profile_str);

    // Build and bundle AU (macOS only) - build once, bundle for each requested format
    if (config.build_auv2 || config.build_auv3) && cfg!(target_os = "macos") {
        let dylib_path = if config.arch == Arch::Universal {
            build::build_universal(&config.package, config.release, &workspace_root, "au", config.verbose)?
        } else {
            build::build_native(&config.package, config.release, &workspace_root, "au", config.arch, config.verbose)?
        };

        if config.build_auv2 {
            auv2::bundle_auv2(&config.package, &target_dir, &dylib_path, config.install, &workspace_root, config.arch, config.verbose)?;
        }
        if config.build_auv3 {
            auv3::bundle_auv3(&config.package, &target_dir, &dylib_path, config.install, &workspace_root, config.arch, config.verbose)?;
        }
    }

    // Build and bundle VST3
    if config.build_vst3 {
        let dylib_path = if config.arch == Arch::Universal {
            build::build_universal(&config.package, config.release, &workspace_root, "vst3", config.verbose)?
        } else {
            build::build_native(&config.package, config.release, &workspace_root, "vst3", config.arch, config.verbose)?
        };
        vst3::bundle_vst3(&config.package, &target_dir, &dylib_path, config.install, &workspace_root, config.verbose)?;
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
