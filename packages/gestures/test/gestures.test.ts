import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterAll, beforeAll, expect, test } from "bun:test";
import type { BeamerParamInfo, Param } from "@helpermedia/beamer-api";
import { KEYBOARD_FOCUS, angleAround, arcPosition, attachGestures, boundedOffset, dragPixels, keyValue, wheelPixels } from "../src/gestures";

beforeAll(() => GlobalRegistrator.register());
afterAll(() => GlobalRegistrator.unregister());

// A parameter that records what reached the host and keeps one gesture
// open at a time, like the one of beamer-api. With `steps` it snaps every
// value to a step, as a host echoes a discrete parameter.
function fakeParam(value: number, steps = 0) {
  const calls: string[] = [];
  let open: (() => void) | null | undefined;
  const snap = (normalized: number) => (steps > 0 ? Math.round(normalized * steps) / steps : normalized);
  const param = {
    info: { steps, defaultValue: 0.25 } as BeamerParamInfo,
    get: () => value,
    begin(onInterrupt?: () => void) {
      if (open !== undefined) {
        const interrupted = open;
        param.end();
        interrupted?.();
      }
      open = onInterrupt ?? null;
      calls.push("begin");
    },
    set(normalized: number) {
      value = snap(Math.min(1, Math.max(0, normalized)));
      calls.push(`set ${value.toFixed(3)}`);
    },
    end() {
      if (open === undefined) return;
      open = undefined;
      calls.push("end");
    },
    edit(normalized: number) {
      param.begin();
      param.set(normalized);
      param.end();
    },
    reset() {
      param.edit(0.25);
    },
  };
  return { param: param as unknown as Param, calls };
}

function knob() {
  const element = document.createElement("div");
  element.tabIndex = 0;
  // happy-dom has no pointer to capture.
  element.setPointerCapture = () => {};
  document.body.append(element);
  return element;
}

// A knob of 100 by 100 pixels at the top left, its center at 50, 50.
function arcKnob() {
  const element = knob();
  element.getBoundingClientRect = () => ({ left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}) });
  return element;
}

// A point at `degrees` clockwise from the right of the center, 40 px out.
function at(degrees: number) {
  const radians = (degrees * Math.PI) / 180;
  return { clientX: 50 + 40 * Math.cos(radians), clientY: 50 + 40 * Math.sin(radians) };
}

function pointer(element: Element, type: string, init: PointerEventInit = {}) {
  element.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init }));
}

test("an angle runs clockwise on screen", () => {
  expect(angleAround(0, 0, 10, 0)).toBe(0);
  expect(angleAround(0, 0, 0, 10)).toBe(90);
});

test("a point on the arc gives its position, one in the gap the nearer end", () => {
  // From the lower left (135) clockwise over 270 degrees to the lower right (45).
  expect(arcPosition(135, 135, 270)).toBe(0);
  expect(arcPosition(-90, 135, 270)).toBe(0.5);
  expect(arcPosition(45, 135, 270)).toBe(1);
  expect(arcPosition(80, 135, 270)).toBe(1);
  expect(arcPosition(100, 135, 270)).toBe(0);
});

test("a drag raises the value up, to the right or both", () => {
  // Screen y grows downwards, so moving up is a negative dy.
  expect(dragPixels("vertical", 5, -10)).toBe(10);
  expect(dragPixels("horizontal", 5, -10)).toBe(5);
  expect(dragPixels("both", 5, -10)).toBe(15);
});

test("scrolling up counts positive, with natural scrolling too", () => {
  expect(wheelPixels({ deltaX: 0, deltaY: -4, deltaMode: 0, shiftKey: false }, 10)).toBe(4);
  expect(wheelPixels({ deltaX: 0, deltaY: 4, deltaMode: 0, shiftKey: false, webkitDirectionInvertedFromDevice: true }, 10)).toBe(4);
});

test("a wheel notch counts for at most the cap, in pixels, lines or pages", () => {
  // Chromium reports 100 pixels per notch.
  expect(wheelPixels({ deltaX: 0, deltaY: -100, deltaMode: 0, shiftKey: false }, 10)).toBe(10);
  expect(wheelPixels({ deltaX: 0, deltaY: 1, deltaMode: 1, shiftKey: false }, 10)).toBe(-8);
  expect(wheelPixels({ deltaX: 0, deltaY: -1, deltaMode: 2, shiftKey: false }, 10)).toBe(10);
});

