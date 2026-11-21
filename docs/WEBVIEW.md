# WebView Editor

Beamer provides a platform-native WebView layer for building plugin editors with standard web technologies (HTML, CSS, JavaScript). The system is designed around a clear separation: the `beamer-webview` crate holds both ends of the protocol between the plugin and its web UI, keeps each open editor in step and embeds the platform-specific WebView, while the format wrappers (AU and VST3) use it to present the editor and tell the host what the web UI edits.

Two words in this document mean different things. The **editor** is the plugin's own view, which a host opens and closes in a window of its own; it is never the generic parameter view a host builds for a plugin without one. The **web UI** is the page in it and its code, the files in `webview/`. Every editor of a Beamer plugin shows a web UI.

Building and bundling plugins with a web UI is covered in [BUILDING.md](BUILDING.md).

## Overview

### Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Plugin crate (your code)                                       │
│  ┌───────────────────────┐  ┌──────────────────────────────┐    │
│  │  Rust: Model          │  │  JS: React/Svelte/vanilla    │    │
│  │  WebViewHandler       │  │  window.__BEAMER__ runtime   │    │
│  └───────────┬───────────┘  └───────────────┬──────────────┘    │
│              │                              │                   │
├──────────────┼──────────────────────────────┼───────────────────┤
│  Format wrapper (beamer-vst3 / beamer-au)                       │
│  - Creates WebView via beamer-webview                           │
│  - 60 Hz parameter sync timer                                   │
│  - Hands the page's messages to the EditorSync                  │
│  - Tells the host what the web UI edits (HostEdits)             │
├─────────────────────────────────────────────────────────────────┤
│  beamer-webview                                                 │
│  - Both ends of the protocol: beamer_runtime.js (the            │
│    window.__BEAMER__ runtime), WebViewDispatch, EditorSync,     │
│    the event queue and its senders, the editor's notes          │
│  - WKWebView on macOS, WebView2 stub on Windows                 │
│  - Custom URL scheme handler (beamer://localhost/...)           │
│  - Message handler (WKScriptMessageHandler)                     │
└─────────────────────────────────────────────────────────────────┘
```

### Crate Responsibilities

| Crate | Role |
|-------|------|
| `beamer-core` | The editor settings of `Config` and the `EmbeddedAssets` types: which editor a plugin has |
| `beamer-webview` | Both ends of the protocol: the injected `beamer_runtime.js`, `WebViewHandler` (the model's answers to `invoke()` calls and events), the `WebViewDispatch` that `#[beamer::model]` generates for the wrappers to call it through, `WebViewEventQueue`, the declared fields `WebViewValue`, `WebViewStream` and `WebViewEvent`, `WebViewValueSender` and `WebViewStreamSender` (the senders for the audio thread), `WebViewFields` (where the model's queue is, the connection of its fields, the model's types), `TypeScript` and `ModelTypes` (the page's model types), `EditorSync` (the scripts of the init dump and of each frame, the messages of the page), `EditorNotes` (the notes the page plays, on their way to the render), `parameters_to_init_json()`. Also platform WebView creation, custom URL scheme and C-ABI bridge |
| `beamer-vst3` | `WebViewPlugView` (IPlugView impl), 60 Hz sync timer, `HostEdits` through the `IComponentHandler` |
| `beamer-au` | C-ABI bridge for the generated Objective-C wrappers, which hold the AU view classes, 60 Hz sync timer and the host side of `HostEdits` (`BeamerAuEditorHost`) |

## Configuration

Editor settings are declared in `Config.toml`:

```toml
name = "Beamer WebView Demo"
category = "effect"
manufacturer_code = "Bmer"
plugin_code = "wvdm"
editor_size = [600, 400]
editor_background_color = "#1a1a2e"
```

**Editor Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `editor_size` | Array | With a web UI | Initial editor size as `[width, height]` in pixels. A VST3 host cannot make the editor smaller; no format has a maximum. A plugin with a web UI and no size is a compile error; without a web UI the size is ignored |
| `editor_background_color` | String | No | Hex color (e.g. `"#1a1a2e"`) painted on the parent view while web content loads |

A plugin has an editor when it has a web UI: the `#[beamer::export]` macro finds one in `webview/` or at `BEAMER_DEV_URL` (see [Asset Embedding](#asset-embedding)). There is no setting to turn the editor on. The macro reads these fields at compile time and generates the appropriate `Config` builder calls.

When a host resizes the editor, the WebView fills it in every format, so the page sees the new window size. How the layout uses it is up to the page. The `compressor` example scales its whole design with the window: it sets the root font size from the window (`font-size: min(100vw / 47.5, 100vh / 26.75)` for a 760 x 428 design) and gives its sizes in `rem`. A tall window gets a portrait layout of its own, with a second size under `@media (orientation: portrait)`.

## JavaScript Runtime API

The Beamer runtime (`beamer_runtime.js`) is injected as a user script before any page content loads. It creates the `window.__BEAMER__` global object, which is the sole interface between JavaScript and the plugin's native code.

### TypeScript Declaration

`@helpermedia/beamer-api` (see [Packages for the Web UI](#packages-for-the-web-ui)) declares these types. The ids and values follow its `ModelTypes` interface, which a page augments with the model's parameter ids, `State` paths, sender names, events and methods. A section the page does not declare stays loose: any name is accepted and values are `unknown`. Events and stream senders share their names, so a page that declares one of the two gets the other as empty. Without any declaration, `ParameterId`, `StatePath`, `ValueSenderName`, `PluginEventName`, `PageEventName` and `MethodName` are plain strings.

A plugin with a `webview/package.json` gets the declaration generated from its model, in `webview/src/model-types.ts` (see [Model Types](#model-types)).

```typescript
interface BeamerParameterInfo {
  id: number;
  stringId: ParameterId;
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
  valueTexts: string[];
}

interface BeamerParameters {
  get(stringId: ParameterId): number;
  getPlain(stringId: ParameterId): number;
  getDisplayText(stringId: ParameterId): string;
  set(stringId: ParameterId, value: number): void;
  beginEdit(stringId: ParameterId): void;
  endEdit(stringId: ParameterId): void;
  on(stringId: ParameterId, callback: (value: number) => void): () => void;
  all(): BeamerParameterInfo[];
  info(stringId: ParameterId): BeamerParameterInfo | undefined;
}

interface BeamerState {
  get<P extends StatePath>(path: P): StateValue<P> | undefined;
  set<P extends StatePath>(path: P, value: StateValue<P>): void;
  on<P extends StatePath>(path: P, callback: (value: StateValue<P>) => void): () => void;
  ids(): StatePath[];
}

interface BeamerValues {
  get<N extends ValueSenderName>(name: N): SenderValue<N> | undefined;
  on<N extends ValueSenderName>(name: N, callback: (value: SenderValue<N>) => void): () => void;
}

interface Beamer {
  readonly ready: Promise<void>;
  readonly pluginFormat: "vst3" | "auv2" | "auv3" | null;
  readonly middleC: "C3" | "C4" | null;
  readonly parameters: BeamerParameters;
  readonly state: BeamerState;
  readonly values: BeamerValues;
  invoke<M extends MethodName>(method: M, ...args: MethodArguments<M>): Promise<MethodResult<M>>;
  on<N extends PluginEventName>(event: N, callback: (data: PluginEventData<N>) => void): () => void;
  emit<N extends PageEventName>(event: N, ...data: PageEventArguments<N>): void;
  noteOn(note: number, velocity?: number, channel?: number): void;
  noteOff(note: number, velocity?: number, channel?: number): void;
  allNotesOff(): void;
}

declare const __BEAMER__: Beamer;
```

### Parameter Methods

All parameter methods use string identifiers: the `id` from the `#[parameter(id = "...")]` attribute, e.g. `"gain"`. A parameter in a nested group has the names of the fields before it, e.g. `"filter/cutoff"`.

| Method | Description |
|--------|-------------|
| `parameters.get(stringId)` | Get the current normalized value (0.0 - 1.0) |
| `parameters.getPlain(stringId)` | Get the current value in plain units (e.g. dB) |
| `parameters.getDisplayText(stringId)` | Get the current value as formatted by the parameter, without the unit |
| `parameters.set(stringId, value)` | Set the normalized value and notify the host |
| `parameters.beginEdit(stringId)` | Start an automation gesture (call before a drag) |
| `parameters.endEdit(stringId)` | End an automation gesture (call after a drag) |
| `parameters.on(stringId, callback)` | Subscribe to value changes. Returns an unsubscribe function. |
| `parameters.info(stringId)` | Get the full `BeamerParameterInfo` object for a parameter |
| `parameters.all()` | Get an array of all `BeamerParameterInfo` objects |

Subscriptions registered before `ready` resolves are queued, attached once the init dump arrives and called with the value it brings. The unsubscribe function returned by `parameters.on()` works in both cases.

### State Methods

The `State` fields of the model (see [State Beyond Parameters](REFERENCE.md#state-beyond-parameters)) are known by their path, like parameters: the `id` from the `#[state(id = "...")]` attribute, behind the names of the nested fields it sits in (`"pan_law"`, `"layer/sample"`). A value is what JSON makes of the Rust value: a string for a unit enum variant with serde's naming, an object for a struct.

| Method | Description |
|--------|-------------|
| `state.get(path)` | Get the value the UI knows. The same object until the value changes, so it works with React's `useSyncExternalStore`. `undefined` before `ready` and for a path the plugin does not have |
| `state.set(path, value)` | Set the value. Listeners hear of it at once. The plugin answers with the value in effect: a value that does not fit the Rust type leaves the field as it is and the answer sets the UI back. A path the plugin does not have is ignored |
| `state.on(path, callback)` | Subscribe to changes, also those behind the UI's back (a loaded project, a handler that sets the field). Returns an unsubscribe function |
| `state.ids()` | Get the paths of all `State` fields. Empty before `ready` |

The value's type comes from the page's `ModelTypes` (see [Model Types](#model-types)); without it the value is `unknown` and the page checks its shape. Listeners are called only when the value differs from the one the UI has, compared as JSON with the keys of objects sorted. Subscriptions before `ready` are queued as with parameters.

Every `State` field is visible to the web UI and can be set by it. Keep data that the UI must not see or change in a `Handover` field instead. A large value (a wavetable, a sample) is turned into JSON and sent with every change; above 1 MB the plugin logs a warning, once per open editor.

### Value Methods

The values that the plugin's value senders send from the audio thread (see [Data from the Audio Thread](#data-from-the-audio-thread)) are known by the name the sender was created with. A value is what JSON makes of the Rust value.

| Method | Description |
|--------|-------------|
| `values.get(name)` | Get the last value. The same object until a new one arrives, so it works with React's `useSyncExternalStore`. `undefined` before the first |
| `values.on(name, callback)` | Subscribe to the values that arrive from now on. The one that arrived already is `values.get(name)`. Returns an unsubscribe function |

The value's type comes from the page's `ModelTypes`, as for `State` fields; without it the value is `unknown`.

### Invoke

`invoke()` calls a method of the model (see [Answering the Web UI](#answering-the-web-ui)) and returns a Promise:

```javascript
const info = await __BEAMER__.invoke("info");
console.log(info.name);   // "Beamer WebView Demo"
console.log(info.version); // The version from Cargo.toml
```

A call does not wait for `ready`: the plugin answers it while the parameters are still on their way, so a page can ask for what it shows as soon as it starts. The demo asks for its version and the sample rate that way.

If the method returns `Err(error)`, the Promise rejects with the error's text. A method the plugin does not know rejects too, with `"unknown method: ..."`, as do arguments that do not fit the method. With the page's generated model types, a name or an argument that does not fit is a compile error of the page instead.

One method is built in and never reaches the model: `_beamer/parameterTextToNormalized` converts text a user typed into a normalized value, using the parameter's own parser. It takes the numeric parameter ID and the text. It resolves to `null` when the text cannot be parsed:

```javascript
const info = __BEAMER__.parameters.info("gain");
const normalized = await __BEAMER__.invoke("_beamer/parameterTextToNormalized", info.id, "-6");
```

### Custom Events

Events provide fire-and-forget communication in both directions:

```javascript
// JS to Rust, handled by a #[page_event] method of the model
__BEAMER__.emit("padHit", { pad: 0, velocity: 0.8 });

// Rust to JS, sent with WebViewEvent::emit
__BEAMER__.on("sampleLoaded", (name) => {
  showSample(name);
});
```

`on()` returns an unsubscribe function. See [Events from Rust](#events-from-rust) for the Rust side. The arrays of stream senders from the audio thread arrive the same way. The values of value senders are in `__BEAMER__.values` (see [Data from the Audio Thread](#data-from-the-audio-thread)).

### Notes for the Processor

A page plays notes in the processor: drum pads that sound when clicked, an on-screen keyboard. The processor gets them with the host's MIDI input, so an instrument plays them with no code of its own:

```javascript
// A pad that sounds while it is held. The capture sends the pointerup
// here also when the pointer leaves the pad.
pad.addEventListener("pointerdown", (event) => {
  pad.setPointerCapture(event.pointerId);
  __BEAMER__.noteOn(36, 0.8);
});
pad.addEventListener("pointerup", () => __BEAMER__.noteOff(36));
pad.addEventListener("pointercancel", () => __BEAMER__.noteOff(36));
```

| Method | Description |
|--------|-------------|
| `noteOn(note, velocity?, channel?)` | Start a note: a note number 0 to 127, a velocity 0 to 1 (default 1) and a channel 0 to 15 (default 0). A velocity of 0 is the smallest, not a note-off, as in MIDI 2.0 |
| `noteOff(note, velocity?, channel?)` | End a note, with a release velocity 0 to 1 (default 64/127, MIDI's) |
| `allNotesOff()` | End every note the page started |

A note number or channel that does not fit is dropped with a warning in the console, as is a velocity that is not a number; a velocity is clamped to 0 to 1. `playNote()` of `beamer-api` plays a short note for a click that does not hold (see [Packages for the Web UI](#packages-for-the-web-ui)).

What the processor gets:

- **The notes are MIDI input.** `process_midi()` gets them and [`ProcessContext::midi_input()`](REFERENCE.md#midi-reference) holds them, at offset 0 of the block that takes them, after the host's events at that offset. A MIDI effect transforms them as the host's notes and a plugin with MIDI output passes them on. They are not sent to the host on their own; a host does not record them. Only a plugin with `midi_input` takes them: for any other one the page's notes are ignored and the plugin logs that once.
- **Note IDs of their own.** A note of the editor has the ID `editor_note_id(channel, pitch)`, from -10000 up: VST3 keeps -10000 to -1000 for a plugin's own notes, which no host uses. A plugin that finds its voices by note ID keeps a note of the editor and one of the host apart, also at the same pitch.
- **MIDI 1.0 and 2.0 alike.** They are added after the host's MIDI was read, in either protocol, as the same events.

No hanging notes:

- The plugin knows which notes of the editor are on, also through a reset of the processor. A note-off for a note that is not on is dropped; a note-on for a note that is on ends it first.
- When the page goes away (the editor closes, the page loads again, its process ends), the plugin ends the notes it left on, as it ends the gestures (see [When the Page Loads Again](#when-the-page-loads-again)).

When the host does not call the plugin:

- The notes wait in a queue of 257 requests between the main thread and the audio thread, which takes them without a lock or allocation. A note-on goes in only while fewer than 128 wait, so that the note-off of each fits; the last place is for the release when the page goes away. A note-off that does not fit (for a note held from before the host stopped calling) is owed: it goes in with the default release velocity once there is room, checked with each note and each frame.
- A note-on that waited longer than 500 ms is dropped when the audio thread takes it, with its note-off: a click sounds now or not at all, not in a burst when the host calls again.
- An offline render plays no note of the editor. Nor does a render that processes nothing, such as an Audio Unit the host bypasses (a bypassed MIDI effect passes them on): the notes wait for the next processed block.

Hosts process a track ahead of time in large blocks when it is not played live: Logic for a track that is not selected, Cubase with ASIO-Guard, Reaper with anticipative FX. A note from the editor of such a track sounds later than one of a selected or armed track.

### Ready Promise

`__BEAMER__.ready` resolves once the init dump (parameters and `State` values) has been received and all parameter and `state.on()` subscriptions registered before that point have been attached:

```javascript
await __BEAMER__.ready;
// All parameters.get() calls now return real values.
```

### Plugin Format

`__BEAMER__.pluginFormat` names the plugin format the editor runs in: `"vst3"`, `"auv2"` or `"auv3"`. The two Audio Unit formats share one plugin library, so only the format wrapper knows which one runs. It sends the value with the initial parameter dump. It is `null` until `__BEAMER__.ready` resolves:

```javascript
await __BEAMER__.ready;
statusLine.textContent = __BEAMER__.pluginFormat; // "auv3"
```

### Middle C

`__BEAMER__.middleC` is the name of MIDI note 60 in the plugin's note names: `"C3"` (the default) or `"C4"`, from `middle_c` in `Config.toml`. Parameters with `kind = "note_number"` name notes with it, so a web UI that names notes itself (a keyboard, a list of played notes) should too. It arrives with the initial parameter dump and is `null` until `__BEAMER__.ready` resolves. `noteName()` of `beamer-api` names a note number with it:

```javascript
import { noteName } from "@helpermedia/beamer-api";

noteName(60); // "C3", or "C4" for a plugin with middle_c = "C4"
```

## Events from Rust

An event is a field of the model with `#[event(id = "...")]`, a `WebViewEvent<T>` typed by its data. The id is the name the page knows it by; in a nested group the group's field name comes in front (`layer/loaded`). The events and senders of a model share one namespace. They deliver through a `WebViewEventQueue`, which the model keeps as a field without an attribute. `#[beamer::model]` finds the queue by its type, also in a nested group. It connects the declared events and senders to it when it assigns the ids. The framework takes it when the plugin instance is created:

```rust
#[beamer::export]
#[beamer::model]
pub struct WebViewDemoModel {
    #[parameter(id = "gain", name = "Gain", default = 0.0, range = -60.0..=12.0, kind = "db")]
    pub gain: FloatParameter,

    /// The sample rate, whenever the host prepares the plugin.
    #[event(id = "sampleRate")]
    pub sample_rate_event: WebViewEvent<f64>,

    pub events: WebViewEventQueue,
}
```

The model's methods can then send the event. So can every other thread that holds the model's `Arc`, except the audio thread. A value of the wrong type does not compile:

```rust
// In a method of the model, such as prepare(), or in its handler.
self.sample_rate_event.emit(&48000.0);
```

```javascript
__BEAMER__.on("sampleRate", (rate) => showSampleRate(rate));
```

What to know about events:

- **They are for an open editor.** An event reaches JavaScript on the editor's next sync tick, about 16 ms later. While no editor is open, events are dropped. So is an event that arrives before the page registered its listener. An editor that opens asks for what it shows with `invoke()`, as the example below does.
- **Not on the audio thread.** `emit()` allocates and takes a lock. It is for host and UI threads and for threads the plugin starts. In debug builds, a call from `process()` panics. For data from the audio thread see [Data from the Audio Thread](#data-from-the-audio-thread).
- **At most 1024 events wait.** When the editor does not take them fast enough, the oldest are dropped. The limit counts events, not bytes, so keep the data of an event small.
- **A listener that throws does not stop the others.** The error goes to the console of the WebView.
- **One queue per plugin.** Two `WebViewEventQueue` fields in one struct are a compile error. A queue of the model itself is the plugin's, also when a nested group with a queue comes before it in field order. Without one, the first nested group in field order that has a queue provides it. The other queues are not delivered. The attribute knows the field by the name of its type, so a type alias, a renamed import or a wrapper such as `Option` hides it. A queue the framework does not know reports so once, at the first event emitted into it or the first sender created from it, on standard error and through the `log` crate. A hand-written model or group (without `#[beamer::model]` or `#[beamer::group]`) returns its queue from `WebViewFields::webview_queue()` and connects its declared fields in `WebViewFields::connect_webview()`.

### Example (from `webview-demo`)

The demo shows the sample rate the host runs at. `prepare()` is called on a host thread whenever the host sets up audio. It keeps the rate for an editor that opens later and tells the one that is open:

```rust
fn prepare(self: Arc<Self>, setup: SampleRate) -> WebViewDemoProcessor {
    let sample_rate = setup.hz();
    self.sample_rate.set(sample_rate);
    self.sample_rate_event.emit(&sample_rate);
    // ...
}
```

The web UI asks once and then follows the event:

```javascript
const unsubscribe = __BEAMER__.on("sampleRate", (rate) => show(rate));
show(await __BEAMER__.invoke("sampleRate"));
```

## Data from the Audio Thread

The audio thread cannot emit: `emit()` serializes and takes a lock. It sends through a sender instead. The model declares it as a field, `#[value(id = "...")]` on a `WebViewValue<T>` or `#[stream(id = "...")]` on a `WebViewStream<T>`; `prepare()` makes the sender from that field and moves it into the processor. Sending never waits, allocates or frees memory. There are two kinds:

| | `WebViewValueSender<T>` | `WebViewStreamSender<T>` |
|---|---|---|
| For | A value the web UI shows as it is now: a meter, a gain reduction, a spectrum | Values the web UI needs all of, in order: a scrolling history, a scope, the notes played |
| Declared as | `#[value(id = "...")]` on a `WebViewValue<T>` | `#[stream(id = "...")]` on a `WebViewStream<T>` |
| Created with | `field.sender(initial)` in `prepare()` | `field.sender(capacity)` in `prepare()` |
| Sent with | `send(&value)` or `send_with(\|value\| ...)` | `push(value)` or `push_slice(&values)` |
| JavaScript reads it with | `__BEAMER__.values.get(name)` and `values.on(name, callback)` | `__BEAMER__.on(name, callback)`, like the events of `emit()` |
| JavaScript receives | The newest value, at most once per tick | An array of every value since the last tick, oldest first |
| Several values between two ticks | The last one counts | All of them, up to `capacity` |
| While no editor is open | Kept: an editor that opens gets the current value | Dropped |
| A part of the page that shows up later | Reads the current value with `values.get(name)` | Gets the values from the next tick on |
| Type | `Clone + Serialize`, such as a struct or a `Vec<f32>` | `Copy + Serialize`, such as `f32` or a small struct |

A value sender's value stays in the runtime, as parameters and `State` fields do, so a component reads it during render. The arrays of a stream sender are events: a listener gets what comes after it registered. `WebViewEventQueue::is_editor_open()` tells the audio thread whether an editor is open at all: measuring costs time, so a plugin skips it while nothing shows the result. It is one atomic load, also fine in `process()`.

### Example (from `webview-demo`)

The demo shows an output meter (a value sender) and a scrolling level history (a stream sender):

```rust
/// The output level as the meter shows it.
#[derive(Debug, Default, Clone, Copy, Serialize, TypeScript)]
#[serde(crate = "beamer::serde")]
pub struct Levels {
    /// The held peak of each channel, linear.
    pub peak: [f32; 2],
}

// In the model:
#[value(id = "levels")]
pub levels: WebViewValue<Levels>,

#[stream(id = "history")]
pub history: WebViewStream<f32>,

fn prepare(self: Arc<Self>, setup: SampleRate) -> WebViewDemoProcessor {
    // ...
    WebViewDemoProcessor {
        // The meter shows the newest level only.
        levels: self.levels.sender(Levels::default()),
        // The history needs every point: 100 a second, room for 640 ms.
        history: self.history.sender(64),
        // ...
        model: self,
    }
}

impl Processor for WebViewDemoProcessor {
    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
        // ...
        // Measuring costs time, so only while the editor is open.
        if self.model.events.is_editor_open() {
            self.measure(buffer);
        } else if self.peaks != [0.0; 2] {
            // The meter comes to rest, so that an editor that opens later does
            // not show a peak from before.
            self.peaks = [0.0; 2];
            self.levels.send(&Levels { peak: self.peaks });
        }
    }
}

// In measure(): the held peaks, sent with every block while the meter
// shows something. A meter at rest sends nothing.
if self.peaks != before || self.peaks != [0.0; 2] {
    self.levels.send(&Levels { peak: self.peaks });
}
// One point of the history per 10 ms window.
self.history.push(self.window_peak);
```

```javascript
// A value sender: the runtime keeps the newest value. Read it, then
// listen to the ones that follow.
meter.show(__BEAMER__.values.get("levels")?.peak);
__BEAMER__.values.on("levels", ({ peak }) => meter.show(peak));
// In React, read it with useSenderValue (see Packages for the Web UI).
const levels = useSenderValue("levels");
// A stream sender: an array with every value since the last tick.
__BEAMER__.on("history", (points) => history.append(points));
```

What to know about senders:

- **Created in `prepare()`, dropped with the processor.** Creating a sender takes a lock and allocates: in debug builds a call from `process()` panics. Dropping one frees memory, which is fine because the framework drops the processor on a host thread. When the host prepares the plugin again, the senders of the new processor take over. A sender of a queue the framework does not deliver (see "One queue per plugin" above) reports it once, when it is created.
- **A value sender delivers what it is sent.** A tick sends the newest value when anything was sent since the last tick, also a value equal to the one before. A meter at rest should send nothing, so that the web UI has nothing to do. After a page loads, each value sender sends its current value again, so an editor that opens shows it at once. A plugin that measures only while an editor is open therefore sends a value at rest when none is, as the demo does, so that the next editor does not show a level from before. The runtime keeps the last value of each value sender in `__BEAMER__.values`, so a part of the page that mounts later (a tab, a panel) reads it at once.
- **A value sender keeps the last value only.** What happened between two ticks is lost unless the plugin holds it: a meter holds its peaks with a decay instead of sending the peak of each block. `send()` copies with `clone_from`, which does not allocate for a `Vec` of the same length as the initial value. `send_with()` fills the value in place; what the closure finds there is an older value, so it has to write the whole value.
- **A stream needs room for two ticks or more.** A tick comes about every 16 ms. When the ring is full, the newest values are dropped: `push()` returns `false`, `push_slice()` returns how many fit. Room for a few hundred milliseconds lets an editor whose main thread is busy for a moment (a project that loads, a heavy redraw) lose nothing.
- **A stream starts with the editor.** A push while no editor is open is dropped. When the first editor opens, values left from before are skipped.
- **JSON as for `emit()`.** Values that are not finite numbers (NaN, infinity) arrive as `null`, so send a level as a linear value and convert it to dB in JavaScript. An integer beyond 2^53 arrives rounded. The page's model types promise a `number` for both, so debug builds report the first such number of each sender and event once, with its place in the value (`-inf at .peak[1]`), on standard error and through the `log` crate. A `State` field reports a float that is not finite when it is saved. serde serializes arrays of up to 32 elements (a longer one does not compile), so a spectrum is a `Vec<f32>` in a value sender.
- **One name per sender.** A name is one thing in the web UI: an event, a value or a stream. In debug builds a second sender with the name of a living one panics, as does an `emit()` with the name of a living sender.
- **One editor.** With two windows of one plugin instance open, each event, value and stream value reaches one of them. The hosts tried open one window per instance.

## Answering the Web UI

A model answers `invoke()` calls and the events the page emits with methods in an impl block marked `#[beamer::handler]`. Parameter and `State` synchronization are automatic and do not go through them.

```rust
#[beamer::handler]
impl SamplerModel {
    /// The name of the loaded sample.
    #[method(id = "sampleName")]
    fn sample_name(&self) -> String {
        (*self.sample_name.get()).clone()
    }

    /// Load the sample at `path`.
    #[method(id = "loadSample")]
    fn load_sample(&self, path: String) -> Result<(), String> {
        self.loader.load(&path)
    }

    /// A pad the page played.
    #[page_event(id = "padHit")]
    fn pad_hit(&self, hit: PadHit) {
        self.pads.push(hit);
    }
}
```

```javascript
const name = await __BEAMER__.invoke("sampleName");
await __BEAMER__.invoke("loadSample", "/samples/kick.wav");
__BEAMER__.emit("padHit", { pad: 0, velocity: 0.8 });
```

- **`#[method(id = "...")]`:** the page calls it with `invoke("id", ...arguments)`. The arguments after `&self` are the positional arguments of `invoke()`, read with serde into owned values (`String`, not `&str`). What the method returns is what the Promise resolves to (`null` for nothing). A `Result` rejects the Promise with the error's text.
- **`#[page_event(id = "...")]`:** the page sends it with `emit("id", data)`. The one argument after `&self` is the data, none for an event without data. It returns nothing: the page does not wait. Data that does not read is logged and dropped.
- **The ids** are written as the page writes them, camelCase as `sampleRate`, so a search finds both ends. A method without an attribute is a helper of the block that the page cannot reach.
- **The types** of the arguments, results and data go into the page's model types (see [Model Types](#model-types)), so `invoke()` and `emit()` are checked like every other name: each type needs `#[derive(TypeScript)]` next to `Serialize` and `Deserialize`.

The attribute generates `WebViewHandler`. The format wrappers reach it through the `WebViewDispatch` that `#[beamer::model]` generates. A model without a handler rejects every call as an unknown method and ignores the events. A handler can also be written by hand: `on_invoke(method, args)` and `on_event(name, data)` match on the names. Its methods are not in the page's model types, so the page's `invoke()` and `emit()` stay loose. A hand-written model (without `#[beamer::model]`) adds an empty `impl WebViewDispatch for MyModel {}`, which rejects every call, or forwards its methods to its handler. The handler of a generic model holds for every type the struct takes, with no bounds beyond the struct's own: the dispatch does not find an impl for one type only.

The methods are called on the main thread. The model lives as long as the plugin instance and holds the parameters and `State` fields, so its methods can answer with anything the plugin knows. Host threads may use the model at the same time, so what these methods change is a value that is safe to share: a `State` field, a `Handover`, an atomic or a `Mutex`. The model cannot reach the processor. Data the audio thread produces for the web UI goes through a sender (see [Data from the Audio Thread](#data-from-the-audio-thread)). Data it produces for the model goes through a value the processor writes without locking, such as an atomic.

### Example (from `webview-demo`)

Here the model answers with its name and version and with the sample rate the host runs at, which the plugin keeps in a `Handover` field: it is neither saved nor a parameter, so the web UI asks for it.

```rust
/// What the editor's About box shows.
#[derive(Debug, Clone, Copy, Serialize, TypeScript)]
#[serde(crate = "beamer::serde")]
pub struct Info {
    pub name: &'static str,
    pub version: &'static str,
    pub framework: &'static str,
}

#[beamer::handler]
impl WebViewDemoModel {
    /// The plugin's name and version, for the About box.
    #[method(id = "info")]
    fn info(&self) -> Info {
        Info {
            name: "Beamer WebView Demo",
            version: env!("CARGO_PKG_VERSION"),
            framework: "Beamer",
        }
    }

    /// The sample rate of the host, 0 before it prepared the plugin.
    #[method(id = "sampleRate")]
    fn sample_rate(&self) -> f64 {
        *self.sample_rate.get()
    }
}
```

On the JavaScript side the UI asks for the value when it opens:

```javascript
const rate = await __BEAMER__.invoke("sampleRate");
```

The demo then follows the sample rate through an event of the same name (see [Events from Rust](#events-from-rust)).

A `State` field needs no method at all: the web UI reads, sets and follows it through `__BEAMER__.state` (see [State Methods](#state-methods)), also when it changes behind the UI's back, for example when the host loads a project while the editor is open.

## Model Types

A page built from a `webview/package.json` is checked against its plugin's model: each id it uses is one the model declares and each value has the model's type. A typo in an id or a value of the wrong shape is a compile error of the page, not a silent `undefined` in a host.

### The Generated File

`webview/src/model-types.ts` is written from the model's Rust code. The one of `webview-demo`, shortened:

```typescript
// Generated from the plugin's model. Do not edit: `cargo test` fails
// while this file differs from the model. `cargo xtask bundle` writes
// it, or on its own:
//
//   cargo xtask model-types webview-demo

/** The output level as the meter shows it. */
export interface Levels {
  /** The held peak of each channel, linear. */
  peak: [number, number];
}

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters: "gain" | "pan";
    state: {
      pan_law: PanLaw;
    };
    values: {
      levels: Levels;
    };
    streams: {
      history: number;
    };
    events: {
      sampleRate: number;
    };
    methods: {
      info: { arguments: []; result: Info };
      sampleRate: { arguments: []; result: number };
    };
    pageEvents: {};
  }
}
```

| Section | Holds |
|---------|-------|
| `parameters` | The string id of each parameter, in the order of the init dump |
| `state` | Each `State` field by its path, with the type of its value |
| `values` | Each `#[value]` sender with the type it sends |
| `streams` | Each `#[stream]` sender with the type of one value: the page receives arrays of them |
| `events` | Each `#[event]` with the type of its data |
| `methods` | Each `#[method]` of the handler with its arguments and result |
| `pageEvents` | Each `#[page_event]` of the handler with the type of its data, `null` for none |

The names in a nested group have the group's field name in front, quoted in the file (`"note/pressed"`). The model's structs and enums without generics become named declarations with their doc comments, which the page imports where it names them (a generic type is written out at each use):

```typescript
import type { Info } from "./model-types";
```

The packages read the file through module augmentation (see [TypeScript Declaration](#typescript-declaration)), so the page needs no import for the ids: `parameter("gian")` is an error as soon as the file is part of the page's TypeScript program (its `tsconfig.json` includes `src`, as the examples' do).

### Writing and Checking It

- **`cargo xtask bundle`** writes the file first. It compiles the test that `#[beamer::export]` adds to the plugin and runs it for the build machine. Then it runs the package manager's install, the page's `check` script and its `build` script, then compiles the plugin. An error of the check fails the bundle. With a dev server (`BEAMER_DEV_URL`) the check reports errors and the bundle goes on, so a half-edited page does not block the editor.
- **The `check` script** is required in `webview/package.json`: vite strips the types without checking them. `"check": "tsc -b"` for a React page, `"check": "svelte-check --tsconfig ./tsconfig.json"` for a Svelte page (which fails on errors, not on warnings). A page of another kind puts the command that typechecks it there; a page in plain JavaScript with a build step, which has nothing to check, puts `"check": "exit 0"`.
- **`cargo xtask model-types <plugin>`** writes the file on its own after a change of the model, so that the language server of your code editor sees the new types without a bundle. The file is only written when it changes.
- **`cargo test`** of the plugin fails while the file differs from the model, naming the command that writes it. Commit the file with the change of the model.

### Types from Rust

Each struct and enum a typed page receives needs `#[derive(TypeScript)]` next to its serde derives: the types of `State` fields, of what senders and events carry and of a handler's arguments and results. Booleans, numbers, strings, `Option`, `Vec`, arrays, tuples and maps of these need none. `TypeScript` comes with `beamer::prelude::*`.

```rust
/// The output level as the meter shows it.
#[derive(Debug, Default, Clone, Copy, Serialize, TypeScript)]
#[serde(crate = "beamer::serde")]
pub struct Levels {
    /// The held peak of each channel, linear.
    pub peak: [f32; 2],
}
```

The derive reads the serde attributes with serde's own parser, so renames, `tag`, `content`, `untagged`, `flatten`, `skip` and `transparent` come out as `serde_json` writes them. A field with `skip_serializing_if` is optional (`name?: T`). The mapping of the standard types is in the documentation of `beamer::webview::type_script`: integers and floats are `number`, `Option<T>` is `T | null`, a map is a `Record`. `#[type_script(type = "...")]` on a type or a field writes its TypeScript by hand. `#[type_script(rename = "Name")]` gives the declaration another name: two Rust types under one name are an error of the file.

The page sends back what it gets, a `State` value or an argument, so Rust has to read every value it writes. The derive refuses the serde attributes under which it would not: a name per direction, `skip_serializing` or `skip_serializing_if` without `default` (an `Option` left out when it is `None` needs none), `skip_deserializing` and a field skipped one way in a tuple. It also refuses the attributes that change the shape where it cannot see it (`with`, `serialize_with`, `into`, `from`, `remote` and the like), unless `#[type_script(type = "...")]` gives the TypeScript by hand.

A type without TypeScript:

- **of a `State` field** is `unknown` in the file, with a note above it that xtask prints when it writes the file. Such a type may exist for saving alone.
- **of a sender, an event or a handler's method** is an error that names it: these exist for the page only, so the file is not written without them.

A generic model gives its type parameters a `TypeScript` bound for the types that use them.

The file gives a float as `number`. A value that is not finite arrives as `null`, which JSON writes for it: debug builds report it (see [Data from the Audio Thread](#data-from-the-audio-thread)).

### Loose Sections

Without the file every id is a plain string and every value `unknown`, as for the `delay` example in plain JavaScript. A section the file does not hold stays loose in the same way: a model without `#[beamer::handler]`, or with a hand-written `WebViewHandler`, gets no `methods` and `pageEvents`, so `invoke()` and `emit()` take any name. An empty section (`{}`, `never` for the parameters) allows no name at all. The built-in method `_beamer/parameterTextToNormalized` is typed in every case.

## Parameter Synchronization

Parameter sync is fully automatic. No plugin code is required. The same holds for `State` fields.

### Init Dump

Each time the WebView finishes loading the page (see [When the Page Loads Again](#when-the-page-loads-again)), the format wrapper injects the parameters (`parameters_to_init_json()`) via `evaluateJavaScript`, followed by an object that names the plugin format (see [Plugin Format](#plugin-format)) and carries the values of the `State` fields by path:

```javascript
window.__BEAMER__._onInit([
  {"id":0,"stringId":"gain","name":"Gain","min":-60.0,"max":12.0,
   "defaultValue":0.8333,"value":0.8333,"plainValue":0.0,"displayText":"0.0",
   "format":"db","units":"dB","steps":0,"valueTexts":[]},
  {"id":1,"stringId":"pan","name":"Pan","min":-1.0,"max":1.0,
   "defaultValue":0.5,"value":0.5,"plainValue":0.0,"displayText":"C",
   "format":"pan","units":"","steps":0,"valueTexts":[]}
], {"pluginFormat":"vst3","middleC":"C3","state":{"pan_law":"constant_power"}})
```

The example is shortened for readability: real numeric IDs are hashes of the string IDs and values carry full precision.

Each entry corresponds to a `ParameterInitEntry`:

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
| `format` | `string` | Formatter kind as reported by the parameter. Float and integer parameters report the kind of their formatter: `float` (the default), `db`, `hz`, `ms`, `s`, `percent`, `pan`, `ratio`, `semitones` or `note_number`. Boolean parameters report `boolean` and enum parameters `enum` |
| `units` | `string` | Unit suffix (e.g. `"dB"`, `""`) |
| `steps` | `i32` | Step count (0 for continuous, >0 for discrete) |
| `valueTexts` | `string[]` | The display text of each value of a discrete parameter, from the lowest up: the choices of an enum parameter, `Off` and `On` for a boolean one. Value `i` has the normalized value `i / steps`. Empty for a continuous parameter and for one with more than 128 values |

The runtime resolves the `ready` promise after processing this dump, so any code awaiting `__BEAMER__.ready` can safely call `parameters.get()` and `state.get()`.

### 60 Hz Polling

Alongside the init dump, a repeating timer fires at 60 Hz on the main thread. It also runs while the host is in a tracking loop (a drag, a resize or an open menu), so the UI keeps following the host. Each tick:

1. Iterates over all parameters
2. Compares each normalized value against a cached snapshot
3. For any parameter whose value changed, appends its normalized value, plain value and display text to a delta object
4. Calls `window.__BEAMER__._onParameters({id: [normalized, plain, displayText], ...})` with only the changed entries
5. If a `State` field was set since the last tick, calls `window.__BEAMER__._onState({path: value, ...})` with the fields that changed. The sum of their generations tells whether any did, so a tick without a change reads no field
6. Delivers the events the plugin emitted since the last tick, one `window.__BEAMER__._onEvent(name, data)` call per event (see [Events from Rust](#events-from-rust))
7. Delivers what the audio thread sent through its senders since the last tick, one call per sender that has something: `_onValue(name, data)` with the newest value of a value sender, which the runtime keeps in `__BEAMER__.values`, or `_onEvent(name, data)` with an array of the values of a stream sender (see [Data from the Audio Thread](#data-from-the-audio-thread)). After the init dump, each value sender sends its current value again

The delta format is a plain object mapping numeric parameter IDs to those three values:

```javascript
window.__BEAMER__._onParameters({0: [0.72, -8.16, "-8.2"], 1: [0.35, -0.3, "L 30"]})
```

The runtime updates its internal map and fires all registered `parameters.on()` callbacks for the affected parameters. A value that is not a finite number arrives as `null`.

Nothing counts as sent when an editor opens, so its first tick sends all values, covering any changes between the init dump and the first tick.

### JS-to-Rust Flow

When JavaScript calls `parameters.set(stringId, value)`:

1. The runtime updates its local cache immediately
2. Posts a `{type: "parameter:set", id, value}` message via `WKScriptMessageHandler`
3. The format wrapper hands the message to its `EditorSync`, which reads the messages of the page for every format and calls `ParameterStore::set_normalized()` on the parameter
4. The wrapper tells the host through its `HostEdits`: `IComponentHandler::performEdit()` (VST3) or the equivalent AU host notification
5. The editor sync sends the parameter's authoritative normalized value, plain value and display text straight back through `_onParameters`, so the display text updates without waiting for the next poll tick

`beginEdit` and `endEdit` follow the same path, posting `parameter:begin` and `parameter:end` messages that map to the host's automation gesture APIs. The editor sync notes each gesture the host was told about and that has not ended. When the editor closes or the page loads again before the end arrives (the window closed during a drag or a scroll), it ends the gesture in the host through `HostEdits`, so the host never keeps a parameter touched.

`noteOn`, `noteOff` and `allNotesOff` post `{type: "note:on", note, velocity, channel}`, `note:off` and `note:all-off`. The editor sync puts them into the plugin's `EditorNotes`, which the format wrapper's render takes into the MIDI input of the next block (see [Notes for the Processor](#notes-for-the-processor)). It notes whether the page started a note. When the page goes away, it ends them with the gestures, also when the editor closes without a word from the wrapper.

When JavaScript calls `state.set(path, value)`, the runtime updates its value and its listeners, then posts `{type: "state:set", id: path, value}`. The editor sync decodes the value into the Rust type of the field and sets it, then answers through `_onState` with the value in effect. A host learns about the change as described in [Telling the host](REFERENCE.md#state-beyond-parameters).

## Packages for the Web UI

A web UI can use `__BEAMER__` directly, as the `delay` example does in plain JavaScript. Four packages in `packages/` add types and the parts that are easy to get wrong:

| Package | Holds | Depends on |
|---------|-------|------------|
| `@helpermedia/beamer-api` | Types for `__BEAMER__`, `parameter()`, `stateField()`, `senderValue()`, `ready()`, `noteName()`, `noteOn()`, `noteOff()`, `playNote()`, `allNotesOff()` | Nothing |
| `@helpermedia/beamer-controls` | Unstyled controls on your own elements: `attachDial()`, `attachSlider()`, `attachSwitch()`, `attachRadioGroup()` and `attachTextField()`, plus `fallingLevels()` | `beamer-api` |
| `@helpermedia/beamer-react` | Unstyled React components and hooks on top of both | `beamer-api`, `beamer-controls`, React 19.2 or later |
| `@helpermedia/beamer-svelte` | Unstyled Svelte components and a parameter view on top of both | `beamer-api`, `beamer-controls`, Svelte 5.29 or later |

A web UI with controls of its own needs only `beamer-api`. The packages draw nothing: the look of each control stays in the web UI. They are not on npm yet, so an example that uses them (`webview-demo`, `compressor`, `midi-transform`, `drum-synth`) does not work when copied out of this repository. In this repository the examples use them through the bun workspace of the root `package.json`. Vite reads them from their TypeScript sources (the `source` export condition in `vite.config.ts`), so they need no build of their own. Each package has a README with the details.

### `beamer-api`

`parameter(id)` is a parameter: its value, plain value and text with the unit, the choices of an enum and the changes. Every change goes to the host as a gesture, which the host records as one undo step:

```typescript
import { parameter, ready } from "@helpermedia/beamer-api";

await ready();
const mix = parameter("mix");
mix.subscribe(() => (label.textContent = mix.text()));
mix.edit(0.5);       // begin, set, end
mix.reset();         // the default value as one gesture
await mix.typeValue("-6 dB"); // the parameter's own parser
```

Where a host keeps these steps differs. Reaper adds them to its undo history, Logic to the Undo and Redo of its plug-in window (not the project's undo history). Cubase keeps them in the MixConsole history for effects only and records none for instruments.

A parameter has at most one open gesture: a gesture that begins while another is open ends that one first, so two controls of one parameter never overlap in the host's undo history. `stateField(path)` and `senderValue(name)` read, follow and (for a `State` field) set the other values. All three return one object per id, whose `get` and `subscribe` work detached, in the shape React's `useSyncExternalStore` and Svelte's stores expect.

`noteOn(noteNumber, options)`, `noteOff()` and `allNotesOff()` play notes in the processor (see [Notes for the Processor](#notes-for-the-processor)), with `velocity` and `channel` as options. `playNote(noteNumber, { duration })` plays a short note for a click that does not hold, such as a key press on a focused pad: it starts the note and ends it after `duration` milliseconds (default 100). Playing the note again before then starts it anew; the older timer does not end the newer note.

```typescript
import { noteOff, noteOn, playNote } from "@helpermedia/beamer-api";

pad.onpointerdown = (event) => {
  pad.setPointerCapture(event.pointerId);
  noteOn(36, { velocity: 0.8 });
};
pad.onpointerup = pad.onpointercancel = () => noteOff(36);
pad.onkeydown = (event) => {
  if (event.key !== " " || event.repeat) return;
  event.preventDefault();
  playNote(36);
};
```

### `beamer-controls`

Each control is a function that takes an element of your own markup and a parameter and returns the function that detaches it. It adds the role, the ARIA attributes and the input, each change one undo step in the host. It sets a role, a `tabindex` or an `aria-label` (from the parameter's name) only where your markup has none. Detaching removes what it set. The same shape is a Svelte attachment, under the components of `beamer-svelte`.

- `attachDial(element, parameter, options)` for a round control and `attachSlider(element, parameter, options)` for one that moves along a line. Both get ARIA's `role="slider"` (ARIA has no role for a dial) with `aria-valuenow`, `aria-valuemin` and `aria-valuemax` in the parameter's plain units and its text as `aria-valuetext`; a slider also gets `aria-orientation`.
  - A dial drags vertical (the default), horizontal or both, ten times finer with Shift, from the value at the press, so a discrete parameter that the host snaps to a step does not stick. Or along an arc: the value jumps to the pointer and follows it around the element's center. A function can pick the drag for each press. Each drag is one gesture from press to release. `dialAngle(value, arcStart, arcSweep)` says where to draw the dial's pointer so an arc drag lands where the dial is drawn.
  - A slider moves along its `orientation`, upright by default. A press on the track brings the thumb to the pointer and a press on the thumb grabs it; the thumb then follows the pointer, ten times finer with Shift. `thumbLength` (a part of the element's length, below 1) keeps the pointer on the spot of the thumb it pressed; `thumbPosition(value, thumbLength)` says where to draw the thumb so the drawing and the drag agree. `orientation` is the direction on screen.
  - The wheel by distance, capped per event so that a mouse notch moves about as far in every browser engine, corrected for natural scrolling. A scroll is one gesture until the wheel rests.
  - Arrow keys, Page Up and Page Down, Home and End once the element has focus; a discrete parameter moves one choice per key.
  - Reset by double-click or by a click with Command or Control held.
- `attachSwitch(button, parameter)` for an on and off parameter: `role="switch"` and `aria-checked`, each click one undo step.
- `attachRadioGroup(container, parameter)` for an enum: a radio group of the container's children, one radio per choice in order, `aria-checked` on the one the parameter is at, arrow keys that check the next or previous choice, one tab stop.
- `attachTextField(input, parameter, options)` for a value to type: read-only while closed, a click (or a double click) opens it with the `data-typing` attribute, Enter or a click elsewhere sets the text through the parameter's own parser, Escape keeps the value, an unchanged text sets nothing. `openTextField(input)` opens it from elsewhere.
- Every control has the `data-keyboard-focus` attribute while it has focus from the keyboard (Tab, or keys after a click), for a focus style that a click does not show.
- `fallingLevels(sender, { read })` follows a value sender of meter levels. When no value comes for 300 ms (the host stopped calling the plugin), the levels fall on their own at 20 dB a second, each towards its rest. The plugin sends the levels with every block while they show something.

### `beamer-react`

Unstyled components, one per control. Each renders one element with the control of `beamer-controls` on it and hands what it shows to its children, as markup or as a function. The look stays yours: the class names you give and the drawing inside. The parameter goes in as `parameter`, the control's options as props of their own names; other props (`className`, `style`, `aria-*`, events) and a `ref` go to the element.

| Component | Renders | Hands to its children |
|-----------|---------|-----------------------|
| `Dial` | A `<div>` with `attachDial` | `value`, `angle` (where it points, `dialAngle`), `text` |
| `Slider` | A `<div>` with `attachSlider`, the track | `value`, `position` (the thumb's center, `thumbPosition`), `text` |
| `Switch` | A `<button>` with `attachSwitch` | `checked` |
| `RadioGroup` | A `<div>` with `attachRadioGroup`; its children are its radios, one per choice in order | `choices` (the names), `checked` (the index) |
| `TextField` | An `<input>` with `attachTextField` | |

The compressor's dial draws a ring with a blue arc up to the value and a pointer inside the `<div>` of `Dial` (`Ring` and `Pointer` stand for its SVG), with the text field under it:

```tsx
function CompressorDial({ parameterId }: { parameterId: ParameterId }) {
  const { parameter } = useParameter(parameterId);
  return (
    <div className="flex flex-col items-center">
      <Dial parameter={parameter} className="group size-12 outline-none">
        {({ value, angle }) => (
          <svg viewBox="0 0 48 48" className="size-full">
            <Ring part={value} />
            <Pointer degrees={angle} />
          </svg>
        )}
      </Dial>
      <TextField parameter={parameter} className="text-center" />
    </div>
  );
}
```

The hooks:

| Hook | Returns |
|------|---------|
| `useParameter(id)` | `{ parameter, value, displayText, text, info }`: the handle for changes and what to show |
| `useStateField(path)` | `[value, set]` for a `State` field, like `useState` |
| `useSenderValue(name)` | The newest value of a value sender from the audio thread |
| `usePluginEvent(name, handler)` | Calls `handler` with each event or stream array while mounted |
| `useReady()` | Whether parameters and state have arrived |
| `useReadable(readable)` | The newest value of a `Readable`, such as the levels of `fallingLevels()` |
| `useDial(element, parameter, options)`, `useSlider(element, parameter, options)`, `useSwitch(button, parameter)`, `useRadioGroup(container, parameter)`, `useTextField(input, parameter, options)` | Attach a control of `beamer-controls` to an element of your own while it is there: the layer under the components. Each takes the element itself, held in state (`const [dial, setDial] = useState<SVGSVGElement \| null>(null)`, `ref={setDial}`), null while there is none |

The demo's pan law and output meter:

```tsx
const [panLaw, setPanLaw] = useStateField("pan_law");
// Made once, when the module loads: the levels in dB, falling on their own.
const levels = fallingLevels("levels", { read: (value) => peaksInDb(value) });
const peaks = useReadable(levels);
```

### `beamer-svelte`

The same five components for Svelte 5: `Dial`, `Slider`, `Switch`, `RadioGroup` and `TextField`, each rendering the same element, with the same props. What a component shows reaches its `children` snippet. Other props go to the element and `bind:ref` gives it. A fader:

```svelte
<Slider {parameter} thumbLength={0.1} class="group relative h-full w-6 outline-none">
  {#snippet children({ position })}
    <!-- The thumb, its center at `position` of the height. -->
    <span class="absolute inset-x-0 h-[10%] bg-neutral-800" style:bottom="{(position - 0.05) * 100}%"></span>
  {/snippet}
</Slider>
<TextField {parameter} />
```

`parameterView(parameter)` is what a parameter shows, the counterpart of `useParameter`: the same `parameter`, `value`, `displayText`, `text` and `info`, each but the handle a reactive read. A readout follows a new text for the same value (the plugin answers an edit) and the info that arrives with `ready`. For a parameter that is a prop, a `$derived` makes a new view when the prop changes. Read the fields from the view; a field taken out of it keeps the value it had then:

```svelte
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { parameterView } from "@helpermedia/beamer-svelte";

  let { parameter }: { parameter: Parameter } = $props();
  const view = $derived(parameterView(parameter));
</script>

<p>{view.info?.name}: {view.text}</p>
```

The objects of `beamer-api` and `fallingLevels` follow Svelte's store contract, so a component reads them with `$`, runes or not: `{$mix}` for a parameter's normalized value, `$levels` for the levels of `fallingLevels`. `$mix` is the value alone: `{mix.text()}` in the markup reads the text once, at the first render, also beside `$mix`, so a readout reads the text from a view. A `State` field is a writable store (`$tab = "mixer"` sets it). A store is a variable at the top of a component, a prop included, so a component in an `{#each}` takes its parameter as a prop; a view also works per item with `{@const}`. Events from the plugin need nothing of `beamer-svelte`: `$effect(() => runtime().on("hits", onHit))` follows them while the component is mounted.

## Asset Embedding

The `#[beamer::export]` attribute macro handles web asset detection and embedding at compile time. It scans the plugin crate's `webview/` directory according to these rules, the same ones `cargo xtask bundle` uses to decide whether the plugin has an editor:

| Condition | Behavior |
|-----------|----------|
| `BEAMER_DEV_URL` env var set | Load from that URL (dev server mode, see [Development Server](#development-server)) |
| `webview/package.json` exists and `webview/dist/` exists | Embed all files from `webview/dist/` (no editor when it is empty) |
| `webview/package.json` exists but no `dist/` | No assets. `cargo xtask bundle` runs the web build first, so this only happens with a plain `cargo build` (run the web build in `webview/` first) |
| `webview/` exists (no `package.json`) | Embed all files from `webview/` (plain HTML project such as the `delay` example, no editor when it is empty) |
| No `webview/` directory | No editor |

The scan skips dotfiles and dot-directories, `node_modules` and `.map` source map files.

Each discovered file is embedded via `include_bytes!()` into a static `EmbeddedAssets` table:

```rust
// Generated by #[beamer::export].
static WEBVIEW_ASSETS: EmbeddedAssets = EmbeddedAssets::new(&[
    EmbeddedAsset { path: "index.html", data: include_bytes!("...") },
    EmbeddedAsset { path: "assets/index-abc123.js", data: include_bytes!("...") },
    EmbeddedAsset { path: "assets/index-def456.css", data: include_bytes!("...") },
]);
```

### Custom URL Scheme

When embedded assets are available, the WebView navigates to `beamer://localhost/index.html`. A per-plugin `WKURLSchemeHandler` intercepts requests to the `beamer://` scheme and serves files from the `EmbeddedAssets` table.

The Objective-C classes behind the WebView (the WebView itself, the scheme handler, the message handler and the navigation delegate) are registered at runtime under names that are unique to the plugin binary, so multiple Beamer plugins loaded in the same host process do not share them. Each handler instance stores its own assets pointer as an ObjC ivar, avoiding any global state.

The WebView is a subclass of `WKWebView` that accepts the first click in a window that is not active. A host may open the editor without activating its window (Logic Pro does). The first click would otherwise only activate it.

Path lookup uses `NSURL::path()` to get the decoded path component, stripping query strings, fragments and percent-encoding. The MIME type is inferred from the file extension.

### Rebuild Considerations

Directory scanning runs at macro expansion time, so its results are cached by the Rust compiler. Changes to existing file contents are tracked automatically by `include_bytes!()`. Setting, changing or clearing `BEAMER_DEV_URL` (see below) expands the macro again, since the expansion names the variable with `option_env!()`. So does a web UI whose files came, went or moved when `cargo xtask bundle` builds: the bundler hands the macro a stamp of the files it found (`BEAMER_WEB_UI_STAMP`). A plain `cargo build` has no such stamp, so after files were added or removed it may need a `touch` of a `.rs` file.

## Development Server

Set the `BEAMER_DEV_URL` environment variable to bypass asset embedding and load the web UI from a dev server:

```bash
BEAMER_DEV_URL=http://localhost:5173 cargo xtask bundle webview-demo --vst3
```

When this variable is set:

1. The macro generates `Config::with_webview_url("http://localhost:5173")` instead of `with_webview_assets(...)`
2. The WebView navigates directly to the URL instead of `beamer://localhost/`
3. The custom scheme handler is not registered

This enables hot-reload workflows during development. The Beamer runtime (`beamer_runtime.js`) is still injected as a user script, so all `__BEAMER__` APIs work the same way regardless of asset source.

Developer tools (Web Inspector on macOS) are enabled automatically in debug builds of every format: Beamer sets `WebViewConfig::dev_tools` from `debug_assertions`. On macOS, a right click on the page then offers Reload and Inspect Element. Safari's Develop menu lists the page as well.

## Context Menu

A right click or Control click on the editor shows no menu from the webview in release builds: the Reload, Copy, Look Up, Translate and Share items of a browser have no place in a plugin editor. A page can show a menu of its own (for MIDI learn on a dial, say) by handling the `contextmenu` event and calling `preventDefault()`, in every build.

## When the Page Loads Again

A page loads again after Reload in a debug build, `location.reload()`, a reload of a dev server, or when its process ends. The webview runs the page in a process of its own. When the system ends that process (to reclaim memory, or after a crash in the webview), Beamer loads the page again. The new page starts like the first one: it gets the parameter values and `State` fields from Rust (the init dump). The gestures the old page left open end in the host as soon as the new page replaces it or its process ends, also when Beamer does not load it again. So do the notes it left on. Nothing the old page posted reaches the plugin after that. What only the page held, such as an open panel or a scroll position, is gone. Keep what has to survive in a `State` field. If the process ends again within 10 seconds, Beamer does not load the page a third time: the editor stays blank until it is opened again, so a page that ends its own process does not keep doing so in the host.

## Background Color

The WKWebView's default background is disabled (`drawsBackground = false`), making it fully transparent. Whatever is behind the webview is visible until web content covers it. In some DAWs, the view behind the webview has a white background, causing a visible flash before your UI renders.

To prevent this, set `editor_background_color` in `Config.toml`:

```toml
editor_background_color = "#1a1a2e"
```

The macro parses the hex string into `[R, G, B, A]` bytes (alpha is always 255). At runtime, this color is painted on the parent `NSView`'s `CALayer` before the WKWebView is added to it. It stays visible through the transparent webview until your web content renders its own background on top.

This is independent from any background set in HTML, CSS or JS. You can use either approach, or both.

## Status and Planned Work

### Windows WebView2 Support

The Windows platform backend (`beamer-webview/src/platform/windows.rs`) is a stub that returns `WebViewError::PlatformNotSupported`. All WebView functionality currently targets macOS only.

A future implementation would use Microsoft's WebView2 (Edge/Chromium) control via either the `webview2` crate or direct COM interop. The `WebViewConfig` struct is platform-agnostic. A Windows backend needs `WindowsWebView::attach_to_parent()` with a scheme handler and a message handler, `evaluate_js()` and a 60 Hz sync timer in the VST3 editor in place of the NSTimer. The C-ABI bridge in `ffi.rs` serves only the Audio Unit wrappers, which Windows does not have.
