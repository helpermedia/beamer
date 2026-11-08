// Types of `__BEAMER__`, the runtime that a Beamer plugin injects into the
// page of its editor before any page code runs. See docs/WEBVIEW.md.

/** What the runtime knows of a parameter. */
export interface BeamerParamInfo {
  /** Numeric id, a hash of the string id, for host calls. */
  id: number;
  /** The `id` of the `#[parameter]` attribute, behind the names of nested groups. */
  stringId: string;
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

export interface BeamerParams {
  get(stringId: string): number;
  getPlain(stringId: string): number;
  getDisplayText(stringId: string): string;
  set(stringId: string, value: number): void;
  beginEdit(stringId: string): void;
  endEdit(stringId: string): void;
  on(stringId: string, callback: (value: number) => void): () => void;
  all(): BeamerParamInfo[];
  info(stringId: string): BeamerParamInfo | undefined;
}

export interface BeamerState {
  /** The value the UI knows, the same object until it changes. */
  get<T = unknown>(path: string): T | undefined;
  /** Set a value; the plugin answers with the value in effect. */
  set(path: string, value: unknown): void;
  /** Subscribe to changes, also those behind the UI's back. */
  on<T = unknown>(path: string, callback: (value: T) => void): () => void;
  /** The paths of all State fields, empty before `ready`. */
  ids(): string[];
}

export interface BeamerValues {
  /** The last value of a value sender, the same object until a new one arrives. */
  get<T = unknown>(name: string): T | undefined;
  /** Subscribe to the values that arrive from now on. */
  on<T = unknown>(name: string, callback: (value: T) => void): () => void;
}

export interface Beamer {
  /** Resolves once parameters and State fields have arrived. */
  readonly ready: Promise<void>;
  /** The plugin format the editor runs in, set before `ready` resolves. */
  readonly pluginFormat: "vst3" | "auv2" | "auv3" | null;
  /** The name of MIDI note 60 in the plugin's note names, set before `ready` resolves. */
  readonly middleC: "C3" | "C4" | null;
  readonly params: BeamerParams;
  readonly state: BeamerState;
  readonly values: BeamerValues;
  invoke(method: string, ...args: unknown[]): Promise<unknown>;
  /** Events the plugin emits and the arrays of its stream senders. */
  on(event: string, callback: (data: unknown) => void): () => void;
  emit(event: string, data?: unknown): void;
}

declare global {
  // The runtime, undefined when the page runs outside a plugin editor.
  var __BEAMER__: Beamer | undefined;
}
