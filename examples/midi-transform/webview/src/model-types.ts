// Generated from the plugin's model. Do not edit: `cargo test` fails
// while this file differs from the model. `cargo xtask bundle` writes
// it, or on its own:
//
//   cargo xtask model-types midi-transform

/**
 * The tab the editor shows: notes and CCs, or the velocity curve. A
 * setting of the editor, not a parameter: a host does not automate it. It
 * is saved with the project, so the editor opens on the tab it was left
 * on.
 */
export type Tab = "notes_and_ccs" | "velocity";

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

/**
 * A curve through points, from 0.0 to 1.0 in and out.
 * At least two points, sorted by x and none at the same x.
 */
export interface Curve {
  shape: CurveShape;
  points: CurvePoint[];
}

/** How a `Curve` runs between its points. */
export type CurveShape =
  /** Straight lines from point to point. */
  | "linear"
  /** Smooth through every point without overshooting them: a monotone cubic. */
  | "smooth";

/** A point of a `Curve`, both coordinates from 0.0 to 1.0. */
export interface CurvePoint {
  /** The value that goes in. */
  x: number;
  /** The value that comes out. */
  y: number;
}

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters:
      | "bypass"
      | "note/enabled"
      | "note/mode"
      | "note/transpose"
      | "note/input"
      | "note/output"
      | "velocity/enabled"
      | "cc/enabled"
      | "cc/mode"
      | "cc/input"
      | "cc/output"
      | "cc/scale";
    state: {
      tab: Tab;
      "velocity/curve": Curve;
    };
    values: {
      pressed: number;
    };
    streams: {
      transformed: Transformed;
    };
    events: {};
  }
}
