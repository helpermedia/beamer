import { useMemo } from "react";
import { outputDb } from "../curve";
import type { Curve } from "../curve";
import { useMeters } from "../hooks/usePluginValues";
import { FLOOR_DB } from "../meters";
import { withMinus } from "../format";

// The display shows levels from the floor of the meters to 0 dB on both
// axes.
const TOP_DB = 0;

// The plot within the display, in SVG units: pixels at the editor's own
// size.
const SIZE = 300;
const LEFT = 34;
const TOP = 12;
const PLOT = 250;
const RIGHT = LEFT + PLOT;
const BOTTOM = TOP + PLOT;

function xAt(db: number): number {
  return LEFT + ((db - FLOOR_DB) / (TOP_DB - FLOOR_DB)) * PLOT;
}

function yAt(db: number): number {
  return BOTTOM - ((db - FLOOR_DB) / (TOP_DB - FLOOR_DB)) * PLOT;
}

// Grid lines every 6 dB, labeled every 12.
const GRID = Array.from({ length: 11 }, (_, index) => FLOOR_DB + index * 6);
const LABELS = GRID.filter((db) => db % 12 === 0);

// The step of the paths in dB.
const STEP = 0.25;

// The curve as a path.
function curvePath(curve: Curve): string {
  const points: string[] = [];
  for (let db = FLOOR_DB; db <= TOP_DB; db += STEP) {
    points.push(`${xAt(db).toFixed(2)},${yAt(outputDb(db, curve)).toFixed(2)}`);
  }
  return `M ${points.join(" L ")}`;
}

// What the compressor turns down: the area between the curve and the line
// it would follow without compression (the input plus the makeup), from
// the start of the knee on.
function reductionPath(curve: Curve): string {
  const points: string[] = [];
  const start = Math.max(FLOOR_DB, curve.thresholdDb - curve.kneeDb / 2);
  // The steps rarely land on the ends, so each side ends on its end.
  const along = (from: number, to: number, step: number) => {
    const levels: number[] = [];
    for (let db = from; step > 0 ? db < to : db > to; db += step) levels.push(db);
    levels.push(to);
    return levels;
  };
  for (const db of along(start, TOP_DB, STEP)) {
    points.push(`${xAt(db).toFixed(2)},${yAt(outputDb(db, curve)).toFixed(2)}`);
  }
  for (const db of along(TOP_DB, start, -STEP)) {
    points.push(`${xAt(db).toFixed(2)},${yAt(db + curve.makeupDb).toFixed(2)}`);
  }
  return `M ${points.join(" L ")} Z`;
}

interface TransferCurveProps {
  curve: Curve;
  // The threshold as the parameter writes it, with its unit.
  thresholdText: string;
  // Whether the detector keys on the sidechain, which the level then is.
  keyedOnSidechain: boolean;
  // Whether the display is lit: not while the plugin is bypassed.
  lit: boolean;
}

// The transfer curve: the output level for each steady input level. The
// signal is paper (the curve, the level and its dot), what the compressor
// does is blue (the threshold and the area it turns down).
function TransferCurve({ curve, thresholdText, keyedOnSidechain, lit }: TransferCurveProps) {
  const { keyDb } = useMeters();
  // The meters render the display at every frame; the paths change only
  // with the parameters.
  const { thresholdDb, ratio, kneeDb, makeupDb } = curve;
  const paths = useMemo(() => {
    const settings = { thresholdDb, ratio, kneeDb, makeupDb };
    return { curve: curvePath(settings), reduction: reductionPath(settings) };
  }, [thresholdDb, ratio, kneeDb, makeupDb]);

  const hasLevel = keyDb > FLOOR_DB;
  const dotY = Math.max(TOP, yAt(outputDb(keyDb, curve)));
  const threshold = xAt(thresholdDb);
  // The threshold's text right of its line, or left of it near the end.
  // Near the start it goes a line lower, below the axis caption.
  const textOnLeft = threshold > RIGHT - 60;
  const textY = threshold < LEFT + 70 ? TOP + 28 : TOP + 14;

  return (
    <div className="display shrink-0 rounded-xl bg-well ring-1 ring-neutral-800 ring-inset" data-unlit={lit ? undefined : ""}>
      <svg
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        role="img"
        aria-label="Transfer curve: the output level for each input level"
        className="block size-75 text-[10px] tabular-nums portrait:size-83"
      >
        <defs>
          <clipPath id="plot">
            <rect x={LEFT} y={TOP} width={PLOT} height={PLOT} />
          </clipPath>
        </defs>

        {GRID.map((db) => (
          <g key={db} className={db % 12 === 0 ? "stroke-neutral-800" : "stroke-neutral-800/50"}>
            <line x1={xAt(db)} y1={TOP} x2={xAt(db)} y2={BOTTOM} />
            <line x1={LEFT} y1={yAt(db)} x2={RIGHT} y2={yAt(db)} />
          </g>
        ))}
        {LABELS.map((db) => (
          <g key={db} className="fill-neutral-500">
            <text x={xAt(db)} y={BOTTOM + 28} textAnchor="middle">
              {withMinus(String(db))}
            </text>
            {db > FLOOR_DB && (
              <text x={LEFT - 6} y={yAt(db) + 3.5} textAnchor="end">
                {withMinus(String(db))}
              </text>
            )}
          </g>
        ))}

        <g clipPath="url(#plot)">
          <path d={paths.reduction} className="fill-brand-blue/20" />
          <line
            x1={LEFT}
            y1={yAt(FLOOR_DB + makeupDb)}
            x2={RIGHT}
            y2={yAt(TOP_DB + makeupDb)}
            className="stroke-neutral-600"
            strokeDasharray="2 4"
          />
          <line x1={threshold} y1={TOP} x2={threshold} y2={BOTTOM} className="stroke-brand-blue/70" />
          <path d={paths.curve} className="fill-none stroke-paper" strokeWidth={2} strokeLinejoin="round" />
        </g>
        {/* The axis captions over the plot, with a halo in the display's
            color, so that a line behind them does not cut them. */}
        <g className="fill-neutral-500 stroke-well" strokeWidth={3} paintOrder="stroke">
          <text x={LEFT + 6} y={TOP + 14}>
            Output dB
          </text>
          <text x={RIGHT - 6} y={BOTTOM - 8} textAnchor="end">
            {keyedOnSidechain ? "Sidechain dB" : "Input dB"}
          </text>
        </g>
        <text
          x={textOnLeft ? threshold - 6 : threshold + 6}
          y={textY}
          textAnchor={textOnLeft ? "end" : "start"}
          className="fill-brand-blue font-semibold"
        >
          {withMinus(thresholdText)}
        </text>

        {/* The level the detector keys on, along the input axis. */}
        <rect x={LEFT} y={BOTTOM + 9} width={PLOT} height={3} rx={1.5} className="fill-paper/8" />
        <rect x={LEFT} y={BOTTOM + 9} width={xAt(keyDb) - LEFT} height={3} rx={1.5} className="fill-paper/70" />
        {hasLevel && (
          <g className="pointer-events-none">
            <circle cx={xAt(keyDb)} cy={dotY} r={9} className="fill-paper/12" />
            <circle cx={xAt(keyDb)} cy={dotY} r={4.5} className="fill-paper" />
          </g>
        )}
      </svg>
    </div>
  );
}

export default TransferCurve;
