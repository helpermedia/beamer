# @helpermedia/beamer-controls

Unstyled controls for the web UI of a
[Beamer](https://github.com/helpermedia/beamer) plugin. Each one makes an
element of your own markup a dial, a slider, a switch, a radio group or a
text field for a parameter: the role, the ARIA attributes and the input,
each change one undo step in the host. They draw nothing: the look stays in
your web UI. Builds on [`beamer-api`](../api). A web UI with controls of its
own needs only that one.

## Example

```ts
import { parameter, ready } from "@helpermedia/beamer-api";
import { attachDial, attachTextField } from "@helpermedia/beamer-controls";

await ready();

const mix = parameter("mix");

// Drag, scroll, arrow keys and double click to reset.
const detachDial = attachDial(document.querySelector<SVGElement>("#mix")!, mix);

// The value; a click opens it to type one.
const detachField = attachTextField(document.querySelector<HTMLInputElement>("#mix-value")!, mix);
```

Each attach function takes the element, the parameter and its options. It
returns the function that detaches it: the shape of a Svelte attachment or
a React effect, under the components of [`beamer-svelte`](../svelte) and
[`beamer-react`](../react).

## Controls

| Function | Makes the element | Sets |
|----------|-------------------|------|
| `attachDial(element, parameter, options)` | A dial: a round control | `role="slider"`, `tabindex="0"`, `aria-valuenow`, `aria-valuemin` and `aria-valuemax` in plain units, `aria-valuetext` with the unit |
| `attachSlider(element, parameter, options)` | A slider: a control along a line, upright or lying | The same and `aria-orientation` |
| `thumbPosition(value, thumbLength)` | Nothing; where to draw a slider's thumb: its center as a part of the length from the low end | |
| `dialAngle(value, arcStart, arcSweep)` | Nothing; where a dial points: degrees clockwise on screen from the right of its center | |
| `attachSwitch(button, parameter)` | A switch for an on and off parameter | `role="switch"`, `aria-checked` |
| `attachRadioGroup(container, parameter)` | A radio group of its children, one per choice in order | `role="radiogroup"`, on each child `role="radio"`, `aria-checked` and a tabindex |
| `attachTextField(input, parameter, options)` | The field that shows the value and takes a typed one | `readonly` while closed, `data-typing` while open |
| `openTextField(input)` | Opens an attached field, for a double click on its dial | |

A control sets a role, a tabindex or an `aria-label` (from the parameter's
name) only where your markup has none, so your own stay. Detaching removes
what it set. While a control has focus from the keyboard (Tab, or keys
after a click) it has the `data-keyboard-focus` attribute, for a focus
style. Use it instead of `:focus-visible`, which WebKit also shows after a
click, since a dial or a slider focuses itself from script on a press.
Style a checked switch or radio with `[aria-checked="true"]`.

### Dial and slider

Both get ARIA's `role="slider"`: ARIA has no role of its own for a dial.

- **Drag of a dial:** vertical (up raises the value, the default),
  horizontal (to the right) or both together, ten times finer with shift. Or
  `"arc"`: the value jumps to the pointer and follows it around the
  element's center. When the pointer passes through the gap between the
  arc's ends, the value waits at the end it left until the pointer comes
  back to that side. A function can pick the drag for each press, such as
  the arc on a dial's handle and vertical elsewhere. Draw the dial's
  pointer at `dialAngle(value, arcStart, arcSweep)`, so that an arc drag
  lands where the dial is drawn.
- **Drag of a slider:** along its `orientation`, upright by default. A
  press on the track brings the thumb to the pointer and a press on the
  thumb grabs it where it is pressed. The thumb then follows the pointer,
  ten times finer with shift. The thumb's center travels the element's
  length, less half of `thumbLength` (a part of that length, below 1) at
  each end. Draw the thumb where `thumbPosition(value, thumbLength)` puts
  it, so that the drawing and the drag agree. `orientation` is the
  direction on screen, also for an element turned with a CSS rotation.
  With nothing left to travel the slider moves by distance, as a dial
  does.
- Either drags with the primary button only, one gesture from press to
  release, also when the window loses the pointer. A press focuses the
  element.
- **Wheel:** by distance, scrolling up raises the value also with macOS
  natural scrolling. One event counts for a few pixels at most, so a mouse
  notch moves about as far in every browser engine. A scroll is one gesture
  until the wheel rests.
- **Keys:** arrows, Page Up and Page Down, Home and End, finer with shift.
  A float parameter with a step (`stepSize` above 0) moves one step per
  arrow and ten per page key, Shift or not, through `Parameter.step()`:
  the plugin works out the next step, whatever its mapping.
  A discrete parameter moves one choice per key.
- **Reset:** a double click (default), a click with Command or Control
  held, or both. `reset` takes a list of the ones to use; an empty list
  turns reset off.

A dial's drag and a scroll move the value from where they started, a
slider's drag from the pointer's position, so a discrete parameter that
the host snaps to a step does not stick. A key or a reset
during a drag or scroll ends that one first. The element gets
`touch-action: none`, so that a touch drag turns the control instead of
scrolling the page.

`DialOptions` sets the drag, the arc's start, sweep and dead zone.
`SliderOptions` sets the orientation and the thumb's length. Both take the
`InputOptions` they share: the steps, the wheel cap and rest time and the
reset.

### Switch

Each click flips the value as one undo step, also from Space or Enter, as
the button is a button. Its text is its name.

### Radio group

The container's element children are its radios, one per choice, the
first for the lowest value. Their names come from the parameter's
`choices()` or are your own (an on and off parameter shown as two
choices). Children rendered later, once the choices are known, count
too. A click sets a choice, a click on the checked one sets nothing. The
arrow keys check the next or previous choice and move the focus there,
from the last back to the first. Only the checked child is in the tab
order.

### Text field

Closed, the input is read-only and shows the value with its unit, or what
`text` returns. A click (or a double click, with `open: "double-click"`),
Enter or Space opens it: it shows the value without the unit, all
selected. Enter or a click elsewhere sets what the user typed, read by the
parameter's own parser, so "-12", "-12 dB" and "250ms" all work. A text
left as it was sets nothing, so no undo step. Escape keeps the value. The
focus stays in the field. A gesture that begins while the plugin reads the
text (a drag on the dial that took the focus) wins: the typed value is
dropped.

## Meter levels

```ts
import { fallingLevels } from "@helpermedia/beamer-controls";

const FLOOR_DB = -60;

// The levels in dB from what the sender sends, the floor for undefined.
const levels = fallingLevels("levels", {
  read: (value) => (typeof value === "number" ? Math.max(FLOOR_DB, value) : FLOOR_DB),
});

levels.subscribe((db) => drawBar(db));
```

`fallingLevels(sender, options)` follows a value sender of meter levels: one
number, one per channel (an array) or one per name (an object). A plugin
sends its meters with every block while they show something. When nothing
comes for 300 ms, the host stopped calling the plugin (its bypass, a host
that rests while stopped). The levels then fall on their own at 20 dB a
second, each towards its rest. `read` takes what arrives (typed by the
page's `ModelTypes`, see beamer-api, or `unknown` without them) and gives
the levels at rest for undefined: the floor for a level in dB, 0 for a gain
reduction. `fallPerSecond` and `waitMs` set the rate and the wait; match
the rate to how the processor lets its held peaks fall.

It returns a `Readable` (`get` and `subscribe`), so Svelte's `$levels` and
React's `useReadable` take it as it is. It needs the runtime only while it
has subscribers, so it can be made when a module loads.
