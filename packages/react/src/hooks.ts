import { useEffect, useEffectEvent, useState, useSyncExternalStore } from "react";
import type { RefObject } from "react";
import { parameter, ready, runtime, senderValue, stateField } from "@helpermedia/beamer-api";
import type { BeamerParameterInfo, Parameter, Readable } from "@helpermedia/beamer-api";
import { attachDial, attachRadioGroup, attachSlider, attachSwitch, attachTextField } from "@helpermedia/beamer-controls";
import type { DialDrag, DialOptions, SliderOptions, TextFieldOptions } from "@helpermedia/beamer-controls";

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

// The hooks below attach a control to an element, each in an effect of its
// own. The caller holds the element in state and hands it over as it is,
// null while there is none:
//
//     const [dial, setDial] = useState<SVGSVGElement | null>(null);
//     useDial(dial, parameter);
//     return <svg ref={setDial} />;
//
// The element is a dependency of the effect, so the control follows it: it
// attaches when the element appears, detaches when it goes and moves when
// another one takes its place, also while the component stays mounted. A
// dial and a text field need the effect: a function that may be a new one
// on every render is an Effect Event, which only the effect of its own
// hook may call. The others have the same shape.
//
// Each hook is a thin layer over an inner one (`useDialControl` under
// `useDial`), which also takes the ref that holds the element. That is for
// the components alone: each renders its one element for as long as it is
// mounted, so its ref holds the element whenever the effect runs. The
// control then attaches in the commit that mounts the element, before the
// effects of a parent run. With the element in state it would attach one
// render later, after them.

/**
 * Where an inner control hook finds its element: the element itself, null
 * while there is none, or the ref that holds it.
 */
export type ElementSource<E extends Element> = E | null | RefObject<E | null>;

// A ref object is told by what it lacks. An element can have a `current`
// of its own: a form has one for a control of that name.
function isRef<E extends Element>(source: E | RefObject<E | null>): source is RefObject<E | null> {
  return !("nodeType" in source);
}

// The element of `source` now. Read in an effect, never while rendering.
function elementOf<E extends Element>(source: ElementSource<E>): E | null {
  if (source === null) return null;
  return isRef(source) ? source.current : source;
}

/** `useDial` for an element or, in the Dial component, the ref that holds it. */
export function useDialControl(
  source: ElementSource<HTMLElement | SVGElement>,
  target: Parameter,
  options: DialOptions,
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
    const element = elementOf(source);
    if (!element) return;
    return attachDial(element, target, { ...(JSON.parse(key) as DialOptions), drag: (event) => pickDrag(event) });
  }, [source, target, key]);
}

/**
 * Make `element` a dial for `target` while it is there: `attachDial` of
 * beamer-controls. Hold the element in state (`ref={setElement}`), null
 * while there is none. It attaches again when the element, the parameter
 * or an option other than `drag` changes. The newest `drag`, a fixed one
 * or a function, is read for each press, so a new one does not end a
 * running drag.
 */
export function useDial(
  element: HTMLElement | SVGElement | null,
  target: Parameter,
  options: DialOptions = {},
): void {
  useDialControl(element, target, options);
}

/** `useSlider` for an element or, in the Slider component, the ref that holds it. */
export function useSliderControl(
  source: ElementSource<HTMLElement | SVGElement>,
  target: Parameter,
  options: SliderOptions,
): void {
  // The options are plain values, so their text tells a change from a new
  // object with the same options.
  const key = JSON.stringify(options);
  useEffect(() => {
    const element = elementOf(source);
    if (!element) return;
    return attachSlider(element, target, JSON.parse(key) as SliderOptions);
  }, [source, target, key]);
}

/**
 * Make `element` a slider for `target` while it is there: `attachSlider`
 * of beamer-controls. Hold the element in state (`ref={setElement}`), null
 * while there is none. It attaches again when the element, the parameter
 * or an option changes.
 */
export function useSlider(
  element: HTMLElement | SVGElement | null,
  target: Parameter,
  options: SliderOptions = {},
): void {
  useSliderControl(element, target, options);
}

/** `useSwitch` for a button or, in the Switch component, the ref that holds it. */
export function useSwitchControl(source: ElementSource<HTMLButtonElement>, target: Parameter): void {
  useEffect(() => {
    const button = elementOf(source);
    if (!button) return;
    return attachSwitch(button, target);
  }, [source, target]);
}

/**
 * Make `button` a switch for `target` while it is there: `attachSwitch` of
 * beamer-controls. Hold the button in state (`ref={setButton}`), null
 * while there is none. It attaches again when the button or the parameter
 * changes.
 */
export function useSwitch(button: HTMLButtonElement | null, target: Parameter): void {
  useSwitchControl(button, target);
}

/** `useRadioGroup` for a container or, in the RadioGroup component, the ref that holds it. */
export function useRadioGroupControl(source: ElementSource<HTMLElement>, target: Parameter): void {
  useEffect(() => {
    const container = elementOf(source);
    if (!container) return;
    return attachRadioGroup(container, target);
  }, [source, target]);
}

/**
 * Make `container` a radio group for `target` while it is there, one
 * choice per child: `attachRadioGroup` of beamer-controls. Hold the
 * container in state (`ref={setContainer}`), null while there is none. It
 * attaches again when the container or the parameter changes. Children
 * rendered later, once the choices are known, count too.
 */
export function useRadioGroup(container: HTMLElement | null, target: Parameter): void {
  useRadioGroupControl(container, target);
}

/** `useTextField` for an input or, in the TextField component, the ref that holds it. */
export function useTextFieldControl(
  source: ElementSource<HTMLInputElement>,
  target: Parameter,
  options: TextFieldOptions,
): void {
  const open = options.open;
  // The text of the newest options.
  const shownText = useEffectEvent((shown: Parameter): string => options.text?.(shown) ?? shown.text());
  useEffect(() => {
    const input = elementOf(source);
    if (!input) return;
    return attachTextField(input, target, { open, text: (shown) => shownText(shown) });
  }, [source, target, open]);
}

/**
 * Make `input` the text field of `target` while it is there:
 * `attachTextField` of beamer-controls. Hold the input in state
 * (`ref={setInput}`), null while there is none. It attaches again when the
 * input, the parameter or `open` changes. The field owns its text, so give
 * the input no `value`. A `text` function may be a new one on every
 * render: the newest is called when the value changes.
 */
export function useTextField(
  input: HTMLInputElement | null,
  target: Parameter,
  options: TextFieldOptions = {},
): void {
  useTextFieldControl(input, target, options);
}
