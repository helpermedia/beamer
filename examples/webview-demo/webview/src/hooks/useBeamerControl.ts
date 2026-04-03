import { useSyncExternalStore, useCallback, useMemo } from "react";
import { useBeamerParam } from "./useBeamerParam";

interface BeamerControl {
  value: number;
  displayValue: number;
  displayText: string;
  info: BeamerParamInfo | undefined;
  beginEdit: () => void;
  set: (normalized: number) => void;
  endEdit: () => void;
  resetToDefault: () => void;
}

export function useBeamerControl(paramId: string): BeamerControl {
  const value = useBeamerParam(paramId);
  const raw = __BEAMER__.params.info(paramId);
  const info = useMemo(
    () => raw,
    [raw?.id, raw?.stringId, raw?.name, raw?.min, raw?.max, raw?.units, raw?.steps, raw?.defaultValue],
  );

  const beginEdit = useCallback(() => {
    __BEAMER__.params.beginEdit(paramId);
  }, [paramId]);

  const set = useCallback(
    (normalized: number) => {
      __BEAMER__.params.set(paramId, Math.max(0, Math.min(1, normalized)));
    },
    [paramId],
  );

  const endEdit = useCallback(() => {
    __BEAMER__.params.endEdit(paramId);
  }, [paramId]);

  const resetToDefault = useCallback(() => {
    const i = __BEAMER__.params.info(paramId);
    if (i) {
      __BEAMER__.params.beginEdit(paramId);
      __BEAMER__.params.set(paramId, i.defaultValue);
      __BEAMER__.params.endEdit(paramId);
    }
  }, [paramId]);

  // Use authoritative values from the Rust parameter store rather than
  // recomputing from normalized, avoiding f32 round-trip artifacts.
  const displayValue = useMemo(() => {
    if (!info) return value;
    return __BEAMER__.params.getPlain(paramId);
  }, [value, info, paramId]);

  // Subscribe independently so displayText updates even when the
  // normalized value hasn't changed (e.g. echo after param:set).
  const displayText = useSyncExternalStore(
    (cb) => __BEAMER__.params.on(paramId, cb),
    () => __BEAMER__.params.getDisplayText(paramId),
  );

  return { value, displayValue, displayText, info, beginEdit, set, endEdit, resetToDefault };
}
