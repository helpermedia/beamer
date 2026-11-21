// The base package for the web UI of a Beamer plugin: typed access to the
// plugin's parameters, State fields and values from the audio thread,
// plus notes for its processor. Ids and values follow the model's types
// when the page has its generated file. The other packages build on it.

export type {
  Beamer,
  BeamerParameterInfo,
  BeamerParameters,
  BeamerState,
  BeamerValues,
} from "./types.js";
export { ready, runtime } from "./runtime.js";
export { parameter } from "./parameter.js";
export type { Parameter } from "./parameter.js";
export { senderValue, stateField } from "./stores.js";
export { allNotesOff, noteName, noteOff, noteOn, playNote } from "./notes.js";
export type { NoteOptions, PlayNoteOptions } from "./notes.js";
export type { Readable, StateField } from "./stores.js";
export type {
  MethodArguments,
  MethodName,
  MethodResult,
  ModelTypes,
  PageEventArguments,
  PageEventName,
  ParameterId,
  PluginEventData,
  PluginEventName,
  SenderValue,
  StatePath,
  StateValue,
  ValueSenderName,
} from "./model-types.js";
