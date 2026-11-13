import type { Beamer, BeamerParameterInfo } from "@helpermedia/beamer-api";

// A stand-in for `__BEAMER__` for the tests of the hooks and the
// components: a continuous parameter "mix" (a percentage), a discrete
// parameter "mode" with three choices, the State field "pan_law", the value
// sender "levels" and the event "history".

/** What reached the host, in order. Emptied by `install`. */
export const calls: string[] = [];

/**
 * What the plugin pushes: a change of "mix", a State change, a value, an
 * event. And `init`, the init dump of a stand-in installed before `ready`.
 */
export const push = {
  init(): void {},
  parameter(_value: number, _text: string): void {},
  state(_value: string): void {},
  value(_levels: { peak: number }): void {},
  event(_data: unknown): void {},
};

const base: Omit<BeamerParameterInfo, "id" | "stringId" | "name"> = {
  value: 0, plainValue: 0, displayText: "", format: "float", defaultValue: 0, min: 0, max: 1, units: "", steps: 0, valueTexts: [],
};

/**
 * Install a fresh stand-in as `globalThis.__BEAMER__`. With `ready = false`
 * the parameters stay unknown, as before the init dump, until
 * `push.init()`.
 */
export function install(ready = true): void {
  calls.length = 0;
  let known = ready;
  const listeners: Record<string, Array<(value: unknown) => void>> = {};
  const on = (key: string, callback: (value: unknown) => void) => {
    (listeners[key] ??= []).push(callback);
    return () => {
      listeners[key] = listeners[key].filter((f) => f !== callback);
    };
  };
  const emit = (key: string, value: unknown) => listeners[key]?.forEach((callback) => callback(value));
  const shown: Record<string, { value: number; text: string }> = {
    mix: { value: 0.5, text: "50" },
    mode: { value: 0.5, text: "B" },
  };
  let panLaw = "linear";
  let levels: { peak: number } | undefined;
  const infos: Record<string, BeamerParameterInfo> = {
    mix: { ...base, id: 3, stringId: "mix", name: "Mix", value: 0.5, plainValue: 0.5, displayText: "50", format: "percent", defaultValue: 0.25, units: "%" },
    mode: { ...base, id: 4, stringId: "mode", name: "Mode", value: 0.5, plainValue: 1, displayText: "B", format: "enum", steps: 2, valueTexts: ["A", "B", "C"] },
  };
  const valueTexts = (id: string, value: number) => infos[id].valueTexts[Math.round(value * infos[id].steps)];
  const beamer: Beamer = {
    ready: Promise.resolve(),
    pluginFormat: "vst3",
    middleC: "C3",
    parameters: {
      get: (id) => (known ? shown[id].value : 0),
      getPlain: (id) => (known ? shown[id].value : 0),
      getDisplayText: (id) => (known ? shown[id].text : ""),
      set: (id, value) => {
        shown[id].value = value;
        if (id === "mode") shown[id].text = valueTexts(id, value);
        calls.push(id === "mix" ? `set ${value}` : `set ${id} ${value}`);
        emit(`parameter:${id}`, value);
      },
      beginEdit: () => calls.push("begin"),
      endEdit: () => calls.push("end"),
      on: (id, callback) => on(`parameter:${id}`, callback as (value: unknown) => void),
      all: () => (known ? Object.values(infos) : []),
      info: (id) => (known ? infos[id] : undefined),
    },
    state: {
      get: <T,>() => panLaw as T,
      set: (_path, value) => {
        panLaw = String(value);
        calls.push(`state ${value}`);
        emit("state:pan_law", panLaw);
      },
      on: <T,>(path: string, callback: (value: T) => void) => on(`state:${path}`, callback as (value: unknown) => void),
      ids: () => ["pan_law"],
    },
    values: {
      get: <T,>() => levels as T,
      on: <T,>(name: string, callback: (value: T) => void) => on(`value:${name}`, callback as (value: unknown) => void),
    },
    invoke: async () => null,
    on: (name, callback) => on(`event:${name}`, callback),
    emit: () => {},
    noteOn: () => {},
    noteOff: () => {},
    allNotesOff: () => {},
  };
  globalThis.__BEAMER__ = beamer;
  push.init = () => {
    known = true;
    for (const id of Object.keys(shown)) emit(`parameter:${id}`, shown[id].value);
  };
  push.parameter = (value, text) => {
    shown.mix = { value, text };
    emit("parameter:mix", value);
  };
  push.state = (value) => {
    panLaw = value;
    emit("state:pan_law", value);
  };
  push.value = (next) => {
    levels = next;
    emit("value:levels", next);
  };
  push.event = (data) => emit("event:history", data);
}
