import { useImperativeHandle, useRef, useSyncExternalStore } from "react";
import type { ComponentPropsWithoutRef, ReactNode, Ref } from "react";
import type { Parameter } from "@helpermedia/beamer-api";
import { dialAngle, thumbPosition } from "@helpermedia/beamer-controls";
import type { DialOptions, SliderOptions, ValueEntryOptions } from "@helpermedia/beamer-controls";
import { useChoices, useDial, useSlider, useSwitch, useValueEntry } from "./hooks.js";

// Unstyled components: each renders one element of its own, with the role,
// the ARIA attributes and the input of a control of beamer-controls. It
// hands the values to draw with to its children. The look stays with the
// caller: its class names, its markup inside, its SVG.

/** Children as they are, or a function of what the control shows. */
type Children<State> = ReactNode | ((state: State) => ReactNode);

function render<State>(children: Children<State>, state: State): ReactNode {
  return typeof children === "function" ? children(state) : children;
}

// The value, the text with its unit and what the runtime knows of
// `parameter`. The component renders again when one of them changes: the
// text can change while the value stays (the plugin answers an edit), the
// info arrives with `ready`.
function useShown(parameter: Parameter) {
  const value = useSyncExternalStore(parameter.subscribe, parameter.get);
  const text = useSyncExternalStore(parameter.subscribe, () => parameter.text());
  const info = useSyncExternalStore(parameter.subscribe, () => parameter.info);
  return { value, text, info };
}

// The component's own element, also in the caller's `ref`.
function useElement<E extends Element>(ref: Ref<E> | undefined) {
  const element = useRef<E>(null);
  useImperativeHandle(ref, () => element.current as E, []);
  return element;
}

/** What a dial hands to its children. */
export interface DialState {
  /** The normalized value (0 to 1). */
  value: number;
  /**
   * Where the dial points, in degrees clockwise on screen from the right of
   * its center (`dialAngle` of beamer-controls, with the dial's arc).
   */
  angle: number;
  /** The value with its unit, as a host shows it. */
  text: string;
}

export type DialProps = DialOptions &
  Omit<ComponentPropsWithoutRef<"div">, "children"> & {
    parameter: Parameter;
    /** The drawing of the dial, or a function of the values to draw it with. */
    children?: Children<DialState>;
    ref?: Ref<HTMLDivElement>;
  };

/**
 * A dial for `parameter`: a `<div>` with the input of `attachDial` (drag,
 * wheel, keys, reset) and ARIA's slider role. Draw the dial inside it,
 * filling it: the drag measures the `<div>`. The dial's options are props;
 * other props go to the `<div>`. The newest `drag`, a fixed one or a
 * function, is read for each press, so it may be a new one on every
 * render.
 */
export function Dial({
  parameter,
  children,
  ref,
  drag,
  arcStart,
  arcSweep,
  arcDeadZone,
  dragStep,
  fineFactor,
  keyStep,
  pageStep,
  wheelCap,
  wheelRestMs,
  reset,
  ...rest
}: DialProps) {
  const element = useElement(ref);
  useDial(element, parameter, {
    drag,
    arcStart,
    arcSweep,
    arcDeadZone,
    dragStep,
    fineFactor,
    keyStep,
    pageStep,
    wheelCap,
    wheelRestMs,
    reset,
  });
  const { value, text } = useShown(parameter);
  return (
    <div {...rest} ref={element}>
      {render(children, { value, angle: dialAngle(value, arcStart, arcSweep), text })}
    </div>
  );
}

/** What a slider hands to its children. */
export interface SliderState {
  /** The normalized value (0 to 1). */
  value: number;
  /**
   * Where the thumb's center goes, as a part of the slider's length from its
   * low end (`thumbPosition` of beamer-controls, with the slider's
   * `thumbLength`).
   */
  position: number;
  /** The value with its unit, as a host shows it. */
  text: string;
}

export type SliderProps = SliderOptions &
  Omit<ComponentPropsWithoutRef<"div">, "children"> & {
    parameter: Parameter;
    /** The drawing of the slider, or a function of the values to draw it with. */
    children?: Children<SliderState>;
    ref?: Ref<HTMLDivElement>;
  };

