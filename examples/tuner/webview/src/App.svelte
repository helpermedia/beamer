<!--
  The tuner's editor: the display of what the processor hears, the
  reference pitch and the mute. The audio thread sends a pitch about 50
  times a second while the editor is open; the display works out the note
  and the cents from it with the reference.
-->
<script lang="ts">
  import { parameter, ready } from "@helpermedia/beamer-api";
  // The Beamer logo for dark grounds, a copy of
  // docs/branding/beamer-on-dark.svg so that the example works on its own.
  import beamerLogo from "./beamer-logo.svg";
  import Dial from "./components/Dial.svelte";
  import Display from "./components/Display.svelte";
  import Switch from "./components/Switch.svelte";

  // The reference has a step of 0.1 Hz: its arrow keys move one step, as
  // the plugin works it out.
  const reference = parameter("reference");
  const mute = parameter("mute");
</script>

<!-- The bar spans the window; its content lines up with the panel. -->
<header class="h-[3.25rem] shrink-0 border-b border-neutral-800 bg-bar">
  <div class="mx-auto flex h-full w-[32.5rem] items-center px-5">
    <h1 class="text-[0.9375rem] font-semibold tracking-tight">Tuner</h1>
    <img src={beamerLogo} alt="Beamer" class="ml-auto h-3.5" />
  </div>
</header>

<!-- The parameters arrive with `ready`. Until then the panel stays empty. -->
{#await ready() then}
  <main class="mx-auto my-auto h-[19.25rem] w-[32.5rem] shrink-0 px-5 pt-4">
    <div class="h-[12.75rem] overflow-hidden rounded-xl border border-neutral-800 bg-well">
      <Display />
    </div>
    <div class="flex h-[4.5rem] items-center">
      <Dial parameter={reference} />
      <div class="ml-auto">
        <Switch parameter={mute} />
      </div>
    </div>
  </main>
{/await}
