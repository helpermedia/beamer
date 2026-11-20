// Compile-time checks on the options of a control: that a component
// takes every one (none lands on its element unseen) and that the
// options compared by a JSON key hold no function, which JSON drops.

/**
 * An object literal with every key of `T` and no other, each given or
 * undefined. The keys come from `Required<T>` so that each value keeps
 * its `undefined`, which a `-?` mapping would strip.
 */
export type EveryKeyOf<T> = { [K in keyof Required<T>]: T[K] | undefined };

/** Compiles only when `K` is never; the error names the keys that are left. */
export type NoKeyLeft<K extends never> = K;

/**
 * The keys of `T` whose value can be a function. A JSON key drops such a
 * value, so the options compared by one must have none. Only an option's
 * own value counts, not what it holds: a function inside an object or a
 * `Set` passes, though JSON loses it too.
 */
export type FunctionKeys<T> = {
  [K in keyof T]-?: Extract<T[K], (...args: never[]) => unknown> extends never ? never : K;
}[keyof T];
