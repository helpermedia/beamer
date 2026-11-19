<!--
  The drum synth's editor in two tabs: Pads with a pad per drum to play
  it, Mixer with a channel strip of one shape per drum. The audio thread
  sends the hits (the pads and the strips' lights flash) and the level of
  each drum (the meters). The tabs follow ARIA's tabs pattern: one tab
  stop, the arrow keys move between the tabs and show the one they reach.
-->
<script lang="ts">
  import { parameter, ready, stateField } from "@helpermedia/beamer-api";
  // The Beamer logo for dark grounds, a copy of
  // docs/branding/beamer-on-dark.svg so that the example works on its own.
  import beamerLogo from "./beamer-logo.svg";
  import { DRUMS } from "./drums";
  import { meters } from "./meters";
  import Pad from "./components/Pad.svelte";
  import Strip from "./components/Strip.svelte";

  // A tab as the `State` field holds it: the variants of `Tab` in
  // src/lib.rs in JSON.
  type Tab = "pads" | "mixer";

  const TABS: { id: Tab; label: string }[] = [
    { id: "pads", label: "Pads" },
    { id: "mixer", label: "Mixer" },
  ];

  // The tab the editor shows, kept in the `State` field "tab" and saved
  // with the project. Pads before `ready` and for a value it does not know.
  const tab = stateField<Tab>("tab");
  const shown: Tab = $derived(TABS.find(({ id }) => id === $tab)?.id ?? "pads");

  // The tab buttons in order, to move the focus between them.
  const buttons: HTMLButtonElement[] = $state([]);

  // The arrow keys go to the tab before or after, around at the ends; Home
  // and End go to the first and the last. The tab they reach shows at once.
  // Shortcuts with Command, Control or Alt belong to the page.
  function onkeydown(event: KeyboardEvent, index: number) {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const last = TABS.length - 1;
    const next =
      event.key === "ArrowRight"
        ? (index + 1) % TABS.length
        : event.key === "ArrowLeft"
          ? (index + last) % TABS.length
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? last
              : -1;
    if (next < 0) return;
    event.preventDefault();
    tab.set(TABS[next].id);
    buttons[next]?.focus();
  }

  // The name of a drum: its level parameter's. Read after `ready`.
  const nameOf = (levelParameterId: string) => parameter(levelParameterId).info?.name ?? levelParameterId;
</script>

<!-- The bar spans the window; its content lines up with the panel. -->
<header class="h-[3.25rem] shrink-0 border-b border-neutral-800 bg-bar">
  <div class="mx-auto flex h-full w-[35rem] items-center px-5">
    <h1 class="text-[0.9375rem] font-semibold tracking-tight">Drum Synth</h1>
    <!-- The tabs come with the `State` field, so they never show the wrong one first. A segmented
         control in a well, as each strip's output, so that they read as a control, not as a title. -->
    {#await ready() then}
      <div role="tablist" aria-label="View" class="ml-6 flex gap-0.5 rounded-md bg-panel p-0.5 ring-1 ring-neutral-800 ring-inset">
        {#each TABS as { id, label }, index (id)}
          <button
            type="button"
            role="tab"
            id="tab-{id}"
            aria-controls="panel-{id}"
            aria-selected={shown === id}
            tabindex={shown === id ? 0 : -1}
            class="rounded px-3 py-1 text-xs font-medium text-neutral-400 outline-none hover:text-paper focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-solid focus-visible:outline-brand-blue aria-selected:bg-neutral-700 aria-selected:text-paper"
            bind:this={buttons[index]}
            onclick={() => tab.set(id)}
            onkeydown={(event) => onkeydown(event, index)}
          >
            {label}
          </button>
        {/each}
      </div>
    {/await}
    <img src={beamerLogo} alt="Beamer" class="ml-auto h-3.5" />
  </div>
</header>

<!-- The parameters arrive with `ready`. Until then the panel stays empty. -->
{#await ready() then}
  <main class="mx-auto my-auto h-[28rem] w-[35rem] shrink-0 px-5 py-4">
    <div
      role="tabpanel"
      id="panel-pads"
      aria-labelledby="tab-pads"
      hidden={shown !== "pads"}
      class="grid h-full grid-cols-2 grid-rows-2 gap-4"
    >
      {#each DRUMS as drum, index (drum.levelParameterId)}
        <Pad drum={index} note={drum.note} name={nameOf(drum.levelParameterId)} />
      {/each}
    </div>

    <div
      role="tabpanel"
      id="panel-mixer"
      aria-labelledby="tab-mixer"
      hidden={shown !== "mixer"}
      class="flex h-full flex-col justify-center"
    >
      <div class="flex justify-between">
        {#each DRUMS as drum, index (drum.levelParameterId)}
          <Strip {drum} {index} level={$meters[index]} />
        {/each}
      </div>
      <p class="mt-3 text-[0.6875rem] text-neutral-500">
        To start with, every drum plays through Output 1, the main output. Send a drum to an output of its own under its
        fader, then turn that output on in the host.
      </p>
    </div>
  </main>
{/await}
