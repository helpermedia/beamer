<!--
  The channel strip of one drum: its name with a light that flashes on each
  hit, its balance with its value to type, the fader of its level with its
  meter, the level as a value to type and the output it plays through. A
  double click on the balance or the fader opens its value to type one.
-->
<script lang="ts">
  import { parameter } from "@helpermedia/beamer-api";
  import { openTextField } from "@helpermedia/beamer-controls";
  import { parameterView } from "@helpermedia/beamer-svelte";
  import Balance from "./Balance.svelte";
  import type { Drum } from "../drums";
  import { flash } from "../flash";
  import Level from "./Level.svelte";
  import Output from "./Output.svelte";
  import TextField from "./TextField.svelte";

  let { drum, index, level }: { drum: Drum; index: number; level: number } = $props();

  const levelParameter = $derived(parameter(drum.levelParameterId));
  const panParameter = $derived(parameter(drum.panParameterId));
  const outputParameter = $derived(parameter(drum.outputParameterId));
  // The level parameter's name is the drum's, known once the parameters
  // arrive with `ready`.
  const levelView = $derived(parameterView(levelParameter));
  const name = $derived(levelView.info?.name ?? drum.levelParameterId);

  // A double click on the balance or the fader types a value in its field.
  let panField: HTMLInputElement | null = $state(null);
  let levelField: HTMLInputElement | null = $state(null);
  const typeIn = (field: HTMLInputElement | null) => {
    if (field) openTextField(field);
  };
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

  <div class="mt-2 w-full">
    <Balance parameter={panParameter} ondblclick={() => typeIn(panField)} />
  </div>

  <div class="mt-0.5 w-full">
    <TextField parameter={panParameter} bind:ref={panField} />
  </div>

  <div class="mt-1">
    <Level parameter={levelParameter} {level} {name} ondblclick={() => typeIn(levelField)} />
  </div>

  <div class="mt-1.5 w-full">
    <TextField parameter={levelParameter} bind:ref={levelField} />
  </div>

  <p class="mt-1.5 text-[0.625rem] text-neutral-500">Output</p>
  <div class="mt-1 w-full">
    <Output parameter={outputParameter} />
  </div>
</section>
