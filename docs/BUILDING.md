# Building, Installing and Testing Plugins

Plugins are built with `cargo xtask bundle`. One command builds the
plugin's web assets if it has any, compiles the Rust library, wraps it in
the bundle each plugin format expects and optionally installs it.

| Format | Flag | Platform |
|--------|------|----------|
| AUv2 | `--auv2` | macOS |
| AUv3 | `--auv3` | macOS |
| VST3 | `--vst3` | macOS (Windows in progress) |

xtask is the build tool of this workspace. It finds the plugin package by
its name among the members of the workspace, wherever its folder is, and
reads `Config.toml` and the optional `webview/` folder from that folder.
The version of the package becomes the version of the bundle, without a
pre-release or build suffix. An Audio Unit stores it as one number, so for
AUv2 and AUv3 the minor and the patch version can be at most 255.

## Prerequisites

- Rust, installed with [rustup](https://rustup.rs).
- Run the commands inside the repository: `cargo xtask` is an alias
  defined in its `.cargo/config.toml`.
- For plugins with a `webview/package.json`: the JavaScript package manager
  the web project uses. xtask picks bun, pnpm, yarn or npm from the
  lockfile and falls back to bun.

### macOS

- Bundles are built for macOS 11.0 and later, on Apple Silicon and Intel.
  xtask sets that minimum for every binary in a bundle, whatever macOS
  version the build machine runs.
- The Xcode Command Line Tools (`xcode-select --install`). Rust needs them
  to link, and xtask uses clang, ar, install_name_tool and codesign for the
  Audio Unit formats and lipo for universal binaries.
- For universal binaries, or to build for the other architecture, both
  Apple targets:

  ```bash
  rustup target add x86_64-apple-darwin aarch64-apple-darwin
  ```

## Plugin crate requirements

A plugin crate needs a dynamic library target and one feature per format,
because xtask builds it with `--features au` or `--features vst3`:

```toml
[lib]
crate-type = ["cdylib"]

[features]
au = ["beamer/au"]
vst3 = ["beamer/vst3"]

[dependencies]
beamer = { workspace = true }
```

The crate must be listed under `[workspace] members` in the root
`Cargo.toml` and keep its default library name: xtask looks for
`lib<package>.dylib`, with hyphens as underscores.

`Config.toml` sits next to `Cargo.toml` and describes the plugin (see
[Plugin Configuration](REFERENCE.md#plugin-configuration)). A `webview/`
folder with the plugin's web UI is optional.

## Building

```bash
cargo xtask bundle <package> <formats> [options]
```

At least one format flag is required, and formats can be combined:

```bash
cargo xtask bundle gain --auv2 --release
cargo xtask bundle gain --auv3 --release
cargo xtask bundle gain --vst3 --release
cargo xtask bundle gain --auv2 --auv3 --vst3 --release
```

| Option | Effect |
|--------|--------|
| `--release` | Optimized build. The default debug build can be too slow for real-time audio |
| `--arch <arch>` | `native` (default), `universal` (x86_64 and arm64), `arm64` or `x86_64` |
| `--install` | Install the bundles after building (see [Installing](#installing)) |
| `--clean` | Force a rebuild of the plugin and remove its previous bundles first. Normally not needed |
| `--verbose`, `-v` | Show detailed build output |

Bundles land in `target/release` (or `target/debug`) and are named after
the package: `BeamerGain.component` (AUv2), `BeamerGain.app` (AUv3) and
`BeamerGain.vst3`. A package name with hyphens becomes one word:
`midi-transform` gives `BeamerMidiTransform.vst3`.

`cargo xtask generate-uuid` prints a new UUID, for plugins that set
`vst3_id` in `Config.toml`.

### Audio Units

For AUv2 and AUv3, xtask generates Objective-C wrapper code for the plugin,
compiles it with clang and signs the bundles ad hoc. An AUv3 is an app that
contains the Audio Unit as an app extension. The component's `Info.plist`
(for an AUv3, the app extension's) registers the Audio Unit under the type,
plugin code and manufacturer code from `Config.toml`. VST3 bundles need no
wrapper and xtask does not sign them (on Apple Silicon the linker already
signs the binary ad hoc).

Ad hoc signatures are enough for local development. Distributing plugins
outside the Mac App Store needs a Developer ID signature and notarization,
which xtask does not do.

### WebView assets

If the plugin has a `webview/package.json`, xtask runs the package
manager's install and build before compiling and expects the result in
`webview/dist/index.html`. A plain `webview/index.html` needs no build
step. For a dev server with live reload, see
[Development Server](WEBVIEW.md#development-server).

### Compiler flags

Release builds do not carry the source paths of the machine that built
them. The compiler keeps source paths for panic messages, and xtask
replaces their local prefixes with `--remap-path-prefix`: the workspace
becomes `.`, the Cargo home `/cargo` and the standard library sources
`/rustc/<commit>`. Debug builds keep the full paths. Dependencies that
live outside the workspace and the Cargo home are not covered.

The remap flags are the only flags xtask sets for every crate, and only for
release builds. It keeps the flags from `CARGO_ENCODED_RUSTFLAGS`, or else
`RUSTFLAGS`, in front of them. `rustflags` from Cargo configuration,
whether from config files or from `CARGO_BUILD_RUSTFLAGS` and
`CARGO_TARGET_<TRIPLE>_RUSTFLAGS`, do not apply to release builds. A change
of your own flags rebuilds all dependencies of a release build.

### What xtask produces

AUv2:

```
BeamerGain.component/
└── Contents/
    ├── Info.plist
    ├── PkgInfo
    ├── MacOS/
    │   ├── BeamerGain        (component binary built from the wrapper)
    │   └── libgain.dylib     (the plugin's Rust library)
    └── Resources/
```

AUv3:

```
BeamerGain.app/
└── Contents/
    ├── Info.plist
    ├── PkgInfo
    ├── MacOS/
    │   └── BeamerGain                (container app)
    ├── Frameworks/
    │   └── BeamerGainAU.framework/   (the plugin's Rust library)
    ├── PlugIns/
    │   └── BeamerGain.appex/         (the Audio Unit extension)
    └── Resources/
```

VST3:

```
BeamerGain.vst3/
└── Contents/
    ├── Info.plist
    ├── PkgInfo
    ├── MacOS/
    │   └── BeamerGain
    └── Resources/
```

## Installing

`--install` copies the bundles to the user's plugin folders, replacing an
earlier install:

| Format | Location |
|--------|----------|
| AUv2 | `~/Library/Audio/Plug-Ins/Components/` |
| AUv3 | `~/Applications/` |
| VST3 | `~/Library/Audio/Plug-Ins/VST3/` |

After installing an Audio Unit, xtask stops the system's Audio Unit
registry (`AudioComponentRegistrar`), which then restarts and picks up the
new build. For an AUv3 it also launches the container app once, which
registers the extension. Rescan in your DAW if the plugin does not appear.

## Uninstalling

```bash
cargo xtask uninstall <package> [formats]
```

This removes the bundles that `--install` copied to the plugin folders.
With format flags it removes only those formats. Without one it removes
every format:

```bash
cargo xtask uninstall gain
cargo xtask uninstall gain --auv3
```

A format that is not installed is skipped. After removing an Audio Unit,
xtask restarts the system's Audio Unit registry, so that hosts stop listing
the plugin. The bundles in `target/` are left alone.

## Testing

### Validators

`auval` ships with macOS. `auval -a` lists the installed Audio Units and
`auval -v` validates one. It takes the component type and the plugin and
manufacturer codes from `Config.toml`:

```bash
auval -v aufx gain Bmer
```

The type is `aufx` for effects, `aumu` for instruments, `aumi` for MIDI
effects and `augn` for generators.

The app extension of an AUv3 names a framework with its code
(`AudioComponentBundle` in its `Info.plist`), so `auval` loads the AUv3
into the `auval` process, as it does an AUv2. `-strict` turns warnings
into failures and `-stress 20` adds a multi-threaded stress test over 20
seconds of simulated audio. For an AUv3 instrument loaded this way, the
stress test never finishes: macOS hands the render block an event list in
which a MIDI event points to itself. AUv3 instruments of other vendors
hang the same way, so this is not a bug in the plugin. Effects pass. Run
the stress test of an AUv3 instrument out of process with `-oop`, here
for the drums example:

```bash
auval -strict -stress 20 -v aumu drum Bmer -oop
```

[pluginval](https://github.com/Tracktion/pluginval) validates VST3 plugins
and AUv2 components. Its command-line binary is inside the app bundle (the
example assumes the app is in `/Applications`):

```bash
/Applications/pluginval.app/Contents/MacOS/pluginval --strictness-level 5 --validate ~/Library/Audio/Plug-Ins/VST3/BeamerGain.vst3
```

A VST3 bundle is validated by its path, also straight from `target/`. An
AUv2 has to be installed, because pluginval looks it up in the Audio Unit
registry. Name it by the path of the installed component or by the same
type and codes as for `auval`:

```bash
/Applications/pluginval.app/Contents/MacOS/pluginval --strictness-level 5 --validate "AudioUnit:Effects/aufx,gain,Bmer"
```

The word before the slash is a label that pluginval ignores.

pluginval (as of 1.0.4) does not find an AUv3. While the AUv3 of a plugin
is installed, it does not find the AUv2 with the same codes either: see
"AUv2 and AUv3 side by side" below. Use `auval` to validate an AUv3.

### In a DAW

Install the plugin and rescan in your DAW after each new build. With
`log_level` in `Config.toml`, the plugin's log messages show up in the
macOS system log while the DAW runs it (see
[Logging](REFERENCE.md#logging)):

```bash
cargo xtask log gain
```

### AUv2 and AUv3 side by side

macOS treats an AUv2 and an AUv3 with the same type, plugin code and
manufacturer code as one plugin, and hosts show only one of them, usually
the AUv3. This is intended: an AUv3 can replace its AUv2 without breaking
existing projects. To test both formats of one plugin, build one format
with a different `plugin_code` in `Config.toml`, or uninstall one format:

```bash
cargo xtask uninstall gain --auv3
```

### Running the tests

```bash
cargo test
cargo clippy
```

## Troubleshooting

- A reinstalled Audio Unit does not appear, or the DAW still loads the old
  build: run `killall -9 AudioComponentRegistrar` and rescan in the DAW.
- `webview build did not produce dist/index.html`: the web project's build
  script must write its output to `webview/dist`.
