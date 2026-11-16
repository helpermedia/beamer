<!--
  The output a drum plays through, as a radio group on its output
  parameter: Output 1 (the main output) to Output 4. The buttons show the
  numbers, each with its full name for a screen reader.
-->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { Choices } from "@helpermedia/beamer-svelte";

  let { parameter }: { parameter: Parameter } = $props();

  // The text and the label of each choice, from the lowest value up, as
  // the parameter's choices: Output 1 to Output 4.
  const OUTPUTS = [
    ["1", "Output 1"],
    ["2", "Output 2"],
    ["3", "Output 3"],
    ["4", "Output 4"],
  ];
</script>

<Choices {parameter} class="flex w-full gap-0.5 rounded-md bg-panel p-0.5 ring-1 ring-neutral-800 ring-inset">
  {#snippet children()}
    {#each OUTPUTS as [text, label] (text)}
      <button
        type="button"
        aria-label={label}
        class="flex-auto rounded px-1 py-1 text-[0.625rem] font-medium tabular-nums text-neutral-400 outline-none hover:text-paper data-[keyboard-focus]:outline-2 data-[keyboard-focus]:outline-solid data-[keyboard-focus]:outline-brand-blue aria-checked:bg-brand-blue aria-checked:text-white"
      >
        {text}
      </button>
    {/each}
  {/snippet}
</Choices>
