<!--
  A switch for an on and off parameter with a light: the family's look over
  beamer-svelte's Switch, a button whose text is the parameter's name.
-->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { Switch as SwitchControl, parameterView } from "@helpermedia/beamer-svelte";

  let { parameter }: { parameter: Parameter } = $props();

  const view = $derived(parameterView(parameter));
</script>

<SwitchControl
  {parameter}
  class="group flex items-center gap-2.5 rounded-lg bg-well px-3.5 py-2 text-sm font-medium text-paper ring-1 ring-neutral-800 ring-inset outline-none hover:bg-neutral-800 data-[keyboard-focus]:outline-2 data-[keyboard-focus]:outline-offset-2 data-[keyboard-focus]:outline-brand-blue data-[keyboard-focus]:outline-solid"
>
  <span
    aria-hidden="true"
    class="size-2 rounded-full bg-neutral-700 group-aria-checked:bg-brand-blue group-aria-checked:shadow-[0_0_0.375rem_var(--color-brand-blue)]"
  ></span>
  {view.info?.name ?? parameter.id}
</SwitchControl>
