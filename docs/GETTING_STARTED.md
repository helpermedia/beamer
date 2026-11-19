# Getting Started

This guide takes you from a clone of the repository to a plugin of your own with an editor. A Beamer plugin has two sides in one crate: Rust code that processes the audio and holds the parameters, plus a web page in the plugin's editor that shows and edits them. One build turns both into a bundle per format.

## Prerequisites

- A current stable Rust, installed with [rustup](https://rustup.rs) (`rustup update` keeps it current).
- On macOS the Xcode Command Line Tools (`xcode-select --install`).
- For a web UI with a build step (React, Svelte, TypeScript): [bun](https://bun.sh) or the package manager whose lockfile the web project has.

[BUILDING.md](BUILDING.md#prerequisites) has the details.

## Build an Example

```bash
git clone https://github.com/helpermedia/beamer.git
cd beamer
cargo xtask bundle gain --auv3 --vst3 --release --install
```

`cargo xtask` is the build tool of the repository and runs inside it. This builds the gain example as an AUv3 and a VST3 and installs both. Rescan the plugins in your DAW and load **Beamer Gain**.

The gain example has no editor: the host shows its parameter in a generic view of its own. Bundle `delay` the same way to see one with an editor.

## A Plugin of Your Own

Start from the gain example, the smallest one:

1. Copy `examples/gain` to a folder of your own inside the repository (for example `plugins/my-gain`) and give the package a name of its own in its `Cargo.toml` (`name = "my-gain"`).
2. Add the folder to `[workspace] members` in the root `Cargo.toml`.
3. In its `Config.toml`, set `name` and `vendor`, then pick a `manufacturer_code` and a `plugin_code` of your own: four characters each, which identify the plugin to hosts.
4. Build it with `cargo xtask bundle my-gain --auv3 --vst3 --release --install`.

## The Rust Side

A Beamer plugin is two structs: the **Model** that every thread shares and the **Processor** that the audio thread owns. The model holds the parameters, the `State` fields that are saved with them and everything else the host, the editor and the audio thread use. The framework keeps it in one `Arc` for the life of the plugin instance. When the audio setup is known, the framework calls `prepare(setup)` on the model to create the Processor, so audio buffers are allocated before `process()` runs. When the setup changes, it drops the Processor and prepares a new one.

The gain example in short:

```rust
use beamer::prelude::*;

// 1. Model - what every thread shares. It holds the parameters,
//    describes the plugin to the host and lives as long as the plugin
//    instance.
#[beamer::export]
#[beamer::model]
struct GainModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    gain: FloatParameter,
}

impl Model for GainModel {
    // A plain gain needs no setup. Use SampleRate for delays and
    // MaxBufferSize for FFT.
    type Setup = ();
    type Processor = GainProcessor;

    fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
        GainProcessor { model: self }
    }
}

// 2. Processor - what the audio thread owns. It shares the model with the
//    host and the editor.
struct GainProcessor {
    model: Arc<GainModel>,
}

impl Processor for GainProcessor {
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        let gain = self.model.gain.as_linear() as f32;
        // The buffer is processed in place: each channel holds its input.
        for channel in buffer.channels_mut() {
            for sample in channel {
                *sample *= gain;
            }
        }
    }
}
```

`#[beamer::model]` turns the fields into parameters the host can automate and saves them with the project. `#[beamer::export]` reads `Config.toml` at compile time and generates the entry points of each format. `Config.toml` sits next to `Cargo.toml`:

```toml
name = "My Gain Plugin"
category = "effect"
subcategories = ["dynamics"]
manufacturer_code = "Manu"
plugin_code = "mygn"
vendor = "My Company"
url = "https://example.com"
email = "support@example.com"
log_level = "warn"
```

[REFERENCE.md](REFERENCE.md) covers the fields of `Config.toml`, the kinds of parameters, the setups `prepare()` can ask for, MIDI and more. [ARCHITECTURE.md](../ARCHITECTURE.md#plugin-lifecycle) explains why the plugin is split this way.

## An Editor

A plugin has an editor when its crate has a web UI in a `webview/` folder next to `Config.toml`. `#[beamer::export]` embeds its files in the plugin at compile time and the editor shows `index.html` in the system's webview (WKWebView on macOS). There is no setting to turn the editor on.

Give the editor its size in `Config.toml`:

```toml
editor_size = [320, 120]
```

Then add `webview/index.html` with a slider for the gain:

```html
<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <style>
    body { font: 14px system-ui; padding: 24px; background: #171717; color: #fafafa; }
  </style>
</head>
<body>
  <label>Gain <input id="gain" type="range" min="0" max="1" step="0.001"></label>
  <output id="text"></output>
  <script>
    const parameters = __BEAMER__.parameters;
    const slider = document.getElementById("gain");
    const text = document.getElementById("text");
    let editing = false;

    // Follow the parameter, whoever changes it: this page, the host or
    // its automation. During a drag the slider stays under the pointer.
    parameters.on("gain", (value) => {
      if (!editing) slider.value = value;
      text.textContent = `${parameters.getDisplayText("gain")} dB`;
    });

    // An edit is one gesture from the first change to the release, which
    // the host records as one undo step.
    slider.addEventListener("input", () => {
      if (!editing) parameters.beginEdit("gain");
      editing = true;
      parameters.set("gain", Number(slider.value));
    });
    slider.addEventListener("change", () => {
      if (editing) parameters.endEdit("gain");
      editing = false;
    });
  </script>
</body>
</html>
```

Bundle the plugin again and the host opens this page as its editor. `__BEAMER__` is the plugin's runtime in the page: it holds the parameters by the `id` of their attribute, with values from 0 to 1 (`parameters.get()`), in their own units (`parameters.getPlain()`) or as text (`parameters.getDisplayText()`).

### Beyond One Slider

- **Frameworks and packages.** A `webview/` folder with a `package.json` is a web project with a build step: `cargo xtask bundle` runs its build first and embeds `webview/dist/`. The packages in `packages/` give such a project types for `__BEAMER__` (`beamer-api`), unstyled dials, sliders, switches and more (`beamer-controls`) and components for React (`beamer-react`) and Svelte (`beamer-svelte`). They are not on npm yet; inside the repository the examples use them through its bun workspace. A web project of your own joins it when you add its folder (`plugins/my-gain/webview`) to `workspaces` in the root `package.json`. See [Packages for the Web UI](WEBVIEW.md#packages-for-the-web-ui).
- **Examples to start from.** `delay` is plain HTML and JavaScript without a build step. `compressor` and `midi-transform` use React, `drum-synth` uses Svelte. The [examples](../examples/README.md#editors) show each editor.
- **Data from the audio thread.** Meters, levels and notes come to the page from senders the processor writes to without allocating. See [Data from the Audio Thread](WEBVIEW.md#data-from-the-audio-thread).
- **State beyond parameters.** `State` fields of the model (a tab the editor shows, a pan law) are saved with the project and read and set from the page. See [State Methods](WEBVIEW.md#state-methods).
- **Development.** With `BEAMER_DEV_URL` the editor loads the page from a dev server with hot reload instead of the embedded files. Debug builds (bundle without `--release`) have the webview's developer tools: on macOS, right click the page and pick Inspect Element. See [Development Server](WEBVIEW.md#development-server).

## Next Steps

- [Examples](../examples/README.md) - working plugins, each showing a set of features
- [BUILDING.md](BUILDING.md) - formats, options, installing and testing with validators and in hosts
- [REFERENCE.md](REFERENCE.md) - the API: configuration, parameters, traits, MIDI and format notes
- [WEBVIEW.md](WEBVIEW.md) - the editor, the JavaScript API and the packages for web UIs
- [ARCHITECTURE.md](../ARCHITECTURE.md) - design decisions, threading model and guarantees
