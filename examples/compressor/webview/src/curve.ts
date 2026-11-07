// The compressor's curve: the gain for a steady level. The same formula as
// `gain_reduction_db()` in src/lib.rs, so keep the two alike.

// The width of the soft knee in dB, `SOFT_KNEE_WIDTH_DB` in src/lib.rs.
export const SOFT_KNEE_DB = 6;

// What shapes the curve, from the parameters.
export interface Curve {
  thresholdDb: number;
  ratio: number;
  // The width of the knee in dB, 0 for a hard knee.
  kneeDb: number;
  // The gain after the reduction in dB.
  makeupDb: number;
}

// The gain reduction in dB (0 or negative) for a level `overDb` above the
// start of the knee: nothing below the knee, then a quadratic knee centered
// on the threshold, then the full ratio.
export function gainReductionDb(overDb: number, ratio: number, kneeDb: number): number {
  const slope = 1 - 1 / ratio;
  if (overDb <= 0) return 0;
  if (overDb < kneeDb) return (-slope * overDb * overDb) / (2 * kneeDb);
  return -slope * (overDb - kneeDb / 2);
}

// The output level in dB for a steady input level in dB.
export function outputDb(inputDb: number, curve: Curve): number {
  const overDb = inputDb - (curve.thresholdDb - curve.kneeDb / 2);
  return inputDb + gainReductionDb(overDb, curve.ratio, curve.kneeDb) + curve.makeupDb;
}

// The ratio of a choice of the ratio parameter, read from its name ("4:1"),
// so that the curve follows the `Ratio` enum in src/lib.rs. 1 (no
// reduction) for a name without a number.
export function ratioOf(name: string | undefined): number {
  const ratio = Number.parseFloat(name ?? "");
  return Number.isFinite(ratio) && ratio >= 1 ? ratio : 1;
}
