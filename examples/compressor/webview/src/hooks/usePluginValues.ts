import { useReadable, useSenderValue } from "@helpermedia/beamer-react";
import { meters } from "../meters";
import type { Meters } from "../meters";

// What the audio thread sends through its value senders: the status
// (`Status` in src/lib.rs) and the meters, which ../meters.ts follows. The
// shape of what arrives is the caller's claim, so each field is checked as
// it is read.

export interface Status {
  // The sample rate the host prepared the plugin for.
  sampleRate: number;
  // Whether the detector keys on the sidechain.
  keyedOnSidechain: boolean;
}

function finite(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

// The status, null until the host prepared the plugin.
export function useStatus(): Status | null {
  const status = useSenderValue<Partial<Status>>("status");
  if (typeof status !== "object" || status === null) return null;
  return {
    sampleRate: finite(status.sampleRate, 0),
    keyedOnSidechain: status.keyedOnSidechain === true,
  };
}

// The meters as the editor shows them, at rest until the host prepared
// the plugin. They fall on their own once the host stops calling it.
export function useMeters(): Meters {
  return useReadable(meters);
}
