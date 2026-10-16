//! Build orchestration and AU ObjC code generation.

use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{combine_or_rename_binaries, to_au_bundle_name, to_auv2_component_name, to_vst3_bundle_name, to_vst3_executable_name, Arch, AuNames, PathExt, MACOS_DEPLOYMENT_TARGET, MACOS_MIN_VERSION_FLAG};
use crate::BundleConfig;

/// The format a plugin library is built for.
///
/// The two Audio Unit formats share the plugin's AU code but not their
/// Objective-C classes. A library for one format holds only the classes of
/// that format, so loading the AUv2 and the AUv3 of a plugin in one process
/// gives no class twice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Auv2,
    Auv3,
    Vst3,
}

impl Format {
    /// The cargo feature of the plugin crate that selects the format.
    fn feature(self) -> &'static str {
        match self {
            Format::Auv2 | Format::Auv3 => "au",
            Format::Vst3 => "vst3",
        }
    }

    /// The name of the format in messages.
    fn label(self) -> &'static str {
        match self {
            Format::Auv2 => "AUv2",
            Format::Auv3 => "AUv3",
            Format::Vst3 => "VST3",
        }
    }
}

/// The version of a bundle, from the version of its package.
///
/// Returns the version as `major.minor.patch`. A bundle version is made of
/// numbers only, so a pre-release or build suffix of the package version
/// (`1.0.0-rc.1`, `1.0.0+5`) is left out.
pub fn bundle_version(version: &str) -> Result<String, String> {
    let numbers = version.split(['-', '+']).next().unwrap_or(version);

    let parts: Vec<&str> = numbers.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|part| part.parse::<u32>().is_err()) {
        return Err(format!("Invalid version format: {}", version));
    }

    Ok(numbers.to_string())
}

/// The version of an Audio Unit bundle: as `major.minor.patch` and as
/// Apple's version integer.
///
/// The integer is `(major << 16) | (minor << 8) | patch`, so the minor and
/// the patch version must be at most 255 and the major version at most
/// 65535. A larger number would run into its neighbour and give the
/// component another version than the bundle.
pub fn get_version_info(version: &str) -> Result<(String, u32), String> {
    let numbers = bundle_version(version)?;

    let parts: Vec<u32> = numbers
        .split('.')
        .map(|part| part.parse().map_err(|_| format!("Invalid version format: {}", version)))
        .collect::<Result<_, _>>()?;
    let (major, minor, patch) = (parts[0], parts[1], parts[2]);

    if major > 0xffff || minor > 0xff || patch > 0xff {
        return Err(format!(
            "Version {} does not fit an Audio Unit version: the major version can be at most 65535, \
             the minor and the patch version at most 255",
            version
        ));
    }

    // Convert to Apple's version format: (major << 16) | (minor << 8) | patch
    let version_int = (major << 16) | (minor << 8) | patch;

    Ok((numbers, version_int))
}

