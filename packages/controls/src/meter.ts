import { senderValue } from "@helpermedia/beamer-api";
import type { Readable } from "@helpermedia/beamer-api";

/**
 * The levels of a meter: one, one per channel or one per name. `T` is the
 * type itself, so that an interface of numbers counts without an index
 * signature.
 */
export type MeterLevels<T> = number | readonly number[] | { readonly [K in keyof T]: number };

/** How a falling meter reads its sender and falls. */
export interface FallingMeterOptions<T extends MeterLevels<T>> {
  /**
   * The levels in what the sender sent, each a finite number. Called with
   * undefined for the levels at rest, where each one falls to: the floor
   * of a level in dB, 0 for a gain reduction. What arrives is the
   * sender's claim, so check its shape here.
   */
  read: (value: unknown) => T;
  /** How far a level falls in a second, in its own unit. Default 20 (dB). */
  fallPerSecond?: number;
  /**
   * How long the meter waits for a value before it falls on its own, in
   * ms. Default 300, longer than the longest block a host renders (8192
   * samples at 44.1 kHz last 186 ms).
   */
  waitMs?: number;
}

function mapLevels<T extends MeterLevels<T>>(levels: T, rest: T, change: (level: number, rest: number) => number): T {
  if (typeof levels === "number") return change(levels, rest as number) as T;
  if (Array.isArray(levels)) return levels.map((level, index) => change(level, (rest as readonly number[])[index] ?? 0)) as unknown as T;
  const restByName = rest as { readonly [name: string]: number };
  const entries = Object.entries(levels).map(([name, level]) => [name, change(level as number, restByName[name] ?? 0)]);
  return Object.fromEntries(entries) as T;
}

function everyLevel<T extends MeterLevels<T>>(levels: T, rest: T, test: (level: number, rest: number) => boolean): boolean {
  let all = true;
  mapLevels(levels, rest, (level, restLevel) => {
    if (!test(level, restLevel)) all = false;
    return level;
  });
  return all;
}

/**
 * A meter on the value sender `sender` that falls on its own when no value
 * comes. A plugin sends its meters from the audio thread with every block
 * while they show something. When nothing comes for `waitMs`, the host
 * stopped calling the plugin (a host's bypass or power button, a host that
 * rests while stopped). The meter then falls at `fallPerSecond` from the
 * last value, each level towards its rest. Match the rate to how the
 * processor lets its held peaks fall.
 *
 * Returns a `Readable` of the levels as shown, the same object until they
 * change: React's `useSyncExternalStore` and Svelte's stores take it as it
 * is. It follows the sender while it has subscribers and needs the runtime
 * only then, so it can be made when a module loads. The value the sender
 * holds when the first subscriber comes counts as fresh, so an editor that
 * opens while the host does not call the plugin shows it until it falls.
 */
export function fallingMeter<T extends MeterLevels<T>>(sender: string, options: FallingMeterOptions<T>): Readable<T> {
  const fallPerSecond = options.fallPerSecond ?? 20;
  const waitMs = options.waitMs ?? 300;
  let rest: T | undefined;
  const restLevels = (): T => (rest ??= options.read(undefined));

  let shown: T | undefined;
  // The last value that arrived and when the meter began to fall from it.
  let last: T | undefined;
  let fallingSince = 0;
  let wait: ReturnType<typeof setTimeout> | undefined;
  let frame = 0;
  const listeners = new Set<(levels: T) => void>();
  let unsubscribe: (() => void) | null = null;

  const get = (): T => (shown ??= restLevels());
  const show = (levels: T) => {
    shown = levels;
    for (const listener of listeners) listener(levels);
  };
  const atRest = (levels: T) => everyLevel(levels, restLevels(), (level, restLevel) => level === restLevel);
  const stopFalling = () => {
    clearTimeout(wait);
    cancelAnimationFrame(frame);
    frame = 0;
  };
  const fall = (now: number) => {
    const distance = (fallPerSecond * Math.max(0, now - fallingSince)) / 1000;
    const levels = mapLevels(last ?? restLevels(), restLevels(), (level, restLevel) =>
      level > restLevel ? Math.max(restLevel, level - distance) : Math.min(restLevel, level + distance),
    );
    show(levels);
    frame = atRest(levels) ? 0 : requestAnimationFrame(fall);
  };
  const arrive = (value: unknown) => {
    stopFalling();
    const levels = options.read(value);
    last = levels;
    show(levels);
    if (atRest(levels)) return;
    wait = setTimeout(() => {
      fallingSince = performance.now();
      frame = requestAnimationFrame(fall);
    }, waitMs);
  };

  return {
    get,
    subscribe: (callback: (levels: T) => void) => {
      if (!unsubscribe) unsubscribe = senderValue<unknown>(sender).subscribe(arrive);
      listeners.add(callback);
      callback(get());
      return () => {
        listeners.delete(callback);
        if (listeners.size > 0 || !unsubscribe) return;
        unsubscribe();
        unsubscribe = null;
        stopFalling();
      };
    },
  };
}
