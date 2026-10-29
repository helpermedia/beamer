import { useCallback, useSyncExternalStore } from "react";

/**
 * The value of a State field of the plugin, by its path. Undefined until
 * the runtime is ready. The type is the caller's claim, not a check.
 */
export function useBeamerState<T>(path: string): T | undefined {
  // The same function while the path stays: React subscribes again when
  // it gets another one.
  const subscribe = useCallback((cb: () => void) => __BEAMER__.state.on(path, cb), [path]);
  return useSyncExternalStore(subscribe, () => __BEAMER__.state.get<T>(path));
}
