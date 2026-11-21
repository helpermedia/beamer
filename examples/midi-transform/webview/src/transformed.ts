import { runtime, senderValue } from "@helpermedia/beamer-api";
import type { Transformed as Passed } from "./model-types";

// What passes through the plugin, from the stream sender "transformed"
// (`Transformed` in model-types.ts): every note and CC as it came in and as
// it went out, in arrays of what arrived since the last tick. This store
// keeps what the displays show: the notes that sound and the last note
// and CC.
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

function key(channel: number, pitch: number): string {
  return `${channel}:${pitch}`;
}

// The state after one event. Returns `state` itself when the event changes
// nothing.
function apply(state: Transformed, event: Passed): Transformed {
  switch (event.type) {
    case "noteOn": {
      const { pitch, outPitch, velocity, outVelocity } = event;
      const note: Note = { pitch, outPitch, velocity, outVelocity };
      const held = new Map(state.held);
      held.set(key(event.channel, pitch), note);
      return { ...state, held, lastNote: note };
    }
    case "noteOff": {
      const noteKey = key(event.channel, event.pitch);
      if (!state.held.has(noteKey)) return state;
      const held = new Map(state.held);
      held.delete(noteKey);
      return { ...state, held };
    }
    case "cc": {
      const { channel, controller, value, outController, outValue } = event;
      const cc: Cc = { controller, value, outController, outValue };
      let held = state.held;
      if (ENDS_NOTES.includes(outController)) {
        held = new Map([...held].filter(([noteKey]) => !noteKey.startsWith(`${channel}:`)));
      }
      return { ...state, held, lastCc: cc };
    }
    case "reset":
      return state.held.size > 0 ? { ...state, held: new Map() } : state;
  }
}

function arrive(events: Passed[]): void {
  show(events.reduce<Transformed>(apply, current));
  waitForProcessor();
}

// The number of keys that are down, from the processor.
function arrivePressed(pressed: number | undefined): void {
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
    const stopPressed = senderValue("pressed").subscribe(arrivePressed);
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
