import { useEffect, useState } from "react";
import { meterPosition } from "../levels";

// The output meter. The plugin sends the level through a value sender, so
// each event holds the newest level and replaces the one before.
export default function LevelMeter() {
  const [peaks, setPeaks] = useState<[number, number]>([0, 0]);

  useEffect(
    () =>
      __BEAMER__.on("levels", (data) => {
        const peak = (data as { peak?: unknown } | null)?.peak;
        if (Array.isArray(peak) && peak.length === 2) {
          // A level that was not a finite number arrives as null.
          setPeaks([Number(peak[0]) || 0, Number(peak[1]) || 0]);
        }
      }),
    [],
  );

  return (
    <div className="flex gap-1 h-16" aria-label="Output level">
      {peaks.map((peak, channel) => (
        <div key={channel} className="relative w-2 h-full bg-slate-800 rounded-sm overflow-hidden">
          <div
            className="absolute bottom-0 w-full bg-cyan-400"
            style={{ height: `${meterPosition(peak) * 100}%` }}
          />
        </div>
      ))}
    </div>
  );
}
