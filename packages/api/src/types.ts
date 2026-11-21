// Types of `__BEAMER__`, the runtime that a Beamer plugin injects into the
// page of its editor before any page code runs. See docs/WEBVIEW.md. Ids
// and values follow the model's types (model-types.ts).

import type {
  MethodArguments,
  MethodName,
  MethodResult,
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

/** What the runtime knows of a parameter. */
export interface BeamerParameterInfo {
  /** Numeric id, a hash of the string id, for host calls. */
  id: number;
  /** The `id` of the `#[parameter]` attribute, behind the names of nested groups. */
  stringId: ParameterId;
  name: string;
  /** Current normalized value (0 to 1). */
  value: number;
  /** Current value in plain units. */
  plainValue: number;
  /** Current value as the parameter formats it, without the unit. */
  displayText: string;
  /** Formatter kind: float, db, hz, ms, s, percent, pan, ratio, semitones, note_number, boolean or enum. */
  format: string;
  /** Default value, normalized. */
  defaultValue: number;
  /** Plain value at normalized 0. */
  min: number;
  /** Plain value at normalized 1. */
  max: number;
  units: string;
  /** 0 for a continuous parameter, the number of steps for a discrete one. */
  steps: number;
  /** The display text of each value of a discrete parameter, from the lowest up. Empty for a continuous one and above 128 values. */
  valueTexts: string[];
}

export interface BeamerParameters {
  get(stringId: ParameterId): number;
  getPlain(stringId: ParameterId): number;
  getDisplayText(stringId: ParameterId): string;
  set(stringId: ParameterId, value: number): void;
  beginEdit(stringId: ParameterId): void;
  endEdit(stringId: ParameterId): void;
  on(stringId: ParameterId, callback: (value: number) => void): () => void;
  all(): BeamerParameterInfo[];
  info(stringId: ParameterId): BeamerParameterInfo | undefined;
}

export interface BeamerState {
  /** The value the UI knows, the same object until it changes. */
  get<P extends StatePath>(path: P): StateValue<P> | undefined;
  /** Set a value; the plugin answers with the value in effect. */
  set<P extends StatePath>(path: P, value: StateValue<P>): void;
  /** Subscribe to changes, also those behind the UI's back. */
  on<P extends StatePath>(path: P, callback: (value: StateValue<P>) => void): () => void;
  /** The paths of all State fields, empty before `ready`. */
  ids(): StatePath[];
}

export interface BeamerValues {
  /** The last value of a value sender, the same object until a new one arrives. */
  get<N extends ValueSenderName>(name: N): SenderValue<N> | undefined;
  /** Subscribe to the values that arrive from now on. */
  on<N extends ValueSenderName>(name: N, callback: (value: SenderValue<N>) => void): () => void;
}

export interface Beamer {
  /** Resolves once parameters and State fields have arrived. */
  readonly ready: Promise<void>;
  /** The plugin format the editor runs in, set before `ready` resolves. */
  readonly pluginFormat: "vst3" | "auv2" | "auv3" | null;
  /** The name of MIDI note 60 in the plugin's note names, set before `ready` resolves. */
  readonly middleC: "C3" | "C4" | null;
  readonly parameters: BeamerParameters;
  readonly state: BeamerState;
  readonly values: BeamerValues;
  /** Call a method of the plugin with its arguments, in order. */
  invoke<M extends MethodName>(method: M, ...args: MethodArguments<M>): Promise<MethodResult<M>>;
  /** Events the plugin emits and the arrays of its stream senders. */
  on<N extends PluginEventName>(event: N, callback: (data: PluginEventData<N>) => void): () => void;
  /** Send an event to the plugin. */
  emit<N extends PageEventName>(event: N, ...data: PageEventArguments<N>): void;
  /**
   * Start a note in the processor, which gets it with the host's MIDI
   * input: a note number 0 to 127, a velocity 0 to 1 (default 1) and a
   * channel 0 to 15 (default 0). A velocity of 0 is the smallest, not a
   * note-off. A note that does not fit is dropped with a warning.
   */
  noteOn(note: number, velocity?: number, channel?: number): void;
  /** End a note, with a release velocity 0 to 1 (default 64/127). */
  noteOff(note: number, velocity?: number, channel?: number): void;
  /** End every note the page started. */
  allNotesOff(): void;
}

declare global {
  // The runtime, undefined when the page runs outside a plugin editor.
  var __BEAMER__: Beamer | undefined;
}
