<!--
  The level of a drum as a console's fader with the meter of its output
  beside it: a scale in dB, beamer-svelte's Slider and the meter. The thumb,
  the ticks and the meter sit where `thumbPosition` puts a value, so the
  thumb's center is on the tick of its level and the meter reads on the same
  scale. Drag the thumb or the track (finer with shift), scroll, use the
  arrow keys once focused, double click to reset.
-->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { thumbPosition } from "@helpermedia/beamer-controls";
  import { Slider } from "@helpermedia/beamer-svelte";
  import { withMinus } from "./format";
  import { FLOOR_DB } from "./meters";

  let { parameter, level, name }: { parameter: Parameter; level: number; name: string } = $props();

  // The thumb's length as a part of the track's. Its center travels from
  // half of it in from the bottom to half of it in from the top.
  const THUMB = 0.1;

  // The marks of the scale in dB.
  const TICKS = [6, 0, -12, -24, -36, -48, -60];

  // The level's range in dB, from the parameter (-60 to +6).
  const min = $derived(parameter.info?.min ?? FLOOR_DB);
  const max = $derived(parameter.info?.max ?? 6);

  // Where a level in dB sits along the track, as a part of its height from
  // the bottom: where the slider puts the thumb's center for it. The level
  // maps linearly in dB.
  const at = (db: number) => thumbPosition((db - min) / (max - min), THUMB);

  // A height from the bottom of the travel, in percent.
  const travel = (position: number) => `${(position - THUMB / 2) * 100}%`;
  const END = `${(THUMB / 2) * 100}%`;
</script>

<div class="flex h-full gap-1.5">
  <div class="relative h-full w-5" aria-hidden="true">
    {#each TICKS as db (db)}
      <span
        class="absolute right-0 translate-y-1/2 text-[0.5625rem] leading-none tabular-nums text-neutral-500"
        style:bottom="{at(db) * 100}%"
      >
        {db > 0 ? `+${db}` : withMinus(String(db))}
      </span>
    {/each}
  </div>

  <Slider {parameter} thumbLength={THUMB} class="group relative h-full w-6 outline-none">
    {#snippet children({ position })}
      <!-- The groove along the thumb's travel, blue up to the thumb. -->
      <span
        class="absolute inset-x-0 mx-auto w-1 rounded-full bg-neutral-800 group-data-[keyboard-focus]:bg-neutral-600"
        style:top={END}
        style:bottom={END}
      ></span>
      <span class="absolute inset-x-0 mx-auto w-1 rounded-full bg-brand-blue" style:bottom={END} style:height={travel(position)}
      ></span>
      <!-- The thumb, centered at `position`, with a line across like a dial's pointer. -->
      <span
        class="absolute inset-x-0 flex items-center justify-center rounded-[0.1875rem] bg-neutral-800 ring-1 ring-neutral-700 ring-inset"
        style:bottom={travel(position)}
        style:height="{THUMB * 100}%"
      >
        <span class="h-0.5 w-3 rounded-full bg-paper"></span>
      </span>
    {/snippet}
  </Slider>

  <div
    class="relative h-full w-1.5"
    role="meter"
    aria-label="{name} output"
    aria-valuemin={min}
    aria-valuemax={max}
    aria-valuenow={Math.min(max, level)}
    aria-valuetext="{withMinus(level.toFixed(1))} dB"
  >
    <span class="absolute inset-x-0 rounded-full bg-neutral-800" style:top={END} style:bottom={END}></span>
    <span class="absolute inset-x-0 rounded-full bg-paper" style:bottom={END} style:height={travel(at(level))}></span>
  </div>
</div>
