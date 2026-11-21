// Generated from the plugin's model. Do not edit: `cargo test` fails
// while this file differs from the model. `cargo xtask bundle` writes
// it, or on its own:
//
//   cargo xtask model-types drum-synth

/**
 * The tab the editor shows: the pads or the mixer. A setting of the editor,
 * not a parameter: a host does not automate it. It is saved with the
 * project, so the editor opens on the tab it was left on.
 */
export type Tab = "pads" | "mixer";

/**
 * A drum that was hit, sent to the editor, which flashes the drum's pad
 * and the light of its strip.
 * As JSON an object with the drum (0 kick, 1 snare, 2 hi-hat, 3 crash) and
 * the velocity from 0.0 to 1.0.
 */
export interface Hit {
  drum: number;
  velocity: number;
}

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters:
      | "kick_level"
      | "snare_level"
      | "hihat_level"
      | "crash_level"
      | "kick_pan"
      | "snare_pan"
      | "hihat_pan"
      | "crash_pan"
      | "kick_output"
      | "snare_output"
      | "hihat_output"
      | "crash_output";
    state: {
      tab: Tab;
    };
    values: {
      meters: [number, number, number, number];
    };
    streams: {
      hits: Hit;
    };
    events: {};
  }
}
