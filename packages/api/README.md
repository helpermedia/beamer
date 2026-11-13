# @helpermedia/beamer-api

The base package for the web UI of a [Beamer](https://github.com/helpermedia/beamer)
plugin: typed access to the plugin's parameters, `State` fields and values
from the audio thread, plus notes for its processor. Everything else builds
on it:
[`beamer-controls`](../controls) for unstyled controls,
[`beamer-react`](../react) for React components and
[`beamer-svelte`](../svelte) for Svelte components. It has no
dependencies.

A web UI does not need this package. The plugin injects `__BEAMER__` into
every editor. Plain JavaScript can use it directly (see the delay
example). The package adds types and undo gestures that are hard to get
wrong.

## Example

```ts
import { parameter, ready } from "@helpermedia/beamer-api";

await ready();

const mix = parameter("mix");
const label = document.querySelector("#mix-value")!;

// Follow the value, also when the host automates it.
mix.subscribe(() => {
  label.textContent = mix.text();
});

// One undo step in the host.
document.querySelector("#reset")!.addEventListener("click", () => mix.reset());
```

## Contents

| Export | What it does |
|--------|--------------|
| `ready()`, `runtime()` | The runtime once parameters and state have arrived |
| `parameter(id)` | A parameter: value, plain value, text with unit, gestures (`begin`, `set`, `end`, `edit`, `reset`), typed values, the choices of an enum |
| `stateField(path)` | A `State` field to read, follow and set |
| `senderValue(name)` | The newest value of a value sender from the audio thread |
| `noteName(noteNumber)` | The name of a MIDI note number as the plugin's parameters name it: 60 is "C3", or "C4" with `middle_c = "C4"` in Config.toml |
| `noteOn(noteNumber, options)`, `noteOff(noteNumber, options)` | Start and end a note in the processor, with a `velocity` (0 to 1) and a `channel` (0 to 15) |
| `playNote(noteNumber, options)` | A short note: on now, off after `duration` milliseconds (default 100) |
| `allNotesOff()` | End every note the page started |
| Types | `Beamer`, `BeamerParameterInfo` and the rest of `__BEAMER__` |

Each parameter has at most one open gesture: a gesture that begins while
another is open ends that one first, so two controls of one parameter never
overlap in the host's undo history.

`parameter()`, `stateField()` and `senderValue()` return one object per id for
as long as the page has its runtime, so a component that asks on every
render gets the same one. Their `get` and `subscribe` work detached, in the
shape React's `useSyncExternalStore` and Svelte's stores expect:
`subscribe` calls back at once with the value now, then on every change.
Before `ready` a parameter reads as empty (0, "", no choices) and changes do
nothing.

The processor gets the notes with the host's MIDI input, at the start of
its next block, so an instrument plays them with no code of its own. A pad
that sounds while held uses `noteOn` and `noteOff`; `playNote` is for a
click that does not hold, such as a key press on a focused pad. Playing a
note again before its `playNote` ended starts it anew; the older timer
does not end the newer note. The plugin ends the notes the page left on
when the editor closes or the page loads again.

```ts
import { noteOff, noteOn, playNote } from "@helpermedia/beamer-api";

// The capture sends the pointerup here also when the pointer leaves the pad.
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

`parameter()` and the stores need the runtime when they are called. To work on
a web UI in a browser, define `globalThis.__BEAMER__` with a stand-in in a
script that runs before your own.
