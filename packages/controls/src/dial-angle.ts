import { DEFAULT_ARC_START, DEFAULT_ARC_SWEEP } from "./gestures.js";

/**
 * Where a dial points for the normalized `value`, in degrees clockwise on
 * screen from the right of its center, as `arcStart` counts: `arcStart`
 * at 0, `arcStart + arcSweep` at 1. Draw the pointer there with the arc
 * given to `attachDial` and an arc drag lands where the dial is drawn. A
 * point at `radius` from the center is at `x = cx + radius * cos(angle)`
 * and `y = cy + radius * sin(angle)`, with the angle in radians.
 */
export function dialAngle(value: number, arcStart = DEFAULT_ARC_START, arcSweep = DEFAULT_ARC_SWEEP): number {
  return arcStart + Math.min(1, Math.max(0, value)) * arcSweep;
}
