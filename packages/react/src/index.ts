// React hooks for the web UI of a Beamer plugin, on top of beamer-api and
// beamer-controls. Each one follows the plugin through React's
// useSyncExternalStore or an effect.

export {
  useChoices,
  useDial,
  useEvent,
  useParam,
  useReadable,
  useReady,
  useSenderValue,
  useSlider,
  useStateField,
  useSwitch,
  useValueEntry,
} from "./hooks.js";
export type { ParamView } from "./hooks.js";
