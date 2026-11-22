<!--
  A dial for a continuous parameter with its name and value beside it: the
  family's classic look over beamer-svelte's Dial, which brings the input:
  drag up and down (finer with shift), scroll, arrow keys once focused, a
  Command or Option click (Control or Alt outside macOS) to reset. A double
  click on the dial or a click on the value types one.
-->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { openTextField } from "@helpermedia/beamer-controls";
  import type { ResetGesture } from "@helpermedia/beamer-controls";
  import { Dial as DialControl, parameterView } from "@helpermedia/beamer-svelte";
  import TextField from "./TextField.svelte";

  // The arc of a dial in degrees clockwise from the right, as the gestures
  // count them: 270, open at the bottom.
  const ARC_START = 135;
  const ARC_SWEEP = 270;

  // The dial in SVG units: rem-sized pixels at the editor's own size.
  const SIZE = 48;
  const CENTER = SIZE / 2;
  const RING = 21;

  // A double click types a value, so only a click with a modifier resets.
  const TYPING_RESETS: ResetGesture[] = ["ctrl-click", "alt-click"];

  let { parameter }: { parameter: Parameter } = $props();

  let field: HTMLInputElement | null = $state(null);
  const typeValue = () => {
    if (field) openTextField(field);
  };

  const view = $derived(parameterView(parameter));
  const name = $derived(view.info?.name ?? parameter.id);

  function pointAt(degrees: number, radius: number): [number, number] {
    const radians = (degrees * Math.PI) / 180;
    return [CENTER + radius * Math.cos(radians), CENTER + radius * Math.sin(radians)];
  }

  // An SVG path along the ring, from its start to `part` of it.
  function ringPath(part: number): string {
    const [x0, y0] = pointAt(ARC_START, RING);
    const sweep = part * ARC_SWEEP;
    const [x1, y1] = pointAt(ARC_START + sweep, RING);
    return `M ${x0} ${y0} A ${RING} ${RING} 0 ${sweep > 180 ? 1 : 0} 1 ${x1} ${y1}`;
  }

  const TRACK = ringPath(1);
</script>

<div class="flex items-center gap-3">
  <DialControl
    {parameter}
    arcStart={ARC_START}
    arcSweep={ARC_SWEEP}
    reset={TYPING_RESETS}
    ondblclick={typeValue}
    aria-label={name}
    class="group size-11 outline-none"
  >
    {#snippet children({ value, angle })}
      {@const pointer = [...pointAt(angle, 7), ...pointAt(angle, 14)]}
      <svg viewBox="0 0 {SIZE} {SIZE}" class="size-full">
        <path
          d={TRACK}
          class="fill-none stroke-neutral-800 group-data-[keyboard-focus]:stroke-neutral-600"
          stroke-width="3"
          stroke-linecap="round"
        />
        {#if value > 0.001}
          <path d={ringPath(value)} class="fill-none stroke-brand-blue" stroke-width="3" stroke-linecap="round" />
        {/if}
        <circle cx={CENTER} cy={CENTER} r="16" class="fill-neutral-800 stroke-neutral-700" stroke-width="1" />
        <line x1={pointer[0]} y1={pointer[1]} x2={pointer[2]} y2={pointer[3]} class="stroke-paper" stroke-width="2.5" stroke-linecap="round" />
      </svg>
    {/snippet}
  </DialControl>
  <div class="flex flex-col items-start">
    <span class="text-xs text-neutral-400">{name}</span>
    <TextField {parameter} bind:ref={field} />
  </div>
</div>
