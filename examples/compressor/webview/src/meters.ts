import { fallingMeter } from "@helpermedia/beamer-controls";

// The meters as the editor shows them: the values of the value sender
// "meters" (`Meters` in src/lib.rs), which the audio thread sends with
// every block while they show something. When no value comes for a while,
// the host stopped calling the plugin (Logic's power button, Reaper's FX
// switch, a host that rests while stopped). The meters then fall on their
// own at the rate of the processor's held peaks.

// The lowest level the meters show in dB, `METER_FLOOR_DB` in src/lib.rs.
export const FLOOR_DB = -60;

// How fast the meters fall in dB per second, `METER_FALL_DB_PER_SECOND` in
// src/lib.rs.
const FALL_DB_PER_SECOND = 20;

export interface Meters {
  // The held peak of the level the detector keys on in dB.
  keyDb: number;
  // The held gain reduction in dB, a positive number.
  reductionDb: number;
  // The auto makeup gain in effect in dB.
  autoMakeupDb: number;
}

function finite(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

// A value as it arrives, or the meters at rest for undefined. Its shape is
// the caller's claim, so each field is checked.
function read(value: unknown): Meters {
  const meters = (typeof value === "object" && value !== null ? value : {}) as Partial<Meters>;
  return {
    keyDb: Math.max(FLOOR_DB, finite(meters.keyDb, FLOOR_DB)),
    reductionDb: Math.max(0, finite(meters.reductionDb, 0)),
    autoMakeupDb: finite(meters.autoMakeupDb, 0),
  };
}

// The meters, falling from the last value once no value comes for 300 ms.
export const meters = fallingMeter("meters", { read, fallPerSecond: FALL_DB_PER_SECOND });
