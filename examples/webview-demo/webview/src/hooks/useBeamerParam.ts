import { useCallback, useSyncExternalStore } from "react";

export function useBeamerParam(name: string): number {
  // The same function while the name stays: React subscribes again when
  // it gets another one.
  const subscribe = useCallback((cb: () => void) => __BEAMER__.params.on(name, cb), [name]);
  return useSyncExternalStore(subscribe, () => __BEAMER__.params.get(name));
}