/// Extra rustc arguments for the plugin crate of an AUv3 build.
///
/// They link the AUv3 static libraries into the plugin. The `objc_lib_dir`
/// parameter points to where plugin-specific ObjC static libraries were
/// compiled (contains libplugin_au_objc.a and libplugin_au_extension.a).
fn auv3_rustc_args(
    plugin_name: &str,
    names: &AuNames,
    objc_lib_dir: &Path,
) -> Result<Vec<String>, String> {
    let objc_lib_dir_str = objc_lib_dir.to_str()
        .ok_or_else(|| "Invalid ObjC library directory path".to_string())?;
    let wrapper_class = &names.wrapper_class;
    let extension_class = &names.extension_class;

    // Library names based on plugin
    let lib_name = plugin_name.replace('-', "_");

    // Every linker argument the plugin needs, as flag and value pairs.
    // Note: beamer_au_appex_force_link is internal (not exported) to avoid symbol collisions.
    let args = [
        ("-C", format!("link-arg=-Wl,-force_load,{}/lib{}_au_objc.a", objc_lib_dir_str, lib_name)),
        ("-C", format!("link-arg=-Wl,-force_load,{}/lib{}_au_extension.a", objc_lib_dir_str, lib_name)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_{}", wrapper_class)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_{}", extension_class)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_OBJC_METACLASS_$_{}", wrapper_class)),
        ("-C", format!("link-arg=-Wl,-exported_symbol,_OBJC_METACLASS_$_{}", extension_class)),
        ("-C", deployment_target_link_arg()),
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

    vec![
        "-C".to_string(),
        format!("link-arg=-Wl,-install_name,@loader_path/{}", to_vst3_executable_name(package)),
        "-C".to_string(),
        deployment_target_link_arg(),
    ]
}

/// Extra rustc arguments for the plugin crate of a build for `format`.
///
/// For an AUv3 this compiles the plugin's Objective-C code first. An AUv2
/// gets its Objective-C code when it is bundled, so its library has none.
fn plugin_rustc_args(
    package: &str,
    workspace_root: &Path,
    format: Format,
    target: &str,
    verbose: bool,
) -> Result<Vec<String>, String> {
    match format {
        Format::Auv2 => Ok(vec!["-C".to_string(), deployment_target_link_arg()]),
        Format::Auv3 => {
            crate::verbose!(verbose, "    Generating plugin-specific ObjC for {} ({})...", package, target);
            let names = AuNames::detect(package, &crate::util::find_package(package, workspace_root)?.dir);
            let objc_lib_dir = compile_plugin_objc(package, &names, workspace_root, target)?;
            auv3_rustc_args(package, &names, &objc_lib_dir)
        }
        Format::Vst3 => Ok(vst3_rustc_args(package, target)),
    }
}

/// The minimum macOS version as a linker argument for the plugin crate.
///
/// The build also gets the version through `MACOSX_DEPLOYMENT_TARGET`, but
/// cargo does not notice when that variable changes. It does notice a changed
/// argument, so this makes it link the library again when the version changes.
fn deployment_target_link_arg() -> String {
    format!("link-arg={MACOS_MIN_VERSION_FLAG}")
}

/// Create the cargo command that builds the plugin crate for one target.
///
/// `plugin_args` are extra rustc arguments for the plugin crate alone, from
/// `plugin_rustc_args`. They go through `cargo rustc`, which passes them to
/// that crate only. Without them (a VST3 for a target other than Apple's)
/// the command is a plain `cargo build`.
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
    format: Format,
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
        .arg(format.feature())
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

    // Build the library for the same minimum macOS version as the parts of
    // the bundle that clang compiles. The compiler's own default differs per
    // architecture.
    if target.ends_with("-apple-darwin") {
        cmd.env("MACOSX_DEPLOYMENT_TARGET", MACOS_DEPLOYMENT_TARGET);
    }

    Ok(cmd)
}

/// Build for a single architecture (native, arm64, or x86_64).
pub fn build_native(
    package: &str,
    release: bool,
    workspace_root: &Path,
    format: Format,
    arch: Arch,
    verbose: bool,
) -> Result<PathBuf, String> {
    // Always use explicit target to prevent the rustc flags leaking into build scripts.
    let [target] = target_triples(arch)[..] else {
        unreachable!("Universal should use build_universal");
    };

    let arch_name = clang_arch(target)?;
    crate::status!("  Building {} ({})...", format.label(), arch_name);

    let profile = if release { "release" } else { "debug" };
    let lib_name = package.replace('-', "_");
    let dylib_name = format!("lib{}.dylib", lib_name);

    let plugin_args = plugin_rustc_args(package, workspace_root, format, target, verbose)?;

    let mut cmd = plugin_build_command(package, target, format, release, workspace_root, &plugin_args)?;

    let status = cmd.status().map_err(|e| format!("Failed to run cargo: {}", e))?;
    if !status.success() {
        return Err(format!("{} build failed", format.label()));
    }

    // Output is always in target/<target>/<profile>/
    let dylib_path = workspace_root.join("target").join(target).join(profile).join(&dylib_name);

    if !dylib_path.exists() {
        return Err(format!("Built library not found: {}", dylib_path.display()));
    }

    if target.ends_with("-apple-darwin") {
        check_string_table_alignment(&dylib_path)?;
    }

    crate::verbose!(verbose, "    Binary: {}", dylib_path.display());
    Ok(dylib_path)
}

