import { runtime } from "./runtime.js";

// The names of the notes of an octave from C, as the plugin names them.
const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/**
 * The name of a MIDI note number, as the plugin's parameters of `kind =
 * "note_number"` name it: 60 is "C3", or "C4" for a plugin with `middle_c =
 * "C4"` in its Config.toml. Before `ready` it names middle C C3, the
 * default.
 */
export function noteName(noteNumber: number): string {
  const note = Math.round(noteNumber);
  const middleOctave = runtime().middleC === "C4" ? 4 : 3;
  const octave = Math.floor(note / 12) + middleOctave - 5;
  return `${NAMES[((note % 12) + 12) % 12]}${octave}`;
}

/** How a note plays: a velocity 0 to 1 and a channel 0 to 15. */
export interface NoteOptions {
  /** 0 to 1. A note-on defaults to 1, a note-off to MIDI's release velocity (64/127). */
  velocity?: number;
  /** 0 to 15, default 0. */
  channel?: number;
}

/** How `playNote` plays: a note's options and how long it lasts. */
export interface PlayNoteOptions extends NoteOptions {
  /** How long the note lasts in milliseconds, default 100. */
  duration?: number;
}

// The note-offs that `playNote` has waiting, per runtime and by channel and
// note. A newer start of the note takes its place, so that the timer of an
// older one does not end it.
const pendingOffs = new WeakMap<object, Map<string, object>>();

function pendingOf(beamer: object): Map<string, object> {
  let pending = pendingOffs.get(beamer);
  if (!pending) {
    pending = new Map();
    pendingOffs.set(beamer, pending);
  }
  return pending;
}

function noteKey(noteNumber: number, channel = 0): string {
  return `${channel}:${noteNumber}`;
}

/**
 * Start a note in the processor, which gets it with the host's MIDI input:
 * a pad pressed, a key of an on-screen keyboard held down. End it with
 * `noteOff`. A note number is 0 to 127. A velocity of 0 is the smallest,
 * not a note-off. The plugin ends the notes the page left on when the
 * editor closes or the page loads again.
 */
export function noteOn(noteNumber: number, options: NoteOptions = {}): void {
  const beamer = runtime();
  pendingOf(beamer).delete(noteKey(noteNumber, options.channel));
  beamer.noteOn(noteNumber, options.velocity, options.channel);
}

/** End a note that `noteOn` or `playNote` started. */
export function noteOff(noteNumber: number, options: NoteOptions = {}): void {
  const beamer = runtime();
  pendingOf(beamer).delete(noteKey(noteNumber, options.channel));
  beamer.noteOff(noteNumber, options.velocity, options.channel);
}

/**
 * Play a short note: start it now and end it after `duration` milliseconds
 * (default 100). For a click that does not hold, such as a key press on a
 * focused pad. A pad that sounds while held uses `noteOn` and `noteOff`.
 * Playing the note again before it ended starts it anew and ends it after
 * the new duration.
 */
export function playNote(noteNumber: number, options: PlayNoteOptions = {}): void {
  const { duration = 100, ...note } = options;
  const beamer = runtime();
  const pending = pendingOf(beamer);
  const key = noteKey(noteNumber, note.channel);
  beamer.noteOn(noteNumber, note.velocity, note.channel);
  const token = {};
  pending.set(key, token);
  setTimeout(() => {
    if (pending.get(key) !== token) return;
    pending.delete(key);
    beamer.noteOff(noteNumber, undefined, note.channel);
  }, duration);
}

/** End every note the page started, also those `playNote` has yet to end. */
export function allNotesOff(): void {
  const beamer = runtime();
  pendingOf(beamer).clear();
  beamer.allNotesOff();
}
