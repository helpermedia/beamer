// The lowest level the meter and the history show, in dB.
const FLOOR_DB = -60;

// Where a linear level sits on the meter: 0 at the floor or in silence, 1
// at 0 dB and above.
export function meterPosition(linear: number): number {
  if (!(linear > 0)) return 0;
  const db = 20 * Math.log10(linear);
  return Math.min(1, Math.max(0, 1 - db / FLOOR_DB));
}
