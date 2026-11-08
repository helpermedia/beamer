import { runtime, senderValue } from "@helpermedia/beamer-api";

// What passes through the plugin, from the stream sender "transformed"
// (`Transformed` in src/lib.rs): every note and CC as it came in and as it
// went out, in arrays of what arrived since the last tick. This store
// keeps what the displays show: the notes that sound and the last note
// and CC. The shape of what arrives is the caller's claim, so each field
// is checked.
//
// While keys are down the processor sends their number with every block
// through the value sender "pressed". When it stops coming, the host
// stopped calling the plugin (Logic's power button, Reaper's FX switch)
// and no note-off will pass: the notes go out after a wait.

// A note as it came in and went out. Velocities from 0 to 1.
export interface Note {
  pitch: number;
  // Null when the note was filtered out.
  outPitch: number | null;
  velocity: number;
  outVelocity: number;
}

// A CC as it came in and went out. Values from 0 to 1.
export interface Cc {
  controller: number;
  value: number;
  outController: number;
  outValue: number;
}

export interface Transformed {
  // The notes that sound, by channel and incoming pitch.
  held: ReadonlyMap<string, Note>;
  // The last note that started and the last CC, null before the first.
  lastNote: Note | null;
  lastCc: Cc | null;
}

// The CCs that end every note of their channel: All Sound Off and All
// Notes Off, by the number they go out with, as that is what ends the
// notes downstream.
const ENDS_NOTES = [120, 123];

// How long the notes wait for the count of keys before they go out, in
// ms. Longer than the longest block a host renders (8192 samples at 44.1
// kHz last 186 ms).
const WAIT_MS = 300;

const EMPTY: Transformed = { held: new Map(), lastNote: null, lastCc: null };

let current = EMPTY;
const listeners = new Set<() => void>();
let unsubscribe: (() => void) | null = null;
let wait: ReturnType<typeof setTimeout> | undefined;

function show(next: Transformed): void {
  if (next === current) return;
  current = next;
  for (const listener of listeners) listener();
}

// No note sounds any more.
function endNotes(): void {
  if (current.held.size > 0) show({ ...current, held: new Map() });
}

// The processor runs: wait again for the next sign of it.
function waitForProcessor(): void {
  clearTimeout(wait);
  if (current.held.size > 0) wait = setTimeout(endNotes, WAIT_MS);
}

function midiNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= 127 ? value : null;
}

function unit(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 0;
}

function key(channel: number, pitch: number): string {
  return `${channel}:${pitch}`;
}

// The state after one event. Returns `state` itself when the event changes
// nothing or is not one this store reads.
function apply(state: Transformed, event: unknown): Transformed {
  if (typeof event !== "object" || event === null) return state;
  const fields = event as Record<string, unknown>;
  const channel = midiNumber(fields.channel) ?? 0;

  switch (fields.type) {
    case "noteOn": {
      const pitch = midiNumber(fields.pitch);
      if (pitch === null) return state;
      const note: Note = {
        pitch,
        outPitch: midiNumber(fields.outPitch),
        velocity: unit(fields.velocity),
        outVelocity: unit(fields.outVelocity),
      };
      const held = new Map(state.held);
      held.set(key(channel, pitch), note);
      return { ...state, held, lastNote: note };
    }
    case "noteOff": {
      const pitch = midiNumber(fields.pitch);
      if (pitch === null || !state.held.has(key(channel, pitch))) return state;
      const held = new Map(state.held);
      held.delete(key(channel, pitch));
      return { ...state, held };
    }
    case "cc": {
      const controller = midiNumber(fields.controller);
      if (controller === null) return state;
      const cc: Cc = {
        controller,
        value: unit(fields.value),
        outController: midiNumber(fields.outController) ?? controller,
        outValue: unit(fields.outValue),
      };
      let held = state.held;
      if (ENDS_NOTES.includes(cc.outController)) {
        held = new Map([...held].filter(([noteKey]) => !noteKey.startsWith(`${channel}:`)));
      }
      return { ...state, held, lastCc: cc };
    }
    case "reset":
      return state.held.size > 0 ? { ...state, held: new Map() } : state;
    default:
      return state;
  }
}

function arrive(events: unknown): void {
  if (!Array.isArray(events)) return;
  show(events.reduce<Transformed>(apply, current));
  waitForProcessor();
}

// The number of keys that are down, from the processor.
function arrivePressed(pressed: unknown): void {
  if (pressed === 0) endNotes();
  waitForProcessor();
}

// For React's useSyncExternalStore: what passed through, the same object
// until it changes.
export function transformed(): Transformed {
  return current;
}

// Follow what passes through. The first listener subscribes to the
// stream, the last one leaves it. Events that come while nobody listens
// are lost, so a store without listeners starts afresh.
export function subscribeTransformed(listener: () => void): () => void {
  listeners.add(listener);
  if (!unsubscribe) {
    const stopEvents = runtime().on("transformed", arrive);
    const stopPressed = senderValue<unknown>("pressed").subscribe(arrivePressed);
    unsubscribe = () => {
      stopEvents();
      stopPressed();
    };
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size > 0 || !unsubscribe) return;
    unsubscribe();
    unsubscribe = null;
    clearTimeout(wait);
    current = EMPTY;
  };
}