/// Build universal binary (x86_64 + arm64) for the given format.
pub fn build_universal(
    package: &str,
    release: bool,
    workspace_root: &Path,
    format: Format,
    verbose: bool,
) -> Result<PathBuf, String> {
    crate::status!("  Building {} (universal)...", format.label());

    let profile = if release { "release" } else { "debug" };
    let lib_name = package.replace('-', "_");
    let dylib_name = format!("lib{}.dylib", lib_name);

    let plugin_args_x86 = plugin_rustc_args(package, workspace_root, format, "x86_64-apple-darwin", verbose)?;
    let plugin_args_arm = plugin_rustc_args(package, workspace_root, format, "aarch64-apple-darwin", verbose)?;

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

    check_string_table_alignment(&universal_path)?;

    crate::verbose!(verbose, "    Binary: {}", universal_path.display());

    Ok(universal_path)
}

/// Stop when a Mach-O library has a string table that is not 8-byte aligned.
///
/// The strip step of Rust toolchains whose LLVM is older than version 23 can
/// leave the string table 4 bytes off. Apple's linker refuses to link against
/// such a library, which the AU bundles do, and according to the LLVM fix
/// dyld of macOS 27 refuses to load it, whatever the format.
fn check_string_table_alignment(library: &Path) -> Result<(), String> {
    let data = fs::read(library).map_err(|e| format!("Failed to read {}: {}", library.display(), e))?;
    let misaligned = misaligned_string_tables(&data).map_err(|e| format!("{}: {}", library.display(), e))?;
    if misaligned.is_empty() {
        return Ok(());
    }

    Err(format!(
        "The string table of {} is not 8-byte aligned. Apple's linker refuses such a library and \
         newer macOS versions do not load it. The strip step of a Rust toolchain whose LLVM is \
         older than version 23 causes this (`rustc -vV` shows the version). Update Rust, or set \
         `strip = false` in the profile.",
        library.display()
    ))
}

/// The error for a Mach-O file that ends in the middle of a structure.
const MACH_O_CUT_SHORT: &str = "The Mach-O file is cut short";

fn read_u32(data: &[u8], at: usize, big_endian: bool) -> Result<u32, String> {
    let bytes: [u8; 4] = at
        .checked_add(4)
        .and_then(|end| data.get(at..end))
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or(MACH_O_CUT_SHORT)?;
    Ok(if big_endian { u32::from_be_bytes(bytes) } else { u32::from_le_bytes(bytes) })
}

fn read_u64(data: &[u8], at: usize, big_endian: bool) -> Result<u64, String> {
    let high_first = [read_u32(data, at, big_endian)?, read_u32(data, at + 4, big_endian)?];
    let [high, low] = if big_endian { high_first } else { [high_first[1], high_first[0]] };
    Ok((u64::from(high) << 32) | u64::from(low))
}

