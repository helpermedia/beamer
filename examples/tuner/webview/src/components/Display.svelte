<!--
  The tuner's display: a needle on an arc from -50 to +50 cents, the note
  under it, the frequency and the cents beside the note, a strobe band and
  the input level. Blue when the note is in tune. When no pitch comes the
  last note stays dimmed for a moment, then the display rests and names
  the reference instead.
-->
<script lang="ts">
  import { noteName, parameter, senderValue } from "@helpermedia/beamer-api";
  import { fallingLevels } from "@helpermedia/beamer-controls";
  import { parameterView } from "@helpermedia/beamer-svelte";
  import { withMinus } from "../format";
  import { IN_TUNE_CENTS, Tuning } from "../tuning";
  import type { Shown } from "../tuning";

  // The display in SVG units: the well's inside at the editor's own size.
  const WIDTH = 478;
  const HEIGHT = 202;

  // The arc: -50 to +50 cents over 100 degrees around a pivot below the
  // display.
  const PIVOT_X = WIDTH / 2;
  const PIVOT_Y = 252;
  const RADIUS = 222;
  const DEGREES_PER_CENT = 1;

  // The level bar's range in dB; the processor's floor is lower.
  const LEVEL_FLOOR_DB = -60;

  const reference = parameter("reference");
  const referenceView = parameterView(reference);
  const pitch = senderValue("pitch");
  // The input level, which falls on its own when the host stops calling
  // the plugin.
  const level = fallingLevels("pitch", {
    read: (value) => Math.max(LEVEL_FLOOR_DB, value?.levelDb ?? LEVEL_FLOOR_DB),
    fallPerSecond: 40,
  });

  const tuning = new Tuning();
  // A new object every frame, so no deep reactivity.
  let shown: Shown = $state.raw({ phase: "rest", note: 69, cents: 0, frequencyHz: 0, strobe: 0 });

  // A frame for each screen refresh while something moves or fades; none
  // at rest.
  let frame = 0;
  const tick = (now: number) => {
    shown = tuning.frame(now, reference.plain());
    frame = shown.phase === "rest" ? 0 : requestAnimationFrame(tick);
  };
  $effect(() => {
    const unsubscribe = pitch.subscribe((value) => {
      // Only a pitch with a frequency moves something.
      if (value && tuning.arrive(value.frequencyHz, performance.now()) && frame === 0) {
        frame = requestAnimationFrame(tick);
      }
    });
    return () => {
      unsubscribe();
      cancelAnimationFrame(frame);
      frame = 0;
    };
  });

  // A point at `cents` on a circle of `radius` around the pivot.
  function at(cents: number, radius: number): [number, number] {
    const radians = (cents * DEGREES_PER_CENT * Math.PI) / 180;
    return [PIVOT_X + radius * Math.sin(radians), PIVOT_Y - radius * Math.cos(radians)];
  }

  const signed = (value: number) => withMinus(value > 0 ? `+${value}` : `${value}`);

  const TICKS = Array.from({ length: 21 }, (_, index) => {
    const cents = index * 5 - 50;
    const major = cents % 25 === 0;
    const [x1, y1] = at(cents, RADIUS);
    const [x2, y2] = at(cents, RADIUS - (major ? 16 : 9));
    const [labelX, labelY] = at(cents, RADIUS + 12);
    return { cents, major, x1, y1, x2, y2, labelX, labelY: labelY + 4, label: cents === 0 ? "0" : signed(cents) };
  });

  // The in-tune zone on the arc.
  const ZONE = (() => {
    const [ax, ay] = at(-IN_TUNE_CENTS, RADIUS);
    const [bx, by] = at(IN_TUNE_CENTS, RADIUS);
    const [cx, cy] = at(IN_TUNE_CENTS, RADIUS - 16);
    const [dx, dy] = at(-IN_TUNE_CENTS, RADIUS - 16);
    return `M${ax} ${ay} A${RADIUS} ${RADIUS} 0 0 1 ${bx} ${by} L${cx} ${cy} A${RADIUS - 16} ${RADIUS - 16} 0 0 0 ${dx} ${dy} Z`;
  })();

  const STROBE = { x: 16, y: 172, width: WIDTH - 32, height: 12, stripe: 14 };
  const LEVEL = { x: 16, y: 192, width: WIDTH - 32, height: 3 };

  const resting = $derived(shown.phase === "rest");
  const inTune = $derived(shown.phase === "live" && Math.abs(shown.cents) <= IN_TUNE_CENTS);
  const needleCents = $derived(Math.max(-50, Math.min(50, shown.cents)));
  const needle = $derived([...at(needleCents, RADIUS - 78), ...at(needleCents, RADIUS + 2)]);
  // The note's letter (with its sharp) and its octave, by `middle_c`.
  const name = $derived(/^([A-G]#?)(-?\d+)$/.exec(noteName(shown.note)) ?? ["", "", ""]);
  const roundedCents = $derived(Math.round(shown.cents));
  const centsText = $derived(`${signed(roundedCents)} ${Math.abs(roundedCents) === 1 ? "cent" : "cents"}`);
  const summary = $derived(
    resting ? "No pitch" : `${name[1]}${name[2]}, ${centsText}${shown.phase === "held" ? ", held" : ""}`,
  );
  const levelWidth = $derived((LEVEL.width * ($level - LEVEL_FLOOR_DB)) / -LEVEL_FLOOR_DB);
</script>

<svg viewBox="0 0 {WIDTH} {HEIGHT}" class="size-full" role="img" aria-label={summary}>
  <defs>
    <pattern id="strobe" width={STROBE.stripe} height={STROBE.height} patternUnits="userSpaceOnUse" x={STROBE.x + shown.strobe * STROBE.stripe} y={STROBE.y}>
      <rect width={STROBE.stripe / 2} height={STROBE.height} class={inTune ? "fill-brand-blue" : "fill-neutral-600"} />
    </pattern>
    <clipPath id="strobe-clip">
      <rect x={STROBE.x} y={STROBE.y} width={STROBE.width} height={STROBE.height} rx="3" />
    </clipPath>
  </defs>

  <path d={ZONE} class="fill-brand-blue" opacity={inTune ? 0.35 : 0.18} />
  {#each TICKS as tick (tick.cents)}
    <line
      x1={tick.x1}
      y1={tick.y1}
      x2={tick.x2}
      y2={tick.y2}
      class={tick.major ? "stroke-neutral-400" : "stroke-neutral-600"}
      stroke-width={tick.major ? 1.5 : 1}
      stroke-linecap="round"
    />
    {#if tick.major}
      <text x={tick.labelX} y={tick.labelY} text-anchor="middle" class="fill-neutral-500 text-[11px] tabular-nums">{tick.label}</text>
    {/if}
  {/each}

  <line
    x1={needle[0]}
    y1={needle[1]}
    x2={needle[2]}
    y2={needle[3]}
    class={resting ? "stroke-neutral-700" : inTune ? "stroke-brand-blue" : "stroke-paper"}
    opacity={shown.phase === "held" ? 0.4 : 1}
    stroke-width="2.5"
    stroke-linecap="round"
  />

  {#if resting}
    <text x={WIDTH / 2} y="158" text-anchor="middle" class="fill-neutral-500 text-[13px] font-medium tabular-nums">
      {noteName(69)} = {withMinus(referenceView.text)}
    </text>
  {:else}
    <g opacity={shown.phase === "held" ? 0.4 : 1}>
      <text x={WIDTH / 2} y="158" text-anchor="middle" class="text-[46px] font-semibold {inTune ? 'fill-brand-blue' : 'fill-paper'}">
        {name[1]}<tspan dx="2" class="text-[19px] {inTune ? 'fill-brand-blue' : 'fill-neutral-400'}">{name[2]}</tspan>
      </text>
      <text x="24" y="158" class="fill-neutral-400 text-[13px] font-medium tabular-nums">{shown.frequencyHz.toFixed(2)} Hz</text>
      <text x={WIDTH - 24} y="158" text-anchor="end" class="text-[13px] font-medium tabular-nums {inTune ? 'fill-brand-blue' : 'fill-paper'}">
        {centsText}
      </text>
    </g>
  {/if}

  <g clip-path="url(#strobe-clip)" opacity={shown.phase === "live" ? 1 : 0.4}>
    <rect x={STROBE.x} y={STROBE.y} width={STROBE.width} height={STROBE.height} class="fill-neutral-800" />
    <rect x={STROBE.x} y={STROBE.y} width={STROBE.width} height={STROBE.height} fill="url(#strobe)" />
  </g>

  <rect x={LEVEL.x} y={LEVEL.y} width={LEVEL.width} height={LEVEL.height} rx="1.5" class="fill-neutral-800" />
  <rect x={LEVEL.x} y={LEVEL.y} width={levelWidth} height={LEVEL.height} rx="1.5" class="fill-neutral-500" />
</svg>
