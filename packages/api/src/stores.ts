import { runtime } from "./runtime.js";
import type { Beamer } from "./types.js";

/**
 * A value to read and follow, in the shape that React's
 * `useSyncExternalStore` and Svelte's stores expect: `get` returns the same
 * object until the value changes, `subscribe` calls back at once with the
 * value now and then on every change. It returns the function that
 * unsubscribes. Both work detached from the object.
 */
export interface Readable<T> {
  readonly get: () => T;
  readonly subscribe: (callback: (value: T) => void) => () => void;
}

/** A `State` field of the plugin, by its path. */
export interface StateField<T> extends Readable<T | undefined> {
  /** Set the value. The plugin answers with the value in effect. */
  readonly set: (value: T) => void;
}

// One object per path or name, so that a component that asks for it on
// every render gets the same one and does not subscribe again. Kept per
// runtime, like the parameter handles. The value types differ per path or
// name, so the caches hold `object` and each function knows the type of
// what it put in.
const stateFields = new WeakMap<Beamer, Map<string, object>>();
const senderValues = new WeakMap<Beamer, Map<string, object>>();

function cached<V extends object>(cache: WeakMap<Beamer, Map<string, object>>, key: string, make: (beamer: Beamer) => V): V {
  const beamer = runtime();
  let byKey = cache.get(beamer);
  if (!byKey) {
    byKey = new Map();
    cache.set(beamer, byKey);
  }
  let value = byKey.get(key) as V | undefined;
  if (value === undefined) {
    value = make(beamer);
    byKey.set(key, value);
  }
  return value;
}

/** The `State` field at `path`. Undefined before `ready`. */
export function stateField<T>(path: string): StateField<T> {
  return cached(stateFields, path, (beamer): StateField<T> => {
    const get = () => beamer.state.get<T>(path);
    return {
      get,
      subscribe: (callback: (value: T | undefined) => void) => {
        callback(get());
        return beamer.state.on<T>(path, callback);
      },
      set: (value: T) => beamer.state.set(path, value),
    };
  });
}

/**
 * The newest value of the value sender `name`, which the plugin sends from
 * the audio thread. Undefined before the first one.
 */
export function senderValue<T>(name: string): Readable<T | undefined> {
  return cached(senderValues, name, (beamer): Readable<T | undefined> => {
    const get = () => beamer.values.get<T>(name);
    return {
      get,
      subscribe: (callback: (value: T | undefined) => void) => {
        callback(get());
        return beamer.values.on<T>(name, callback);
      },
    };
  });
}
