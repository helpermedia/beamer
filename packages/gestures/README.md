# @helpermedia/beamer-gestures

Input handling for the controls of a [Beamer](https://github.com/helpermedia/beamer)
web UI: drag, wheel, keys and reset, each change one undo step in the
host. It draws nothing: the look of each control stays in your web UI.
Builds on [`beamer-api`](../api). A web UI with gestures of its own needs
only that one.

## Example

```ts
import { param, ready } from "@helpermedia/beamer-api";
import { attachGestures } from "@helpermedia/beamer-gestures";

await ready();

const knob = document.querySelector<SVGElement>("#mix")!;
knob.tabIndex = 0;

// Drag, scroll, arrow keys and double click to reset.
const detach = attachGestures(knob, param("mix"));
```

## What it handles

- **Drag:** vertical (up raises the value, the default), horizontal (to the
  right) or both together, ten times finer with shift. Or `"arc"`: the
  value jumps to the pointer and follows it around the element's center.
  When the pointer passes through the gap between the arc's ends, the
  value waits at the end it left until the pointer comes back to that
  side. A function can pick the mode for each press, such as the arc on a
  knob's handle and vertical elsewhere. The primary button only, one
  gesture from press to release, also when the window loses the pointer.
  A click focuses the element.
- **Wheel:** by distance, scrolling up raises the value also with macOS
  natural scrolling. One event counts for a few pixels at most, so a mouse
  notch moves about as far in every browser engine. A scroll is one gesture
  until the wheel rests.
- **Keys:** arrows, Page Up and Page Down, Home and End, finer with shift.
  A discrete parameter moves one choice per key. The element must be
  focusable.
- **Reset:** a double click (default), a click with Command or Control
  held, or both. `reset` takes a list of the ones to use; an empty list
  turns reset off.
- **Focus:** while the element has focus from the keyboard (Tab, or keys
  after a click) it has the `data-keyboard-focus` attribute, for a focus
  style. Use it instead of `:focus-visible`, which WebKit also shows after
  a click, since the click focuses the element from script.

A drag or scroll moves the value from where it started, so a discrete
parameter that the host snaps to a step does not stick. A key or a reset
during a drag or scroll ends that one first. The element gets
`touch-action: none`, so that a touch drag turns the control instead of
scrolling the page.

`GestureOptions` sets the drag mode, the arc's start, sweep and dead zone,
the steps, the wheel cap and rest time and the reset.
