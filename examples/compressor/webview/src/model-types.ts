// Generated from the plugin's model. Do not edit: `cargo test` fails
// while this file differs from the model. `cargo xtask bundle` writes
// it, or on its own:
//
//   cargo xtask model-types compressor

/**
 * What the editor needs to know of the processor, sent from the audio
 * thread.
 */
export interface Status {
  /**
   * The sample rate the host prepared the plugin for, which turns the
   * lookahead into samples.
   */
  sampleRate: number;
  /**
   * Whether the detector keys on the sidechain: the parameter asks for
   * it, the bus is on and the host passes its samples.
   */
  keyedOnSidechain: boolean;
}

/** The levels as the editor's meters show them. */
export interface Meters {
  /**
   * The held peak of the level the detector keys on (the main input or
   * the sidechain) in dB, at least the floor of the meters.
   */
  keyDb: number;
  /** The held gain reduction in dB, a positive number. */
  reductionDb: number;
  /** The auto makeup gain in effect in dB, 0 while auto makeup is off. */
  autoMakeupDb: number;
}

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters:
      | "threshold"
      | "ratio"
      | "attack"
      | "release"
      | "knee"
      | "lookahead"
      | "auto_makeup"
      | "makeup"
      | "bypass"
      | "sidechain";
    state: {};
    values: {
      status: Status;
      meters: Meters;
    };
    streams: {};
    events: {};
  }
}