test("a horizontal wheel counts only with shift", () => {
  expect(wheelPixels({ deltaX: -3, deltaY: 0, deltaMode: 0, shiftKey: true }, 10)).toBe(3);
  expect(Object.is(wheelPixels({ deltaX: -3, deltaY: 0, deltaMode: 0, shiftKey: false }, 10), 0)).toBe(true);
});

test("keys step, page and jump to the ends", () => {
  expect(keyValue("ArrowUp", 0.5, 0.01, 0.1)).toBeCloseTo(0.51);
  expect(keyValue("ArrowLeft", 0.5, 0.01, 0.1)).toBeCloseTo(0.49);
  expect(keyValue("PageDown", 0.5, 0.01, 0.1)).toBeCloseTo(0.4);
  expect(keyValue("End", 0.5, 0.01, 0.1)).toBe(1);
  expect(keyValue("a", 0.5, 0.01, 0.1)).toBeUndefined();
});

test("an offset stops at the ends", () => {
  expect(boundedOffset(0.9, 0.3)).toBeCloseTo(0.1);
  expect(boundedOffset(0.1, -0.3)).toBeCloseTo(-0.1);
});

test("shift makes a drag ten times finer", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param);
  pointer(element, "pointerdown", { clientY: 100 });
  pointer(element, "pointermove", { clientY: 80, shiftKey: true });
  pointer(element, "pointerup");
  expect(calls).toEqual(["begin", "set 0.510", "end"]);
});

test("a drag is one gesture from the value at the press", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param);
  pointer(element, "pointerdown", { clientY: 100 });
  pointer(element, "pointermove", { clientY: 80 });
  pointer(element, "pointerup", { clientY: 80 });
  expect(calls).toEqual(["begin", "set 0.600", "end"]);
});

test("a drag on a discrete parameter does not stick on the snapped value", () => {
  const element = knob();
  // Four steps: 0, 0.25, 0.5, 0.75, 1.
  const { param, calls } = fakeParam(0.5, 4);
  attachGestures(element, param);
  pointer(element, "pointerdown", { clientY: 100 });
  // Each move is less than a step; together they pass one.
  for (const y of [90, 80, 70, 60, 50]) pointer(element, "pointermove", { clientY: y });
  pointer(element, "pointerup", { clientY: 50 });
  expect(calls.at(-2)).toBe("set 0.750");
});

test("a click focuses the control, a right click does nothing", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param);
  pointer(element, "pointerdown", { button: 2 });
  expect(calls).toEqual([]);
  pointer(element, "pointerdown", { clientY: 10 });
  expect(document.activeElement).toBe(element);
  pointer(element, "pointerup");
});

test("a scroll is one gesture until the wheel rests", async () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { wheelRestMs: 20 });
  for (let i = 0; i < 3; i++) {
    element.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, cancelable: true }));
  }
  expect(calls).toEqual(["begin", "set 0.550", "set 0.600", "set 0.650"]);
  await new Promise((resolve) => setTimeout(resolve, 40));
  expect(calls.at(-1)).toBe("end");
});

test("a key during a scroll ends the scroll first", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { wheelRestMs: 1000 });
  element.dispatchEvent(new WheelEvent("wheel", { deltaY: -10, cancelable: true }));
  element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true }));
  expect(calls).toEqual(["begin", "set 0.550", "end", "begin", "set 0.560", "end"]);
});

test("a key moves a discrete parameter one choice", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5, 4);
  attachGestures(element, param);
  element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true }));
  expect(calls).toEqual(["begin", "set 0.750", "end"]);
});

test("a modifier click resets with Command or Control", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { reset: ["modifier-click"] });
  pointer(element, "pointerdown", { ctrlKey: true });
  expect(calls).toEqual(["begin", "set 0.250", "end"]);
});

test("by default a double click resets, a modifier click does not", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param);
  // A Command click is a drag that does not move.
  pointer(element, "pointerdown", { metaKey: true });
  pointer(element, "pointerup", {});
  expect(calls).toEqual(["begin", "end"]);
  calls.length = 0;
  element.dispatchEvent(new MouseEvent("dblclick"));
  expect(calls).toEqual(["begin", "set 0.250", "end"]);
});

