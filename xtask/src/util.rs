//! Shared utilities for xtask.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Oldest macOS version the bundles run on.
///
/// Every binary of a bundle is built for it: the Rust library through
/// `MACOSX_DEPLOYMENT_TARGET`, and what clang compiles through
/// [`MACOS_MIN_VERSION_FLAG`]. Without them clang targets the macOS version of
/// the build machine and the bundle does not load on anything older.
pub const MACOS_DEPLOYMENT_TARGET: &str = "11.0";

/// The clang flag that selects [`MACOS_DEPLOYMENT_TARGET`].
pub const MACOS_MIN_VERSION_FLAG: &str = "-mmacosx-version-min=11.0";

/// Simplified plugin config from Config.toml for xtask use.
#[derive(Deserialize)]
struct ConfigFile {
    name: String,
    category: String,
    manufacturer_code: String,
    plugin_code: String,
    vendor: Option<String>,
    has_gui: Option<bool>,
}

/// Extension trait for converting paths to strings with proper error handling.
pub trait PathExt {
    /// Convert path to string, returning an error if the path contains invalid UTF-8.
    fn to_str_safe(&self) -> Result<&str, String>;
}

impl PathExt for std::path::Path {
    fn to_str_safe(&self) -> Result<&str, String> {
        self.to_str()
            .ok_or_else(|| format!("Path contains invalid UTF-8: {}", self.display()))
    }
}

impl PathExt for std::path::PathBuf {
    fn to_str_safe(&self) -> Result<&str, String> {
        self.to_str()
            .ok_or_else(|| format!("Path contains invalid UTF-8: {}", self.display()))
    }
}

/// Print an error message, with red color if stderr is a terminal.
pub fn print_error(msg: &str) {
    if std::io::stderr().is_terminal() {
        eprintln!("\x1b[1;31mError:\x1b[0m {}", msg);
    } else {
        eprintln!("Error: {}", msg);
    }
}

/// Print status message (always shown)
#[macro_export]
macro_rules! status {
    ($($arg:tt)*) => {
        println!($($arg)*)
    };
}

/// Print verbose message (only in verbose mode)
#[macro_export]
macro_rules! verbose {
    ($verbose:expr, $($arg:tt)*) => {
        if $verbose {
            println!($($arg)*)
        }
    };
}

/// Shorten home directory in path for display
#[must_use]
pub fn shorten_path(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME") {
        let home_path = PathBuf::from(home);
        if let Ok(stripped) = path.strip_prefix(&home_path) {
            return format!("~/{}", stripped.display());
        }
    }
    path.display().to_string()
}

/// Architecture configuration for builds
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Arch {
    /// Build for current machine's architecture only
    Native,
    /// Build universal binary (x86_64 + arm64)
    Universal,
    /// Build for arm64 only
    Arm64,
    /// Build for x86_64 only
    X86_64,
}

impl Arch {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "native" => Some(Arch::Native),
            "universal" => Some(Arch::Universal),
            "arm64" | "aarch64" => Some(Arch::Arm64),
            "x86_64" | "x86-64" | "intel" => Some(Arch::X86_64),
            _ => None,
        }
    }
}

/// Convert plugin name to PascalCase for class names.
/// "midi-transform" → "MidiTransform"
#[must_use]
pub fn to_pascal_case(name: &str) -> String {
    name.split(['-', '_'])
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}

/// Whether `name` can be a Cargo package name: letters, digits, `-` and `_`.
///
/// The bundle names below are built from the package name, so a name with
/// anything else in it (a path separator in particular) must not reach them.
#[must_use]
pub fn is_package_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Convert package name to the name of the binary inside a VST3 bundle.
/// "gain" -> "BeamerGain"
/// "midi-transform" -> "BeamerMidiTransform"
#[must_use]
pub fn to_vst3_executable_name(package: &str) -> String {
    format!("Beamer{}", to_pascal_case(package))
}

/// Convert package name to VST3 bundle name.
/// "gain" -> "BeamerGain.vst3"
/// "midi-transform" -> "BeamerMidiTransform.vst3"
#[must_use]
pub fn to_vst3_bundle_name(package: &str) -> String {
    format!("Beamer{}.vst3", to_pascal_case(package))
}

/// Convert package name to AUv3 app bundle name.
/// "gain" -> "BeamerGain.app"
/// "midi-transform" -> "BeamerMidiTransform.app"
#[must_use]
pub fn to_au_bundle_name(package: &str) -> String {
    format!("Beamer{}.app", to_pascal_case(package))
}

/// Convert package name to AUv2 component bundle name.
/// "gain" -> "BeamerGain.component"
/// "midi-transform" -> "BeamerMidiTransform.component"
#[must_use]
pub fn to_auv2_component_name(package: &str) -> String {
    format!("Beamer{}.component", to_pascal_case(package))
}

