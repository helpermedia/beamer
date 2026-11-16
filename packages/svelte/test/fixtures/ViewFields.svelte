<!-- Reports each field of a parameter's view from an effect of its own. -->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { parameterView } from "../../src/parameter-view";
  import type { ParameterView } from "../../src/parameter-view";

  let { parameter, record }: { parameter: Parameter; record: (field: string, value: unknown) => void } = $props();

  const view: ParameterView = $derived(parameterView(parameter));
  $effect(() => record("handle", view.parameter === parameter));
  $effect(() => record("value", view.value));
  $effect(() => record("displayText", view.displayText));
  $effect(() => record("text", view.text));
  // The runtime updates the info in place, so the effect copies what it reads.
  $effect(() => record("info", view.info && { name: view.info.name, displayText: view.info.displayText }));
</script>
