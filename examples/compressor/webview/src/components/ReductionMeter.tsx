import { useMeters } from "../hooks/usePluginValues";

// The most reduction the meter shows in dB.
const RANGE_DB = 24;

// The bar within the display, in SVG units (pixels at the editor's own size),
// level with the plot of the transfer curve.
const WIDTH = 50;
const HEIGHT = 300;
const TOP = 12;
const BAR = 250;

const TICKS = [0, 6, 12, 18, 24];

function yAt(db: number): number {
  return TOP + (Math.min(db, RANGE_DB) / RANGE_DB) * BAR;
}

interface ReductionMeterProps {
  // Whether the display is lit: not while the plugin is bypassed.
  lit: boolean;
}

// The gain reduction in dB, falling from the top in blue (what the
// compressor does), with its value below.
function ReductionMeter({ lit }: ReductionMeterProps) {
  const { reductionDb } = useMeters();
  const bar = yAt(reductionDb) - TOP;

  return (
    <div className="display shrink-0 rounded-xl bg-well ring-1 ring-neutral-800 ring-inset" data-unlit={lit ? undefined : ""}>
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        role="meter"
        aria-label="Gain reduction"
        aria-valuemin={0}
        aria-valuemax={RANGE_DB}
        aria-valuenow={reductionDb}
        aria-valuetext={`${reductionDb.toFixed(1)} dB`}
        className="block h-75 w-12.5 text-[10px] tabular-nums portrait:h-83 portrait:w-[3.4583rem]"
      >
        {TICKS.map((db) => (
          <text key={db} x={20} y={yAt(db) + 3.5} textAnchor="end" className="fill-neutral-500">
            {db}
          </text>
        ))}
        <rect x={27} y={TOP} width={8} height={BAR} rx={4} className="fill-paper/6" />
        {bar > 0 && <rect x={27} y={TOP} width={8} height={bar} rx={Math.min(4, bar / 2)} className="fill-brand-blue" />}
        <text x={WIDTH / 2} y={HEIGHT - 19} textAnchor="middle" className="fill-paper text-[12px] font-semibold">
          {reductionDb >= 0.05 ? `\u2212${reductionDb.toFixed(1)}` : "0.0"}
        </text>
        <text x={WIDTH / 2} y={HEIGHT - 6} textAnchor="middle" className="fill-neutral-500 text-[9px]">
          Reduction
        </text>
      </svg>
    </div>
  );
}

export default ReductionMeter;
