// A generated file as `cargo xtask model-types` writes it, with every
// section, for the type tests of the packages. The other packages'
// typed checks include it too.

export interface Meters {
  keyDb: number;
  reductionDb: number;
}

export type Tab = "pads" | "mixer";

export type Transformed =
  | { type: "noteOn"; pitch: number; outPitch: number | null }
  | { type: "reset" };

export interface Info {
  name: string;
  version: string;
}

export interface PadHit {
  drum: number;
  velocity: number;
}

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters: "gain" | "mode" | "note/enabled";
    state: { tab: Tab };
    values: { meters: Meters; pressed: number };
    streams: { transformed: Transformed };
    events: { sampleRate: number };
    pageEvents: { padHit: PadHit; reset: null };
    methods: {
      info: { arguments: []; result: Info };
      setGain: { arguments: [db: number]; result: boolean };
    };
  }
}
