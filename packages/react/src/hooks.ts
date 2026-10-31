import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { RefObject } from "react";
import { param, ready, runtime, senderValue, stateField } from "@helpermedia/beamer-api";
import type { BeamerParamInfo, Param } from "@helpermedia/beamer-api";
import { attachGestures } from "@helpermedia/beamer-gestures";
import type { DragMode, GestureOptions } from "@helpermedia/beamer-gestures";

/** A parameter as a component shows it. */
export interface ParamView {
  /** The handle, for changes: `param.edit()`, `param.reset()`, `param.typeValue()`. */
  param: Param;
  /** The normalized value (0 to 1). */
  value: number;
  /** The value as the parameter formats it, without the unit. */
  displayText: string;
  /** The value with its unit, as a host shows it. */
  text: string;
  /** What the runtime knows of it, undefined before `ready`. */
  info: BeamerParamInfo | undefined;
}

/**
 * The parameter with the string id `id`. The component renders again when
 * the value or its text changes, also when the host automates it.
 */
export function useParam(id: string): ParamView {
  const handle = param(id);
  const value = useSyncExternalStore(handle.subscribe, handle.get);
  // The text can change while the normalized value stays, when the plugin
  // answers an edit, so it has a snapshot of its own.
  const displayText = useSyncExternalStore(handle.subscribe, () => handle.displayText());
  const info = useSyncExternalStore(handle.subscribe, () => handle.info);
  return { param: handle, value, displayText, text: handle.text(), info };
}

/**
 * The `State` field at `path` and the function that sets it, like
 * `useState`. The value is undefined before `ready`.
 */
export function useStateField<T>(path: string): [T | undefined, (value: T) => void] {
  const field = stateField<T>(path);
  const value = useSyncExternalStore(field.subscribe, field.get);
  return [value, field.set];
}

/**
 * The newest value of the value sender `name`, which the plugin sends from
 * the audio thread. Undefined before the first one.
 */
export function useSenderValue<T>(name: string): T | undefined {
  const value = senderValue<T>(name);
  return useSyncExternalStore(value.subscribe, value.get);
}

/**
 * Call `handler` with each event `name`: what the plugin emits and the
 * arrays of its stream senders. Events that come before the component
 * mounts are not kept.
 */
export function useEvent(name: string, handler: (data: unknown) => void): void {
  // The newest handler, so that a handler defined in the component's body
  // does not subscribe again on every render.
  const latest = useRef(handler);
  latest.current = handler;
  useEffect(() => runtime().on(name, (data) => latest.current(data)), [name]);
}

/** Whether the plugin's parameters and State fields have arrived. */
export function useReady(): boolean {
  const [isReady, setReady] = useState(false);
  useEffect(() => {
    let mounted = true;
    ready().then(() => {
      if (mounted) setReady(true);
    });
    return () => {
      mounted = false;
    };
  }, []);
  return isReady;
}

/**
 * Attach the gestures of `beamer-gestures` to the element in `ref` while
 * the component is mounted. They attach again when the parameter or an
 * option changes. A `drag` function may be a new one on every render: the
 * newest is called for each press.
 */
export function useGestures(
  ref: RefObject<HTMLElement | SVGElement | null>,
  target: Param,
  options: GestureOptions = {},
): void {
  const latest = useRef(options);
  latest.current = options;
  const picksDrag = typeof options.drag === "function";
  // The other options are plain values, so their text tells a change from
  // a new object with the same options. JSON leaves a function out.
  const key = JSON.stringify(options);
  const attach = useCallback(
    (element: HTMLElement | SVGElement) => {
      const settings = JSON.parse(key) as GestureOptions;
      if (picksDrag) {
        // The newest options may have switched to a fixed mode before the
        // gestures attach again.
        settings.drag = (event: PointerEvent): DragMode => {
          const drag = latest.current.drag;
          return typeof drag === "function" ? drag(event) : (drag ?? "vertical");
        };
      }
      return attachGestures(element, target, settings);
    },
    [target, key, picksDrag],
  );
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    return attach(element);
  }, [ref, attach]);
}
