<!-- Reads a parameter and meter levels as Svelte stores. -->
<script lang="ts">
  import type { Parameter, Readable } from "@helpermedia/beamer-api";

  let { parameter, levels }: { parameter: Parameter; levels: Readable<number> } = $props();
</script>

<span>{$parameter} {$levels}</span>
