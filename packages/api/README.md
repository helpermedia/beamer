# @helpermedia/beamer-api

The base package for the web UI of a [Beamer](https://github.com/helpermedia/beamer)
plugin: typed access to the plugin's parameters, `State` fields and values
from the audio thread. Everything else builds on it:
[`beamer-controls`](../controls) for unstyled controls and
[`beamer-react`](../react) for React components. It has no dependencies.

A web UI does not need this package. The plugin injects `__BEAMER__` into
every editor. Plain JavaScript can use it directly (see the delay
example). The package adds types and undo gestures that are hard to get
wrong.

## Example

```ts
import { param, ready } from "@helpermedia/beamer-api";

await ready();

const mix = param("mix");
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
| `param(id)` | A parameter: value, plain value, text with unit, gestures (`begin`, `set`, `end`, `edit`, `reset`), typed values, the choices of an enum |
| `stateField(path)` | A `State` field to read, follow and set |
| `senderValue(name)` | The newest value of a value sender from the audio thread |
| `noteName(noteNumber)` | The name of a MIDI note number as the plugin's parameters name it: 60 is "C3", or "C4" with `middle_c = "C4"` in Config.toml |
| Types | `Beamer`, `BeamerParamInfo` and the rest of `__BEAMER__` |

Each parameter has at most one open gesture: a gesture that begins while
another is open ends that one first, so two controls of one parameter never
overlap in the host's undo history.

`param()`, `stateField()` and `senderValue()` return one object per id for
as long as the page has its runtime, so a component that asks on every
render gets the same one. Their `get` and `subscribe` work detached, in the
shape React's `useSyncExternalStore` and Svelte's stores expect:
`subscribe` calls back at once with the value now, then on every change.
Before `ready` a parameter reads as empty (0, "", no choices) and changes do
nothing.

`param()` and the stores need the runtime when they are called. To work on
a web UI in a browser, define `globalThis.__BEAMER__` with a stand-in in a
script that runs before your own.
