//! Build tooling for Beamer plugins.
//!
//! Usage: `cargo xtask bundle <package> [--vst3] [--auv2] [--auv3] [--arch <arch>] [--release] [--install] [--clean]`
//!
//! `cargo xtask uninstall <package> [--vst3] [--auv2] [--auv3]` removes an installed plugin.

mod auv2;
mod auv3;
mod build;
mod util;
mod vst3;

use std::path::PathBuf;
use std::process::Command;

use util::{print_error, Arch, AuCodes};

// =============================================================================
// Configuration Structs
// =============================================================================

/// Configuration for creating appex Info.plist (AUv3)
pub struct AppexPlistConfig<'a> {
    pub package: &'a str,
    pub executable_name: &'a str,
    pub codes: &'a AuCodes,
    pub extension_class: &'a str,
    pub framework_bundle_id: &'a str,
    pub version_string: &'a str,
    pub version_int: u32,
    pub plugin_name: Option<&'a str>,
    pub vendor_name: Option<&'a str>,
    pub has_gui: bool,
}

/// Configuration for creating AUv2 component Info.plist
pub struct ComponentPlistConfig<'a> {
    pub package: &'a str,
    pub executable_name: &'a str,
    pub codes: &'a AuCodes,
    pub factory_name: &'a str,
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

/// Configuration for the uninstall command.
struct UninstallConfig {
    package: String,
    auv2: bool,
    auv3: bool,
    vst3: bool,
    verbose: bool,
}

// =============================================================================
// UUID Generation
// =============================================================================

