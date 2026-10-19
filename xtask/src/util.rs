//! Shared utilities for xtask.

use std::collections::HashMap;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

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
    subcategories: Option<Vec<String>>,
    midi_input: Option<bool>,
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

/// The manufacturer code of a plugin that does not name one.
pub const DEFAULT_AU_MANUFACTURER: &str = "Bmer";

/// The codes that identify an Audio Unit on a system.
pub struct AuCodes {
    pub component_type: String,
    pub subtype: String,
    pub manufacturer: String,
}

impl AuCodes {
    /// The codes of a package, as its bundles register them.
    #[must_use]
    pub fn detect(package: &str, package_dir: &Path) -> Self {
        let (component_type, manufacturer, subtype, ..) = detect_au_component_info(package_dir);
        Self {
            component_type,
            subtype: subtype.unwrap_or_else(|| generate_au_subtype(package)),
            manufacturer: manufacturer.unwrap_or_else(|| DEFAULT_AU_MANUFACTURER.to_string()),
        }
    }
}

/// The names of the Objective-C classes and the factory function generated
/// for one Audio Unit.
///
/// A process has one namespace for Objective-C classes. When two loaded
/// binaries define a class of the same name, the runtime uses one of them
/// for both, and a host looks classes up by name. The names therefore carry
/// the codes of the Audio Unit, which no other Audio Unit on the system has,
/// besides the package name for readability. The codes are written in hex,
/// since they may hold any ASCII character. Each code has four characters,
/// so the hex of the three cannot be read in two ways.
pub struct AuNames {
    /// The `AUAudioUnit` subclass of the AUv3.
    pub wrapper_class: String,
    /// The principal class of the AUv3 extension.
    pub extension_class: String,
    /// The factory function of the AUv2 component.
    pub auv2_factory: String,
    /// The class that gives an AUv2 host the editor view.
    pub cocoa_view_factory_class: String,
    /// The view of the AUv2 editor.
    pub cocoa_gui_view_class: String,
}

impl AuNames {
    /// The names for a package with these codes.
    #[must_use]
    pub fn new(package: &str, codes: &AuCodes) -> Self {
        let codes_hex: String = [&codes.component_type, &codes.subtype, &codes.manufacturer]
            .iter()
            .flat_map(|code| code.bytes())
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let name = |kind: &str| format!("Beamer{}{}_{}", to_pascal_case(package), kind, codes_hex);
        Self {
            wrapper_class: name("AuWrapper"),
            extension_class: name("AuExtension"),
            auv2_factory: name("Factory"),
            cocoa_view_factory_class: name("CocoaViewFactory"),
            cocoa_gui_view_class: name("GuiView"),
        }
    }

    /// The names for a package, with the codes its bundles register.
    #[must_use]
    pub fn detect(package: &str, package_dir: &Path) -> Self {
        Self::new(package, &AuCodes::detect(package, package_dir))
    }
}

/// Maps a `Config.toml` category and `midi_input`, `None` when it is not
/// set, to the AU component type code. An effect with MIDI input is a music
/// effect.
///
/// Has to match `Config::au_component_type` in beamer-core, which is the
/// type the plugin reports at runtime. Unknown categories map to `aufx`.
#[must_use]
pub fn au_component_type(category: &str, midi_input: Option<bool>) -> &'static str {
    match category {
        "effect" if midi_input == Some(true) => "aumf",
        "instrument" => "aumu",
        "midi_effect" => "aumi",
        "generator" => "augn",
        _ => "aufx",
    }
}

/// The tags of an Audio Unit in its Info.plist: one for the component type,
/// then one for each subcategory that has an AU tag.
///
/// DAWs use these tags for plugin categorization.
#[must_use]
pub fn au_tags(component_type: &str, subcategories: &[String]) -> Vec<&'static str> {
    let type_tag = match component_type {
        // Music device (instrument).
        "aumu" => "Synthesizer",
        // MIDI processor.
        "aumi" => "MIDI",
        // Generator.
        "augn" => "Generator",
        // Audio effect, music effect and the fallback.
        _ => "Effects",
    };

    let mut tags = vec![type_tag];
    for tag in subcategories.iter().filter_map(|name| au_subcategory_tag(name)) {
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

/// Maps a `Config.toml` subcategory to its AU tag, if it has one.
///
/// Has to match `Subcategory::to_au_tag` in beamer-core.
#[must_use]
fn au_subcategory_tag(subcategory: &str) -> Option<&'static str> {
    match subcategory {
        "analyzer" => Some("Analyzer"),
        "delay" => Some("Delay"),
        "distortion" => Some("Distortion"),
        "dynamics" => Some("Dynamics"),
        "eq" => Some("EQ"),
        "filter" => Some("Filter"),
        "mastering" => Some("Mastering"),
        "modulation" => Some("Modulation"),
        "pitch_shift" => Some("Pitch Shift"),
        "restoration" => Some("Restoration"),
        "reverb" => Some("Reverb"),
        "drum" => Some("Drums"),
        "sampler" => Some("Sampler"),
        "synth" => Some("Synth"),
        "piano" => Some("Piano"),
        "generator" => Some("Generator"),
        _ => None,
    }
}

