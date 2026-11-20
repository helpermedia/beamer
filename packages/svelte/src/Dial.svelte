<!--
  A dial for `parameter`: a `<div>` with the input of `attachDial` (drag,
  wheel, keys, reset) and ARIA's slider role. Draw the dial inside it,
  filling it: the drag measures the `<div>`. The `children` snippet gets
  the value, where the dial points and the text. The dial's options are
  props; other props go to the `<div>`, `bind:ref` gives it.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import type { Parameter } from "@helpermedia/beamer-api";
  import { dialAngle } from "@helpermedia/beamer-controls";
  import type { DialDrag, DialOptions } from "@helpermedia/beamer-controls";
  import { dial } from "./attachments.js";
  import type { EveryKeyOf, FunctionKeys, NoKeyLeft } from "./option-keys.js";
  import { parameterView } from "./parameter-view.js";
  import type { DialState } from "./types.js";

  type Props = DialOptions &
    Omit<HTMLAttributes<HTMLDivElement>, "children"> & {
      parameter: Parameter;
      /** The drawing of the dial, from the values to draw it with. */
      children?: Snippet<[DialState]>;
      /** The `<div>`, for `bind:ref`. */
      ref?: HTMLDivElement | null;
    };

  let {
    parameter,
    children,
    ref = $bindable(null),
    drag,
    arcStart,
    arcSweep,
    arcDeadZone,
    dragStep,
    fineFactor,
    keyStep,
    pageStep,
    wheelCap,
    wheelRestMs,
    reset,
    ...rest
  }: Props = $props();
  // Every option is taken above, none goes to the `<div>`.
  type _OptionsTaken = NoKeyLeft<Extract<keyof typeof rest, keyof DialOptions>>;

  const view = $derived(parameterView(parameter));
  // The drag is picked for each press from the newest `drag`, so a new
  // function does not attach the dial again.
  const pickDrag = (event: PointerEvent): DialDrag => (typeof drag === "function" ? drag(event) : (drag ?? "vertical"));
  // The other options are plain values, so their text tells a change from
  // a new object or array with the same content.
  type OtherOptions = Omit<DialOptions, "drag">;
  type _OthersPlain = NoKeyLeft<FunctionKeys<OtherOptions>>;
  const options = $derived(
    JSON.stringify({
      arcStart,
      arcSweep,
      arcDeadZone,
      dragStep,
      fineFactor,
      keyStep,
      pageStep,
      wheelCap,
      wheelRestMs,
      reset,
    } satisfies EveryKeyOf<OtherOptions>),
  );
  const attach = $derived(dial(parameter, { ...(JSON.parse(options) as DialOptions), drag: pickDrag }));
</script>

<!-- The gestures set `touch-action: none`; a `style` prop that changes would drop it. -->
<div {...rest} style:touch-action="none" bind:this={ref} {@attach attach}>
  {@render children?.({ value: view.value, angle: dialAngle(view.value, arcStart, arcSweep), text: view.text })}
</div>
