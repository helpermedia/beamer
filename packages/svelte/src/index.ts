// Unstyled Svelte components for the web UI of a Beamer plugin, on top of
// beamer-api and beamer-controls. Each one renders one element with a
// control of beamer-controls on it and hands what it shows to its
// `children` snippet. The stores of beamer-api follow Svelte's store
// contract, so `$parameter` and a `fallingMeter` work as they are.

export { default as Choices } from "./Choices.svelte";
export { default as Dial } from "./Dial.svelte";
export { default as Slider } from "./Slider.svelte";
export { default as Switch } from "./Switch.svelte";
export { default as ValueEntry } from "./ValueEntry.svelte";
export type { ChoicesState, DialState, SliderState, SwitchState } from "./types.js";
