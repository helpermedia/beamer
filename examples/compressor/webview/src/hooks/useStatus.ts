import { useSenderValue } from "@helpermedia/beamer-react";
import type { Status } from "../model-types";

// The status the audio thread sends through the value sender "status"
// (`Status` in model-types.ts), null until the host prepared the plugin.
export function useStatus(): Status | null {
  return useSenderValue("status") ?? null;
}