test("both resets together", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { reset: ["double-click", "modifier-click"] });
  element.dispatchEvent(new MouseEvent("dblclick"));
  expect(calls).toEqual(["begin", "set 0.250", "end"]);
  param.set(0.5);
  calls.length = 0;
  pointer(element, "pointerdown", { metaKey: true });
  expect(calls).toEqual(["begin", "set 0.250", "end"]);
});

test("an option given as undefined keeps its default", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { reset: undefined, dragStep: undefined });
  element.dispatchEvent(new MouseEvent("dblclick"));
  expect(calls).toEqual(["begin", "set 0.250", "end"]);
});

test("an empty reset list turns reset off", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { reset: [] });
  element.dispatchEvent(new MouseEvent("dblclick"));
  pointer(element, "pointerdown", { ctrlKey: true });
  pointer(element, "pointerup", {});
  expect(calls).toEqual(["begin", "end"]);
});

test("an arc drag jumps to the pointer and follows it", () => {
  const element = arcKnob();
  const { param, calls } = fakeParam(0.2);
  attachGestures(element, param, { drag: "arc" });
  // Straight up is the middle of the arc.
  pointer(element, "pointerdown", at(-90));
  pointer(element, "pointermove", at(0));
  pointer(element, "pointerup", at(0));
  expect(calls).toEqual(["begin", "set 0.500", "set 0.833", "end"]);
});

test("an arc drag through the gap waits at the end, then follows the pointer again", () => {
  const element = arcKnob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { drag: "arc" });
  pointer(element, "pointerdown", at(0));
  // Into the gap near the end, across it to the start, then back.
  pointer(element, "pointermove", at(60));
  pointer(element, "pointermove", at(140));
  pointer(element, "pointermove", at(0));
  pointer(element, "pointerup", at(0));
  // No jump to the start. The handle is under the pointer again.
  expect(calls).toEqual(["begin", "set 0.833", "set 1.000", "set 0.833", "end"]);
});

test("a parked value sends nothing until the pointer is back, also after a full turn", () => {
  const element = arcKnob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { drag: "arc" });
  pointer(element, "pointerdown", at(30));
  // Past the end into the gap, on through it and once around over the top.
  for (const degrees of [60, 100, 140, 200, 260, 320, 0]) pointer(element, "pointermove", at(degrees));
  // Back on the end's side the handle is under the pointer again.
  pointer(element, "pointermove", at(30));
  pointer(element, "pointerup", at(30));
  expect(calls).toEqual(["begin", "set 0.944", "set 1.000", "set 0.833", "set 0.944", "end"]);
});

test("an arc of another sweep parks and follows the same way", () => {
  const element = arcKnob();
  const { param, calls } = fakeParam(0.5);
  // From the left (180) clockwise over the top to the right (0).
  attachGestures(element, param, { drag: "arc", arcStart: 180, arcSweep: 180 });
  pointer(element, "pointerdown", at(-90));
  pointer(element, "pointermove", at(-10));
  // Through the gap below to the start, then back.
  pointer(element, "pointermove", at(60));
  pointer(element, "pointermove", at(120));
  pointer(element, "pointermove", at(-10));
  pointer(element, "pointerup", at(-10));
  expect(calls).toEqual(["begin", "set 0.500", "set 0.944", "set 1.000", "set 0.944", "end"]);
});

test("a fast flick within the arc follows the pointer", () => {
  const element = arcKnob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param, { drag: "arc" });
  // From near the end over the top to near the start, 170 degrees at once.
  pointer(element, "pointerdown", at(10));
  pointer(element, "pointermove", at(-160));
  pointer(element, "pointerup", at(-160));
  expect(calls).toEqual(["begin", "set 0.870", "set 0.241", "end"]);
});

test("after a press at the center the first move jumps to the pointer", () => {
  const element = arcKnob();
  const { param, calls } = fakeParam(0.1);
  attachGestures(element, param, { drag: "arc" });
  pointer(element, "pointerdown", { clientX: 50, clientY: 50 });
  pointer(element, "pointermove", at(-90));
  pointer(element, "pointerup", at(-90));
  expect(calls).toEqual(["begin", "set 0.100", "set 0.500", "end"]);
});

