# WebView GUI

Beamer provides a platform-native WebView GUI layer for building plugin UIs with standard web technologies (HTML, CSS, JavaScript). The system is designed around a clear separation: the `beamer-webview` crate handles all platform-specific WebView embedding, while the format wrappers (AU and VST3) use it to present the editor and drive parameter synchronization.

Building and bundling plugins with a web UI is covered in [BUILDING.md](BUILDING.md).

## Overview

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Plugin crate (your code)                                       │
│  ┌───────────────────────┐  ┌──────────────────────────────┐    │
│  │  Rust: WebViewHandler │  │  JS: React/Svelte/vanilla    │    │
│  │  on_invoke / on_event │  │  window.__BEAMER__ runtime   │    │
│  └───────────┬───────────┘  └──────────────┬───────────────┘    │
│              │                              │                   │
├──────────────┼──────────────────────────────┼───────────────────┤
│  beamer-core │ WebViewHandler trait         │ beamer_runtime.js │
│              │ ParameterStore               │                   │
├──────────────┼──────────────────────────────┼───────────────────┤
│  Format wrapper (beamer-vst3 / beamer-au)                       │
│  - Creates WebView via beamer-webview                           │
│  - 60 Hz parameter sync timer                                   │
│  - Dispatches IPC messages to WebViewHandler                    │
├─────────────────────────────────────────────────────────────────┤
│  beamer-webview (platform layer)                                │
│  - WKWebView on macOS, WebView2 stub on Windows                 │
│  - Custom URL scheme handler (beamer://localhost/...)           │
│  - Message handler (WKScriptMessageHandler)                     │
└─────────────────────────────────────────────────────────────────┘
```

### Crate Responsibilities

| Crate | Role |
|-------|------|
| `beamer-core` | `WebViewHandler` trait, `WebViewEvents` queue, `EditorSync` (the scripts of the init dump and of each frame), `EmbeddedAssets` types, `params_to_init_json()` |
| `beamer-webview` | Platform WebView creation, custom URL scheme, C-ABI bridge |
| `beamer-vst3` | `WebViewPlugView` (IPlugView impl), 60 Hz sync timer, IPC dispatch |
| `beamer-au` | C-ABI bridge for the generated Objective-C wrappers, which hold the AU view classes, 60 Hz sync timer and IPC dispatch |

## Configuration

GUI settings are declared in `Config.toml`:

```toml
name = "Beamer WebView Demo"
category = "effect"
manufacturer_code = "Bmer"
plugin_code = "wvdm"
has_gui = true
gui_size = [600, 400]
gui_background_color = "#1a1a2e"
```

**GUI Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `has_gui` | Boolean | No | Enable the GUI (default: `false`). Automatically set to `true` when the macro detects a `webview/` directory. |
| `gui_size` | Array | When `has_gui = true` | Initial window size as `[width, height]` in pixels |
| `gui_background_color` | String | No | Hex color (e.g. `"#1a1a2e"`) painted on the parent view while web content loads |

The `#[beamer::export]` macro reads these fields at compile time and generates the appropriate `Config` builder calls. See [Asset Embedding](#asset-embedding) for how the `webview/` directory is detected and embedded.

## JavaScript Runtime API

The Beamer runtime (`beamer_runtime.js`) is injected as a user script before any page content loads. It creates the `window.__BEAMER__` global object, which is the sole interface between JavaScript and the plugin's native code.

### TypeScript Declaration

```typescript
interface BeamerParamInfo {
  id: number;
  stringId: string;
  name: string;
  value: number;
  plainValue: number;
  displayText: string;
  format: string;
  defaultValue: number;
  min: number;
  max: number;
  units: string;
  steps: number;
}

interface BeamerParams {
  get(stringId: string): number;
  getPlain(stringId: string): number;
  getDisplayText(stringId: string): string;
  set(stringId: string, value: number): void;
  beginEdit(stringId: string): void;
  endEdit(stringId: string): void;
  on(stringId: string, callback: (value: number) => void): () => void;
  all(): BeamerParamInfo[];
  info(stringId: string): BeamerParamInfo | undefined;
}

interface Beamer {
  readonly ready: Promise<void>;
  readonly pluginFormat: "vst3" | "auv2" | "auv3" | null;
  readonly params: BeamerParams;
  invoke(method: string, ...args: unknown[]): Promise<unknown>;
  on(event: string, callback: (data: unknown) => void): () => void;
  emit(event: string, data?: unknown): void;
}

declare const __BEAMER__: Beamer;
```

### Parameter Methods

All parameter methods use string identifiers: the `id` from the `#[parameter(id = "...")]` attribute, e.g. `"gain"`. A parameter in a nested struct has the names of the fields before it, e.g. `"filter/cutoff"`.

| Method | Description |
|--------|-------------|
| `params.get(stringId)` | Get the current normalized value (0.0 - 1.0) |
| `params.getPlain(stringId)` | Get the current value in plain units (e.g. dB) |
| `params.getDisplayText(stringId)` | Get the current value as formatted by the parameter, without the unit |
| `params.set(stringId, value)` | Set the normalized value and notify the host |
| `params.beginEdit(stringId)` | Start an automation gesture (call before a drag) |
| `params.endEdit(stringId)` | End an automation gesture (call after a drag) |
| `params.on(stringId, callback)` | Subscribe to value changes. Returns an unsubscribe function. |
| `params.info(stringId)` | Get the full `BeamerParamInfo` object for a parameter |
| `params.all()` | Get an array of all `BeamerParamInfo` objects |

Subscriptions registered before `ready` resolves are queued and attached once the init dump arrives. The unsubscribe function returned by `params.on()` works in both cases.

### Invoke

`invoke()` sends a request to the Rust `WebViewHandler` and returns a Promise:

```javascript
const info = await __BEAMER__.invoke("getInfo");
console.log(info.name);   // "Beamer WebView Demo"
console.log(info.version); // "0.1.0"
```

If the handler returns `Err(message)`, the Promise rejects with that message string.

One method is built in and never reaches the handler: `_beamer/paramTextToNormalized` converts text a user typed into a normalized value, using the parameter's own parser. It takes the numeric parameter ID and the text, and resolves to `null` when the text cannot be parsed:

```javascript
const info = __BEAMER__.params.info("gain");
const normalized = await __BEAMER__.invoke("_beamer/paramTextToNormalized", info.id, "-6");
```

### Custom Events

Events provide fire-and-forget communication in both directions:

```javascript
// JS -> Rust, handled by WebViewHandler::on_event
__BEAMER__.emit("myEvent", { some: "data" });

// Rust -> JS, sent with WebViewEvents::emit
__BEAMER__.on("sampleLoaded", (name) => {
  showSample(name);
});
```

`on()` returns an unsubscribe function. See [Events from Rust](#events-from-rust) for the Rust side.

### Ready Promise

`__BEAMER__.ready` resolves once the initial parameter dump has been received and all parameter subscriptions registered before that point have been attached:

```javascript
await __BEAMER__.ready;
// All params.get() calls now return real values.
```

### Plugin Format

`__BEAMER__.pluginFormat` names the plugin format the editor runs in: `"vst3"`, `"auv2"` or `"auv3"`. The two Audio Unit formats share one plugin library, so only the format wrapper knows which one runs, and it sends the value with the initial parameter dump. It is `null` until `__BEAMER__.ready` resolves:

```javascript
await __BEAMER__.ready;
statusLine.textContent = __BEAMER__.pluginFormat; // "auv3"
```

## Events from Rust

A `WebViewEvents` is a queue of named events for the editor. Keep one as a field of the parameter struct, without an attribute, and return it from `Descriptor::webview_events()`. The framework asks for it once, when the plugin instance is created:

```rust
#[derive(Parameters)]
pub struct WebViewDemoParameters {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,

    pub events: WebViewEvents,
}

impl Descriptor for WebViewDemoDescriptor {
    // ...
    fn webview_events(&self) -> Option<&WebViewEvents> {
        Some(&self.parameters.events)
    }
}
```

Everything that holds the parameters can then send an event. The data can be anything that serializes to JSON:

```rust
self.parameters.events.emit("sampleRate", &48000.0);
```

```javascript
__BEAMER__.on("sampleRate", (rate) => showSampleRate(rate));
```

What to know about events:

- **They are for an open editor.** An event reaches JavaScript on the editor's next sync tick, about 16 ms later. While no editor is open, events are dropped, and so is an event that arrives before the page registered its listener. An editor that opens asks for what it shows with `invoke()`, as the example below does.
- **Not on the audio thread.** `emit()` allocates and takes a lock. It is for host and UI threads and for threads the plugin starts. For data from the audio thread see [Real-Time Visualization](#real-time-visualization).
- **At most 1024 events wait.** When the editor does not take them fast enough, the oldest are dropped. The limit counts events, not bytes, so keep the data of an event small.
- **A listener that throws does not stop the others.** The error goes to the console of the WebView.
- **The method is needed.** A queue that the Descriptor does not return is not known to the framework. The first event emitted into it is reported on standard error and through the `log` crate.

### Example (from `webview-demo`)

The demo shows the sample rate the host runs at. `prepare()` is called on a host thread whenever the host sets up audio. It keeps the rate for an editor that opens later and tells the one that is open:

```rust
fn prepare(&self, setup: SampleRate) -> WebViewDemoProcessor {
    let sample_rate = setup.hz();
    self.parameters.sample_rate.set(sample_rate);
    self.parameters.events.emit("sampleRate", &sample_rate);
    // ...
}
```

The web UI asks once and then follows the event:

```javascript
const unsubscribe = __BEAMER__.on("sampleRate", (rate) => show(rate));
await __BEAMER__.ready;
show(await __BEAMER__.invoke("getSampleRate"));
```

## WebViewHandler Trait

Implement `WebViewHandler` to handle `invoke()` calls and custom events from JavaScript. Parameter synchronization is automatic and does not require this trait.

```rust
pub trait WebViewHandler: Send + Sync {
    fn on_invoke(
        &self,
        _method: &str,
        _args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::Value::Null)
    }

    fn on_event(&self, _name: &str, _data: &serde_json::Value) {}
}
```

Both methods are called on the main thread.

### Example (from `webview-demo`)

The handler of the demo holds the parameters of the plugin. They live in an `Arc` for the life of the instance, and the Processor uses the same ones, so what the web UI sets through the handler is what the audio thread reads. Here the handler reads and sets a `State` field, a setting that is saved with the plugin state without being a parameter (see [State Beyond Parameters](REFERENCE.md#state-beyond-parameters)):

```rust
use beamer::prelude::*;

struct DemoHandler {
    parameters: Arc<WebViewDemoParameters>,
}

impl WebViewHandler for DemoHandler {
    fn on_invoke(
        &self,
        method: &str,
        args: &[serde_json::Value],
    ) -> Result<serde_json::Value, String> {
        match method {
            "getInfo" => Ok(serde_json::json!({
                "name": "Beamer WebView Demo",
                "version": env!("CARGO_PKG_VERSION"),
                "framework": "Beamer",
            })),
            "getPanLaw" => serde_json::to_value(*self.parameters.pan_law.get())
                .map_err(|error| error.to_string()),
            "setPanLaw" => {
                let law = args.first().cloned().ok_or("setPanLaw needs a pan law")?;
                let law: PanLaw = serde_json::from_value(law).map_err(|error| error.to_string())?;
                self.parameters.pan_law.set(law);
                Ok(serde_json::Value::Null)
            }
            _ => Err(format!("unknown method: {method}")),
        }
    }
}
```

The handler is provided by implementing `Descriptor::webview_handler()`. The framework asks for it once, when the plugin instance is created:

```rust
impl Descriptor for WebViewDemoDescriptor {
    // ...
    fn webview_handler(&self) -> Option<Arc<dyn WebViewHandler>> {
        Some(Arc::new(DemoHandler {
            parameters: Arc::clone(&self.parameters),
        }))
    }
}
```

On the JavaScript side the UI asks for the value when it opens and sets it on a click:

```javascript
await __BEAMER__.ready;
const law = await __BEAMER__.invoke("getPanLaw"); // "constant_power" or "linear"
await __BEAMER__.invoke("setPanLaw", "linear");
```

A state that changes behind the UI, for example when the host loads a preset while the editor is open, is not pushed to JavaScript. A UI that has to follow it asks again on a timer, as the demo does. For a value that is expensive to send, the handler can expose `State::generation()`, which counts how often the value was set, and the UI asks for the value only when that number changed.

## Parameter Synchronization

Parameter sync is fully automatic. No plugin code is required.

### Init Dump

When the WebView finishes loading its initial page, the format wrapper calls `params_to_init_json()` and injects it via `evaluateJavaScript`, followed by an object that names the plugin format (see [Plugin Format](#plugin-format)):

```javascript
window.__BEAMER__._onInit([
  {"id":0,"stringId":"gain","name":"Gain","min":-60.0,"max":12.0,
   "defaultValue":0.8333,"value":0.8333,"plainValue":0.0,"displayText":"0.0",
   "format":"db","units":"dB","steps":0},
  {"id":1,"stringId":"pan","name":"Pan","min":-1.0,"max":1.0,
   "defaultValue":0.5,"value":0.5,"plainValue":0.0,"displayText":"C",
   "format":"pan","units":"","steps":0}
], {"pluginFormat":"vst3"})
```

The example is shortened for readability: real numeric IDs are hashes of the string IDs, and values carry full precision.

Each entry corresponds to a `ParamInitEntry`:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `u32` | Numeric parameter ID, a hash of the string ID (used internally for host calls) |
| `stringId` | `string` | String identifier: the `id` of the `#[parameter(id = "...")]` attribute, behind the names of the nested fields it sits in (`"filter/cutoff"`) |
| `name` | `string` | Display name |
| `min` | `f64` | Minimum plain value (`normalized_to_plain(0.0)`) |
| `max` | `f64` | Maximum plain value (`normalized_to_plain(1.0)`) |
| `defaultValue` | `f64` | Default value in normalized form (0.0 - 1.0) |
| `value` | `f64` | Current value in normalized form (0.0 - 1.0) |
| `plainValue` | `f64` | Current value in plain units |
| `displayText` | `string` | Current value as formatted by the parameter, without the unit |
| `format` | `string` | Formatter kind as reported by the parameter. Float and integer parameters report the kind of their formatter: `float` (the default), `db`, `hz`, `ms`, `s`, `percent`, `pan`, `ratio` or `semitones`. Boolean parameters report `boolean` and enum parameters `enum` |
| `units` | `string` | Unit suffix (e.g. `"dB"`, `""`) |
| `steps` | `i32` | Step count (0 for continuous, >0 for discrete) |

The runtime resolves the `ready` promise after processing this dump, so any code awaiting `__BEAMER__.ready` can safely call `params.get()`.

### 60 Hz Polling

Alongside the init dump, a repeating timer fires at 60 Hz on the main thread. It also runs while the host is in a tracking loop (a drag, a resize or an open menu), so the UI keeps following the host. Each tick:

1. Iterates over all parameters
2. Compares each normalized value against a cached snapshot
3. For any parameter whose value changed, appends its normalized value, plain value and display text to a delta object
4. Calls `window.__BEAMER__._onParams({id: [normalized, plain, displayText], ...})` with only the changed entries
5. Delivers the events the plugin emitted since the last tick, one `window.__BEAMER__._onEvent(name, data)` call per event (see [Events from Rust](#events-from-rust))

The delta format is a plain object mapping numeric parameter IDs to those three values:

```javascript
window.__BEAMER__._onParams({0: [0.72, -8.16, "-8.2"], 1: [0.35, -0.3, "L 30"]})
```

The runtime updates its internal map and fires all registered `params.on()` callbacks for the affected parameters. A value that is not a finite number arrives as `null`.

Nothing counts as sent when an editor opens, so its first tick sends all values, covering any changes between the init dump and the first tick.

### JS-to-Rust Flow

When JavaScript calls `params.set(stringId, value)`:

1. The runtime updates its local cache immediately
2. Posts a `{type: "param:set", id, value}` message via `WKScriptMessageHandler`
3. The native message callback calls `ParameterStore::set_normalized()` on the parameter
4. The native callback calls `IComponentHandler::performEdit()` (VST3) or the equivalent AU host notification
5. The native side sends the parameter's authoritative normalized value, plain value and display text straight back through `_onParams`, so the display text updates without waiting for the next poll tick

`beginEdit` and `endEdit` follow the same path, posting `param:begin` and `param:end` messages that map to the host's automation gesture APIs.

## React Integration

The `webview-demo` example demonstrates React integration patterns using two hooks.

### `useBeamerParam`

Bridges the `__BEAMER__.params` subscription model to React's `useSyncExternalStore`:

```typescript
import { useSyncExternalStore } from "react";

export function useBeamerParam(name: string): number {
  return useSyncExternalStore(
    (cb) => __BEAMER__.params.on(name, cb),
    () => __BEAMER__.params.get(name),
  );
}
```

This is a one-liner because the `params.on()` API was designed to match `useSyncExternalStore`'s subscribe/getSnapshot contract: `on()` returns an unsubscribe function, and `get()` returns the current value.

### `useBeamerControl`

Wraps `useBeamerParam` with convenience methods for building knobs, sliders and other controls:

```typescript
interface BeamerControl {
  value: number;           // Current normalized value (0.0 - 1.0)
  displayValue: number;    // Current value in plain units
  displayText: string;     // Current value as formatted by the parameter
  info: BeamerParamInfo | undefined;
  beginEdit: () => void;
  set: (normalized: number) => void;
  endEdit: () => void;
  resetToDefault: () => void;
}

export function useBeamerControl(paramId: string): BeamerControl;
```

Usage in a component:

```tsx
function MyKnob({ paramId }: { paramId: string }) {
  const { value, displayText, info, beginEdit, set, endEdit } =
    useBeamerControl(paramId);

  return (
    <Knob
      value={value}
      label={info?.name}
      displayText={`${displayText} ${info?.units ?? ""}`.trim()}
      onDragStart={beginEdit}
      onDrag={set}
      onDragEnd={endEdit}
    />
  );
}
```

`displayValue` and `displayText` come from the Rust parameter store, so they match what the host shows. The `set()` method clamps the value to `[0, 1]`. The `resetToDefault()` method wraps a full `beginEdit` / `set` / `endEdit` gesture so the host records it as an undoable edit.

## Asset Embedding

The `#[beamer::export]` attribute macro handles web asset detection and embedding at compile time. It scans the plugin crate's `webview/` directory according to these rules:

| Condition | Behavior |
|-----------|----------|
| `BEAMER_DEV_URL` env var set | Load from that URL (dev server mode, see [Development Server](#development-server)) |
| `webview/package.json` exists and `webview/dist/` exists | Embed all files from `webview/dist/` |
| `webview/package.json` exists but no `dist/` | No assets. `cargo xtask bundle` runs the web build first, so this only happens with a plain `cargo build` (run the web build in `webview/` first) |
| `webview/` exists (no `package.json`) | Embed all files from `webview/` (plain HTML project) |
| No `webview/` directory | No GUI |

The scan skips dotfiles, `node_modules` directories and `.map` source map files.

Each discovered file is embedded via `include_bytes!()` into a static `EmbeddedAssets` table:

```rust
// Generated by #[beamer::export]
static WEBVIEW_ASSETS: EmbeddedAssets = EmbeddedAssets::new(&[
    EmbeddedAsset { path: "index.html", data: include_bytes!("...") },
    EmbeddedAsset { path: "assets/index-abc123.js", data: include_bytes!("...") },
    EmbeddedAsset { path: "assets/index-def456.css", data: include_bytes!("...") },
]);
```

### Custom URL Scheme

When embedded assets are available, the WebView navigates to `beamer://localhost/index.html`. A per-plugin `WKURLSchemeHandler` intercepts requests to the `beamer://` scheme and serves files from the `EmbeddedAssets` table.

The Objective-C classes behind the WebView (the scheme handler, the message handler and the navigation delegate) are registered at runtime under names that are unique to the plugin binary, so multiple Beamer plugins loaded in the same host process do not share them. Each handler instance stores its own assets pointer as an ObjC ivar, avoiding any global state.

Path lookup uses `NSURL::path()` to get the decoded path component, stripping query strings, fragments and percent-encoding. The MIME type is inferred from the file extension.

### Rebuild Considerations

Directory scanning runs at macro expansion time, so its results are cached by the Rust compiler. After running a web build for the first time, you may need to `touch` a `.rs` file to trigger recompilation. Changes to existing file contents are tracked automatically by `include_bytes!()`.

## Development Server

Set the `BEAMER_DEV_URL` environment variable to bypass asset embedding and load the UI from a dev server:

```bash
BEAMER_DEV_URL=http://localhost:5173 cargo xtask bundle webview-demo --vst3
```

When this variable is set:

1. The macro generates `Config::with_gui_url("http://localhost:5173")` instead of `with_gui_assets(...)`
2. The WebView navigates directly to the URL instead of `beamer://localhost/`
3. The custom scheme handler is not registered

This enables hot-reload workflows during development. The Beamer runtime (`beamer_runtime.js`) is still injected as a user script, so all `__BEAMER__` APIs work the same way regardless of asset source.

Developer tools (Web Inspector on macOS) are enabled in debug builds automatically via `WebViewConfig::dev_tools`.

## Background Color

The WKWebView's default background is disabled (`drawsBackground = false`), making it fully transparent. Whatever is behind the webview is visible until web content covers it. In some DAWs, the view behind the webview has a white background, causing a visible flash before your UI renders.

To prevent this, set `gui_background_color` in `Config.toml`:

```toml
gui_background_color = "#1a1a2e"
```

The macro parses the hex string into `[R, G, B, A]` bytes (alpha is always 255). At runtime, this color is painted on the parent `NSView`'s `CALayer` before the WKWebView is created. It stays visible through the transparent webview until your web content renders its own background on top.

This is independent from any background set in HTML, CSS or JS. You can use either approach, or both.

## Status and Planned Work

### Real-Time Visualization

Lock-free data transfer from the audio thread to the WebView for spectrum analyzers, level meters, waveform displays and other real-time visualizations.

**Problem:** `WebViewEvents::emit()` allocates (JSON serialization) and takes a lock, and is therefore not safe to call from the audio thread.

**Planned approach:**

- A lock-free ring buffer in `beamer-core` for fixed-size visualization payloads
- An `emit_rt()` method that writes into the ring buffer without allocating, on an audio end of the event queue that the Descriptor creates in `prepare()`
- The 60 Hz sync timer drains the ring buffer on the main thread, next to the events it already delivers

This is a core-level feature (not webview-specific) because the ring buffer and thread-safety guarantees belong in `beamer-core`.

**Status:** Not implemented. Events from host and UI threads work (see [Events from Rust](#events-from-rust)).

### Windows WebView2 Support

The Windows platform backend (`beamer-webview/src/platform/windows.rs`) is a stub that returns `WebViewError::PlatformNotSupported`. All WebView functionality currently targets macOS only.

A future implementation would use Microsoft's WebView2 (Edge/Chromium) control via either the `webview2` crate or direct COM interop. The `WebViewConfig` struct and C-ABI bridge are already platform-agnostic, so the main work is implementing `WindowsWebView::attach_to_parent()` and the corresponding scheme handler.
