// React hooks for the web UI of a Beamer plugin, on top of beamer-api and
// beamer-gestures. Each one follows the plugin through React's
// useSyncExternalStore or an effect.

export { useEvent, useGestures, useParam, useReady, useSenderValue, useStateField } from "./hooks.js";
export type { ParamView } from "./hooks.js";
