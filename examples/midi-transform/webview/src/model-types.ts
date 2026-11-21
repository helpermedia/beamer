// Generated from the plugin's model. Do not edit: `cargo test` fails
// while this file differs from the model. `cargo xtask bundle` writes
// it, or on its own:
//
//   cargo xtask model-types midi-transform

/**
 * A note or a CC as it came in and as it went out, sent to the editor,
 * which lights up the notes that sound and shows the last CC. As JSON an
 * object with its `type` ("noteOn", "noteOff", "cc" or "reset") and the
 * fields in camelCase.
 */
export type Transformed =
  /**
   * A note started: its pitch in and out, `None` when it was filtered
   * out. Velocities from 0.0 to 1.0.
   */
  | {
      type: "noteOn";
      channel: number;
      pitch: number;
      outPitch: number | null;
      velocity: number;
      outVelocity: number;
    }
  /** The note on `pitch` ended, by its incoming pitch. */
  | { type: "noteOff"; channel: number; pitch: number }
  /** A CC in and out. Values from 0.0 to 1.0. */
  | {
      type: "cc";
      channel: number;
      controller: number;
      value: number;
      outController: number;
      outValue: number;
    }
  /** The host reset the plugin: no note sounds any more. */
  | { type: "reset" };

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters:
      | "bypass"
      | "note/enabled"
      | "note/mode"
      | "note/transpose"
      | "note/input"
      | "note/output"
      | "note/velocity"
      | "cc/enabled"
      | "cc/mode"
      | "cc/input"
      | "cc/output"
      | "cc/scale";
    state: {};
    values: {
      pressed: number;
    };
    streams: {
      transformed: Transformed;
    };
    events: {};
  }
}
