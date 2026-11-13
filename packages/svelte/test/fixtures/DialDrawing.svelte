<!-- A dial whose drawing reports what it gets. -->
<script lang="ts">
  import type { DialOptions } from "@helpermedia/beamer-controls";
  import type { Parameter } from "@helpermedia/beamer-api";
  import { Dial } from "../../src/index";
  import type { DialState } from "../../src/index";

  let {
    parameter,
    record,
    ...options
  }: DialOptions & { parameter: Parameter; record: (state: DialState) => void; class?: string; style?: string; ondblclick?: () => void } =
    $props();
</script>

<Dial {parameter} {...options}>
  {#snippet children(state)}
    <svg data-seen={record(state)}></svg>
  {/snippet}
</Dial>
