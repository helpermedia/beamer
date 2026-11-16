<!--
  A slider for `parameter`: a `<div>` with the input of `attachSlider` (the
  thumb follows the pointer along it, wheel, keys, reset) and ARIA's slider
  role. The `<div>` is the track: draw the slider inside it, filling it,
  with the thumb's center at `position` of its length. The `children`
  snippet gets the value, the position and the text. The slider's options
  are props; other props go to the `<div>`, `bind:ref` gives it.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import type { Parameter } from "@helpermedia/beamer-api";
  import { thumbPosition } from "@helpermedia/beamer-controls";
  import type { SliderOptions } from "@helpermedia/beamer-controls";
  import { slider } from "./attachments.js";
  import { parameterView } from "./parameter-view.js";
  import type { SliderState } from "./types.js";

  type Props = SliderOptions &
    Omit<HTMLAttributes<HTMLDivElement>, "children"> & {
      parameter: Parameter;
      /** The drawing of the slider, from the values to draw it with. */
      children?: Snippet<[SliderState]>;
      /** The `<div>`, for `bind:ref`. */
      ref?: HTMLDivElement | null;
    };

  let {
    parameter,
    children,
    ref = $bindable(null),
    orientation,
    thumbLength,
    dragStep,
    fineFactor,
    keyStep,
    pageStep,
    wheelCap,
    wheelRestMs,
    reset,
    ...rest
  }: Props = $props();

  const view = $derived(parameterView(parameter));
  // The options are plain values, so their text tells a change from a new
  // object or array with the same content.
  const options = $derived(
    JSON.stringify({ orientation, thumbLength, dragStep, fineFactor, keyStep, pageStep, wheelCap, wheelRestMs, reset }),
  );
  const attach = $derived(slider(parameter, JSON.parse(options) as SliderOptions));
</script>

<!-- The gestures set `touch-action: none`; a `style` prop that changes would drop it. -->
<div {...rest} style:touch-action="none" bind:this={ref} {@attach attach}>
  {@render children?.({ value: view.value, position: thumbPosition(view.value, thumbLength), text: view.text })}
</div>
