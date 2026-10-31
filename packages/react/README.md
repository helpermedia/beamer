# @helpermedia/beamer-react

React hooks for the web UI of a [Beamer](https://github.com/helpermedia/beamer)
plugin, on top of [`beamer-api`](../api) and [`beamer-gestures`](../gestures).
Each one follows the plugin through `useSyncExternalStore` or an effect, so
a component renders again when the host changes a value.

## Example

```tsx
import { useRef } from "react";
import { useGestures, useParam, useStateField } from "@helpermedia/beamer-react";

function MixKnob() {
  const ref = useRef<SVGSVGElement>(null);
  const { param, value, text } = useParam("mix");
  // Drag, scroll, arrow keys and double click to reset.
  useGestures(ref, param);
  return (
    <svg ref={ref} tabIndex={0} viewBox="0 0 64 64">
      {/* Draw the knob from `value`. */}
      <text x="32" y="60" textAnchor="middle">{text}</text>
    </svg>
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
| `useGestures(ref, param, options)` | Nothing; attaches the gestures of `beamer-gestures` while mounted |

React 18 or later is a peer dependency.