/// The subcategories in the `Config.toml` of a package, none without one.
#[must_use]
pub fn detect_subcategories(package_dir: &Path) -> Vec<String> {
    fs::read_to_string(package_dir.join("Config.toml"))
        .ok()
        .and_then(|toml_str| toml::from_str::<ConfigFile>(&toml_str).ok())
        .and_then(|config| config.subcategories)
        .unwrap_or_default()
}

/// Values as `<string>` elements of a plist array, one per line with the
/// given indentation before all but the first.
#[must_use]
pub fn plist_strings(values: &[&str], indent: &str) -> String {
    values
        .iter()
        .map(|value| format!("<string>{value}</string>"))
        .collect::<Vec<_>>()
        .join(&format!("\n{indent}"))
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
// Workspace Packages
// =============================================================================

/// A package of the workspace, as `cargo metadata` reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspacePackage {
    /// The folder that holds the `Cargo.toml` of the package. The plugin's
    /// `Config.toml` and its optional `webview/` folder sit next to it.
    pub dir: PathBuf,
    /// The version of the package.
    pub version: String,
}

/// The part of the output of `cargo metadata` that xtask reads.
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
}

#[derive(Deserialize)]
struct MetadataPackage {
    name: String,
    version: String,
    manifest_path: PathBuf,
}

/// Read the packages of a workspace from the output of `cargo metadata`.
fn parse_metadata(json: &str) -> Result<HashMap<String, WorkspacePackage>, String> {
    let metadata: Metadata = serde_json::from_str(json)
        .map_err(|e| format!("Failed to read the output of cargo metadata: {}", e))?;

    metadata
        .packages
        .into_iter()
        .map(|package| {
            let dir = package
                .manifest_path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| format!("Invalid manifest path of package `{}`", package.name))?;
            let info = WorkspacePackage {
                dir,
                version: package.version,
            };
            Ok((package.name, info))
        })
        .collect()
}

/// Ask cargo for the packages of the workspace.
fn workspace_packages(workspace_root: &Path) -> Result<HashMap<String, WorkspacePackage>, String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| format!("Failed to run cargo metadata: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    parse_metadata(&String::from_utf8_lossy(&output.stdout))
}

/// Find a package of the workspace by its name.
///
/// The package can sit anywhere in the workspace. Cargo is asked once per
/// run of xtask, which works on one workspace.
pub fn find_package(package: &str, workspace_root: &Path) -> Result<WorkspacePackage, String> {
    static PACKAGES: OnceLock<Result<HashMap<String, WorkspacePackage>, String>> = OnceLock::new();

    let packages = PACKAGES
        .get_or_init(|| workspace_packages(workspace_root))
        .as_ref()
        .map_err(Clone::clone)?;

    packages.get(package).cloned().ok_or_else(|| {
        format!(
            "No package named `{}` in this workspace. To make a plugin package a member, add its folder to `members` in the Cargo.toml of the workspace.",
            package
        )
    })
}

// =============================================================================
// Plugin Feature Detection
// =============================================================================

/// Detect whether a plugin has a custom GUI (WebView UI).
///
/// Checks `Config.toml` for `has_gui = true` and also checks for
/// a `webview/index.html` file in the folder of the package.
pub fn detect_has_gui(package_dir: &Path) -> bool {
    // Check Config.toml for explicit has_gui flag
    let config_path = package_dir.join("Config.toml");
    if let Ok(toml_str) = fs::read_to_string(&config_path) {
        if let Ok(config) = toml::from_str::<ConfigFile>(&toml_str) {
            if config.has_gui == Some(true) {
                return true;
            }
        }
    }

    // Also check for webview/index.html (mirrors macro behavior)
    package_dir.join("webview/index.html").exists()
}