/// Generates a 4-character AU subtype code from a package name.
///
/// Takes alphanumeric characters, lowercases them and pads with underscores if needed.
/// Examples: "gain" -> "gain", "midi-transform" -> "midi", "fx" -> "fx__"
#[must_use]
pub fn generate_au_subtype(package: &str) -> String {
    let gen: String = package
        .chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .take(4)
        .collect();
    if gen.len() < 4 {
        format!("{:_<4}", gen)
    } else {
        gen
    }
}

/// Maps a `Config.toml` category to its AU component type code.
///
/// Has to match `Category::to_au_component_type` in beamer-core, which is the
/// type the plugin reports at runtime. Unknown categories map to `aufx`.
#[must_use]
pub fn au_component_type(category: &str) -> &'static str {
    match category {
        "instrument" => "aumu",
        "midi_effect" => "aumi",
        "generator" => "augn",
        _ => "aufx",
    }
}

/// Maps AU component type code to appropriate tags for Info.plist.
///
/// DAWs use these tags for plugin categorization.
#[must_use]
pub fn get_au_tags(component_type: &str) -> &'static str {
    match component_type {
        // Audio effect.
        "aufx" => "Effects",
        // Music device (instrument).
        "aumu" => "Synthesizer",
        // MIDI processor.
        "aumi" => "MIDI",
        // Generator.
        "augn" => "Generator",
        // Default fallback.
        _ => "Effects",
    }
}

/// Combines multiple architecture-specific binaries into a universal binary using lipo,
/// or renames a single binary to the output path.
///
/// This consolidates the common pattern used across AUv2, AUv3 and build modules:
/// - If only one binary: rename it to the output path
/// - If multiple binaries: combine with `lipo -create`
///
/// Set `cleanup` to true to delete intermediate binaries after combining (useful for
/// temporary build artifacts). Set to false when the source binaries are in standard
/// cargo output directories and should be preserved.
pub fn combine_or_rename_binaries(
    built_paths: &[PathBuf],
    output_path: &Path,
    cleanup: bool,
) -> Result<(), String> {
    use std::process::Command;

    if built_paths.len() == 1 {
        // Single architecture - just rename
        fs::rename(&built_paths[0], output_path)
            .map_err(|e| format!("Failed to rename binary: {}", e))?;
    } else {
        // Multiple architectures - combine with lipo
        let mut lipo_cmd = Command::new("lipo");
        lipo_cmd.arg("-create");
        for path in built_paths {
            lipo_cmd.arg(path);
        }
        lipo_cmd.arg("-output").arg(output_path);

        let lipo_status = lipo_cmd
            .status()
            .map_err(|e| format!("Failed to run lipo: {}", e))?;

        if !lipo_status.success() {
            return Err("Failed to create universal binary with lipo".to_string());
        }

        // Clean up intermediate binaries if requested
        if cleanup {
            for path in built_paths {
                let _ = fs::remove_file(path);
            }
        }
    }

    Ok(())
}

/// Ad-hoc code sign a bundle with optional entitlements.
///
/// This handles the common codesign pattern used for frameworks, appex and app bundles.
/// Prints verbose output on success and returns an error on failure. A bundle
/// that could not be signed does not load on Apple Silicon.
pub fn codesign_bundle(
    target_path: &Path,
    entitlements: Option<&Path>,
    label: &str,
    verbose: bool,
) -> Result<(), String> {
    use std::process::Command;

    let mut cmd = Command::new("codesign");
    cmd.args(["--force", "--sign", "-"]);

    if let Some(ent_path) = entitlements {
        cmd.args(["--entitlements", &ent_path.to_string_lossy()]);
    }

    cmd.arg(target_path);

    let result = cmd.output();

    match result {
        Ok(output) if output.status.success() => {
            if verbose {
                let stderr = String::from_utf8_lossy(&output.stderr);
                for line in stderr.lines() {
                    crate::verbose!(verbose, "    {}", line);
                }
            }
            crate::verbose!(verbose, "    {} code signing successful", label);
            Ok(())
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = match stderr.trim() {
                "" => String::new(),
                message => format!(": {}", message),
            };
            Err(format!("{} code signing failed{}", label, reason))
        }
        Err(e) => Err(format!("Could not run codesign on {}: {}", label.to_lowercase(), e)),
    }
}

