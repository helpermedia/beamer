import { useSenderValue } from "@helpermedia/beamer-react";
import { meterPosition } from "../levels";

// What the plugin sends: the held peak of each channel, linear. A level
// that was not a finite number arrives as null.
interface Levels {
  peak: [number | null, number | null];
}

// The output meter. The plugin sends the level through a value sender, so
// the runtime holds the newest one and the meter reads it.
export default function LevelMeter() {
  const levels = useSenderValue<Levels>("levels");
  // Zero until the value is an array, one bar per channel.
  const peak = levels?.peak;
  const peaks = (Array.isArray(peak) ? peak : [0, 0]).map((value) => Number(value) || 0);

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