// =============================================================================
// AU Plugin Metadata Detection (from source code)
// =============================================================================

/// Detect AU component info by reading Config.toml or parsing plugin source code.
///
/// Returns (component_type, manufacturer, subtype, plugin_name, vendor_name, has_gui).
/// Used by both AUv2 and AUv3 bundlers.
///
/// Tries to read `Config.toml` in the folder of the package first. Falls
/// back to parsing the source code in its `src/lib.rs` if the TOML file is
/// missing or cannot be parsed.
///
/// The `has_gui` field is computed via `detect_has_gui`, avoiding
/// a second parse of Config.toml by callers that need both pieces of info.
pub fn detect_au_component_info(package_dir: &Path) -> (String, Option<String>, Option<String>, Option<String>, Option<String>, bool) {
    let has_gui = detect_has_gui(package_dir);

    // Try Config.toml first
    let config_path = package_dir.join("Config.toml");
    if let Ok(toml_str) = fs::read_to_string(&config_path) {
        if let Ok(config) = toml::from_str::<ConfigFile>(&toml_str) {
            let component_type = au_component_type(&config.category, config.midi_input).to_string();

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
    let lib_path = package_dir.join("src/lib.rs");

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
        // Detect MIDI input from the builder call on the config.
        let midi_input = content.contains(".with_midi_input(true)").then_some(true);
        let component_type = au_component_type(category, midi_input).to_string();

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
    use beamer_core::config::{Category, Config};

    use super::*;

    #[test]
    fn test_min_version_flag_matches_the_deployment_target() {
        assert_eq!(
            MACOS_MIN_VERSION_FLAG,
            format!("-mmacosx-version-min={MACOS_DEPLOYMENT_TARGET}")
        );
    }

    #[test]
    fn test_packages_are_read_from_cargo_metadata() {
        let json = r#"{
            "packages": [
                {
                    "name": "gain",
                    "version": "0.2.4",
                    "manifest_path": "/work/examples/gain/Cargo.toml",
                    "dependencies": []
                },
                {
                    "name": "my-synth",
                    "version": "1.3.0",
                    "manifest_path": "/work/plugins/synths/my-synth/Cargo.toml"
                }
            ],
            "workspace_root": "/work"
        }"#;

        let packages = parse_metadata(json).expect("valid metadata");
        assert_eq!(packages.len(), 2);
        assert_eq!(
            packages["my-synth"],
            WorkspacePackage {
                dir: PathBuf::from("/work/plugins/synths/my-synth"),
                version: "1.3.0".to_string(),
            }
        );
        assert_eq!(packages["gain"].dir, PathBuf::from("/work/examples/gain"));

        assert!(parse_metadata("not json").is_err());
    }

    #[test]
    fn test_package_of_this_workspace_is_found() {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits in the workspace root");

        let gain = find_package("gain", workspace_root).expect("gain is a member");
        assert!(gain.dir.join("Config.toml").exists());
        assert_eq!(gain.dir, workspace_root.join("examples").join("gain"));
        assert!(crate::build::get_version_info(&gain.version).is_ok());

        let error = find_package("no-such-plugin", workspace_root).expect_err("not a member");
        assert!(error.contains("no-such-plugin"), "{error}");
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
        // reports at runtime, with MIDI input set or not.
        let categories = [
            Category::Effect,
            Category::Instrument,
            Category::MidiEffect,
            Category::Generator,
        ];
        for category in categories {
            let name = config_name(category);
            let config = Config::new("Test", category, "Test", "test");
            assert_eq!(
                au_component_type(name, None).as_bytes(),
                config.au_component_type().to_be_bytes(),
                "category \"{}\"",
                name
            );
            // A MIDI effect always has MIDI input, a generator never.
            for midi_input in [false, true] {
                let valid = match category {
                    Category::MidiEffect => midi_input,
                    Category::Generator => !midi_input,
                    Category::Effect | Category::Instrument => true,
                };
                if valid {
                    assert_eq!(
                        au_component_type(name, Some(midi_input)).as_bytes(),
                        config.clone().with_midi_input(midi_input).au_component_type().to_be_bytes(),
                        "category \"{}\", midi_input {}",
                        name,
                        midi_input
                    );
                }
            }
        }
        assert_eq!(au_component_type("effect", Some(true)), "aumf");
    }

    #[test]
    fn test_au_subcategory_tags_match_core() {
        use beamer_core::config::Subcategory;

        // Every subcategory name of Config.toml, with its type in
        // beamer-core.
        let subcategories = [
            ("analyzer", Subcategory::Analyzer),
            ("bass", Subcategory::Bass),
            ("channel_strip", Subcategory::ChannelStrip),
            ("delay", Subcategory::Delay),
            ("distortion", Subcategory::Distortion),
            ("drums", Subcategory::Drums),
            ("dynamics", Subcategory::Dynamics),
            ("eq", Subcategory::Eq),
            ("filter", Subcategory::Filter),
            ("generator", Subcategory::Generator),
            ("guitar", Subcategory::Guitar),
            ("mastering", Subcategory::Mastering),
            ("microphone", Subcategory::Microphone),
            ("modulation", Subcategory::Modulation),
            ("network", Subcategory::Network),
            ("pitch_shift", Subcategory::PitchShift),
            ("restoration", Subcategory::Restoration),
            ("reverb", Subcategory::Reverb),
            ("spatial", Subcategory::Spatial),
            ("surround", Subcategory::Surround),
            ("tools", Subcategory::Tools),
            ("vocals", Subcategory::Vocals),
            ("drum", Subcategory::Drum),
            ("external", Subcategory::External),
            ("piano", Subcategory::Piano),
            ("sampler", Subcategory::Sampler),
            ("synth", Subcategory::Synth),
            ("mono", Subcategory::Mono),
            ("stereo", Subcategory::Stereo),
            ("ambisonics", Subcategory::Ambisonics),
            ("up_down_mix", Subcategory::UpDownMix),
            ("only_realtime", Subcategory::OnlyRealTime),
            ("only_offline", Subcategory::OnlyOfflineProcess),
            ("no_offline", Subcategory::NoOfflineProcess),
        ];
        for (name, subcategory) in subcategories {
            assert_eq!(au_subcategory_tag(name), subcategory.to_au_tag(), "subcategory \"{}\"", name);
        }
    }

    #[test]
    fn test_au_tags_start_with_the_type() {
        let names = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<Vec<_>>();

        assert_eq!(au_tags("aufx", &[]), ["Effects"]);
        assert_eq!(au_tags("aufx", &names(&["eq", "mono"])), ["Effects", "EQ"]);
        assert_eq!(au_tags("aumu", &names(&["drum", "sampler"])), ["Synthesizer", "Drums", "Sampler"]);

        // A tag appears once.
        assert_eq!(au_tags("augn", &names(&["generator"])), ["Generator"]);
    }

    #[test]
    fn test_plist_strings() {
        assert_eq!(plist_strings(&["Effects"], "  "), "<string>Effects</string>");
        assert_eq!(plist_strings(&["Effects", "EQ"], "  "), "<string>Effects</string>\n  <string>EQ</string>");
    }

    fn codes(component_type: &str, subtype: &str, manufacturer: &str) -> AuCodes {
        AuCodes {
            component_type: component_type.to_string(),
            subtype: subtype.to_string(),
            manufacturer: manufacturer.to_string(),
        }
    }

    #[test]
    fn test_au_names_carry_the_codes_in_hex() {
        let names = AuNames::new("midi-transform", &codes("aumi", "midi", "Bmer"));
        assert_eq!(names.wrapper_class, "BeamerMidiTransformAuWrapper_61756d696d696469426d6572");
        assert_eq!(names.auv2_factory, "BeamerMidiTransformFactory_61756d696d696469426d6572");

        // A code may hold characters that a class name cannot.
        let names = AuNames::new("gain", &codes("aufx", "g n!", "Bmer"));
        assert!(names.extension_class.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }

    #[test]
    fn test_au_names_differ_per_audio_unit() {
        // Two plugins with one package name have different codes.
        let first = AuNames::new("gain", &codes("aufx", "gain", "Acme"));
        let second = AuNames::new("gain", &codes("aufx", "gain", "Bmer"));
        let third = AuNames::new("gain", &codes("aumu", "gain", "Acme"));
        for (a, b) in [(&first, &second), (&first, &third), (&second, &third)] {
            assert_ne!(a.wrapper_class, b.wrapper_class);
            assert_ne!(a.extension_class, b.extension_class);
            assert_ne!(a.auv2_factory, b.auv2_factory);
            assert_ne!(a.cocoa_view_factory_class, b.cocoa_view_factory_class);
            assert_ne!(a.cocoa_gui_view_class, b.cocoa_gui_view_class);
        }
    }
}
