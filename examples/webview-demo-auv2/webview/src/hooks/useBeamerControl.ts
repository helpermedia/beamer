import { useCallback, useMemo } from "react";
import { useBeamerParam } from "./useBeamerParam";

interface BeamerControl {
  value: number;
  displayValue: number;
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

  const displayValue = useMemo(() => {
    if (!info) return value;
    return info.min + value * (info.max - info.min);
  }, [value, info]);

  return { value, displayValue, info, beginEdit, set, endEdit, resetToDefault };
}
