<!--
  The drums editor: four channel strips of one shape, one per drum in the
  order of the output buses. The audio thread sends the hits (the lamps)
  and the level of each output (the meters).
-->
<script lang="ts">
  import { ready } from "@helpermedia/beamer-api";
  // The Beamer logo for dark grounds, a copy of
  // docs/branding/beamer-on-dark.svg so that the example works on its own.
  import beamerLogo from "./beamer-logo.svg";
  import { DRUMS } from "./drums";
  import { meters } from "./meters";
  import Strip from "./Strip.svelte";
</script>

<!-- The bar spans the window; its content lines up with the panel. -->
<header class="h-[3.25rem] shrink-0 border-b border-neutral-800 bg-bar">
  <div class="mx-auto flex h-full w-[35rem] items-center justify-between px-5">
    <h1 class="text-[0.9375rem] font-semibold tracking-tight">Drum Synth</h1>
    <img src={beamerLogo} alt="Beamer" class="h-3.5" />
  </div>
</header>

<!-- The parameters arrive with `ready`. Until then the panel stays empty. -->
{#await ready() then}
  <main class="mx-auto my-auto flex w-[35rem] shrink-0 flex-col px-5 py-4">
    <div class="flex justify-between">
      {#each DRUMS as drum, index (drum.parameterId)}
        <Strip {drum} {index} level={$meters[index]} />
      {/each}
    </div>
    <p class="mt-3 text-[0.6875rem] text-neutral-500">
      The kick plays through the main output, the other drums through outputs of their own: turn those on in the host to hear
      them.
    </p>
  </main>
{/await}
