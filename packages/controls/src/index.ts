// Unstyled controls for the web UI of a Beamer plugin: attach functions
// that make the author's own elements a dial, a slider, a switch, a radio
// group or a value field for a parameter, with roles, ARIA and input, each
// change one undo step in the host. And a meter that falls on its own.
// They draw nothing.

export { attachDial } from "./dial.js";
export { attachSlider } from "./slider.js";
export { thumbPosition } from "./thumb-position.js";
export type { DialDrag, DialOptions, InputOptions, ResetGesture, SliderOptions, SliderOrientation } from "./gestures.js";
export { attachSwitch } from "./switch.js";
export { attachChoices } from "./choices.js";
export { attachValueEntry, openValueEntry } from "./value-entry.js";
export type { ValueEntryOptions } from "./value-entry.js";
export { fallingMeter } from "./meter.js";
export type { FallingMeterOptions, MeterLevels } from "./meter.js";
export { KEYBOARD_FOCUS } from "./focus.js";
