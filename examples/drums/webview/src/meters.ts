import { fallingMeter } from "@helpermedia/beamer-controls";

// The level of each output as the meters show it: the values of the value
// sender "meters" (`Meters` in src/lib.rs), the held peak of each bus in
// dB, which the audio thread sends with every block while they show
// something. When no value comes for a while, the host stopped calling the
// plugin (Logic's power button, Reaper's FX switch): the meters then fall
// on their own at the rate of the processor's held peaks.

// The lowest level the meters show in dB, `METER_FLOOR_DB` in src/lib.rs.
export const FLOOR_DB = -60;

// How fast the meters fall in dB per second, `METER_FALL_DB_PER_SECOND` in
// src/lib.rs.
const FALL_DB_PER_SECOND = 20;

// The four levels in what arrived, the floor where it holds no number. Its
// shape is the sender's claim, so each level is checked.
function read(value: unknown): number[] {
  const levels = Array.isArray(value) ? value : [];
  return [0, 1, 2, 3].map((bus) => {
    const level = levels[bus];
    return typeof level === "number" && Number.isFinite(level) ? Math.max(FLOOR_DB, level) : FLOOR_DB;
  });
}

// The meters, falling from the last value once no value comes for 300 ms.
export const meters = fallingMeter("meters", { read, fallPerSecond: FALL_DB_PER_SECOND });
