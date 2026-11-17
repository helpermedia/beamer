<!--
  The value of a control, beamer-svelte's TextField in the family look: a
  field that opens on a click. Enter or a click elsewhere sets what the user
  typed, read by the parameter's own parser, so "-12" and "-12 dB" both
  work. Escape leaves the value as it was. A text left as it was sets
  nothing, so no undo step.
-->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { TextField } from "@helpermedia/beamer-svelte";
  import { withMinus } from "../format";

  let { parameter }: { parameter: Parameter } = $props();
</script>

<TextField
  {parameter}
  text={(shown) => withMinus(shown.text())}
  title="Type a value"
  class="w-full cursor-default rounded-[0.1875rem] border border-transparent px-0 py-[0.0625rem] text-center text-xs font-medium tabular-nums text-paper outline-none hover:border-neutral-700 data-[keyboard-focus]:border-brand-blue data-[typing]:cursor-text data-[typing]:border-brand-blue data-[typing]:bg-panel data-[typing]:font-normal data-[typing]:select-text"
/>
