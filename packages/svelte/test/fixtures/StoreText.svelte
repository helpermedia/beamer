<!-- Reads a parameter and a meter as Svelte stores. -->
<script lang="ts">
  import type { Parameter, Readable } from "@helpermedia/beamer-api";

  let { parameter, meter }: { parameter: Parameter; meter: Readable<number> } = $props();
</script>

<span>{$parameter} {$meter}</span>
