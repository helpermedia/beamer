<!-- A text field with an optional prefix in its text. -->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { TextField } from "../../src/index";

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

<TextField {parameter} {open} text={(target) => `${prefix}${target.text()}`} class="field" bind:ref />
