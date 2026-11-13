<!-- A value field with an optional prefix in its text. -->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { ValueEntry } from "../../src/index";

  let {
    parameter,
    prefix = "",
    open,
    onRef,
  }: { parameter: Parameter; prefix?: string; open?: "click" | "double-click"; onRef?: (input: HTMLInputElement | null) => void } =
    $props();
  let ref: HTMLInputElement | null = $state(null);
  $effect(() => onRef?.(ref));
</script>

<ValueEntry {parameter} {open} text={(shown) => `${prefix}${shown.text()}`} class="field" bind:ref />
