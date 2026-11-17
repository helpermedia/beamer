// Unstyled controls for the web UI of a Beamer plugin: attach functions
// that make the author's own elements a dial, a slider, a switch, a radio
// group or a text field for a parameter, with roles, ARIA and input, each
// change one undo step in the host. And meter levels that fall on their
// own. The controls draw nothing.

export { attachDial } from "./dial.js";
export { attachSlider } from "./slider.js";
export { thumbPosition } from "./thumb-position.js";
export { dialAngle } from "./dial-angle.js";
export type { DialDrag, DialOptions, InputOptions, ResetGesture, SliderOptions, SliderOrientation } from "./gestures.js";
export { attachSwitch } from "./switch.js";
export { attachRadioGroup } from "./radio-group.js";
export { attachTextField, openTextField } from "./text-field.js";
export type { TextFieldOptions } from "./text-field.js";
export { fallingLevels } from "./falling-levels.js";
export type { FallingLevelsOptions, MeterLevels } from "./falling-levels.js";
export { KEYBOARD_FOCUS_ATTRIBUTE } from "./focus.js";
