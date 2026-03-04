interface BeamerParamInfo {
  id: number;
  stringId: string;
  name: string;
  value: number;
  defaultValue: number;
  min: number;
  max: number;
  units: string;
  steps: number;
}

interface BeamerParams {
  get(stringId: string): number;
  set(stringId: string, value: number): void;
  beginEdit(stringId: string): void;
  endEdit(stringId: string): void;
  on(stringId: string, callback: (value: number) => void): () => void;
  all(): BeamerParamInfo[];
  info(stringId: string): BeamerParamInfo | undefined;
}

interface Beamer {
  readonly ready: Promise<void>;
  readonly params: BeamerParams;
  invoke(method: string, ...args: unknown[]): Promise<unknown>;
  on(event: string, callback: (data: unknown) => void): () => void;
  emit(event: string, data?: unknown): void;

  /** @internal Called by native code to initialize parameters. */
  _onInit(params: BeamerParamInfo[]): void;
  /** @internal Called by native code to push parameter changes. */
  _onParams(changed: Record<number, number>): void;
  /** @internal Called by native code to resolve/reject invoke promises. */
  _onResult(callId: number, result: { ok?: unknown; err?: string }): void;
  /** @internal Called by native code to dispatch events. */
  _onEvent(name: string, data: unknown): void;
}

declare const __BEAMER__: Beamer;
