import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterAll, beforeAll, expect, test } from "bun:test";
import { fallingMeter } from "../src/index";
import { installRuntime } from "./runtime";

beforeAll(() => GlobalRegistrator.register());
afterAll(() => GlobalRegistrator.unregister());

const FLOOR_DB = -60;

interface Meters {
  levelDb: number;
  reductionDb: number;
  makeupDb: number;
}

function read(value: unknown): Meters {
  const fields = (typeof value === "object" && value !== null ? value : {}) as Partial<Meters>;
  return {
    levelDb: fields.levelDb ?? FLOOR_DB,
    reductionDb: fields.reductionDb ?? 0,
    makeupDb: fields.makeupDb ?? 0,
  };
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

test("a meter reads at rest before the first subscriber, without the runtime", () => {
  globalThis.__BEAMER__ = undefined;
  const meters = fallingMeter("meters", { read });
  expect(meters.get()).toEqual({ levelDb: FLOOR_DB, reductionDb: 0, makeupDb: 0 });
  // The same object until it changes.
  expect(meters.get()).toBe(meters.get());
});

test("a meter shows each value at once and the value held when it opens", () => {
  const runtime = installRuntime([]);
  runtime.send("meters", { levelDb: -12, reductionDb: 3, makeupDb: 1 });
  const meters = fallingMeter("meters", { read, waitMs: 1000 });
  const seen: Meters[] = [];
  const unsubscribe = meters.subscribe((levels) => seen.push(levels));
  expect(seen).toEqual([{ levelDb: -12, reductionDb: 3, makeupDb: 1 }]);
  runtime.send("meters", { levelDb: -6, reductionDb: 0, makeupDb: 0 });
  expect(meters.get()).toEqual({ levelDb: -6, reductionDb: 0, makeupDb: 0 });
  unsubscribe();
});

test("without values a meter waits, then falls towards each rest from either side", async () => {
  const runtime = installRuntime([]);
  const meters = fallingMeter("meters", { read, waitMs: 20, fallPerSecond: 1000 });
  const unsubscribe = meters.subscribe(() => {});
  runtime.send("meters", { levelDb: -20, reductionDb: 30, makeupDb: -40 });
  await sleep(10);
  expect(meters.get()).toEqual({ levelDb: -20, reductionDb: 30, makeupDb: -40 });
  await sleep(40);
  const falling = meters.get();
  expect(falling.levelDb).toBeLessThan(-20);
  expect(falling.reductionDb).toBeLessThan(30);
  expect(falling.makeupDb).toBeGreaterThan(-40);
  // At a second per 1000 dB the rest is reached well within 200 ms.
  await sleep(150);
  expect(meters.get()).toEqual({ levelDb: FLOOR_DB, reductionDb: 0, makeupDb: 0 });
  unsubscribe();
});

test("a new value stops the fall", async () => {
  const runtime = installRuntime([]);
  const meters = fallingMeter("level", { read: (value) => (typeof value === "number" ? value : FLOOR_DB), waitMs: 10, fallPerSecond: 100 });
  const unsubscribe = meters.subscribe(() => {});
  runtime.send("level", -10);
  await sleep(60);
  expect(meters.get()).toBeLessThan(-10);
  runtime.send("level", -3);
  expect(meters.get()).toBe(-3);
  unsubscribe();
});

test("levels per channel fall as an array", async () => {
  const runtime = installRuntime([]);
  const meters = fallingMeter("peaks", {
    read: (value) => (Array.isArray(value) ? value.map(Number) : [FLOOR_DB, FLOOR_DB]),
    waitMs: 10,
    fallPerSecond: 10000,
  });
  const unsubscribe = meters.subscribe(() => {});
  runtime.send("peaks", [-1, -59]);
  await sleep(80);
  expect(meters.get()).toEqual([FLOOR_DB, FLOOR_DB]);
  unsubscribe();
});

test("the last subscriber leaving stops following the sender", () => {
  const runtime = installRuntime([]);
  const meters = fallingMeter("level", { read: (value) => (typeof value === "number" ? value : FLOOR_DB) });
  const unsubscribe = meters.subscribe(() => {});
  runtime.send("level", -10);
  unsubscribe();
  runtime.send("level", -2);
  expect(meters.get()).toBe(-10);
});
