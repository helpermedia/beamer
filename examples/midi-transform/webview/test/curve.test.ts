import { expect, test } from "bun:test";
import {
  IDENTITY,
  MAX_POINTS,
  MIN_GAP,
  sampleCurve,
  valueFunction,
  withPointAdded,
  withPointMoved,
  withPointRemoved,
} from "../src/curve";
import type { Curve } from "../src/model-types";

const smooth = (points: [number, number][]): Curve => ({
  shape: "smooth",
  points: points.map(([x, y]) => ({ x, y })),
});

test("the editor computes the curve the processor plays", () => {
  // The same values as test_values_the_web_ui_checks in beamer-core's
  // curve.rs.
  const curve = valueFunction(smooth([[0, 0.1], [0.3, 0.2], [0.45, 0.8], [0.7, 0.6], [1, 1]]));
  const floor = valueFunction(smooth([[0.2, 0], [0.5, 0.9], [1, 1]]));
  const expected: [(x: number) => number, number, number][] = [
    [curve, 0.1, 0.11083158630328444],
    [curve, 0.35, 0.37064989517819696],
    [curve, 0.5, 0.7792000000000001],
    [curve, 0.65, 0.6208],
    [curve, 0.9, 0.7853198653198653],
    [floor, 0.1, 0],
    [floor, 0.3, 0.4043445692883894],
    [floor, 0.45, 0.8474133895131085],
    [floor, 0.75, 0.9752808988764046],
  ];
  for (const [valueAt, x, y] of expected) {
    expect(valueAt(x)).toBeCloseTo(y, 12);
  }
});

test("the linear shape runs straight and holds beyond its points", () => {
  const valueAt = valueFunction({ shape: "linear", points: [{ x: 0.2, y: 0.3 }, { x: 0.6, y: 0.5 }] });
  expect(valueAt(0)).toBe(0.3);
  expect(valueAt(0.4)).toBeCloseTo(0.4, 12);
  expect(valueAt(1)).toBe(0.5);
  expect(valueAt(Number.NaN)).toBe(0.3);
});

test("the drawn curve keeps the corners of the linear shape", () => {
  const curve: Curve = { shape: "linear", points: [{ x: 0, y: 0 }, { x: 0.333, y: 0.9 }, { x: 1, y: 1 }] };
  const drawn = sampleCurve(curve, 10);
  expect(drawn).toContainEqual({ x: 0.333, y: 0.9 });
  expect(drawn[0]).toEqual({ x: 0, y: 0 });
  expect(drawn[drawn.length - 1]).toEqual({ x: 1, y: 1 });
});

test("a point moves no closer to its neighbors than MIN_GAP", () => {
  const curve = smooth([[0, 0], [0.5, 0.5], [1, 1]]);
  expect(withPointMoved(curve, 1, 0, 2).points[1]).toEqual({ x: MIN_GAP, y: 1 });
  expect(withPointMoved(curve, 1, 1, -1).points[1]).toEqual({ x: 1 - MIN_GAP, y: 0 });
  // The end points move within 0 to 1.
  expect(withPointMoved(curve, 0, -0.5, 0.2).points[0]).toEqual({ x: 0, y: 0.2 });
  expect(withPointMoved(curve, 2, 0.9, 0.8).points[2]).toEqual({ x: 0.9, y: 0.8 });
});

test("a point closer to its neighbors than MIN_GAP still moves by y", () => {
  const curve = smooth([[0.5, 0], [0.501, 0.5], [0.502, 1]]);
  expect(withPointMoved(curve, 1, 0.9, 0.7).points[1]).toEqual({ x: 0.501, y: 0.7 });
  // Away from a neighbor it is too close to, but not toward it.
  const near = smooth([[0, 0], [0.002, 0.5], [1, 1]]);
  expect(withPointMoved(near, 1, 0.001, 0.5).points[1].x).toBe(0.002);
  expect(withPointMoved(near, 1, 0.5, 0.5).points[1].x).toBe(0.5);
});

test("a point is added in its place by x", () => {
  const added = withPointAdded(IDENTITY, 0.25, 0.6);
  expect(added?.index).toBe(1);
  expect(added?.curve.points.map((point) => point.x)).toEqual([0, 0.25, 1]);
  // Not on top of another point.
  expect(withPointAdded(IDENTITY, MIN_GAP / 2, 0.5)).toBeNull();
});

test("a curve takes no more than MAX_POINTS", () => {
  let curve = IDENTITY;
  for (let step = 1; step < MAX_POINTS - 1; step++) {
    curve = withPointAdded(curve, step / MAX_POINTS, 0.5)!.curve;
  }
  expect(curve.points.length).toBe(MAX_POINTS);
  expect(withPointAdded(curve, 0.95, 0.5)).toBeNull();
});

test("a curve keeps at least two points", () => {
  const three = smooth([[0, 0], [0.5, 0.8], [1, 1]]);
  const two = withPointRemoved(three, 1);
  expect(two.points.map((point) => point.x)).toEqual([0, 1]);
  expect(withPointRemoved(two, 0)).toBe(two);
});
