interface BeamerParamInfo {
  id: number;
  stringId: string;
  name: string;
  value: number;
  plainValue: number;
  displayText: string;
  format: string;
  defaultValue: number;
  min: number;
  max: number;
  units: string;
  steps: number;
  /** The display text of each value of a discrete parameter, from the lowest up. Empty for a continuous one and above 128 values. */
  valueTexts: string[];
}

interface BeamerParams {
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

interface BeamerState {
  /** The value the UI knows, the same object until it changes. */
  get<T = unknown>(path: string): T | undefined;
  /** Set a value; the plugin answers with the value in effect. */
  set(path: string, value: unknown): void;
  /** Subscribe to changes, also those behind the UI's back. Returns the unsubscribe function. */
  on<T = unknown>(path: string, callback: (value: T) => void): () => void;
  /** The paths of all State fields, empty before `ready`. */
  ids(): string[];
}

interface BeamerValues {
  /** The last value of a value sender, the same object until a new one arrives. Undefined before the first. */
  get<T = unknown>(name: string): T | undefined;
  /** Subscribe to the values that arrive from now on; the one that arrived already is `get()`. Returns the unsubscribe function. */
  on<T = unknown>(name: string, callback: (value: T) => void): () => void;
}

interface Beamer {
  readonly ready: Promise<void>;
  /** The plugin format the editor runs in, set before `ready` resolves. */
  readonly pluginFormat: "vst3" | "auv2" | "auv3" | null;
  readonly params: BeamerParams;
  /** The State fields of the plugin, by path. */
  readonly state: BeamerState;
  /** The values the value senders of the plugin send from the audio thread, by name. */
  readonly values: BeamerValues;
  invoke(method: string, ...args: unknown[]): Promise<unknown>;
  /**
   * Listen to events from Rust: what the plugin emits and the arrays of its
   * stream senders. Not kept: a listener gets what comes after it
   * registered. Returns the function that unsubscribes.
   */
  on(event: string, callback: (data: unknown) => void): () => void;
  emit(event: string, data?: unknown): void;

  /** @internal Called by native code to initialize parameters and state. */
  _onInit(
    params: BeamerParamInfo[],
    info?: { pluginFormat?: string; state?: Record<string, unknown> },
  ): void;
  /** @internal Called by native code to push parameter changes. */
  _onParams(changed: Record<string, [number, number, string]>): void;
  /** @internal Called by native code to push State changes. */
  _onState(changed: Record<string, unknown>): void;
  /** @internal Called by native code to resolve/reject invoke promises. */
  _onResult(callId: number, result: { ok?: unknown; err?: string }): void;
  /** @internal Called by native code to dispatch events. */
  _onEvent(name: string, data: unknown): void;
  /** @internal Called by native code with the value of a value sender, kept in `values`. */
  _onValue(name: string, data: unknown): void;
}

declare const __BEAMER__: Beamer;
