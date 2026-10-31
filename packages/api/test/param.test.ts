import { beforeEach, expect, test } from "bun:test";
import { param, senderValue, stateField } from "../src/index";
import type { Beamer, BeamerParamInfo } from "../src/types";

// What reached the host, in order.
let calls: string[];
// Listeners the runtime holds, by parameter id.
let listeners: Record<string, Array<(value: number) => void>>;

function info(stringId: string, extra: Partial<BeamerParamInfo> = {}): BeamerParamInfo {
  return {
    id: stringId.length,
    stringId,
    name: stringId,
    value: 0.5,
    plainValue: 0.5,
    displayText: "50",
    format: "percent",
    defaultValue: 0.25,
    min: 0,
    max: 1,
    units: "%",
    steps: 0,
    valueTexts: [],
    ...extra,
  };
}

// A stand-in for the runtime with two parameters, a State field and a
// value. Before `ready` it knows no parameters.
function install(options: { ready?: boolean; invoke?: () => Promise<unknown> } = {}) {
  calls = [];
  listeners = {};
  const infos: Record<string, BeamerParamInfo> =
    options.ready === false
      ? {}
      : {
          mix: info("mix"),
          sync_mode: info("sync_mode", { steps: 4, units: "", valueTexts: ["Free", "1/4", "1/8", "1/16", "1/32"] }),
        };
  const values: Record<string, number> = { mix: 0.5, sync_mode: 0.5 };
  const beamer: Beamer = {
    ready: Promise.resolve(),
    pluginFormat: "auv3",
    params: {
      get: (id) => (infos[id] ? values[id] : 0),
      getPlain: (id) => (infos[id] ? values[id] : 0),
      getDisplayText: (id) => (infos[id] ? String(Math.round(values[id] * 100)) : ""),
      set: (id, value) => {
        values[id] = value;
        calls.push(`set ${id} ${value}`);
      },
      beginEdit: (id) => calls.push(`begin ${id}`),
      endEdit: (id) => calls.push(`end ${id}`),
      on: (id, callback) => {
        (listeners[id] ??= []).push(callback);
        return () => {
          listeners[id] = listeners[id].filter((f) => f !== callback);
        };
      },
      all: () => Object.values(infos),
      info: (id) => infos[id],
    },
    state: {
      get: <T>() => "linear" as T,
      set: (path, value) => calls.push(`state ${path} ${value}`),
      on: () => () => {},
      ids: () => ["pan_law"],
    },
    values: {
      get: <T>() => ({ peak: [0.5, 0.25] }) as T,
      on: () => () => {},
    },
    invoke:
      options.invoke ??
      (async (method, ...args) => {
        calls.push(`invoke ${method} ${args.join(" ")}`);
        return 0.75;
      }),
    on: () => () => {},
    emit: () => {},
  };
  globalThis.__BEAMER__ = beamer;
}

beforeEach(() => install());

test("an edit is one gesture with a clamped value", () => {
  param("mix").edit(1.5);
  expect(calls).toEqual(["begin mix", "set mix 1", "end mix"]);
});

test("a gesture that begins ends the open one and tells its owner", () => {
  let interrupted = false;
  const mix = param("mix");
  mix.begin(() => {
    interrupted = true;
  });
  mix.set(0.6);
  mix.edit(0.2);
  expect(interrupted).toBe(true);
  expect(calls).toEqual(["begin mix", "set mix 0.6", "end mix", "begin mix", "set mix 0.2", "end mix"]);
});

test("ending without an open gesture sends nothing", () => {
  param("mix").end();
  expect(calls).toEqual([]);
});

test("controls of one parameter share its handle, a new runtime gets new ones", () => {
  const first = param("mix");
  expect(param("mix")).toBe(first);
  install();
  expect(param("mix")).not.toBe(first);
});

test("get and subscribe work detached, as React and Svelte call them", () => {
  const { get, subscribe } = param("mix");
  expect(get()).toBe(0.5);
  const seen: number[] = [];
  const unsubscribe = subscribe((value) => seen.push(value));
  listeners.mix.forEach((callback) => callback(0.8));
  unsubscribe();
  expect(listeners.mix).toEqual([]);
  // Svelte's contract: the value now, then each change.
  expect(seen).toEqual([0.5, 0.8]);
});

test("reset sets the default value", () => {
  param("mix").reset();
  expect(calls).toEqual(["begin mix", "set mix 0.25", "end mix"]);
});

test("the text has the unit, without a space before a percent sign", () => {
  expect(param("mix").text()).toBe("50%");
});

test("a discrete parameter knows its choices", () => {
  const sync = param("sync_mode");
  expect(sync.choices()).toEqual(["Free", "1/4", "1/8", "1/16", "1/32"]);
  expect(sync.choice()).toBe(2);
  sync.choose(4);
  expect(calls).toEqual(["begin sync_mode", "set sync_mode 1", "end sync_mode"]);
});

test("a continuous parameter has no choices to pick", () => {
  expect(() => param("mix").choice()).toThrow("continuous");
  expect(() => param("mix").choose(1)).toThrow("continuous");
});

test("a typed value goes through the parameter's parser", async () => {
  expect(await param("mix").typeValue("75")).toBe(true);
  expect(calls).toEqual(["invoke _beamer/paramTextToNormalized 3 75", "begin mix", "set mix 0.75", "end mix"]);
});

test("text that is no value changes nothing", async () => {
  install({ invoke: async () => null });
  expect(await param("mix").typeValue("loud")).toBe(false);
  expect(await param("mix").typeValue("  ")).toBe(false);
  expect(calls).toEqual([]);
});

test("a typed value the plugin cannot read changes nothing", async () => {
  install({ invoke: () => Promise.reject("unknown method") });
  expect(await param("mix").typeValue("75")).toBe(false);
  expect(calls).toEqual([]);
});

test("before ready a parameter reads as empty and ignores changes", async () => {
  install({ ready: false });
  const mix = param("mix");
  expect(mix.info).toBeUndefined();
  expect([mix.get(), mix.text(), mix.choices(), mix.choice()]).toEqual([0, "", [], 0]);
  mix.reset();
  mix.edit(1);
  mix.choose(1);
  mix.begin();
  mix.set(1);
  mix.end();
  expect(await mix.typeValue("75")).toBe(false);
  expect(calls).toEqual([]);
});

test("state fields and sender values read the runtime, one object per name", () => {
  const panLaw = stateField<string>("pan_law");
  expect(stateField<string>("pan_law")).toBe(panLaw);
  expect(panLaw.get()).toBe("linear");
  panLaw.set("constant_power");
  expect(calls).toEqual(["state pan_law constant_power"]);
  const seen: unknown[] = [];
  panLaw.subscribe((value) => seen.push(value));
  expect(seen).toEqual(["linear"]);
  expect(senderValue<{ peak: number[] }>("levels").get()?.peak).toEqual([0.5, 0.25]);
});

test("without a runtime the error explains why", () => {
  globalThis.__BEAMER__ = undefined;
  expect(() => param("mix")).toThrow("outside a plugin editor");
});
