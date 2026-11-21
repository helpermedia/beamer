import { fallingLevels } from "@helpermedia/beamer-controls";
import { useReadable } from "@helpermedia/beamer-react";
import { FLOOR_DB, dbPosition, levelDb } from "../decibels";
import type { Levels } from "../model-types";

// The held peak of each channel in dB from what the plugin sends (linear
// peaks), the floor until the first value.
function read(value: Levels | undefined): [number, number] {
  if (!value) return [FLOOR_DB, FLOOR_DB];
  return [levelDb(value.peak[0]), levelDb(value.peak[1])];
}

// The plugin sends the levels with every block while the meter shows
// something. When they stop, the host stopped calling the plugin (its
// bypass, a host that rests while stopped): the meter falls on its own
// after 300 ms, at the 20 dB a second of the processor's held peaks.
const levels = fallingLevels("levels", { read, fallPerSecond: 20 });

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
