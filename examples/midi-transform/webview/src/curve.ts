// The velocity curve as beamer-core's `Curve` computes it, in TypeScript,
// so that the editor draws the curve the processor plays. Where the curve
// lowers a note below velocity 1 of 127, the processor sends 1. Also the
// edits of the editor, which keep the points as the processor accepts
// them: sorted by x and at least `MIN_GAP` apart (wider than the
// processor's `Curve::MIN_POINT_DISTANCE`), at least two, all from 0 to 1.

import type { Curve, CurvePoint } from "./model-types";

// The most points the editor places.
export const MAX_POINTS = 12;

// The closest two points come by x: one step of a 7-bit velocity.
export const MIN_GAP = 1 / 127;

// The straight line that leaves every velocity as it is: the curve the
// plugin starts with.
export const IDENTITY: Curve = {
  shape: "smooth",
  points: [
    { x: 0, y: 0 },
    { x: 1, y: 1 },
  ],
};

const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

// The slope at each point for the smooth shape: a monotone cubic after
// Fritsch and Carlson, as `monotone_slopes()` in beamer-core's curve.rs.
function monotoneSlopes(points: CurvePoint[]): number[] {
  const count = points.length;
  const widths: number[] = [];
  const secants: number[] = [];
  for (let index = 0; index < count - 1; index++) {
    const width = points[index + 1].x - points[index].x;
    widths.push(width);
    secants.push((points[index + 1].y - points[index].y) / width);
  }
  if (count === 2) return [secants[0], secants[0]];

  const slopes = new Array<number>(count).fill(0);
  for (let index = 1; index < count - 1; index++) {
    const before = secants[index - 1];
    const after = secants[index];
    if (before * after > 0) {
      const h0 = widths[index - 1];
      const h1 = widths[index];
      const w1 = 2 * h1 + h0;
      const w2 = h1 + 2 * h0;
      slopes[index] = (w1 + w2) / (w1 / before + w2 / after);
    }
  }
  slopes[0] = endSlope(widths[0], widths[1], secants[0], secants[1]);
  slopes[count - 1] = endSlope(widths[count - 2], widths[count - 3], secants[count - 2], secants[count - 3]);
  return slopes;
}

// The slope at an end point from the end segment and the one next to it.
function endSlope(h0: number, h1: number, d0: number, d1: number): number {
  const slope = ((2 * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
  if (slope * d0 <= 0) return 0;
  if (d0 * d1 <= 0 && Math.abs(slope) > 3 * Math.abs(d0)) return 3 * d0;
  return slope;
}

// The curve as a function from in to out, as `Curve::value_at()`: before
// the first point it holds that point's value, after the last one too.
export function valueFunction(curve: Curve): (x: number) => number {
  const points = curve.points;
  const slopes = curve.shape === "smooth" ? monotoneSlopes(points) : [];
  const first = points[0];
  const last = points[points.length - 1];
  return (x) => {
    if (Number.isNaN(x) || x <= first.x) return first.y;
    if (x >= last.x) return last.y;
    let right = 1;
    while (points[right].x <= x) right++;
    const left = right - 1;
    const a = points[left];
    const b = points[right];
    const width = b.x - a.x;
    const t = (x - a.x) / width;
    if (curve.shape === "linear") return clamp01(a.y + t * (b.y - a.y));
    const t2 = t * t;
    const t3 = t2 * t;
    const y =
      (2 * t3 - 3 * t2 + 1) * a.y +
      (t3 - 2 * t2 + t) * width * slopes[left] +
      (-2 * t3 + 3 * t2) * b.y +
      (t3 - t2) * width * slopes[right];
    return clamp01(y);
  };
}

// The curve from 0 to 1 as points to draw: `steps` evenly spaced plus
// every point of the curve, so that the corners of the linear shape stay
// sharp.
export function sampleCurve(curve: Curve, steps: number): CurvePoint[] {
  const valueAt = valueFunction(curve);
  const xs = new Set<number>();
  for (let step = 0; step <= steps; step++) xs.add(step / steps);
  for (const point of curve.points) xs.add(point.x);
  return [...xs].sort((a, b) => a - b).map((x) => ({ x, y: valueAt(x) }));
}

// Where point `index` may go by x: within 0 to 1 and between its
// neighbors, with `MIN_GAP` to each.
function xRange(points: CurvePoint[], index: number): [number, number] {
  const low = index > 0 ? points[index - 1].x + MIN_GAP : 0;
  const high = index < points.length - 1 ? points[index + 1].x - MIN_GAP : 1;
  return [low, high];
}

// The curve with point `index` moved as close to `x` and `y` as it may go.
// A point that lies closer to a neighbor than `MIN_GAP` (in a curve the
// editor did not make) may stay where it is by x, so that it can still
// move by y.
export function withPointMoved(curve: Curve, index: number, x: number, y: number): Curve {
  const [low, high] = xRange(curve.points, index);
  const current = curve.points[index].x;
  const points = curve.points.slice();
  points[index] = { x: Math.min(Math.max(high, current), Math.max(Math.min(low, current), x)), y: clamp01(y) };
  return { ...curve, points };
}

// The curve with a point at `x` and `y` plus the index the point got.
// Null when the curve has `MAX_POINTS` already or when the point would
// come closer than `MIN_GAP` to another one by x.
export function withPointAdded(curve: Curve, x: number, y: number): { curve: Curve; index: number } | null {
  if (curve.points.length >= MAX_POINTS) return null;
  const point = { x: clamp01(x), y: clamp01(y) };
  if (curve.points.some((other) => Math.abs(other.x - point.x) < MIN_GAP)) return null;
  const index = curve.points.findIndex((other) => other.x > point.x);
  const at = index < 0 ? curve.points.length : index;
  const points = [...curve.points.slice(0, at), point, ...curve.points.slice(at)];
  return { curve: { ...curve, points }, index: at };
}

// The curve without point `index`. A curve of two points keeps them.
export function withPointRemoved(curve: Curve, index: number): Curve {
  if (curve.points.length <= 2) return curve;
  return { ...curve, points: curve.points.filter((_, other) => other !== index) };
}
