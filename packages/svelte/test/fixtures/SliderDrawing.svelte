<!-- A slider whose drawing reports what it gets. -->
<script lang="ts">
  import type { SliderOptions } from "@helpermedia/beamer-controls";
  import type { Parameter } from "@helpermedia/beamer-api";
  import { Slider } from "../../src/index";
  import type { SliderState } from "../../src/index";

  let { parameter, record, ...options }: SliderOptions & { parameter: Parameter; record: (state: SliderState) => void } =
    $props();
</script>

<Slider {parameter} {...options}>
  {#snippet children(state)}
    <svg data-seen={record(state)}></svg>
  {/snippet}
</Slider>
