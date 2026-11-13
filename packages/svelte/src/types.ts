// What the components hand to their children.

/** What a dial hands to its children. */
export interface DialState {
  /** The normalized value (0 to 1). */
  value: number;
  /**
   * Where the dial points, in degrees clockwise on screen from the right of
   * its center (`dialAngle` of beamer-controls, with the dial's arc).
   */
  angle: number;
  /** The value with its unit, as a host shows it. */
  text: string;
}

/** What a slider hands to its children. */
export interface SliderState {
  /** The normalized value (0 to 1). */
  value: number;
  /**
   * Where the thumb's center goes, as a part of the slider's length from its
   * low end (`thumbPosition` of beamer-controls, with the slider's
   * `thumbLength`).
   */
  position: number;
  /** The value with its unit, as a host shows it. */
  text: string;
}

/** What a switch hands to its children. */
export interface SwitchState {
  /** Whether the switch is on: from the middle of the range up. */
  checked: boolean;
}

/** What a radio group hands to its children. */
export interface ChoicesState {
  /** The display text of each choice, from the lowest value up. */
  choices: string[];
  /** The index of the checked choice, -1 before `ready` or for a continuous parameter. */
  checked: number;
}
