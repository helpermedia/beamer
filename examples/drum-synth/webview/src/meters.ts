import { fallingLevels } from "@helpermedia/beamer-controls";

// The level of each drum as the meters show it: the values of the value
// sender "meters" (in model-types.ts), the held peak of each drum in
// dB, which the audio thread sends with every block while they show
// something. When no value comes for a while, the host stopped calling the
// plugin (Logic's power button, Reaper's FX switch): the meters then fall
// on their own at the rate of the processor's held peaks.

// The lowest level the meters show in dB, `METER_FLOOR_DB` in src/lib.rs.
export const FLOOR_DB = -60;

// How fast the meters fall in dB per second, `METER_FALL_DB_PER_SECOND` in
// src/lib.rs.
const FALL_DB_PER_SECOND = 20;

// The four levels as they arrived, each at least at the floor, or the floor
// before the first value.
function read(value: [number, number, number, number] | undefined): number[] {
  return value ? value.map((level) => Math.max(FLOOR_DB, level)) : [FLOOR_DB, FLOOR_DB, FLOOR_DB, FLOOR_DB];
}

// The meters, falling from the last value once no value comes for 300 ms.
export const meters = fallingLevels("meters", { read, fallPerSecond: FALL_DB_PER_SECOND });
