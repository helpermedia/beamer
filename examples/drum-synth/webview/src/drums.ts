// The drums as the processor has them (`DrumType` and `output_bus_info` in
// src/lib.rs): one strip each, in the order of the output buses. The
// processor maps each GM note to its drum; the editor mirrors that table.

export interface Drum {
  // The string id of the drum's level parameter. Its name is the name of
  // the drum and of its output bus.
  parameterId: string;
  // The MIDI note that plays it, as General MIDI places it.
  note: number;
}

export const DRUMS: Drum[] = [
  { parameterId: "kick_level", note: 36 },
  { parameterId: "snare_level", note: 38 },
  { parameterId: "hihat_level", note: 42 },
  { parameterId: "crash_level", note: 49 },
];
