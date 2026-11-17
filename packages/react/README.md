# @helpermedia/beamer-react

Unstyled components and hooks for the web UI of a
[Beamer](https://github.com/helpermedia/beamer) plugin, on top of
[`beamer-api`](../api) and [`beamer-controls`](../controls). Each one
follows the plugin through `useSyncExternalStore` or an effect, so a
component renders again when the host changes a value.

## Example

```tsx
import { Dial, ValueEntry, useParameter, useStateField } from "@helpermedia/beamer-react";

function MixDial() {
  const { parameter } = useParameter("mix");
  return (
    <div>
      {/* Drag, scroll, arrow keys and double click to reset. */}
      <Dial parameter={parameter} className="size-16">
        {({ angle }) => (
          <svg viewBox="0 0 64 64">
            {/* A pointer from the center at `angle`. */}
            <line
              x1={32}
              y1={32}
              x2={32 + 24 * Math.cos((angle * Math.PI) / 180)}
              y2={32 + 24 * Math.sin((angle * Math.PI) / 180)}
              stroke="currentColor"
            />
          </svg>
        )}
      </Dial>
      {/* The value, a click to type one. */}
      <ValueEntry parameter={parameter} />
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

## Components

Each component renders one element with a control of `beamer-controls`:
the role, the ARIA attributes and the input. They draw nothing: the look
is yours, in the class names you give and the markup you put inside. Each
takes the parameter as `parameter` and the control's options as props.
Other props (`className`, `style`, `aria-*`, events) and a `ref` go to the
element.

| Component | Renders | Hands to its children |
|-----------|---------|-----------------------|
| `Dial` | A `<div>` with `attachDial` | `value`, `angle` (where it points, `dialAngle`), `text` |
| `Slider` | A `<div>` with `attachSlider`: the track | `value`, `position` (the thumb's center, `thumbPosition`), `text` |
| `Switch` | A `<button>` with `attachSwitch` | `checked` |
| `RadioGroup` | A `<div>` with `attachRadioGroup` | `choices` (the names), `checked` (the index) |
| `ValueEntry` | An `<input>` with `attachValueEntry` | |

Children are either markup or a function of what the control shows:

```tsx
<Slider parameter={level} thumbLength={0.1} className="h-48 w-8">
  {({ position }) => (
    <svg className="size-full">
      <rect y={`${(1 - position) * 100 - 5}%`} height="10%" width="100%" />
    </svg>
  )}
</Slider>
```

Draw a dial or a slider inside its `<div>`, filling it: the drag measures
the `<div>`. The children of `RadioGroup` are its radios, one button per
choice in order, the first for the lowest value; a checked one has
`aria-checked="true"`. A `drag` or `text` function may be a new one on
every render: the newest is called. The value field owns its text, so it
takes no `value`.

## Hooks

The hooks under the components are there too, for a control on an element
of your own. A control hook takes the element itself, not a ref: hold it
in state and give the setter to the element as its `ref`.

```tsx
function GainDial() {
  const { parameter } = useParameter("gain");
  const [dial, setDial] = useState<SVGSVGElement | null>(null);
  useDial(dial, parameter, { drag: "arc" });
  return <svg ref={setDial} viewBox="0 0 48 48" />;
}
```

The control follows the element: it attaches when the element appears,
detaches when it goes (the state is null again) and moves when another
one takes its place, also while the component stays mounted. The element
is yours for other things too, such as measuring it or `openValueEntry`.
The control attaches in the hook's effect, one render after the element
mounts, so an effect of your own that needs the control (one that opens
the field with `openValueEntry`) comes after the call of the hook.

Give each element a state of its own: two elements cannot share a setter.
`ref.current` of a ref object is no substitute: a hook that gets it sees
the element only if the component happens to render again.

| Hook | Returns |
|------|---------|
| `useParameter(id)` | `{ parameter, value, displayText, text, info }`: the handle for changes and what to show |
| `useStateField(path)` | `[value, set]` for a `State` field, like `useState` |
| `useSenderValue(name)` | The newest value of a value sender from the audio thread |
| `useEvent(name, handler)` | Nothing; calls `handler` with each event or stream array while mounted |
| `useReady()` | Whether parameters and state have arrived |
| `useDial(element, parameter, options)` | Nothing; makes the element a dial (`attachDial`) while it is there |
| `useSlider(element, parameter, options)` | Nothing; makes the element a slider (`attachSlider`) while it is there |
| `useSwitch(button, parameter)` | Nothing; makes the button a switch (`attachSwitch`) while it is there |
| `useRadioGroup(container, parameter)` | Nothing; makes the container's children a radio group (`attachRadioGroup`) while it is there |
| `useValueEntry(input, parameter, options)` | Nothing; makes the input the value field (`attachValueEntry`) while it is there |
| `useReadable(readable)` | The newest value of a `Readable`, such as a `fallingMeter` |

A control attaches again when its element, its parameter or an option
changes, not on a render that changes none of them. A dial's `drag` and a
value field's `text` are the exceptions among the options: the newest
`drag`, a fixed one or a function, is read for each press and the newest
`text` function is called, so a new one on every render ends no drag and
closes no open field. Give the input of `useValueEntry` no `value`: the
field sets its text itself.

React 19.2 or later is a peer dependency.
