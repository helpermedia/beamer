import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { RefObject } from "react";
import { param, ready, runtime, senderValue, stateField } from "@helpermedia/beamer-api";
import type { BeamerParamInfo, Param, Readable } from "@helpermedia/beamer-api";
import { attachChoices, attachDial, attachSlider, attachSwitch, attachValueEntry } from "@helpermedia/beamer-controls";
import type { DialDrag, DialOptions, SliderOptions, ValueEntryOptions } from "@helpermedia/beamer-controls";

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
 * The newest value of `readable`, such as a meter of beamer-controls
 * (`fallingMeter`). The component renders again when it changes.
 */
export function useReadable<T>(readable: Readable<T>): T {
  return useSyncExternalStore(readable.subscribe, readable.get);
}

// Attach a control to the element in `ref` while the component is
// mounted, again whenever `attach` changes.
function useAttached<E extends Element>(ref: RefObject<E | null>, attach: (element: E) => () => void): void {
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    return attach(element);
  }, [ref, attach]);
}

/**
 * Make the element in `ref` a dial for `target` while the component is
 * mounted: `attachDial` of beamer-controls. It attaches again when the
 * parameter or an option changes. A `drag` function may be a new one on
 * every render: the newest is called for each press.
 */
export function useDial(
  ref: RefObject<HTMLElement | SVGElement | null>,
  target: Param,
  options: DialOptions = {},
): void {
  const latest = useRef(options);
  latest.current = options;
  const picksDrag = typeof options.drag === "function";
  // The other options are plain values, so their text tells a change from
  // a new object with the same options. JSON leaves a function out.
  const key = JSON.stringify(options);
  const attach = useCallback(
    (element: HTMLElement | SVGElement) => {
      const settings = JSON.parse(key) as DialOptions;
      if (picksDrag) {
        // The newest options may have switched to a fixed drag before the
        // dial attaches again.
        settings.drag = (event: PointerEvent): DialDrag => {
          const drag = latest.current.drag;
          return typeof drag === "function" ? drag(event) : (drag ?? "vertical");
        };
      }
      return attachDial(element, target, settings);
    },
    [target, key, picksDrag],
  );
  useAttached(ref, attach);
}

/**
 * Make the element in `ref` a slider for `target` while the component is
 * mounted: `attachSlider` of beamer-controls. It attaches again when the
 * parameter or an option changes.
 */
export function useSlider(
  ref: RefObject<HTMLElement | SVGElement | null>,
  target: Param,
  options: SliderOptions = {},
): void {
  // The options are plain values, so their text tells a change from a new
  // object with the same options.
  const key = JSON.stringify(options);
  const attach = useCallback(
    (element: HTMLElement | SVGElement) => attachSlider(element, target, JSON.parse(key) as SliderOptions),
    [target, key],
  );
  useAttached(ref, attach);
}

/**
 * Make the button in `ref` a switch for `target` while the component is
 * mounted: `attachSwitch` of beamer-controls.
 */
export function useSwitch(ref: RefObject<HTMLButtonElement | null>, target: Param): void {
  const attach = useCallback((button: HTMLButtonElement) => attachSwitch(button, target), [target]);
  useAttached(ref, attach);
}

/**
 * Make the element in `ref` a radio group for `target` while the component
 * is mounted, one choice per child: `attachChoices` of beamer-controls.
 * Children rendered later, once the choices are known, count too.
 */
export function useChoices(ref: RefObject<HTMLElement | null>, target: Param): void {
  const attach = useCallback((container: HTMLElement) => attachChoices(container, target), [target]);
  useAttached(ref, attach);
}

/**
 * Make the input in `ref` the value field of `target` while the component
 * is mounted: `attachValueEntry` of beamer-controls. The field owns its
 * text, so give the input no `value`. A `text` function may be a new one
 * on every render: the newest is called when the value changes.
 */
export function useValueEntry(
  ref: RefObject<HTMLInputElement | null>,
  target: Param,
  options: ValueEntryOptions = {},
): void {
  const latest = useRef(options);
  latest.current = options;
  const open = options.open;
  const attach = useCallback(
    (input: HTMLInputElement) =>
      attachValueEntry(input, target, {
        open,
        text: (param) => latest.current.text?.(param) ?? param.text(),
      }),
    [target, open],
  );
  useAttached(ref, attach);
}
