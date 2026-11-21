// Generated from the plugin's model. Do not edit: `cargo test` fails
// while this file differs from the model. `cargo xtask bundle` writes
// it, or on its own:
//
//   cargo xtask model-types webview-demo

/**
 * How the pan control distributes the signal over the two channels.
 *
 * This is a setting and not a parameter: a host does not automate it. The
 * model keeps it in a `State` field.
 */
export type PanLaw =
  /** Equal loudness across the stereo field (-3 dB at the center). */
  | "constant_power"
  /** A plain crossfade (-6 dB at the center). */
  | "linear";

/** The output level as the meter shows it. */
export interface Levels {
  /**
   * The held peak of each channel, linear. Not in dB: the infinity of
   * silence would reach JavaScript as null.
   */
  peak: [number, number];
}

/** What the editor's About box shows. */
export interface Info {
  /** The plugin's name. */
  name: string;
  /** The plugin's version. */
  version: string;
  /** The framework it is built with. */
  framework: string;
}

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters: "gain" | "pan";
    state: {
      pan_law: PanLaw;
    };
    values: {
      levels: Levels;
    };
    streams: {
      history: number;
    };
    events: {
      sampleRate: number;
    };
    methods: {
      info: { arguments: []; result: Info };
      sampleRate: { arguments: []; result: number };
    };
    pageEvents: {};
  }
}
