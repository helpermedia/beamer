<!--
  The channel strip of one drum: its name with a light that flashes on each
  hit, its balance, the fader of its level with its meter, the level as a
  value to type and the output it plays through.
-->
<script lang="ts">
  import { parameter } from "@helpermedia/beamer-api";
  import Balance from "./Balance.svelte";
  import type { Drum } from "../drums";
  import { flash } from "../flash";
  import Level from "./Level.svelte";
  import OutputChoices from "./OutputChoices.svelte";
  import ValueText from "./ValueText.svelte";

  let { drum, index, level }: { drum: Drum; index: number; level: number } = $props();

  const levelParameter = $derived(parameter(drum.levelParameterId));
  const panParameter = $derived(parameter(drum.panParameterId));
  const outputParameter = $derived(parameter(drum.outputParameterId));
  // The level parameter's name is the drum's. The strips render after
  // `ready`, so the name is known.
  const name = $derived(levelParameter.info?.name ?? drum.levelParameterId);
</script>

<section aria-label={name} class="flex w-30 flex-col items-center rounded-lg bg-well px-2 pt-2 pb-2.5 ring-1 ring-neutral-800 ring-inset">
  <div class="flex items-center gap-1.5">
    <!-- The light, in the brand blue with the glow of the switches' lights. -->
    <span aria-hidden="true" class="relative size-1.5 rounded-full bg-neutral-700">
      <span
        class="absolute inset-0 rounded-full bg-brand-blue opacity-0 shadow-[0_0_0.375rem_var(--color-brand-blue)]"
        {@attach flash(index)}
      ></span>
    </span>
    <h2 class="text-xs font-semibold">{name}</h2>
  </div>

  <div class="mt-2.5 w-full">
    <Balance parameter={panParameter} />
  </div>

  <div class="mt-3">
    <Level parameter={levelParameter} {level} {name} />
  </div>

  <div class="mt-2 w-full">
    <ValueText parameter={levelParameter} />
  </div>

  <p class="mt-2 text-[0.625rem] text-neutral-500">Output</p>
  <div class="mt-1 w-full">
    <OutputChoices parameter={outputParameter} />
  </div>
</section>
