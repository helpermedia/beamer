import { useEffect, useEffectEvent, useState, useSyncExternalStore } from "react";
import type { RefObject } from "react";
import { parameter, ready, runtime, senderValue, stateField } from "@helpermedia/beamer-api";
import type { BeamerParameterInfo, Parameter, Readable } from "@helpermedia/beamer-api";
import { attachChoices, attachDial, attachSlider, attachSwitch, attachValueEntry } from "@helpermedia/beamer-controls";
import type { DialDrag, DialOptions, SliderOptions, ValueEntryOptions } from "@helpermedia/beamer-controls";

/** A parameter as a component shows it. */
export interface ParameterView {
  /** The handle, for changes: `parameter.edit()`, `parameter.reset()`, `parameter.typeValue()`. */
  parameter: Parameter;
  /** The normalized value (0 to 1). */
  value: number;
  /** The value as the parameter formats it, without the unit. */
  displayText: string;
  /** The value with its unit, as a host shows it. */
  text: string;
  /** What the runtime knows of it, undefined before `ready`. */
  info: BeamerParameterInfo | undefined;
}

/**
 * The parameter with the string id `id`. The component renders again when
 * the value or its text changes, also when the host automates it.
 */
export function useParameter(id: string): ParameterView {
  const handle = parameter(id);
  const value = useSyncExternalStore(handle.subscribe, handle.get);
  // The text can change while the normalized value stays, when the plugin
  // answers an edit, so it has a snapshot of its own.
  const displayText = useSyncExternalStore(handle.subscribe, () => handle.displayText());
  const info = useSyncExternalStore(handle.subscribe, () => handle.info);
  return { parameter: handle, value, displayText, text: handle.text(), info };
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
  const onEvent = useEffectEvent(handler);
  useEffect(() => runtime().on(name, (data) => onEvent(data)), [name]);
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

// The hooks below attach a control to the element in `ref` while the
// component is mounted, each in an effect of its own. A dial and a value
// field need that: a function that may be a new one on every render is an
// Effect Event, which only the effect of its own hook may call. The others
// have the same shape.

/**
 * Make the element in `ref` a dial for `target` while the component is
 * mounted: `attachDial` of beamer-controls. It attaches again when the
 * parameter or an option other than `drag` changes. The newest `drag`, a
 * fixed one or a function, is read for each press, so a new one does not
 * end a running drag.
 */
export function useDial(
  ref: RefObject<HTMLElement | SVGElement | null>,
  target: Parameter,
  options: DialOptions = {},
): void {
  const { drag, ...others } = options;
  // The other options are plain values, so their text tells a change from
  // a new object with the same options.
  const key = JSON.stringify(others);
  // The drag of the newest options, picked for each press.
  const pickDrag = useEffectEvent((event: PointerEvent): DialDrag => {
    return typeof drag === "function" ? drag(event) : (drag ?? "vertical");
  });
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    return attachDial(element, target, { ...(JSON.parse(key) as DialOptions), drag: (event) => pickDrag(event) });
  }, [ref, target, key]);
}

/**
 * Make the element in `ref` a slider for `target` while the component is
 * mounted: `attachSlider` of beamer-controls. It attaches again when the
 * parameter or an option changes.
 */
export function useSlider(
  ref: RefObject<HTMLElement | SVGElement | null>,
  target: Parameter,
  options: SliderOptions = {},
): void {
  // The options are plain values, so their text tells a change from a new
  // object with the same options.
  const key = JSON.stringify(options);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    return attachSlider(element, target, JSON.parse(key) as SliderOptions);
  }, [ref, target, key]);
}

/**
 * Make the button in `ref` a switch for `target` while the component is
 * mounted: `attachSwitch` of beamer-controls.
 */
export function useSwitch(ref: RefObject<HTMLButtonElement | null>, target: Parameter): void {
  useEffect(() => {
    const button = ref.current;
    if (!button) return;
    return attachSwitch(button, target);
  }, [ref, target]);
}

/**
 * Make the element in `ref` a radio group for `target` while the component
 * is mounted, one choice per child: `attachChoices` of beamer-controls.
 * Children rendered later, once the choices are known, count too.
 */
export function useChoices(ref: RefObject<HTMLElement | null>, target: Parameter): void {
  useEffect(() => {
    const container = ref.current;
    if (!container) return;
    return attachChoices(container, target);
  }, [ref, target]);
}

/**
 * Make the input in `ref` the value field of `target` while the component
 * is mounted: `attachValueEntry` of beamer-controls. The field owns its
 * text, so give the input no `value`. A `text` function may be a new one
 * on every render: the newest is called when the value changes.
 */
export function useValueEntry(
  ref: RefObject<HTMLInputElement | null>,
  target: Parameter,
  options: ValueEntryOptions = {},
): void {
  const open = options.open;
  // The text of the newest options.
  const shownText = useEffectEvent((shown: Parameter): string => options.text?.(shown) ?? shown.text());
  useEffect(() => {
    const input = ref.current;
    if (!input) return;
    return attachValueEntry(input, target, { open, text: (shown) => shownText(shown) });
  }, [ref, target, open]);
}
