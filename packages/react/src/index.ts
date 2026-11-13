// Unstyled components and hooks for the web UI of a Beamer plugin, on top
// of beamer-api and beamer-controls. Each one follows the plugin through
// React's useSyncExternalStore or an effect.

export {
  useChoices,
  useDial,
  useEvent,
  useParameter,
  useReadable,
  useReady,
  useSenderValue,
  useSlider,
  useStateField,
  useSwitch,
  useValueEntry,
} from "./hooks.js";
export type { ParameterView } from "./hooks.js";
export { Choices, Dial, Slider, Switch, ValueEntry } from "./components.js";
export type {
  ChoicesProps,
  ChoicesState,
  DialProps,
  DialState,
  SliderProps,
  SliderState,
  SwitchProps,
  SwitchState,
  ValueEntryProps,
} from "./components.js";
