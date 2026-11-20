<!--
  The level of a drum as a console's fader with the meter of the drum
  beside it: a scale in dB, beamer-svelte's Slider and the meter. The thumb,
  the ticks and the meter sit where `thumbPosition` puts a value, so the
  thumb's center is on the tick of its level and the meter reads on the same
  scale. Drag the thumb or the track (finer with shift), scroll, use the
  arrow keys once focused, double click to reset.

  The slider is an SVG in device pixels, as the balance: every edge on a
  whole pixel, the thumb's height of the same parity as its line's
  thickness, so the line sits in the middle exactly (see pixels.ts). The
  ticks and the meter stay boxes of HTML.
-->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { thumbPosition } from "@helpermedia/beamer-controls";
  import { parameterView, Slider } from "@helpermedia/beamer-svelte";
  import { withMinus } from "../format";
  import { FLOOR_DB } from "../meters";
  import { atMostSameParity, measureDevicePixels, sameParity } from "../pixels";
  import type { DevicePixels } from "../pixels";

  let { parameter, level, name }: { parameter: Parameter; level: number; name: string } = $props();

  // The thumb's height in design pixels: the face above and below its line
  // grows with it.
  const THUMB_HEIGHT = 36;

  // The track's height in design pixels, the height of the whole fader
  // with its scale and meter (its CSS height below).
  const TRACK_HEIGHT = 208;

  // The thumb's length as a part of the track's, as the slider takes it.
  // Its center travels from half of it in from the bottom to half of it in
  // from the top.
  const THUMB = THUMB_HEIGHT / TRACK_HEIGHT;

  let track: HTMLDivElement | null = $state(null);
  let box: DevicePixels | null = $state(null);

  $effect(() => {
    if (track) return measureDevicePixels(track, (measured) => (box = measured));
  });

  // The drawing in device pixels for the thumb's center at `position` of
  // the track's height from the bottom. Every edge is on a whole pixel.
  function draw(pixels: DevicePixels, position: number) {
    const { width, height, unit } = pixels;
    // The line across: 12 design pixels long, one thick as the balance's (2
    // device pixels on a Retina screen, 1 on a plain one). The thumb's
    // height takes the parity of its thickness and is no taller than the
    // slider takes it to be (see atMostSameParity).
    const lineWidth = sameParity(12 * unit, width);
    const lineHeight = Math.max(1, Math.round(unit));
    const thumbHeight = atMostSameParity(THUMB * height, lineHeight);
    const thumbTop = Math.round((1 - position) * height - thumbHeight / 2);
    // A hairline ring, as the balance's.
    const border = 1;
    // The groove: 4 design pixels wide, along the thumb's travel.
    const grooveWidth = sameParity(4 * unit, width);
    const grooveTop = Math.round((THUMB / 2) * height);
    const grooveBottom = height - grooveTop;
    const center = thumbTop + thumbHeight / 2;
    return {
      thumb: { y: thumbTop, height: thumbHeight, border, radius: 3 * unit },
      line: { x: (width - lineWidth) / 2, y: thumbTop + (thumbHeight - lineHeight) / 2, width: lineWidth, height: lineHeight },
      groove: { x: (width - grooveWidth) / 2, y: grooveTop, width: grooveWidth, height: grooveBottom - grooveTop },
      fill: { y: center, height: Math.max(0, grooveBottom - center) },
    };
  }

  // The marks of the scale in dB.
  const TICKS = [6, 0, -12, -24, -36, -48, -60];

  // The level's range in dB, from the parameter (-60 to +6) once it
  // arrives with `ready`.
  const view = $derived(parameterView(parameter));
  const min = $derived(view.info?.min ?? FLOOR_DB);
  const max = $derived(view.info?.max ?? 6);

  // Where a level in dB sits along the track, as a part of its height from
  // the bottom: where the slider puts the thumb's center for it. The level
  // maps linearly in dB.
  const at = (db: number) => thumbPosition((db - min) / (max - min), THUMB);

  // A height from the bottom of the travel, in percent.
  const travel = (position: number) => `${(position - THUMB / 2) * 100}%`;
  const END = `${(THUMB / 2) * 100}%`;
</script>

<div class="flex gap-1.5" style="height: {TRACK_HEIGHT / 16}rem">
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

  <Slider {parameter} thumbLength={THUMB} class="group h-full w-6 outline-none" bind:ref={track}>
    {#snippet children({ position })}
      {#if box}
        {@const d = draw(box, position)}
        <!-- Visible overflow: the browser can clip the box a pixel short of its size. -->
        <svg
          viewBox="0 0 {box.width} {box.height}"
          class="block overflow-visible"
          style:width="{box.width / box.ratio}px"
          style:height="{box.height / box.ratio}px"
          aria-hidden="true"
        >
          <!-- The groove along the thumb's travel, blue up to the thumb. -->
          <rect
            x={d.groove.x}
            y={d.groove.y}
            width={d.groove.width}
            height={d.groove.height}
            rx={d.groove.width / 2}
            class="fill-neutral-800 group-data-[keyboard-focus]:fill-neutral-600"
          />
          <rect x={d.groove.x} y={d.fill.y} width={d.groove.width} height={d.fill.height} rx={d.groove.width / 2} class="fill-brand-blue" />
          <!-- The thumb, its border a ring around its face, with a line across like a dial's pointer. -->
          <rect x={0} y={d.thumb.y} width={box.width} height={d.thumb.height} rx={d.thumb.radius} class="fill-neutral-700" />
          <rect
            x={d.thumb.border}
            y={d.thumb.y + d.thumb.border}
            width={box.width - 2 * d.thumb.border}
            height={d.thumb.height - 2 * d.thumb.border}
            rx={Math.max(0, d.thumb.radius - d.thumb.border)}
            class="fill-neutral-800"
          />
          <rect x={d.line.x} y={d.line.y} width={d.line.width} height={d.line.height} rx={d.line.height / 2} class="fill-paper" />
        </svg>
      {/if}
    {/snippet}
  </Slider>

  <div
    class="relative h-full w-1.5"
    role="meter"
    aria-label="{name} level"
    aria-valuemin={min}
    aria-valuemax={max}
    aria-valuenow={Math.min(max, level)}
    aria-valuetext="{withMinus(level.toFixed(1))} dB"
  >
    <span class="absolute inset-x-0 rounded-full bg-neutral-800" style:top={END} style:bottom={END}></span>
    <span class="absolute inset-x-0 rounded-full bg-paper" style:bottom={END} style:height={travel(at(level))}></span>
  </div>
</div>