/// Install a plugin bundle to a directory under the user's home directory.
///
/// Handles the common install pattern:
/// 1. Get HOME environment variable
/// 2. Build destination directory from path components
/// 3. Create directory if needed
/// 4. Remove existing installation if present
/// 5. Copy bundle to destination
///
/// Returns the destination path on success for post-install hooks.
pub fn install_bundle(
    bundle_dir: &Path,
    bundle_name: &str,
    install_subdir: &[&str],
    verbose: bool,
) -> Result<PathBuf, String> {
    let dest_dir = home_subdir(install_subdir)?;

    // Create directory if needed
    fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create install directory: {}", e))?;

    let dest = dest_dir.join(bundle_name);

    // Remove existing installation. Unlike `exists`, this also finds a symlink
    // whose target is gone.
    if dest.symlink_metadata().is_ok() {
        fs::remove_dir_all(&dest)
            .map_err(|e| format!("Failed to remove old installation: {}", e))?;
    }

    // Copy bundle
    copy_dir_all(bundle_dir, &dest)?;

    crate::verbose!(verbose, "    Installed to: {}", dest.display());

    Ok(dest)
}

/// Remove an installed plugin bundle from a directory under the user's home
/// directory.
///
/// Prints what happened and returns whether a bundle was removed. A bundle
/// that is not installed is not an error.
pub fn uninstall_bundle(bundle_name: &str, install_subdir: &[&str]) -> Result<bool, String> {
    let dest_dir = home_subdir(install_subdir)?;
    let dest = dest_dir.join(bundle_name);

    // Unlike `exists`, this also finds a symlink whose target is gone.
    if dest.symlink_metadata().is_err() {
        crate::status!("  {} is not installed", bundle_name);
        return Ok(false);
    }

    fs::remove_dir_all(&dest)
        .map_err(|e| format!("Failed to remove {}: {}", dest.display(), e))?;
    crate::status!("  {} removed from {}", bundle_name, shorten_path(&dest_dir));

    Ok(true)
}

/// Build a path under the user's home directory from path components.
fn home_subdir(parts: &[&str]) -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;

    let mut dir = PathBuf::from(&home);
    for part in parts {
        dir = dir.join(part);
    }

    Ok(dir)
}

/// Restart the system's Audio Unit registry.
///
/// The registry rescans the installed Audio Units when it starts again, which
/// is how hosts learn about an installed or removed one.
pub fn refresh_au_registry(verbose: bool) {
    use std::process::Command;

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
}

// =============================================================================
// Plugin Feature Detection
// =============================================================================

/// Detect whether a plugin has a custom GUI (WebView UI).
///
/// Checks `Config.toml` for `has_gui = true` and also checks for
/// a `webview/index.html` file in the example directory.
pub fn detect_has_gui(package: &str, workspace_root: &Path) -> bool {
    let example_dir = workspace_root.join("examples").join(package);

    // Check Config.toml for explicit has_gui flag
    let config_path = example_dir.join("Config.toml");
    if let Ok(toml_str) = fs::read_to_string(&config_path) {
        if let Ok(config) = toml::from_str::<ConfigFile>(&toml_str) {
            if config.has_gui == Some(true) {
                return true;
            }
        }
    }

    // Also check for webview/index.html (mirrors macro behavior)
    example_dir.join("webview/index.html").exists()
}

// =============================================================================
// AU Plugin Metadata Detection (from source code)
// =============================================================================

/// Detect AU component info by reading Config.toml or parsing plugin source code.
///
/// Returns (component_type, manufacturer, subtype, plugin_name, vendor_name, has_gui).
/// Used by both AUv2 and AUv3 bundlers.
///
/// Tries to read `examples/{package}/Config.toml` first. Falls back to
/// parsing the source code in `examples/{package}/src/lib.rs` if the TOML
/// file is missing or cannot be parsed.
///
/// The `has_gui` field is computed via `detect_has_gui`, avoiding
/// a second parse of Config.toml by callers that need both pieces of info.
pub fn detect_au_component_info(package: &str, workspace_root: &Path) -> (String, Option<String>, Option<String>, Option<String>, Option<String>, bool) {
    let has_gui = detect_has_gui(package, workspace_root);

    // Try Config.toml first
    let config_path = workspace_root.join("examples").join(package).join("Config.toml");
    if let Ok(toml_str) = fs::read_to_string(&config_path) {
        if let Ok(config) = toml::from_str::<ConfigFile>(&toml_str) {
            let component_type = au_component_type(&config.category).to_string();

            return (
                component_type,
                Some(config.manufacturer_code),
                Some(config.plugin_code),
                Some(config.name),
                config.vendor,
                has_gui,
            );
        }
    }

    // Fall back to source code parsing
    let lib_path = workspace_root.join("examples").join(package).join("src/lib.rs");

    if let Ok(content) = fs::read_to_string(&lib_path) {
        // Detect component type from Category enum in Config::new().
        let category = if content.contains("Category::Instrument") {
            "instrument"
        } else if content.contains("Category::Generator") {
            "generator"
        } else if content.contains("Category::MidiEffect") {
            "midi_effect"
        } else {
            // Default to effect (aufx).
            "effect"
        };
        let component_type = au_component_type(category).to_string();

        // Detect manufacturer and subtype from Config::new()
        let (manufacturer, subtype) = detect_au_fourcc_codes(&content);

        // Detect plugin name and vendor from Config::new()
        let (plugin_name, vendor_name) = detect_plugin_metadata(&content);

        (component_type, manufacturer, subtype, plugin_name, vendor_name, has_gui)
    } else {
        // Default to effect if we can't read the file
        ("aufx".to_string(), None, None, None, None, has_gui)
    }
}