/// The string table offsets of a Mach-O file that are not 8-byte aligned.
///
/// A universal file is checked slice by slice, with the offsets relative to
/// the slice.
fn misaligned_string_tables(data: &[u8]) -> Result<Vec<u32>, String> {
    const FAT_MAGIC: u32 = 0xcafe_babe;
    const FAT_MAGIC_64: u32 = 0xcafe_babf;

    let magic = read_u32(data, 0, true)?;
    if magic != FAT_MAGIC && magic != FAT_MAGIC_64 {
        return misaligned_string_tables_of_image(data);
    }

    // The header of a universal file is big-endian. Each entry names the
    // offset and the size of one slice.
    let wide = magic == FAT_MAGIC_64;
    let count = read_u32(data, 4, true)? as usize;
    let mut misaligned = Vec::new();
    for index in 0..count {
        let entry = 8 + index * if wide { 32 } else { 20 };
        let (offset, size) = if wide {
            (read_u64(data, entry + 8, true)?, read_u64(data, entry + 16, true)?)
        } else {
            (u64::from(read_u32(data, entry + 8, true)?), u64::from(read_u32(data, entry + 12, true)?))
        };
        let slice = usize::try_from(offset)
            .ok()
            .zip(usize::try_from(size).ok())
            .and_then(|(offset, size)| data.get(offset..offset.checked_add(size)?))
            .ok_or(MACH_O_CUT_SHORT)?;
        misaligned.extend(misaligned_string_tables_of_image(slice)?);
    }
    Ok(misaligned)
}

/// The string table offset of one 64-bit Mach-O image, if it is not 8-byte
/// aligned.
fn misaligned_string_tables_of_image(data: &[u8]) -> Result<Vec<u32>, String> {
    const MH_MAGIC_64: u32 = 0xfeed_facf;
    const HEADER_SIZE: usize = 32;
    const LC_SYMTAB: u32 = 0x2;

    if read_u32(data, 0, false)? != MH_MAGIC_64 {
        return Err("Not a 64-bit Mach-O file".to_string());
    }

    let command_count = read_u32(data, 16, false)?;
    let mut at = HEADER_SIZE;
    let mut misaligned = Vec::new();
    for _ in 0..command_count {
        let command = read_u32(data, at, false)?;
        let size = read_u32(data, at + 4, false)? as usize;
        if size == 0 {
            return Err("The Mach-O file has an empty load command".to_string());
        }
        // The symbol table command holds the offset of the string table
        // after its own header and the symbol offset and count.
        if command == LC_SYMTAB {
            let string_table = read_u32(data, at + 16, false)?;
            if string_table % 8 != 0 {
                misaligned.push(string_table);
            }
        }
        at += size;
    }
    Ok(misaligned)
}

/// Target triples that are built for an architecture selection.
pub fn target_triples(arch: Arch) -> Vec<&'static str> {
    match arch {
        Arch::Native => vec![current_target()],
        Arch::Arm64 => vec!["aarch64-apple-darwin"],
        Arch::X86_64 => vec!["x86_64-apple-darwin"],
        Arch::Universal => vec!["x86_64-apple-darwin", "aarch64-apple-darwin"],
    }
}

/// Architecture name that clang and lipo use for a target triple.
pub fn clang_arch(target: &str) -> Result<&'static str, String> {
    match target {
        "x86_64-apple-darwin" => Ok("x86_64"),
        "aarch64-apple-darwin" => Ok("arm64"),
        _ => Err(format!("Unsupported target: {}", target)),
    }
}

/// Architecture names that clang and lipo use for an architecture selection.
pub fn clang_arches(arch: Arch) -> Result<Vec<&'static str>, String> {
    target_triples(arch).into_iter().map(clang_arch).collect()
}

/// Force a rebuild of the plugin and remove its previous bundles.
///
/// A normal build picks up changes by itself, including a change to the
/// generated AUv3 ObjC code, its headers or its compiler (see
/// `compile_plugin_objc`). This is the way out when something is stale anyway.
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

/// Flags for compiling the generated ObjC sources.
const OBJC_FLAGS: [&str; 3] = ["-fobjc-arc", "-fmodules", MACOS_MIN_VERSION_FLAG];

