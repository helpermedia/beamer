<!-- A radio group of one button per choice of the parameter. -->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { Choices } from "../../src/index";
  import type { ChoicesState } from "../../src/index";

  let { parameter, record }: { parameter: Parameter; record?: (state: ChoicesState) => void } = $props();
</script>

<Choices {parameter}>
  {#snippet children(state)}
    <!-- A text node, as the element children are the choices. -->
    {record?.(state) ?? ""}
    {#each state.choices as name, index (name)}
      <button type="button" data-checked={index === state.checked ? "" : undefined}>{name}</button>
    {/each}
  {/snippet}
</Choices>
