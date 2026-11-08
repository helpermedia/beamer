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
