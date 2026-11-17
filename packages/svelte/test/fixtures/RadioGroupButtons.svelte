<!-- A radio group of one button per choice of the parameter. -->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { RadioGroup } from "../../src/index";
  import type { RadioGroupState } from "../../src/index";

  let { parameter, record }: { parameter: Parameter; record?: (state: RadioGroupState) => void } = $props();
</script>

<RadioGroup {parameter}>
  {#snippet children(state)}
    <!-- A text node, as the element children are the radios. -->
    {record?.(state) ?? ""}
    {#each state.choices as name, index (name)}
      <button type="button" data-checked={index === state.checked ? "" : undefined}>{name}</button>
    {/each}
  {/snippet}
</RadioGroup>
