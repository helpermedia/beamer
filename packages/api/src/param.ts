import { runtime } from "./runtime.js";
import type { Beamer, BeamerParamInfo } from "./types.js";
import type { Readable } from "./stores.js";

/**
 * A parameter of the plugin, by its string id.
 *
 * Every change goes to the host as a gesture (begin, set, end), which the
 * host records as one undo step. A parameter has at most one open gesture:
 * a gesture that begins while another is open ends that one first, so
 * gestures from two controls (a knob and its text field) never overlap.
 *
 * Before `ready` a parameter reads as empty (0, "", no choices) and changes
 * do nothing. `get` and `subscribe` work detached from the handle, as
 * React's `useSyncExternalStore` and Svelte call them.
 */
export interface Param extends Readable<number> {
  readonly id: string;
  /** What the runtime knows of it, undefined before `ready` or for an unknown id. */
  readonly info: BeamerParamInfo | undefined;
  /** The normalized value (0 to 1). */
  readonly get: () => number;
  /** Call `callback` with the value now and on every change. Returns the function that unsubscribes. */
  readonly subscribe: (callback: (value: number) => void) => () => void;
  /** The value in plain units, as the parameter computes it. */
  plain(): number;
  /** The value as the parameter formats it, without the unit. */
  displayText(): string;
  /** The value with its unit, as a host shows it: "-6.0 dB", "40%". */
  text(): string;

  /**
   * Begin a gesture, such as a drag. `onInterrupt` is called when another
   * gesture of this parameter ends it early.
   */
  begin(onInterrupt?: () => void): void;
  /** Set the normalized value, clamped to 0 to 1, within the open gesture. */
  set(normalized: number): void;
  /** End the open gesture. */
  end(): void;
  /** Set the normalized value as a gesture of its own. */
  edit(normalized: number): void;
  /** Set the default value as a gesture of its own. */
  reset(): void;
  /**
   * Set the value a user typed, read by the parameter's own parser. Resolves
   * to false when the text is no value or the plugin could not read it.
   */
  typeValue(text: string): Promise<boolean>;

  /** The display text of each choice of a discrete parameter. */
  choices(): string[];
  /** The index of the choice a discrete parameter is at. Throws for a continuous one. */
  choice(): number;
  /** Set a discrete parameter to the choice at `index`. Throws for a continuous one. */
  choose(index: number): void;
}

function clamp(normalized: number): number {
  return Math.min(1, Math.max(0, normalized));
}

class ParamHandle implements Param {
  #open = false;
  #onInterrupt: (() => void) | null = null;

  constructor(
    readonly id: string,
    readonly beamer: Beamer,
  ) {}

  get info(): BeamerParamInfo | undefined {
    return this.beamer.params.info(this.id);
  }

  readonly get = (): number => this.beamer.params.get(this.id);

  readonly subscribe = (callback: (value: number) => void): (() => void) => {
    callback(this.get());
    return this.beamer.params.on(this.id, callback);
  };

  plain(): number {
    return this.beamer.params.getPlain(this.id);
  }

  displayText(): string {
    return this.beamer.params.getDisplayText(this.id);
  }

  text(): string {
    const units = this.info?.units;
    const text = this.displayText();
    if (!units) return text;
    return units === "%" ? text + units : `${text} ${units}`;
  }

  begin(onInterrupt?: () => void): void {
    if (!this.info) return;
    // End an open gesture first and tell its owner.
    if (this.#open) {
      const interrupted = this.#onInterrupt;
      this.end();
      interrupted?.();
    }
    this.beamer.params.beginEdit(this.id);
    this.#open = true;
    this.#onInterrupt = onInterrupt ?? null;
  }

  set(normalized: number): void {
    if (!this.info) return;
    this.beamer.params.set(this.id, clamp(normalized));
  }

  end(): void {
    if (!this.#open) return;
    this.#open = false;
    this.#onInterrupt = null;
    this.beamer.params.endEdit(this.id);
  }

  edit(normalized: number): void {
    if (!this.info) return;
    this.begin();
    this.set(normalized);
    this.end();
  }

  reset(): void {
    const info = this.info;
    if (info) this.edit(info.defaultValue);
  }

  async typeValue(text: string): Promise<boolean> {
    const info = this.info;
    if (!info || text.trim() === "") return false;
    let normalized: unknown;
    try {
      normalized = await this.beamer.invoke("_beamer/paramTextToNormalized", info.id, text);
    } catch {
      return false;
    }
    if (typeof normalized !== "number") return false;
    this.edit(normalized);
    return true;
  }

  choices(): string[] {
    return this.info?.valueTexts ?? [];
  }

  choice(): number {
    const steps = this.#steps();
    return steps === undefined ? 0 : Math.round(this.get() * steps);
  }

  choose(index: number): void {
    const steps = this.#steps();
    if (steps !== undefined) this.edit(index / steps);
  }

  // The steps of a discrete parameter, undefined before `ready`.
  #steps(): number | undefined {
    const info = this.info;
    if (!info) return undefined;
    if (info.steps <= 0) {
      throw new Error(`Parameter "${this.id}" is continuous: it has no choices.`);
    }
    return info.steps;
  }
}

// One handle per parameter, so that all controls of a parameter share its
// gesture. Kept per runtime: a page that gets a new runtime starts afresh.
const handles = new WeakMap<Beamer, Map<string, ParamHandle>>();

/** The parameter with the string id `id`. */
export function param(id: string): Param {
  const beamer = runtime();
  let byId = handles.get(beamer);
  if (!byId) {
    byId = new Map();
    handles.set(beamer, byId);
  }
  let handle = byId.get(id);
  if (!handle) {
    handle = new ParamHandle(id, beamer);
    byId.set(id, handle);
  }
  return handle;
}
