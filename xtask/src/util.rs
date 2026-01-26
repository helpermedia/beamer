//! Shared utilities for xtask.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

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
/// Takes alphanumeric characters, lowercases them, and pads with underscores if needed.
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

/// Maps AU component type code to appropriate tags for Info.plist.
///
/// DAWs use these tags for plugin categorization.
#[must_use]
pub fn get_au_tags(component_type: &str) -> &'static str {
    match component_type {
        "aufx" => "Effects",           // Audio effect
        "aumu" => "Synth",             // Music device/instrument
        "aumi" => "MIDI",              // MIDI processor
        "aumf" => "Effects",           // Music effect
        _ => "Effects",                // Default fallback
    }
}

/// Combines multiple architecture-specific binaries into a universal binary using lipo,
/// or renames a single binary to the output path.
///
/// This consolidates the common pattern used across AUv2, AUv3, and build modules:
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
/// This handles the common codesign pattern used for frameworks, appex, and app bundles.
/// Prints verbose output on success, warnings on failure.
pub fn codesign_bundle(target_path: &Path, entitlements: Option<&Path>, label: &str, verbose: bool) {
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
        }
        Ok(_) => crate::status!("  Warning: {} code signing failed", label),
        Err(e) => crate::status!("  Warning: Could not run codesign on {}: {}", label.to_lowercase(), e),
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
    let home = std::env::var("HOME").map_err(|_| "HOME not set")?;

    let mut dest_dir = PathBuf::from(&home);
    for part in install_subdir {
        dest_dir = dest_dir.join(part);
    }

    // Create directory if needed
    fs::create_dir_all(&dest_dir)
        .map_err(|e| format!("Failed to create install directory: {}", e))?;

    let dest = dest_dir.join(bundle_name);

    // Remove existing installation
    if dest.exists() {
        fs::remove_dir_all(&dest)
            .map_err(|e| format!("Failed to remove old installation: {}", e))?;
    }

    // Copy bundle
    copy_dir_all(bundle_dir, &dest)?;

    crate::verbose!(verbose, "    Installed to: {}", dest.display());

    Ok(dest)
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
