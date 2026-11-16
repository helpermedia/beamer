<!--
  The balance of a drum, its pan parameter, as a small lying slider:
  beamer-svelte's Slider with `orientation="horizontal"`, between an L and
  an R. A groove with a mark in the middle, blue from the middle to the
  thumb, the thumb drawn with its center at `position`. Drag the thumb or
  the groove (finer with shift), scroll, use the arrow keys once focused,
  double click to set it back to where the drum starts.

  The drawing is an SVG in device pixels, every edge on a whole pixel, the
  thumb's width of the same parity as its line's, so the line sits in the
  middle of the thumb exactly (see pixels.ts).
-->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { Slider } from "@helpermedia/beamer-svelte";
  import { atMostSameParity, measureDevicePixels, sameParity } from "../pixels";
  import type { DevicePixels } from "../pixels";

  let { parameter }: { parameter: Parameter } = $props();

  // The track's width in design pixels (its CSS width below); the
  // drawing's sizes are in them too.
  const WIDTH = 84;

  // The thumb's width and its part of the track's length, as the slider
  // takes it. Its center travels from half of it in from the left to half
  // of it in from the right.
  const THUMB_WIDTH = 12;
  const THUMB = THUMB_WIDTH / WIDTH;

  // The thumb's height in design pixels, the track's height too: the face
  // above and below the line grows with it.
  const THUMB_HEIGHT = 18;

  // The length of the line across the thumb in design pixels.
  const LINE_LENGTH = 8;

  let track: HTMLDivElement | null = $state(null);
  let box: DevicePixels | null = $state(null);

  $effect(() => {
    if (track) return measureDevicePixels(track, (measured) => (box = measured));
  });

  // The drawing in device pixels for the thumb's center at `position`.
  // Every edge is on a whole pixel.
  function draw(pixels: DevicePixels, position: number) {
    const { width, height, unit } = pixels;
    // The line: one design pixel wide (2 device pixels on a Retina screen,
    // 1 on a plain one). The thumb's width takes its parity and is no wider
    // than the slider takes it to be (see atMostSameParity).
    const lineWidth = Math.max(1, Math.round(unit));
    const thumbWidth = atMostSameParity(THUMB * width, lineWidth);
    const thumbLeft = Math.round(position * width - thumbWidth / 2);
    // A hairline: one device pixel at every size, so the thumb does not
    // look ringed with light around its line.
    const border = 1;
    const lineHeight = sameParity(LINE_LENGTH * unit, height);
    const grooveHeight = sameParity(4 * unit, height);
    // The groove runs along the thumb's travel, the same length in from
    // both ends.
    const grooveLeft = Math.round((THUMB / 2) * width);
    // The mark of the middle, of the track's parity, so the middle is its
    // middle exactly.
    const markWidth = sameParity(unit, width);
    const markTop = Math.round(2 * unit);
    const middle = width / 2;
    const center = thumbLeft + thumbWidth / 2;
    return {
      thumb: { x: thumbLeft, width: thumbWidth, border, radius: 2.5 * unit },
      line: { x: thumbLeft + (thumbWidth - lineWidth) / 2, y: (height - lineHeight) / 2, width: lineWidth, height: lineHeight },
      groove: { x: grooveLeft, y: (height - grooveHeight) / 2, width: width - 2 * grooveLeft, height: grooveHeight },
      fill: { x: Math.min(center, middle), width: Math.abs(center - middle) },
      mark: { x: (width - markWidth) / 2, y: markTop, width: markWidth, height: height - 2 * markTop },
    };
  }
</script>

<div class="flex w-full items-center justify-between text-[0.5625rem] leading-none text-neutral-500">
  <span aria-hidden="true">L</span>
  <Slider
    {parameter}
    orientation="horizontal"
    thumbLength={THUMB}
    class="group outline-none"
    style="width: {WIDTH / 16}rem; height: {THUMB_HEIGHT / 16}rem"
    bind:ref={track}
  >
    {#snippet children({ position })}
      {#if box}
        {@const d = draw(box, position)}
        <!-- Visible overflow: the browser can clip the box a pixel short of its size
             (at 500 wide the thumb's bottom ring was cut off). -->
        <svg
          viewBox="0 0 {box.width} {box.height}"
          class="block overflow-visible"
          style:width="{box.width / box.ratio}px"
          style:height="{box.height / box.ratio}px"
          aria-hidden="true"
        >
          <!-- The groove along the thumb's travel, blue from the middle to the thumb. -->
          <rect
            x={d.groove.x}
            y={d.groove.y}
            width={d.groove.width}
            height={d.groove.height}
            rx={d.groove.height / 2}
            class="fill-neutral-800 group-data-[keyboard-focus]:fill-neutral-600"
          />
          <rect x={d.fill.x} y={d.groove.y} width={d.fill.width} height={d.groove.height} class="fill-brand-blue" />
          <!-- The mark of the middle. -->
          <rect x={d.mark.x} y={d.mark.y} width={d.mark.width} height={d.mark.height} class="fill-neutral-600" />
          <!-- The thumb, its border a ring around its face, with a line across like the fader's. -->
          <rect x={d.thumb.x} y={0} width={d.thumb.width} height={box.height} rx={d.thumb.radius} class="fill-neutral-700" />
          <rect
            x={d.thumb.x + d.thumb.border}
            y={d.thumb.border}
            width={d.thumb.width - 2 * d.thumb.border}
            height={box.height - 2 * d.thumb.border}
            rx={Math.max(0, d.thumb.radius - d.thumb.border)}
            class="fill-neutral-800"
          />
          <rect x={d.line.x} y={d.line.y} width={d.line.width} height={d.line.height} rx={d.line.width / 2} class="fill-paper" />
        </svg>
      {/if}
    {/snippet}
  </Slider>
  <span aria-hidden="true">R</span>
</div>
