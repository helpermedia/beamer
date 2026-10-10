# Building, Installing and Testing Plugins

Plugins are built with `cargo xtask bundle`. One command builds the
plugin's web assets if it has any, compiles the Rust library, wraps it in
the bundle each plugin format expects and optionally installs it.

| Format | Flag | Platform |
|--------|------|----------|
| AUv2 | `--auv2` | macOS |
| AUv3 | `--auv3` | macOS |
| VST3 | `--vst3` | macOS (Windows in progress) |

xtask is the build tool of this workspace. It looks for the plugin package
under `examples/<package>`, where it reads `Config.toml` and the optional
`webview/` folder.

## Prerequisites

- Rust, installed with [rustup](https://rustup.rs).
- Run the commands inside the repository: `cargo xtask` is an alias
  defined in its `.cargo/config.toml`.
- For plugins with a `webview/package.json`: the JavaScript package manager
  the web project uses. xtask picks bun, pnpm, yarn or npm from the
  lockfile and falls back to bun.

### macOS

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
| `--clean` | Force a rebuild of the plugin and remove its previous bundles first. Needed after a change to xtask's AUv3 Objective-C templates, since that alone does not relink the plugin |
| `--verbose` | Show detailed build output |

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

[pluginval](https://github.com/Tracktion/pluginval) validates VST3 and
Audio Unit plugins. Its command-line binary is inside the app bundle (the
example assumes the app is in `/Applications`):

```bash
/Applications/pluginval.app/Contents/MacOS/pluginval --strictness-level 5 --validate ~/Library/Audio/Plug-Ins/VST3/BeamerGain.vst3
```

### In a DAW

Install the plugin and rescan in your DAW after each new build.

### AUv2 and AUv3 side by side

macOS treats an AUv2 and an AUv3 with the same type, plugin code and
manufacturer code as one plugin, and hosts show only one of them, usually
the AUv3. This is intended: an AUv3 can replace its AUv2 without breaking
existing projects. To test both formats of one plugin, build one format
with a different `plugin_code` in `Config.toml`, or uninstall one format
and refresh the registry:

```bash
killall -9 AudioComponentRegistrar
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
