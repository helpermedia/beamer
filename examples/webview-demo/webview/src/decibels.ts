// The lowest level the meter and the history show, in dB.
export const FLOOR_DB = -60;

// A linear level in dB, the floor at most: silence is the floor.
export function levelDb(linear: number): number {
  if (!(linear > 0)) return FLOOR_DB;
  return Math.max(FLOOR_DB, 20 * Math.log10(linear));
}

// Where a level in dB sits on the meter: 0 at the floor, 1 at 0 dB and
// above.
export function dbPosition(db: number): number {
  return Math.min(1, Math.max(0, 1 - db / FLOOR_DB));
}

// Where a linear level sits on the meter: 0 at the floor or in silence, 1
// at 0 dB and above.
export function meterPosition(linear: number): number {
  return dbPosition(levelDb(linear));
}
