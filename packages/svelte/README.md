# @helpermedia/beamer-svelte

Unstyled Svelte components for the web UI of a
[Beamer](https://github.com/helpermedia/beamer) plugin, on top of
[`beamer-api`](../api) and [`beamer-controls`](../controls). Each component
renders one element with a control of `beamer-controls` on it: the role,
the ARIA attributes and the input, each change one undo step in the host.
They draw nothing: the look is yours, in the classes you give and the
markup you put inside.

## Example

```svelte
<script lang="ts">
  import { parameter } from "@helpermedia/beamer-api";
  import { Slider, TextField } from "@helpermedia/beamer-svelte";

  const level = parameter("level");
</script>

<!-- Drag the thumb or the track, scroll, arrow keys, double click to reset. -->
<Slider parameter={level} thumbLength={0.1} class="relative h-48 w-8">
  {#snippet children({ position })}
    <span class="absolute inset-x-0 h-[10%] bg-neutral-700" style:bottom="{(position - 0.05) * 100}%"></span>
  {/snippet}
</Slider>

<!-- The value, a click to type one. -->
<TextField parameter={level} />
```

## Components

| Component | Renders | Hands to its `children` snippet |
|-----------|---------|---------------------------------|
| `Dial` | A `<div>` with `attachDial` | `value`, `angle` (where it points, `dialAngle`), `text` |
| `Slider` | A `<div>` with `attachSlider`: the track | `value`, `position` (the thumb's center, `thumbPosition`), `text` |
| `Switch` | A `<button>` with `attachSwitch` | `checked` |
| `RadioGroup` | A `<div>` with `attachRadioGroup` | `choices` (the names), `checked` (the index) |
| `TextField` | An `<input>` with `attachTextField` | |

Each takes the parameter as `parameter` and the control's options as
props (`drag`, `arcStart`, `orientation`, `thumbLength`, `reset`, `text`,
`open` and the rest). Other props (`class`, `style`, `aria-*`, events) go
to the element; `bind:ref` gives the element. A control attaches again
when its parameter or an option changes. A `drag` or `text` function is
read when it is needed, so a new one does not attach the control again
(which would close an open text field).

Draw a dial or a slider inside its `<div>`, filling it: the drag measures
the `<div>`. The children of `RadioGroup` are its radios, one button per
choice in order, the first for the lowest value; a checked one has
`aria-checked="true"`. Text between them does not count.

The text field owns its text and whether it is read-only, so it takes
neither.

## Stores

The objects of `beamer-api` and `fallingLevels` follow Svelte's store
contract (`subscribe` calls back at once and returns the function that
unsubscribes), so a component reads them with `$` as they are, also with
runes:

```svelte
<script lang="ts">
  import { parameter } from "@helpermedia/beamer-api";
  import { fallingLevels } from "@helpermedia/beamer-controls";

  const mix = parameter("mix");
  const levels = fallingLevels("levels", { read: (value) => (typeof value === "number" ? value : -60) });
</script>

<p>{$mix} {$levels}</p>
```

A store must be a variable at the top of a component (a prop counts), so
a component in an `{#each}` takes the parameter as a prop. Name none of
them after a rune (`props`, `state`, `derived`, `effect`): `$props` is
the rune, not the store. Outside a component, Svelte's
`fromStore` makes one reactive.

Svelte 5.29 or later is a peer dependency (attachments).
