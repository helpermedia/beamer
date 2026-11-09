import type { Beamer, BeamerParamInfo } from "@helpermedia/beamer-api";

// A parameter of the stand-in runtime: its info and a text for each
// normalized value.
export interface FakeParam {
  info: Partial<BeamerParamInfo> & { stringId: string };
  text?: (normalized: number) => string;
}

export interface FakeRuntime {
  // What reached the host, in order.
  calls: string[];
  // A change from the host or the plugin.
  push(id: string, value: number): void;
  // A value of a value sender from the audio thread.
  send(name: string, value: unknown): void;
  // Before `ready`: the parameters are not known yet.
  init(): void;
  // What `_beamer/paramTextToNormalized` answers, by text.
  parsed: Map<string, number>;
}

/**
 * Install a stand-in for `__BEAMER__` with the parameters `params`. The
 * runtime notifies a parameter's subscribers on every change, as the real
 * one does. With `ready = false` the parameters stay unknown until
 * `init()`.
 */
export function installRuntime(params: FakeParam[], ready = true): FakeRuntime {
  const calls: string[] = [];
  const listeners: Record<string, Array<(value: unknown) => void>> = {};
  const on = (key: string, callback: (value: unknown) => void) => {
    (listeners[key] ??= []).push(callback);
    return () => {
      listeners[key] = listeners[key].filter((f) => f !== callback);
    };
  };
  const emit = (key: string, value: unknown) => listeners[key]?.forEach((callback) => callback(value));
  let known = ready;
  const values = new Map<string, unknown>();
  const byId = new Map(
    params.map(({ info, text }) => {
      const full: BeamerParamInfo = {
        id: 1, name: info.stringId, value: 0, plainValue: 0, displayText: "", format: "float",
        defaultValue: 0, min: 0, max: 1, units: "", steps: 0, valueTexts: [], ...info,
      };
      return [info.stringId, { info: full, value: full.value, text: text ?? ((v: number) => v.toFixed(2)) }];
    }),
  );
  const entry = (id: string) => {
    const found = byId.get(id);
    if (!found) throw new Error(`No parameter ${id}`);
    return found;
  };
  const parsed = new Map<string, number>();

  const beamer: Beamer = {
    ready: Promise.resolve(),
    pluginFormat: "vst3",
    middleC: "C3",
    params: {
      get: (id) => (known ? entry(id).value : 0),
      getPlain: (id) => {
        if (!known) return 0;
        const { info, value } = entry(id);
        return info.min + value * (info.max - info.min);
      },
      getDisplayText: (id) => (known ? entry(id).text(entry(id).value) : ""),
      set: (id, value) => {
        entry(id).value = value;
        calls.push(`set ${id} ${Number(value.toFixed(3))}`);
        emit(`param:${id}`, value);
      },
      beginEdit: (id) => calls.push(`begin ${id}`),
      endEdit: (id) => calls.push(`end ${id}`),
      on: (id, callback) => on(`param:${id}`, callback as (value: unknown) => void),
      all: () => (known ? [...byId.values()].map(({ info }) => info) : []),
      info: (id) => (known ? byId.get(id)?.info : undefined),
    },
    state: {
      get: () => undefined,
      set: () => {},
      on: () => () => {},
      ids: () => [],
    },
    values: {
      get: <T,>(name: string) => values.get(name) as T,
      on: <T,>(name: string, callback: (value: T) => void) => on(`value:${name}`, callback as (value: unknown) => void),
    },
    invoke: async (method, _id, text) => {
      if (method !== "_beamer/paramTextToNormalized") return null;
      calls.push(`parse ${text}`);
      return parsed.get(String(text)) ?? null;
    },
    on: (name, callback) => on(`event:${name}`, callback),
    emit: () => {},
  };
  globalThis.__BEAMER__ = beamer;

  return {
    calls,
    parsed,
    push(id, value) {
      entry(id).value = value;
      emit(`param:${id}`, value);
    },
    send(name, value) {
      values.set(name, value);
      emit(`value:${name}`, value);
    },
    init() {
      known = true;
      for (const [id, { value }] of byId) emit(`param:${id}`, value);
    },
  };
}
