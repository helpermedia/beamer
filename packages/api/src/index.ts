// The base package for the web UI of a Beamer plugin: typed access to the
// plugin's parameters, State fields and values from the audio thread. The
// other packages build on it.

export type {
  Beamer,
  BeamerParamInfo,
  BeamerParams,
  BeamerState,
  BeamerValues,
} from "./types.js";
export { ready, runtime } from "./runtime.js";
export { param } from "./param.js";
export type { Param } from "./param.js";
export { senderValue, stateField } from "./stores.js";
export { noteName } from "./notes.js";
export type { Readable, StateField } from "./stores.js";
