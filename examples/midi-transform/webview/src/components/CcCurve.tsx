import { CcMode, transformCcValue, usesCcScale } from "../transform";
import type { CcSettings } from "../transform";
import type { Cc } from "../transformed";

// The display in SVG units: rem-sized pixels at the editor's own size.
const SIZE = 132;
// The square of the values, 0 to 127 both ways, with room for the axes.
const LEFT = 18;
const TOP = 8;
const SIDE = SIZE - LEFT - 8;
const BOTTOM = TOP + SIDE;
// The axis names, each centered in the band beside the square: Out in the
// band on the left, In in the band below, both as wide as `LEFT`. They are
// centered on the height of their capitals (Inter's cap height), as they
// have no descenders.
const LABEL_SIZE = 11;
const LABEL_CAP_HEIGHT = LABEL_SIZE * 0.727;
const LABEL_BAND = LEFT;

function x(value: number): number {
  return LEFT + value * SIDE;
}

function y(value: number): number {
  return BOTTOM - value * SIDE;
}

// The curve of a CC value from in to out, as SVG points.
function curvePoints(settings: CcSettings): string {
  if (settings.mode === CcMode.Invert) return `${x(0)},${y(1)} ${x(1)},${y(0)}`;
  // A scale rises until the value reaches 127, then stays there.
  const scale = settings.valueScale;
  const knee = scale > 1 ? 1 / scale : 1;
  return `${x(0)},${y(0)} ${x(knee)},${y(transformCcValue(knee, settings))} ${x(1)},${y(transformCcValue(1, settings))}`;
}

interface CcCurveProps {
  settings: CcSettings;
  // The last CC that passed, null before the first.
  last: Cc | null;
  lit: boolean;
}

// What the CC values do, in to out: the diagonal where they pass unchanged,
// the blue curve where the mode changes them (all CCs for Scale and
// Invert, the input CC for Remap + Scale). The last CC shows as a dot at
// its value in and out, lit also while the plugin is bypassed. Text sizes
// are in the units of the viewBox, which scales with the window.
function CcCurve({ settings, last, lit }: CcCurveProps) {
  const changes = settings.enabled && (settings.mode === CcMode.Invert || usesCcScale(settings));
  const quarters = [0.25, 0.5, 0.75];

  return (
    <svg
      viewBox={`0 0 ${SIZE} ${SIZE}`}
      role="img"
      aria-label="CC values in and out"
      className="display size-[8.25rem] shrink-0 rounded-lg bg-well ring-1 ring-neutral-800 ring-inset"
      data-unlit={lit ? undefined : ""}
    >
      {quarters.map((part) => (
        <g key={part} className="stroke-neutral-800" strokeWidth={1}>
          <line x1={x(part)} y1={y(0)} x2={x(part)} y2={y(1)} />
          <line x1={x(0)} y1={y(part)} x2={x(1)} y2={y(part)} />
        </g>
      ))}
      <rect x={LEFT} y={TOP} width={SIDE} height={SIDE} className="fill-none stroke-neutral-800" strokeWidth={1} />

      {/* Unchanged values. */}
      <line
        x1={x(0)}
        y1={y(0)}
        x2={x(1)}
        y2={y(1)}
        className={changes ? "stroke-neutral-700" : "stroke-neutral-500"}
        strokeWidth={changes ? 1 : 1.5}
        strokeDasharray={changes ? "3 3" : undefined}
      />
      {changes && (
        <polyline points={curvePoints(settings)} className="fill-none stroke-brand-blue" strokeWidth={2} strokeLinejoin="round" />
      )}

      {last && (
        <circle
          cx={x(last.value)}
          cy={y(last.outValue)}
          r={3.5}
          className="fill-signal"
        />
      )}

      <text
        x={LEFT + SIDE / 2}
        y={BOTTOM + LABEL_BAND / 2 + LABEL_CAP_HEIGHT / 2}
        textAnchor="middle"
        fontSize={LABEL_SIZE}
        className="fill-neutral-500"
      >
        In
      </text>
      <text
        x={0}
        y={0}
        transform={`translate(${LABEL_BAND / 2 + LABEL_CAP_HEIGHT / 2} ${TOP + SIDE / 2}) rotate(-90)`}
        textAnchor="middle"
        fontSize={LABEL_SIZE}
        className="fill-neutral-500"
      >
        Out
      </text>
    </svg>
  );
}

export default CcCurve;
