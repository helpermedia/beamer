<!--
  The text field of a control: the look over beamer-svelte's TextField,
  which shows the value and opens on a click. Enter or a click elsewhere
  sets what the user typed, read by the parameter's own parser, so "442"
  and "442 Hz" both work. Escape leaves the value as it was. A text left
  as it was sets nothing, so no undo step.
-->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { TextField as TextFieldControl } from "@helpermedia/beamer-svelte";
  import { withMinus } from "../format";

  // `ref` is the `<input>`, for its dial to open it on a double click.
  let { parameter, ref = $bindable(null) }: { parameter: Parameter; ref?: HTMLInputElement | null } = $props();
</script>

<TextFieldControl
  {parameter}
  bind:ref
  text={(shown) => withMinus(shown.text())}
  title="Type a value"
  class="-ml-1 w-24 cursor-default rounded-[0.1875rem] border border-transparent px-1 py-[0.0625rem] text-[0.9375rem] font-medium tabular-nums text-paper outline-none hover:border-neutral-700 data-[keyboard-focus]:border-brand-blue data-[typing]:cursor-text data-[typing]:border-brand-blue data-[typing]:bg-panel data-[typing]:font-normal data-[typing]:select-text"
/>
