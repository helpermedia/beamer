import { fallingMeter } from "@helpermedia/beamer-controls";
import { useReadable } from "@helpermedia/beamer-react";
import { FLOOR_DB, dbPosition, levelDb } from "../levels";

// What the plugin sends: the held peak of each channel, linear. A level
// that was not a finite number arrives as null.
interface Levels {
  peak: [number | null, number | null];
}

// The held peak of each channel in dB, the floor until the first value
// and for what is not an array of two.
function read(value: unknown): [number, number] {
  const peak = (value as Partial<Levels> | undefined)?.peak;
  if (!Array.isArray(peak) || peak.length !== 2) return [FLOOR_DB, FLOOR_DB];
  return [levelDb(Number(peak[0]) || 0), levelDb(Number(peak[1]) || 0)];
}

// The plugin sends the levels with every block while the meter shows
// something. When they stop, the host stopped calling the plugin (its
// bypass, a host that rests while stopped): the meter falls on its own
// after 300 ms, at the 20 dB a second of the processor's held peaks.
const levels = fallingMeter("levels", { read, fallPerSecond: 20 });

// The output meter, one bar per channel.
export default function LevelMeter() {
  const peaks = useReadable(levels);

  return (
    <div className="flex gap-1 h-16" aria-label="Output level">
      {peaks.map((db, channel) => (
        <div key={channel} className="relative w-2 h-full bg-slate-800 rounded-sm overflow-hidden">
          <div
            className="absolute bottom-0 w-full bg-cyan-400"
            style={{ height: `${dbPosition(db) * 100}%` }}
          />
        </div>
      ))}
    </div>
  );
}