/**
 * A slider for `parameter`: a `<div>` with the input of `attachSlider` (the
 * thumb follows the pointer along it, wheel, keys, reset) and ARIA's slider
 * role. The `<div>` is the track: draw the slider inside it, filling it,
 * with the thumb's center at `position` of its length. The slider's options
 * are props; other props go to the `<div>`.
 */
export function Slider({
  parameter,
  children,
  ref,
  orientation,
  thumbLength,
  dragStep,
  fineFactor,
  keyStep,
  pageStep,
  wheelCap,
  wheelRestMs,
  reset,
  ...rest
}: SliderProps) {
  const element = useElement(ref);
  useSlider(element, parameter, {
    orientation,
    thumbLength,
    dragStep,
    fineFactor,
    keyStep,
    pageStep,
    wheelCap,
    wheelRestMs,
    reset,
  });
  const { value, text } = useShown(parameter);
  return (
    <div {...rest} ref={element}>
      {render(children, { value, position: thumbPosition(value, thumbLength), text })}
    </div>
  );
}

/** What a switch hands to its children. */
export interface SwitchState {
  /** Whether the switch is on: from the middle of the range up. */
  checked: boolean;
}

export type SwitchProps = Omit<ComponentPropsWithoutRef<"button">, "children" | "type"> & {
  parameter: Parameter;
  /** The switch's name and look, or a function of whether it is on. */
  children?: Children<SwitchState>;
  ref?: Ref<HTMLButtonElement>;
};

/**
 * A switch for the on and off `parameter`: a `<button>` with `attachSwitch`
 * (`role="switch"`, `aria-checked` for the style of the on state, each
 * click one undo step). Its text is its name. Other props go to the
 * `<button>`.
 */
export function Switch({ parameter, children, ref, ...rest }: SwitchProps) {
  const element = useElement(ref);
  useSwitch(element, parameter);
  const { value } = useShown(parameter);
  return (
    <button {...rest} type="button" ref={element}>
      {render(children, { checked: value >= 0.5 })}
    </button>
  );
}

/** What a radio group hands to its children. */
export interface ChoicesState {
  /** The display text of each choice, from the lowest value up. */
  choices: string[];
  /** The index of the checked choice, -1 before `ready` or for a continuous parameter. */
  checked: number;
}

export type ChoicesProps = Omit<ComponentPropsWithoutRef<"div">, "children"> & {
  parameter: Parameter;
  /**
   * One button per choice, the first for the lowest value: names of the
   * parameter's choices or names of one's own.
   */
  children?: Children<ChoicesState>;
  ref?: Ref<HTMLDivElement>;
};

/**
 * A radio group for the discrete `parameter`: a `<div>` with
 * `attachChoices`. Its children are the choices in order, made radios
 * (`aria-checked` on the checked one, arrow keys, one tab stop). Other
 * props go to the `<div>`.
 */
export function Choices({ parameter, children, ref, ...rest }: ChoicesProps) {
  const element = useElement(ref);
  useChoices(element, parameter);
  const { info } = useShown(parameter);
  const discrete = (info?.steps ?? 0) > 0;
  return (
    <div {...rest} ref={element}>
      {render(children, {
        choices: parameter.choices(),
        checked: discrete ? parameter.choice() : -1,
      })}
    </div>
  );
}

export type ValueEntryProps = ValueEntryOptions &
  Omit<ComponentPropsWithoutRef<"input">, "children" | "value" | "defaultValue" | "type" | "readOnly"> & {
    parameter: Parameter;
    ref?: Ref<HTMLInputElement>;
  };

/**
 * The value field of `parameter`: an `<input>` with `attachValueEntry`.
 * Closed it shows the value (`text`, the value with its unit by default), a
 * click (or a double click, with `open: "double-click"`) opens it to type
 * one. The field owns its text and whether it is read-only. A `text`
 * function may be a new one on every render. Other props go to the
 * `<input>`; its `ref` can open it with `openValueEntry`.
 */
export function ValueEntry({ parameter, ref, text, open, ...rest }: ValueEntryProps) {
  const element = useElement(ref);
  useValueEntry(element, parameter, { text, open });
  return <input {...rest} type="text" ref={element} />;
}
