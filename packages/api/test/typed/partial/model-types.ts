// A hand-written declaration with only some sections: the sections it
// holds narrow, the others stay loose. It holds events but no streams,
// which then count as empty.

export interface Info {
  name: string;
}

declare module "@helpermedia/beamer-api" {
  interface ModelTypes {
    parameters: "gain";
    state: {};
    events: { sampleRate: number };
    methods: { info: { arguments: []; result: Info } };
  }
}