/// Hash the inputs of a compile step into a directory name.
fn hash_inputs(inputs: &[&[u8]]) -> String {
    let mut hasher = DefaultHasher::new();
    for input in inputs {
        input.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

/// Read every header in the given directories, in a stable order.
fn read_headers(dirs: &[&Path]) -> Result<Vec<Vec<u8>>, String> {
    let mut paths = Vec::new();
    for dir in dirs {
        let entries = fs::read_dir(dir)
            .map_err(|e| format!("Failed to read {}: {}", dir.display(), e))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().is_some_and(|extension| extension == "h") {
                paths.push(path);
            }
        }
    }
    paths.sort();

    paths
        .iter()
        .map(|path| fs::read(path).map_err(|e| format!("Failed to read {}: {}", path.display(), e)))
        .collect()
}

/// Generate and compile the AUv3 ObjC code of a plugin.
///
/// The classes are named by `names`, so that the classes of several Audio
/// Units can be loaded in one process (see `AuNames`).
///
/// Returns the path to the directory containing the compiled static libraries.
/// The directory is named after a hash of the generated sources, the headers
/// they include, the compiler flags and the compiler version. It is part of
/// the linker arguments of the plugin crate, so cargo relinks the plugin when
/// one of them changes. Cargo does not track the libraries themselves.
fn compile_plugin_objc(
    plugin_name: &str,
    names: &AuNames,
    workspace_root: &Path,
    target: &str,
) -> Result<PathBuf, String> {
    let lib_name = plugin_name.replace('-', "_");

    // Generate plugin-specific ObjC source files.
    let wrapper_source = generate_au_wrapper_source(plugin_name, names);
    let package_dir = crate::util::find_package(plugin_name, workspace_root)?.dir;
    let has_gui = crate::util::detect_has_gui(&package_dir);
    let extension_source = generate_au_extension_source(plugin_name, names, has_gui);

    // Header search paths for clang.
    let bridge_header_dir = workspace_root.join("crates/beamer-au/objc");
    let ipc_header_dir = workspace_root.join("xtask/src/au_codegen");

    // The compiler is an input as well: another clang can produce other code.
    let clang_version = Command::new("clang")
        .arg("--version")
        .output()
        .map_err(|e| format!("Failed to run clang: {}", e))?
        .stdout;
    let flags = OBJC_FLAGS.join(" ");

    let headers = read_headers(&[&bridge_header_dir, &ipc_header_dir])?;
    let mut inputs: Vec<&[u8]> = vec![
        wrapper_source.as_bytes(),
        extension_source.as_bytes(),
        flags.as_bytes(),
        &clang_version,
    ];
    inputs.extend(headers.iter().map(Vec::as_slice));

    // Create output directory for generated sources and compiled libraries.
    let target_dir = workspace_root
        .join("target")
        .join("au-gen")
        .join(plugin_name)
        .join(target);
    let gen_dir = target_dir.join(hash_inputs(&inputs));

    // Only the directory for the current inputs is kept.
    if target_dir.exists() {
        for entry in fs::read_dir(&target_dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path == gen_dir {
                continue;
            }
            let removed = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
            removed.map_err(|e| format!("Failed to remove {}: {}", path.display(), e))?;
        }
    }

    fs::create_dir_all(&gen_dir)
        .map_err(|e| format!("Failed to create au-gen directory: {}", e))?;

    let wrapper_path = gen_dir.join("AuWrapper.m");
    let extension_path = gen_dir.join("AuExtension.m");

    fs::write(&wrapper_path, &wrapper_source)
        .map_err(|e| format!("Failed to write AuWrapper.m: {}", e))?;
    fs::write(&extension_path, &extension_source)
        .map_err(|e| format!("Failed to write AuExtension.m: {}", e))?;

    // Determine architecture for clang
    let arch = clang_arch(target)?;

    // Compile AuWrapper.m to static library
    let wrapper_obj = gen_dir.join("AuWrapper.o");
    let wrapper_lib = gen_dir.join(format!("lib{}_au_objc.a", lib_name));

    let status = Command::new("clang")
        .args(["-c", "-arch", arch])
        .args(OBJC_FLAGS)
        .args([
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
        .args(["-c", "-arch", arch])
        .args(OBJC_FLAGS)
        .args([
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
    fn test_bundle_version_is_the_numbers_of_the_package_version() {
        assert_eq!(get_version_info("0.2.4"), Ok(("0.2.4".to_string(), 0x000204)));
        assert_eq!(get_version_info("3.1.4"), Ok(("3.1.4".to_string(), 196868)));

        // A bundle version has no pre-release or build suffix.
        assert_eq!(get_version_info("1.0.0-rc.1"), Ok(("1.0.0".to_string(), 0x010000)));
        assert_eq!(get_version_info("1.2.3+5"), Ok(("1.2.3".to_string(), 0x010203)));

        assert!(get_version_info("1.2").is_err());
        assert!(get_version_info("one.2.3").is_err());
    }

    #[test]
    fn test_audio_unit_version_numbers_stay_in_their_fields() {
        assert_eq!(get_version_info("65535.255.255"), Ok(("65535.255.255".to_string(), 0xffff_ffff)));

        // Each of these would run into the field next to it.
        assert!(get_version_info("1.0.256").is_err());
        assert!(get_version_info("1.256.0").is_err());
        assert!(get_version_info("65536.0.0").is_err());

        // A VST3 bundle has no version integer and takes them.
        assert_eq!(bundle_version("1.0.256"), Ok("1.0.256".to_string()));
        assert_eq!(bundle_version("2.300.0-beta"), Ok("2.300.0".to_string()));
        assert!(bundle_version("1.2").is_err());
        assert!(bundle_version("1.x.3").is_err());
    }

    #[test]
    fn test_clang_arches() {
        // Universal builds list x86_64 first, like the build order.
        assert_eq!(clang_arches(Arch::Universal).unwrap(), ["x86_64", "arm64"]);
        assert_eq!(clang_arches(Arch::Arm64).unwrap(), ["arm64"]);
        assert_eq!(clang_arches(Arch::X86_64).unwrap(), ["x86_64"]);
        assert_eq!(target_triples(Arch::Native).len(), 1);

        assert!(clang_arch("x86_64-pc-windows-msvc").is_err());
    }

    #[test]
    fn test_hash_inputs() {
        let hash = hash_inputs(&[b"wrapper", b"extension"]);
        assert_eq!(hash.len(), 16);
        assert_eq!(hash, hash_inputs(&[b"wrapper", b"extension"]));

        // A change in any input, or content moving between inputs, gives
        // another directory.
        assert_ne!(hash, hash_inputs(&[b"wrapper", b"extension2"]));
        assert_ne!(hash, hash_inputs(&[b"wrapperex", b"tension"]));
    }

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
    fn test_deployment_target_is_set_for_apple_targets() {
        let target_of = |target: &str| {
            let cmd = plugin_build_command("gain", target, Format::Vst3, false, Path::new("."), &[])
                .expect("command");
            cmd.get_envs()
                .find(|(key, _)| *key == "MACOSX_DEPLOYMENT_TARGET")
                .map(|(_, value)| value.map(|v| v.to_string_lossy().into_owned()))
        };

        let expected = Some(Some(MACOS_DEPLOYMENT_TARGET.to_string()));
        assert_eq!(target_of("aarch64-apple-darwin"), expected);
        assert_eq!(target_of("x86_64-apple-darwin"), expected);
        assert_eq!(target_of("x86_64-pc-windows-msvc"), None);
    }

    #[test]
    fn test_vst3_rustc_args() {
        assert_eq!(
            vst3_rustc_args("midi-transform", "aarch64-apple-darwin"),
            [
                "-C",
                "link-arg=-Wl,-install_name,@loader_path/BeamerMidiTransform",
                "-C",
                "link-arg=-mmacosx-version-min=11.0",
            ]
        );

        // Only Mach-O has install names.
        assert!(vst3_rustc_args("midi-transform", "x86_64-pc-windows-msvc").is_empty());
    }

    fn names(component_type: &str, subtype: &str, manufacturer: &str) -> AuNames {
        AuNames::new(
            "midi-transform",
            &crate::util::AuCodes {
                component_type: component_type.to_string(),
                subtype: subtype.to_string(),
                manufacturer: manufacturer.to_string(),
            },
        )
    }

    #[test]
    fn test_auv3_rustc_args_are_separate_arguments() {
        let names = names("aumi", "midi", "Bmer");
        let args = auv3_rustc_args("midi-transform", &names, Path::new("/tmp/My Projects/au-gen")).unwrap();

        // Every flag is followed by its value, and a space in the path does
        // not split an argument.
        assert_eq!(args.len() % 2, 0);
        assert_eq!(
            &args[..2],
            ["-C", "link-arg=-Wl,-force_load,/tmp/My Projects/au-gen/libmidi_transform_au_objc.a"]
        );
        assert!(args.chunks(2).all(|pair| pair[0] == "-C"));
        assert!(args.contains(&format!("link-arg=-Wl,-exported_symbol,_OBJC_CLASS_$_{}", names.wrapper_class)));
    }

    /// A 64-bit Mach-O image with one symbol table command.
    fn image(string_table: u32) -> Vec<u8> {
        let mut data = Vec::new();
        for word in [0xfeed_facf_u32, 0x0100_000c, 0, 6, 1, 24, 0, 0] {
            data.extend(word.to_le_bytes());
        }
        for word in [0x2_u32, 24, 4096, 10, string_table, 64] {
            data.extend(word.to_le_bytes());
        }
        data
    }

    /// A universal file of the images, each padded to 64 bytes.
    fn universal(images: &[Vec<u8>]) -> Vec<u8> {
        let header_size = 8 + 20 * images.len();
        let mut data = Vec::new();
        data.extend(0xcafe_babe_u32.to_be_bytes());
        data.extend((images.len() as u32).to_be_bytes());
        for (index, image) in images.iter().enumerate() {
            let offset = (header_size + 64 * index) as u32;
            for word in [0x0100_000c_u32, 0, offset, image.len() as u32, 3] {
                data.extend(word.to_be_bytes());
            }
        }
        for image in images {
            let mut padded = image.clone();
            padded.resize(64, 0);
            data.extend(padded);
        }
        data
    }

    #[test]
    fn test_string_table_alignment_is_checked_per_slice() {
        assert_eq!(misaligned_string_tables(&image(959_184)), Ok(vec![]));
        assert_eq!(misaligned_string_tables(&image(959_180)), Ok(vec![959_180]));

        let both = universal(&[image(1024), image(1028)]);
        assert_eq!(misaligned_string_tables(&both), Ok(vec![1028]));
    }

    #[test]
    fn test_broken_mach_o_files_are_errors() {
        assert!(misaligned_string_tables(&[]).is_err());
        assert!(misaligned_string_tables(b"not a library").is_err());
        assert!(misaligned_string_tables(&image(1024)[..40]).is_err());

        // A slice that reaches past the end of the file.
        let mut cut = universal(&[image(1024)]);
        cut.truncate(8 + 20 + image(1024).len() - 1);
        assert!(misaligned_string_tables(&cut).is_err());
    }

    #[test]
    fn test_auv2_library_gets_no_objc() {
        // The AUv2 compiles its classes into the component binary. Linking
        // the AUv3 classes into its library as well would define them twice
        // in a process that loads both formats.
        let args = plugin_rustc_args("midi-transform", Path::new("."), Format::Auv2, "aarch64-apple-darwin", false)
            .unwrap();
        assert_eq!(args, ["-C".to_string(), deployment_target_link_arg()]);
    }
}
