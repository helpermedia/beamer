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

interface Beamer {
  readonly ready: Promise<void>;
  /** The plugin format the editor runs in, set before `ready` resolves. */
  readonly pluginFormat: "vst3" | "auv2" | "auv3" | null;
  readonly params: BeamerParams;
  /** The State fields of the plugin, by path. */
  readonly state: BeamerState;
  invoke(method: string, ...args: unknown[]): Promise<unknown>;
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
}

declare const __BEAMER__: Beamer;
