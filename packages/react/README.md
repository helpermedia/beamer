# @helpermedia/beamer-react

React hooks for the web UI of a [Beamer](https://github.com/helpermedia/beamer)
plugin, on top of [`beamer-api`](../api) and [`beamer-controls`](../controls).
Each one follows the plugin through `useSyncExternalStore` or an effect, so
a component renders again when the host changes a value.

## Example

```tsx
import { useRef } from "react";
import { useDial, useParam, useStateField, useValueEntry } from "@helpermedia/beamer-react";

function MixDial() {
  const dial = useRef<SVGSVGElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const { param, value } = useParam("mix");
  // A dial: drag, scroll, arrow keys and double click to reset.
  useDial(dial, param);
  // The value, a click to type one.
  useValueEntry(field, param);
  return (
    <div>
      <svg ref={dial} viewBox="0 0 64 64">
        {/* Draw the dial from `value`. */}
      </svg>
      <input ref={field} />
    </div>
  );
}

function PanLawButton() {
  const [panLaw, setPanLaw] = useStateField<"linear" | "constant_power">("pan_law");
  return (
    <button onClick={() => setPanLaw(panLaw === "linear" ? "constant_power" : "linear")}>
      {panLaw}
    </button>
  );
}
```

## Hooks

| Hook | Returns |
|------|---------|
| `useParam(id)` | `{ param, value, displayText, text, info }`: the handle for changes and what to show |
| `useStateField(path)` | `[value, set]` for a `State` field, like `useState` |
| `useSenderValue(name)` | The newest value of a value sender from the audio thread |
| `useEvent(name, handler)` | Nothing; calls `handler` with each event or stream array while mounted |
| `useReady()` | Whether parameters and state have arrived |
| `useDial(ref, param, options)` | Nothing; makes the element a dial (`attachDial`) while mounted |
| `useSlider(ref, param, options)` | Nothing; makes the element a slider (`attachSlider`) while mounted |
| `useSwitch(ref, param)` | Nothing; makes the button a switch (`attachSwitch`) while mounted |
| `useChoices(ref, param)` | Nothing; makes the element's children a radio group (`attachChoices`) while mounted |
| `useValueEntry(ref, param, options)` | Nothing; makes the input the value field (`attachValueEntry`) while mounted |
| `useReadable(readable)` | The newest value of a `Readable`, such as a `fallingMeter` |

A control attaches again when its parameter or an option changes. A
`drag` or `text` function may be a new one on every render: the newest is
called. Give the input of `useValueEntry` no `value`: the field sets its
text itself.

React 19 or later is a peer dependency.
