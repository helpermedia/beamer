import type { ParameterId } from "@helpermedia/beamer-api";

// The drums as the processor has them (`DrumType` in src/lib.rs): one pad
// and one strip each. The processor maps each GM note to its drum; the
// editor mirrors that table.

export interface Drum {
  // The string id of the drum's level parameter. Its name is the drum's.
  levelParameterId: ParameterId;
  // The string id of the drum's pan parameter, its balance.
  panParameterId: ParameterId;
  // The string id of the parameter that picks the drum's output.
  outputParameterId: ParameterId;
  // The MIDI note that plays it, as General MIDI places it.
  note: number;
}

export const DRUMS: Drum[] = [
  { levelParameterId: "kick_level", panParameterId: "kick_pan", outputParameterId: "kick_output", note: 36 },
  { levelParameterId: "snare_level", panParameterId: "snare_pan", outputParameterId: "snare_output", note: 38 },
  { levelParameterId: "hihat_level", panParameterId: "hihat_pan", outputParameterId: "hihat_output", note: 42 },
  { levelParameterId: "crash_level", panParameterId: "crash_pan", outputParameterId: "crash_output", note: 49 },
];

// The MIDI channel of drums in General MIDI, counted from 0.
export const DRUM_CHANNEL = 9;
