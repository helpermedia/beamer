// Unstyled Svelte components for the web UI of a Beamer plugin, on top of
// beamer-api and beamer-controls. Each one renders one element with a
// control of beamer-controls on it and hands what it shows to its
// `children` snippet. `parameterView` follows what a parameter shows: its
// value, its text and its info. The stores of beamer-api follow Svelte's
// store contract, so `$parameter` (the normalized value) and the levels
// of `fallingLevels` work as they are.

export { default as Dial } from "./Dial.svelte";
export { default as RadioGroup } from "./RadioGroup.svelte";
export { default as Slider } from "./Slider.svelte";
export { default as Switch } from "./Switch.svelte";
export { default as TextField } from "./TextField.svelte";
export type { DialState, RadioGroupState, SliderState, SwitchState } from "./types.js";
export { parameterView } from "./parameter-view.js";
export type { ParameterView } from "./parameter-view.js";