/// Extract AU fourcc codes (manufacturer and subtype) from plugin source code.
///
/// Parses `Config::new("name", Category::Effect, "mfgr", "subt")` to find
/// the 4-character manufacturer and subtype string literals.
fn detect_au_fourcc_codes(content: &str) -> (Option<String>, Option<String>) {
    let Some(start) = content.find("Config::new(") else {
        return (None, None);
    };

    let after_new = &content[start..];
    let Some(end) = after_new.find(')').or_else(|| after_new.find(".with_")) else {
        return (None, None);
    };

    let config_args = &after_new[..end];
    let mut string_literals: Vec<String> = Vec::new();
    let mut remaining = config_args;

    while let Some(quote_start) = remaining.find('"') {
        let after_quote = &remaining[quote_start + 1..];
        if let Some(quote_end) = after_quote.find('"') {
            let literal = &after_quote[..quote_end];
            if literal.len() == 4 && literal.is_ascii() {
                string_literals.push(literal.to_string());
            }
            remaining = &after_quote[quote_end + 1..];
        } else {
            break;
        }
    }

    let manufacturer = string_literals.first().cloned();
    let subtype = string_literals.get(1).cloned();
    (manufacturer, subtype)
}

/// Extract plugin name and vendor from Config in source code.
///
/// Parses `Config::new("Plugin Name")` and `.with_vendor("Vendor Name")`.
fn detect_plugin_metadata(content: &str) -> (Option<String>, Option<String>) {
    let mut plugin_name = None;
    let mut vendor_name = None;

    if let Some(start) = content.find("Config::new(\"") {
        let after_prefix = &content[start + 13..];
        if let Some(end) = after_prefix.find('"') {
            plugin_name = Some(after_prefix[..end].to_string());
        }
    }

    if let Some(start) = content.find(".with_vendor(\"") {
        let after_prefix = &content[start + 14..];
        if let Some(end) = after_prefix.find('"') {
            vendor_name = Some(after_prefix[..end].to_string());
        }
    }

    (plugin_name, vendor_name)
}

/// Recursively copy a directory, preserving symlinks.
pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use beamer_core::config::Category;

    use super::*;

    #[test]
    fn test_min_version_flag_matches_the_deployment_target() {
        assert_eq!(
            MACOS_MIN_VERSION_FLAG,
            format!("-mmacosx-version-min={MACOS_DEPLOYMENT_TARGET}")
        );
    }

    /// Name of a category in `Config.toml`.
    ///
    /// The match is exhaustive, so a new `Category` variant fails to compile
    /// here until it is added to this test.
    fn config_name(category: Category) -> &'static str {
        match category {
            Category::Effect => "effect",
            Category::Instrument => "instrument",
            Category::MidiEffect => "midi_effect",
            Category::Generator => "generator",
        }
    }

    #[test]
    fn test_is_package_name() {
        for name in ["gain", "midi-transform", "webview_demo", "fx2"] {
            assert!(is_package_name(name), "\"{}\"", name);
        }

        // None of these may end up in a path that gets removed.
        for name in ["", ".", "..", "/", "gain/", "a/b", "../x", "gain.vst3/../../Other", "a\\b", "a b"] {
            assert!(!is_package_name(name), "\"{}\"", name);
        }
    }

    #[test]
    fn test_vst3_names() {
        assert_eq!(to_vst3_executable_name("midi-transform"), "BeamerMidiTransform");

        // The bundle is named after the binary inside it.
        assert_eq!(
            to_vst3_bundle_name("midi-transform"),
            format!("{}.vst3", to_vst3_executable_name("midi-transform"))
        );
    }

    #[test]
    fn test_au_component_type_matches_core() {
        // The type written into Info.plist has to equal the type the plugin
        // reports at runtime.
        let categories = [
            Category::Effect,
            Category::Instrument,
            Category::MidiEffect,
            Category::Generator,
        ];
        for category in categories {
            let name = config_name(category);
            assert_eq!(
                au_component_type(name).as_bytes(),
                category.to_au_component_type().to_be_bytes(),
                "category \"{}\"",
                name
            );
        }
    }
}
