<!--
  A switch for the on and off `parameter`: a `<button>` with `attachSwitch`
  (`role="switch"`, `aria-checked` for the style of the on state, each
  click one undo step). Its text is its name. The `children` snippet gets
  whether it is on. Other props go to the `<button>`, `bind:ref` gives it.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";
  import type { Parameter } from "@helpermedia/beamer-api";
  import { onOffSwitch } from "./attachments.js";
  import { shown } from "./shown.js";
  import type { SwitchState } from "./types.js";

  type Props = Omit<HTMLButtonAttributes, "children" | "type"> & {
    parameter: Parameter;
    /** The switch's name and look, from whether it is on. */
    children?: Snippet<[SwitchState]>;
    /** The `<button>`, for `bind:ref`. */
    ref?: HTMLButtonElement | null;
  };

  let { parameter, children, ref = $bindable(null), ...rest }: Props = $props();

  const view = $derived(shown(parameter));
  const attach = $derived(onOffSwitch(parameter));
</script>

<button {...rest} type="button" bind:this={ref} {@attach attach}>
  {@render children?.({ checked: view.value >= 0.5 })}
</button>
