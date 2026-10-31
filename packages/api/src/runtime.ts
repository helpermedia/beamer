import type { Beamer } from "./types.js";

/**
 * The runtime of the plugin.
 *
 * Throws when the page runs outside a plugin editor, for example in a
 * browser during design work without a stand-in for `__BEAMER__`.
 */
export function runtime(): Beamer {
  const beamer = globalThis.__BEAMER__;
  if (!beamer) {
    throw new Error(
      "No Beamer runtime: the page runs outside a plugin editor. Define globalThis.__BEAMER__ to work on it in a browser.",
    );
  }
  return beamer;
}

/** Resolves once the plugin's parameters and State fields have arrived. */
export function ready(): Promise<void> {
  return runtime().ready;
}
