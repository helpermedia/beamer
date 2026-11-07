import { senderValue } from "@helpermedia/beamer-api";

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

// How long the meters wait for a value before they fall on their own, in
// ms. Longer than the longest block a host renders (8192 samples at 44.1
// kHz last 186 ms).
const WAIT_MS = 300;

export interface Meters {
  // The held peak of the level the detector keys on in dB.
  keyDb: number;
  // The held gain reduction in dB, a positive number.
  reductionDb: number;
  // The auto makeup gain in effect in dB.
  autoMakeupDb: number;
}

const REST: Meters = { keyDb: FLOOR_DB, reductionDb: 0, autoMakeupDb: 0 };

function finite(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

// A value as it arrives. Its shape is the caller's claim, so each field is
// checked.
function read(value: unknown): Meters {
  const meters = (typeof value === "object" && value !== null ? value : {}) as Partial<Meters>;
  return {
    keyDb: Math.max(FLOOR_DB, finite(meters.keyDb, FLOOR_DB)),
    reductionDb: Math.max(0, finite(meters.reductionDb, 0)),
    autoMakeupDb: finite(meters.autoMakeupDb, 0),
  };
}

function atRest(meters: Meters): boolean {
  return meters.keyDb <= FLOOR_DB && meters.reductionDb <= 0 && meters.autoMakeupDb === 0;
}

// `meters` after falling for `seconds`, each value towards its rest.
function fallen(meters: Meters, seconds: number): Meters {
  const fall = FALL_DB_PER_SECOND * seconds;
  const makeup = Math.max(0, Math.abs(meters.autoMakeupDb) - fall);
  return {
    keyDb: Math.max(FLOOR_DB, meters.keyDb - fall),
    reductionDb: Math.max(0, meters.reductionDb - fall),
    autoMakeupDb: makeup === 0 ? 0 : Math.sign(meters.autoMakeupDb) * makeup,
  };
}

let shown = REST;
// The last value that arrived and when the meters began to fall from it.
let last = REST;
let fallingSince = 0;
let wait: ReturnType<typeof setTimeout> | undefined;
let frame = 0;
const listeners = new Set<() => void>();
let unsubscribe: (() => void) | null = null;

function show(meters: Meters): void {
  shown = meters;
  for (const listener of listeners) listener();
}

function stopFalling(): void {
  clearTimeout(wait);
  cancelAnimationFrame(frame);
  frame = 0;
}

function fall(now: number): void {
  const meters = fallen(last, Math.max(0, now - fallingSince) / 1000);
  show(meters);
  frame = atRest(meters) ? 0 : requestAnimationFrame(fall);
}

function arrive(value: unknown): void {
  stopFalling();
  last = read(value);
  show(last);
  if (atRest(last)) return;
  wait = setTimeout(() => {
    fallingSince = performance.now();
    frame = requestAnimationFrame(fall);
  }, WAIT_MS);
}

// For React's useSyncExternalStore: the meters as shown, the same object
// until they change.
export function shownMeters(): Meters {
  return shown;
}

// Follow the meters. The first listener subscribes to the value sender,
// which calls back at once with the value it holds. That value counts as
// fresh, so an editor that opens while the host does not call the plugin
// shows it until the meters fall.
export function subscribeMeters(listener: () => void): () => void {
  listeners.add(listener);
  if (!unsubscribe) unsubscribe = senderValue<unknown>("meters").subscribe(arrive);
  return () => {
    listeners.delete(listener);
    if (listeners.size > 0 || !unsubscribe) return;
    unsubscribe();
    unsubscribe = null;
    stopFalling();
  };
}
