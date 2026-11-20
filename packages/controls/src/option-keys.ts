// A compile-time check that a control takes every option it is given, so
// that a new option cannot pass by unseen.

/** Compiles only when `K` is never; the error names the keys that are left. */
export type NoKeyLeft<K extends never> = K;