/// Generate a new UUID for plugin identification.
///
/// Outputs a UUID in the standard format: XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX
/// Use it as the `vst3_id` in `Config.toml` to override the derived VST3 UUID.
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
        "uninstall" => {
            uninstall_command(&args);
            return;
        }
        "log" => {
            log_command(&args);
            return;
        }
        "bundle" => {
            if args.len() < 3 || args[2].starts_with('-') {
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

    // Check for unknown flags. Flags that take a value consume the next arg.
    let known_flags = ["--release", "--install", "--clean", "--verbose", "-v", "--vst3", "--auv2", "--auv3", "--arch"];
    let value_flags = ["--arch"];
    let mut rest = args.iter().skip(3);
    while let Some(arg) = rest.next() {
        if value_flags.contains(&arg.as_str()) {
            // A following flag means the value is missing.
            if rest.next().is_none_or(|value| value.starts_with('-')) {
                print_error(&format!("'{}' requires a value", arg));
                print_usage();
                std::process::exit(1);
            }
            continue;
        }
        if arg.starts_with('-') && !known_flags.contains(&arg.as_str()) {
            print_error(&format!("unknown flag '{}'", arg));
            eprintln!("Known flags: {}", known_flags.join(", "));
            std::process::exit(1);
        } else if !arg.starts_with('-') {
            print_error(&format!("unexpected argument '{}'", arg));
            print_usage();
            std::process::exit(1);
        }
    }

    // Parse --arch flag
    let arch = args.windows(2)
        .find(|w| w[0] == "--arch")
        .map(|w| {
            Arch::from_str(&w[1]).unwrap_or_else(|| {
                print_error(&format!("unrecognized arch '{}'", w[1]));
                print_usage();
                std::process::exit(1);
            })
        })
        .unwrap_or(Arch::Native);

    // Require at least one format flag
    if !build_vst3 && !build_auv2 && !build_auv3 {
        print_error("at least one format flag is required (--auv2, --auv3 or --vst3)");
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

/// Parses the arguments of the uninstall command and runs it.
fn uninstall_command(args: &[String]) {
    let package = match args.get(2) {
        Some(package) if !package.starts_with('-') => package,
        _ => {
            print_error("uninstall command requires a package name");
            print_usage();
            std::process::exit(1);
        }
    };

    let known_flags = ["--auv2", "--auv3", "--vst3", "--verbose", "-v"];
    for arg in args.iter().skip(3) {
        if arg.starts_with('-') && !known_flags.contains(&arg.as_str()) {
            print_error(&format!("unknown flag '{}'", arg));
            eprintln!("Known flags: {}", known_flags.join(", "));
            std::process::exit(1);
        } else if !arg.starts_with('-') {
            print_error(&format!("unexpected argument '{}'", arg));
            print_usage();
            std::process::exit(1);
        }
    }

    let auv2 = args.iter().any(|a| a == "--auv2");
    let auv3 = args.iter().any(|a| a == "--auv3");
    let vst3 = args.iter().any(|a| a == "--vst3");

    // Without a format flag, every format is uninstalled.
    let all = !auv2 && !auv3 && !vst3;

    let config = UninstallConfig {
        package: package.to_string(),
        auv2: auv2 || all,
        auv3: auv3 || all,
        vst3: vst3 || all,
        verbose: args.iter().any(|a| a == "--verbose" || a == "-v"),
    };

    if let Err(e) = uninstall(&config) {
        print_error(&e);
        std::process::exit(1);
    }
}

fn log_command(args: &[String]) {
    let package = match args.get(2) {
        Some(package) if !package.starts_with('-') => package,
        _ => {
            print_error("log command requires a package name");
            print_usage();
            std::process::exit(1);
        }
    };
    if let Some(arg) = args.get(3) {
        print_error(&format!("unexpected argument '{}'", arg));
        print_usage();
        std::process::exit(1);
    }
    // The name goes into the predicate of the log command.
    if !util::is_package_name(package) {
        print_error(&format!("'{}' is not a package name", package));
        std::process::exit(1);
    }
    if !cfg!(target_os = "macos") {
        print_error("the log command reads the macOS system log and runs on macOS only");
        std::process::exit(1);
    }

    // The subsystem a plugin with `log_level` in its Config.toml logs under.
    let subsystem = format!("com.beamer.{}", package);
    eprintln!("Showing the log messages of {} as they come (Ctrl+C to stop).", subsystem);
    eprintln!("The plugin logs only with log_level set in its Config.toml.");
    let status = std::process::Command::new("/usr/bin/log")
        .args(["stream", "--level", "debug", "--style", "compact", "--predicate"])
        .arg(format!("subsystem == \"{}\"", subsystem))
        .status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => {
            print_error(&format!("could not run /usr/bin/log: {}", e));
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("Usage: cargo xtask <command> [options]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  generate-uuid                 Generate a new UUID for plugin identification");
    eprintln!("  bundle <package> [options]    Build and bundle a plugin");
    eprintln!("  uninstall <package> [formats] Remove an installed plugin");
    eprintln!("  log <package>                 Show the log messages of a plugin as they come (macOS)");
    eprintln!();
    eprintln!("Formats (bundle requires at least one):");
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
    eprintln!("  --clean      Force a rebuild of the plugin before building");
    eprintln!("               Removes its build artifacts, generated AU ObjC code and previous bundles.");
    eprintln!("               Normally not needed: use it when a change isn't being picked up.");
    eprintln!("  --verbose    Show detailed build output (default: quiet). Short form: -v");
    eprintln!();
    eprintln!("Uninstall:");
    eprintln!("  Removes what --install copied to the plugin directories. After removing");
    eprintln!("  an Audio Unit it refreshes the Audio Unit registry.");
    eprintln!("  Takes the format flags and --verbose. Without a format flag it removes");
    eprintln!("  every format.");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  cargo xtask bundle gain --auv2 --release --install");
    eprintln!("  cargo xtask bundle gain --auv3 --release --install");
    eprintln!("  cargo xtask bundle gain --vst3 --release --install");
    eprintln!("  cargo xtask bundle gain --auv2 --auv3 --arch universal    # Both AU formats");
    eprintln!("  cargo xtask bundle gain --auv2 --vst3 --arch universal    # AUv2 + VST3");
    eprintln!("  cargo xtask uninstall gain                                # Every format");
    eprintln!("  cargo xtask uninstall gain --auv3                         # AUv3 only");
    eprintln!("  cargo xtask log gain                                      # Its log messages");
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

    // Find the plugin package, wherever it sits in the workspace.
    let package_info = util::find_package(&config.package, &workspace_root)?;

    // Clean build caches if requested
    if config.clean {
        build::clean_build_caches(&workspace_root, config)?;
    }

    // Build webview assets (must happen before cargo build so dist/ exists for proc macro)
    build::build_webview(&package_info.dir, config.verbose)?;

    // Determine paths
    let target_dir = workspace_root.join("target").join(profile_str);

    // Each format gets a library of its own: the AUv2 and the AUv3 library
    // differ in their Objective-C classes. A library is bundled before the
    // next build replaces it.
    let build_library = |format: build::Format| {
        if config.arch == Arch::Universal {
            build::build_universal(&config.package, config.release, &workspace_root, format, config.verbose)
        } else {
            build::build_native(&config.package, config.release, &workspace_root, format, config.arch, config.verbose)
        }
    };

    // Build and bundle AU (macOS only).
    if config.build_auv2 && cfg!(target_os = "macos") {
        let dylib_path = build_library(build::Format::Auv2)?;
        auv2::bundle_auv2(&config.package, &target_dir, &dylib_path, config.install, &workspace_root, config.arch, config.verbose)?;
    }
    if config.build_auv3 && cfg!(target_os = "macos") {
        let dylib_path = build_library(build::Format::Auv3)?;
        auv3::bundle_auv3(&config.package, &target_dir, &dylib_path, config.install, &workspace_root, config.arch, config.verbose)?;
    }

    // Build and bundle VST3
    if config.build_vst3 {
        let dylib_path = build_library(build::Format::Vst3)?;
        vst3::bundle_vst3(&config.package, &target_dir, &dylib_path, config.install, &workspace_root, config.verbose)?;
    }

    Ok(())
}

// =============================================================================
// Uninstall
// =============================================================================

fn uninstall(config: &UninstallConfig) -> Result<(), String> {
    // The bundle names are built from the package name. With a path separator
    // in it they would point outside the plugin folders.
    if !util::is_package_name(&config.package) {
        return Err(format!("invalid package name '{}'", config.package));
    }

    status!("Uninstalling {}...", config.package);

    let mut removed_au = false;
    let result = (|| {
        if config.auv2 {
            removed_au |= auv2::uninstall_auv2(&config.package)?;
        }
        if config.auv3 {
            removed_au |= auv3::uninstall_auv3(&config.package)?;
        }
        if config.vst3 {
            vst3::uninstall_vst3(&config.package)?;
        }
        Ok(())
    })();

    // Hosts keep listing a removed Audio Unit until the registry rescans.
    // This also has to happen when a later format could not be removed.
    if removed_au {
        util::refresh_au_registry(config.verbose);
        verbose!(config.verbose, "    Audio Unit cache refreshed");
    }

    result
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