test("an arc drag ignores the pointer at the center", () => {
  const element = arcKnob();
  const { param, calls } = fakeParam(0.3);
  attachGestures(element, param, { drag: "arc" });
  pointer(element, "pointerdown", { clientX: 51, clientY: 51 });
  pointer(element, "pointermove", { clientX: 52, clientY: 49 });
  pointer(element, "pointerup");
  expect(calls).toEqual(["begin", "set 0.300", "end"]);
});

test("the drag mode can be picked for each press", () => {
  const element = arcKnob();
  const { param, calls } = fakeParam(0.5);
  // On the top half the arc, below it vertical.
  attachGestures(element, param, { drag: (event) => (event.clientY < 50 ? "arc" : "vertical") });
  pointer(element, "pointerdown", { clientX: 50, clientY: 80 });
  pointer(element, "pointermove", { clientX: 50, clientY: 60 });
  pointer(element, "pointerup");
  pointer(element, "pointerdown", at(-90));
  pointer(element, "pointerup");
  expect(calls).toEqual(["begin", "set 0.600", "end", "begin", "set 0.500", "end"]);
});

test("detaching ends an open drag and gives the element its touch action back", () => {
  const element = knob();
  element.style.touchAction = "pan-y";
  const { param, calls } = fakeParam(0.5);
  const detach = attachGestures(element, param);
  expect(element.style.touchAction).toBe("none");
  pointer(element, "pointerdown", { clientY: 100 });
  detach();
  pointer(element, "pointermove", { clientY: 50 });
  expect(calls).toEqual(["begin", "end"]);
  expect(element.style.touchAction).toBe("pan-y");
});

test("the context menu of a Control click ends the drag it began", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  // Without the modifier click reset (the default), a Control click begins
  // a drag.
  attachGestures(element, param);
  pointer(element, "pointerdown", { clientY: 100, ctrlKey: true });
  const menu = new MouseEvent("contextmenu", { cancelable: true });
  element.dispatchEvent(menu);
  pointer(element, "pointermove", { clientY: 50 });
  expect(calls).toEqual(["begin", "end"]);
  expect(menu.defaultPrevented).toBe(false);
});

test("a Control click that resets shows no context menu", () => {
  const element = knob();
  const { param } = fakeParam(0.5);
  attachGestures(element, param, { reset: ["modifier-click"] });
  pointer(element, "pointerdown", { ctrlKey: true });
  const menu = new MouseEvent("contextmenu", { cancelable: true });
  element.dispatchEvent(menu);
  expect(menu.defaultPrevented).toBe(true);
});

test("keys with Command, Control or Alt are left to the page", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  attachGestures(element, param);
  for (const modifier of ["metaKey", "ctrlKey", "altKey"]) {
    element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true, [modifier]: true }));
  }
  expect(calls).toEqual([]);
});

test("input before the parameter is known is ignored", () => {
  const element = knob();
  const { param, calls } = fakeParam(0.5);
  (param as { info: unknown }).info = undefined;
  attachGestures(element, param);
  pointer(element, "pointerdown", { clientY: 100 });
  element.dispatchEvent(new WheelEvent("wheel", { deltaY: -10, cancelable: true }));
  expect(calls).toEqual([]);
});

test("focus from Tab shows as keyboard focus, focus from a click does not", () => {
  const element = knob();
  const { param } = fakeParam(0.5);
  const detach = attachGestures(element, param);
  // Tab: a key, then the focus moves to the element.
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
  element.focus();
  expect(element.hasAttribute(KEYBOARD_FOCUS)).toBe(true);
  element.blur();
  expect(element.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
  // A click focuses it without the attribute.
  pointer(element, "pointerdown", { clientY: 10 });
  expect(document.activeElement).toBe(element);
  expect(element.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
  pointer(element, "pointerup");
  // Keys after the click show it.
  element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true }));
  expect(element.hasAttribute(KEYBOARD_FOCUS)).toBe(true);
  detach();
  expect(element.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
});

test("modifier keys and shortcuts do not show keyboard focus", () => {
  const element = knob();
  const { param } = fakeParam(0.5);
  attachGestures(element, param);
  pointer(element, "pointerdown", { clientY: 10 });
  pointer(element, "pointerup");
  for (const init of [{ key: "Shift", shiftKey: true }, { key: "z", metaKey: true }]) {
    element.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  }
  expect(element.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
  // Nor does focus that comes after them.
  element.blur();
  element.focus();
  expect(element.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
});
