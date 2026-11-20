<!-- The text of each parameter, from a view made per item of an `{#each}`. -->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { parameterView } from "../../src/index";

  let { parameters }: { parameters: Parameter[] } = $props();
</script>

{#each parameters as item (item.id)}
  {@const view = parameterView(item)}
  <span>{view.text}</span>
{/each}
