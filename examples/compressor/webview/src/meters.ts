import { fallingLevels } from "@helpermedia/beamer-controls";
import type { Meters } from "./model-types";

// The meters as the editor shows them: the values of the value sender
// "meters" (`Meters` in model-types.ts), which the audio thread sends with
// every block while they show something. When no value comes for a while,
// the host stopped calling the plugin (Logic's power button, Reaper's FX
// switch, a host that rests while stopped). The meters then fall on their
// own at the rate of the processor's held peaks.

// The lowest level the meters show in dB, `METER_FLOOR_DB` in src/lib.rs.
export const FLOOR_DB = -60;

// How fast the meters fall in dB per second, `METER_FALL_DB_PER_SECOND` in
// src/lib.rs.
const FALL_DB_PER_SECOND = 20;

// The meters at rest: no signal and no reduction.
const REST: Meters = { keyDb: FLOOR_DB, reductionDb: 0, autoMakeupDb: 0 };

// The meters as they arrive, the key's level at least at the floor and the
// reduction at least 0, or the meters at rest for undefined.
function read(value: Meters | undefined): Meters {
  if (!value) return REST;
  return { ...value, keyDb: Math.max(FLOOR_DB, value.keyDb), reductionDb: Math.max(0, value.reductionDb) };
}

// The meters, at rest until the host prepared the plugin and falling from
// the last value once no value comes for 300 ms. Components read them with
// `useReadable(meters)`.
export const meters = fallingLevels("meters", { read, fallPerSecond: FALL_DB_PER_SECOND });
