import { beforeEach, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// The runtime the plugin injects into its WebView (beamer-webview), run
// against a stand-in for the native message handler.
const RUNTIME = readFileSync(
  fileURLToPath(new URL("../../../crates/beamer-webview/src/beamer_runtime.js", import.meta.url)),
  "utf8",
);

interface Runtime {
  parameters: {
    get(id: string): number;
    set(id: string, value: number): void;
    on(id: string, cb: (value: number) => void): () => void;
  };
  state: {
    get(path: string): unknown;
    set(path: string, value: unknown): void;
    on(path: string, cb: (value: unknown) => void): () => void;
  };
  _onInit(parameters: unknown[], info: unknown): void;
  _onParameters(changed: Record<string, [number, number, string]>, seq?: number): void;
  _onState(changed: Record<string, unknown>, seq?: number): void;
}

// The messages the page posted, in order.
let posted: Array<Record<string, unknown>>;
let beamer: Runtime;

beforeEach(() => {
  posted = [];
  const window = {
    webkit: { messageHandlers: { beamer: { postMessage: (json: string) => posted.push(JSON.parse(json)) } } },
  } as { webkit: unknown; __BEAMER__?: Runtime };
  new Function("window", RUNTIME)(window);
  beamer = window.__BEAMER__!;
  beamer._onInit(
    [{ id: 3, stringId: "gain", value: 0.5, plainValue: 0, displayText: "0 dB" }],
    { pluginFormat: "auv2", middleC: "C3", state: { zoom: 100 } },
  );
});

// The answer the plugin gives to a message: the value in effect, with the
// number of the set it answers.
function answerParameter(message: Record<string, unknown>, value = message.value as number) {
  beamer._onParameters({ [String(message.id)]: [value, value, String(value)] }, message.seq as number);
}

function answerState(message: Record<string, unknown>, value = message.value) {
  beamer._onState({ [String(message.id)]: value }, message.seq as number);
}

test("every set carries a number that grows", () => {
  beamer.parameters.set("gain", 0.6);
  beamer.state.set("zoom", 150);
  beamer.parameters.set("gain", 0.7);
  expect(posted.map((message) => message.seq)).toEqual([1, 2, 3]);
  expect(posted[0]).toEqual({ type: "parameter:set", id: 3, value: 0.6, seq: 1 });
  expect(posted[1]).toEqual({ type: "state:set", id: "zoom", value: 150, seq: 2 });
});

test("a late answer does not put an older parameter value back", () => {
  const heard: number[] = [];
  beamer.parameters.on("gain", (value) => heard.push(value));
  beamer.parameters.set("gain", 0.6);
  beamer.parameters.set("gain", 0.7);

  // The answer to the first set arrives after the second was sent.
  answerParameter(posted[0]);
  expect(beamer.parameters.get("gain")).toBe(0.7);
  answerParameter(posted[1]);
  expect(beamer.parameters.get("gain")).toBe(0.7);
  // Heard at once with each set, then with the answer to the newest. The
  // late answer adds nothing.
  expect(heard).toEqual([0.6, 0.7, 0.7]);
});

test("a value that is not a finite number is refused", () => {
  const warn = console.warn;
  console.warn = () => {};
  try {
    beamer.parameters.set("gain", Number.NaN);
    beamer.parameters.set("gain", Number.POSITIVE_INFINITY);
  } finally {
    console.warn = warn;
  }
  expect(posted).toEqual([]);
  expect(beamer.parameters.get("gain")).toBe(0.5);
});

test("the answer to the newest set corrects the page", () => {
  beamer.parameters.set("gain", 0.63);
  // The plugin snaps the value to a step.
  answerParameter(posted[0], 0.6);
  expect(beamer.parameters.get("gain")).toBe(0.6);

  beamer.state.set("zoom", "big");
  answerState(posted[1], 100);
  expect(beamer.state.get("zoom")).toBe(100);
});

test("a late answer does not put an older State value back", () => {
  const heard: unknown[] = [];
  beamer.state.on("zoom", (value) => heard.push(value));
  beamer.state.set("zoom", 150);
  beamer.state.set("zoom", 200);

  answerState(posted[0]);
  expect(beamer.state.get("zoom")).toBe(200);
  answerState(posted[1]);
  expect(heard).toEqual([150, 200]);
});

test("changes the plugin sends of its own always count", () => {
  beamer.parameters.set("gain", 0.6);
  beamer.state.set("zoom", 150);
  // The host automates the parameter and loads a state while the sets are
  // on their way: frames carry no number.
  beamer._onParameters({ 3: [0.2, 0.2, "0.2"] });
  beamer._onState({ zoom: 50 });
  expect(beamer.parameters.get("gain")).toBe(0.2);
  expect(beamer.state.get("zoom")).toBe(50);
});

test("an answer to one key does not hold back another", () => {
  beamer.state.set("zoom", 150);
  beamer.parameters.set("gain", 0.6);
  // The State answer is older than the newest set overall, but the newest
  // of its own field.
  answerState(posted[0], 120);
  expect(beamer.state.get("zoom")).toBe(120);
});
