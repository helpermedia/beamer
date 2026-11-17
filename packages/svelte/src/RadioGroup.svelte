<!--
  A radio group for the discrete `parameter`: a `<div>` with
  `attachRadioGroup`. Its children become its radios, one button per choice
  in order, the first for the lowest value (`aria-checked` on the checked
  one, arrow keys, one tab stop). The `children` snippet gets the names of
  the choices and the index of the checked one. Other props go to the
  `<div>`, `bind:ref` gives it.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import type { Parameter } from "@helpermedia/beamer-api";
  import { radioGroup } from "./attachments.js";
  import { parameterView } from "./parameter-view.js";
  import type { RadioGroupState } from "./types.js";

  type Props = Omit<HTMLAttributes<HTMLDivElement>, "children"> & {
    parameter: Parameter;
    /** One button per choice: names of the parameter's choices or names of one's own. */
    children?: Snippet<[RadioGroupState]>;
    /** The `<div>`, for `bind:ref`. */
    ref?: HTMLDivElement | null;
  };

  let { parameter, children, ref = $bindable(null), ...rest }: Props = $props();

  const view = $derived(parameterView(parameter));
  const attach = $derived(radioGroup(parameter));
  // The choices and the checked one (`parameter.choice()` from the value),
  // none before `ready` or for a continuous parameter.
  const state: RadioGroupState = $derived.by(() => {
    const steps = view.info?.steps ?? 0;
    return {
      choices: parameter.choices(),
      checked: steps > 0 ? Math.round(view.value * steps) : -1,
    };
  });
</script>

<div {...rest} bind:this={ref} {@attach attach}>
  {@render children?.(state)}
</div>
