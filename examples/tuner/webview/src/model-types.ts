// Generated from the plugin's model. Do not edit: `cargo test` fails
// while this file differs from the model. `cargo xtask bundle` writes
// it, or on its own:
//
//   cargo xtask model-types tuner

/**
 * What the detector heard in its newest window. The processor sends it to
 * the editor, which works out the note and the cents from it.
 */
export interface Pitch {
  /** The fundamental in Hz, 0 when the window holds no clear pitch. */
  frequencyHz: number;
  /** How periodic the window is, from 0 (noise) to 1 (a pure tone). */
  clarity: number;
  /** The RMS level of the window in dB, at least -80 dB (the floor). */
  levelDb: number;
}

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters: "reference" | "mute";
    state: {};
    values: {
      pitch: Pitch;
    };
    streams: {};
    events: {};
  }
}
