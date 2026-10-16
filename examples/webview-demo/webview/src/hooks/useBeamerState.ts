import { useSyncExternalStore } from "react";

/**
 * The value of a State field of the plugin, by its path. Undefined until
 * the runtime is ready. The type is the caller's claim, not a check.
 */
export function useBeamerState<T>(path: string): T | undefined {
  return useSyncExternalStore(
    (cb) => __BEAMER__.state.on(path, cb),
    () => __BEAMER__.state.get<T>(path),
  );
}
