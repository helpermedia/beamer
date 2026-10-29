import { useCallback, useSyncExternalStore } from "react";

/**
 * The last value a value sender of the plugin sent from the audio thread,
 * by its name. Undefined before the first. The type is the caller's claim,
 * not a check.
 */
export function useBeamerValue<T>(name: string): T | undefined {
  // The same function while the name stays: React subscribes again when
  // it gets another one.
  const subscribe = useCallback((cb: () => void) => __BEAMER__.values.on(name, cb), [name]);
  return useSyncExternalStore(subscribe, () => __BEAMER__.values.get<T>(name));
}
