import { useSyncExternalStore } from "react";

export function useBeamerParam(name: string): number {
  return useSyncExternalStore(
    (cb) => __BEAMER__.params.on(name, cb),
    () => __BEAMER__.params.get(name),
  );
}
