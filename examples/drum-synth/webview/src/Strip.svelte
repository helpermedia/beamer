<!--
  The channel strip of one drum: a lamp that flashes on each hit, the
  drum's name and note, the fader with its output's meter, the level as a
  value to type and the output the drum plays through.
-->
<script lang="ts">
  import { noteName, parameter } from "@helpermedia/beamer-api";
  import type { Drum } from "./drums";
  import Fader from "./Fader.svelte";
  import { lamp } from "./lamp";
  import ValueField from "./ValueField.svelte";

  let { drum, index, level }: { drum: Drum; index: number; level: number } = $props();

  const levelParameter = $derived(parameter(drum.parameterId));
  // The parameter's name is the drum's and its output bus's. The strips
  // render after `ready`, so the name is known.
  const name = $derived(levelParameter.info?.name ?? drum.parameterId);
</script>

<section aria-label={name} class="flex w-28 flex-col items-center rounded-lg bg-well px-2 pt-3 pb-2.5 ring-1 ring-neutral-800 ring-inset">
  <div class="flex items-center gap-2">
    <span aria-hidden="true" class="relative size-2 rounded-full bg-neutral-700">
      <span
        class="absolute inset-0 rounded-full bg-brand-blue opacity-0 shadow-[0_0_0.375rem_var(--color-brand-blue)]"
        {@attach lamp(index)}
      ></span>
    </span>
    <h2 class="text-xs font-semibold text-paper">{name}</h2>
  </div>
  <p class="mt-0.5 text-[0.625rem] tabular-nums text-neutral-500">{noteName(drum.note)} · {drum.note}</p>

  <div class="mt-3 h-52">
    <Fader parameter={levelParameter} {level} {name} />
  </div>

  <div class="mt-2 w-full">
    <ValueField parameter={levelParameter} />
  </div>
  <p class="mt-0.5 text-[0.625rem] text-neutral-500">Out {index + 1} · {index === 0 ? "Main" : "Aux"}</p>
</section>
