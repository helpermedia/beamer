import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterAll, beforeAll, expect, test } from "bun:test";
import { param } from "@helpermedia/beamer-api";
import { KEYBOARD_FOCUS, attachChoices, attachDial, attachSlider, attachSwitch, attachValueEntry, openValueEntry, thumbPosition } from "../src/index";
import { installRuntime } from "./runtime";

beforeAll(() => GlobalRegistrator.register());
afterAll(() => GlobalRegistrator.unregister());

// A gain from -60 to 0 dB, a switch and a mode of three choices.
function setup(ready = true) {
  return installRuntime(
    [
      { info: { stringId: "gain", name: "Gain", min: -60, max: 0, units: "dB", value: 0.5, defaultValue: 1 }, text: (v) => (-60 + v * 60).toFixed(1) },
      { info: { stringId: "bypass", name: "Bypass", steps: 1, format: "boolean" }, text: (v) => (v >= 0.5 ? "On" : "Off") },
      { info: { stringId: "mode", name: "Mode", steps: 2, format: "enum", valueTexts: ["Peak", "RMS", "Hold"] }, text: (v) => ["Peak", "RMS", "Hold"][Math.round(v * 2)] },
    ],
    ready,
  );
}

function element<K extends keyof HTMLElementTagNameMap>(tag: K): HTMLElementTagNameMap[K] {
  const created = document.createElement(tag);
  document.body.append(created);
  return created;
}

function key(target: Element, name: string, init: KeyboardEventInit = {}) {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, cancelable: true, ...init }));
}

function pointer(target: Element, type: string, init: PointerEventInit = {}) {
  target.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, button: 0, ...init }));
}

test("a dial has its role, tab stop and ARIA values in plain units", () => {
  setup();
  const dial = element("div");
  const detach = attachDial(dial, param("gain"));
  expect(dial.getAttribute("role")).toBe("slider");
  expect(dial.getAttribute("tabindex")).toBe("0");
  expect(dial.getAttribute("aria-label")).toBe("Gain");
  expect(dial.getAttribute("aria-valuemin")).toBe("-60");
  expect(dial.getAttribute("aria-valuemax")).toBe("0");
  expect(dial.getAttribute("aria-valuenow")).toBe("-30");
  expect(dial.getAttribute("aria-valuetext")).toBe("-30.0 dB");
  detach();
  for (const name of ["role", "tabindex", "aria-label", "aria-valuenow", "aria-valuetext"]) {
    expect(dial.hasAttribute(name)).toBe(false);
  }
});

test("a dial follows the host and keeps the author's own role and label", () => {
  const runtime = setup();
  const dial = element("div");
  dial.setAttribute("aria-label", "Output");
  dial.setAttribute("tabindex", "-1");
  const detach = attachDial(dial, param("gain"));
  runtime.push("gain", 1);
  expect(dial.getAttribute("aria-valuenow")).toBe("0");
  expect(dial.getAttribute("aria-valuetext")).toBe("0.0 dB");
  detach();
  expect(dial.getAttribute("aria-label")).toBe("Output");
  expect(dial.getAttribute("tabindex")).toBe("-1");
});

test("a label the author sets or removes later wins over the fallback", () => {
  const runtime = setup();
  const dial = element("div");
  const detach = attachDial(dial, param("gain"));
  expect(dial.getAttribute("aria-label")).toBe("Gain");
  // A framework renders a label of its own after the control attached.
  dial.setAttribute("aria-label", "Output");
  runtime.push("gain", 0.6);
  expect(dial.getAttribute("aria-label")).toBe("Output");
  // It removes it again: the parameter's name comes back.
  dial.removeAttribute("aria-label");
  runtime.push("gain", 0.7);
  expect(dial.getAttribute("aria-label")).toBe("Gain");
  dial.setAttribute("aria-label", "Output");
  runtime.push("gain", 0.8);
  // Detaching leaves the author's label.
  detach();
  expect(dial.getAttribute("aria-label")).toBe("Output");
});

test("a dial gets its label and values once the parameters arrive", () => {
  const runtime = setup(false);
  const dial = element("div");
  dial.setAttribute("aria-labelledby", "gain-label");
  attachDial(dial, param("gain"));
  expect(dial.hasAttribute("aria-valuenow")).toBe(false);
  runtime.init();
  expect(dial.getAttribute("aria-valuenow")).toBe("-30");
  expect(dial.hasAttribute("aria-label")).toBe(false);
});

test("a dial takes the gestures and their options", () => {
  const runtime = setup();
  const dial = element("div");
  attachDial(dial, param("gain"), { keyStep: 0.25, reset: ["modifier-click"] });
  key(dial, "ArrowUp");
  pointer(dial, "pointerdown", { metaKey: true });
  expect(runtime.calls).toEqual(["begin gain", "set gain 0.75", "end gain", "begin gain", "set gain 1", "end gain"]);
});

// A slider of 100 pixels along its orientation and 20 across, at the top
// left.
function slider(orientation: "vertical" | "horizontal" = "vertical") {
  const created = element("div");
  created.setPointerCapture = () => {};
  const [width, height] = orientation === "vertical" ? [20, 100] : [100, 20];
  created.getBoundingClientRect = () => ({ left: 0, top: 0, width, height, right: width, bottom: height, x: 0, y: 0, toJSON: () => ({}) });
  return created;
}

test("a slider is upright by default", () => {
  setup();
  const upright = slider();
  const detach = attachSlider(upright, param("gain"));
  expect(upright.getAttribute("role")).toBe("slider");
  expect(upright.getAttribute("aria-orientation")).toBe("vertical");
  detach();
  expect(upright.hasAttribute("aria-orientation")).toBe(false);
});

test("a press on the track brings the thumb to the pointer, which it then follows", () => {
  const runtime = setup();
  const upright = slider();
  attachSlider(upright, param("gain"));
  pointer(upright, "pointerdown", { clientX: 10, clientY: 25 });
  pointer(upright, "pointermove", { clientX: 30, clientY: 60 });
  // Past the end the thumb stops there.
  pointer(upright, "pointermove", { clientX: 10, clientY: -40 });
  pointer(upright, "pointerup");
  expect(runtime.calls).toEqual(["begin gain", "set gain 0.75", "set gain 0.4", "set gain 1", "end gain"]);
});

test("a press on the thumb grabs it where it is pressed", () => {
  const runtime = setup();
  const upright = slider();
  // A thumb of 20 pixels: its center travels 80, from 90 up to 10. At 0.5
  // it is at 50.
  attachSlider(upright, param("gain"), { thumbLength: 0.2 });
  pointer(upright, "pointerdown", { clientX: 10, clientY: 55 });
  pointer(upright, "pointermove", { clientX: 10, clientY: 45 });
  pointer(upright, "pointerup");
  // No jump on the press; 10 pixels up are an eighth of the travel.
  expect(runtime.calls).toEqual(["begin gain", "set gain 0.625", "end gain"]);
});

test("thumbPosition puts the thumb's center within the travel", () => {
  expect(thumbPosition(0)).toBe(0);
  expect(thumbPosition(1)).toBe(1);
  expect(thumbPosition(0, 0.2)).toBeCloseTo(0.1);
  expect(thumbPosition(0.5, 0.2)).toBeCloseTo(0.5);
  expect(thumbPosition(1, 0.2)).toBeCloseTo(0.9);
  // Out of range values and thumbs are clamped, a thumb of NaN is none.
  expect(thumbPosition(2, 0.2)).toBeCloseTo(0.9);
  expect(thumbPosition(0.25, Number.NaN)).toBe(0.25);
  expect(thumbPosition(0.25, Number.POSITIVE_INFINITY)).toBe(0.5);
});

test("a press where thumbPosition draws the thumb grabs it", () => {
  const runtime = setup();
  const upright = slider();
  attachSlider(upright, param("gain"), { thumbLength: 0.3 });
  runtime.push("gain", 0.8);
  // The thumb's center in screen pixels, from the top of the 100 pixel
  // slider.
  const y = 100 * (1 - thumbPosition(0.8, 0.3));
  pointer(upright, "pointerdown", { clientX: 10, clientY: y });
  pointer(upright, "pointermove", { clientX: 10, clientY: y - 7 });
  pointer(upright, "pointerup");
  // 7 pixels of a 70 pixel travel.
  expect(runtime.calls).toEqual(["begin gain", "set gain 0.9", "end gain"]);
});

test("shift moves a slider finer, without a jump when it is released", () => {
  const runtime = setup();
  const upright = slider();
  attachSlider(upright, param("gain"));
  pointer(upright, "pointerdown", { clientX: 10, clientY: 50 });
  pointer(upright, "pointermove", { clientX: 10, clientY: 40, shiftKey: true });
  pointer(upright, "pointermove", { clientX: 10, clientY: 30 });
  pointer(upright, "pointerup");
  // The press is where the thumb is: nothing to set.
  expect(runtime.calls).toEqual(["begin gain", "set gain 0.51", "set gain 0.61", "end gain"]);
});

test("a lying slider follows the pointer to the right", () => {
  const runtime = setup();
  const lying = slider("horizontal");
  attachSlider(lying, param("gain"), { orientation: "horizontal" });
  expect(lying.getAttribute("aria-orientation")).toBe("horizontal");
  pointer(lying, "pointerdown", { clientX: 25, clientY: 10 });
  pointer(lying, "pointermove", { clientX: 80, clientY: 0 });
  pointer(lying, "pointerup");
  expect(runtime.calls).toEqual(["begin gain", "set gain 0.25", "set gain 0.8", "end gain"]);
});

test("a slider without a size to measure moves by distance", () => {
  const runtime = setup();
  const unmeasured = element("div");
  unmeasured.setPointerCapture = () => {};
  attachSlider(unmeasured, param("gain"), { dragStep: 1 / 100 });
  pointer(unmeasured, "pointerdown", { clientX: 0, clientY: 100 });
  pointer(unmeasured, "pointermove", { clientX: 0, clientY: 90 });
  pointer(unmeasured, "pointerup");
  expect(runtime.calls).toEqual(["begin gain", "set gain 0.6", "end gain"]);
});

test("focus from Tab shows as keyboard focus, focus from a click does not", () => {
  setup();
  const dial = element("div");
  dial.setPointerCapture = () => {};
  const detach = attachDial(dial, param("gain"));
  // Tab: a key, then the focus moves to the element.
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
  dial.focus();
  expect(dial.hasAttribute(KEYBOARD_FOCUS)).toBe(true);
  dial.blur();
  expect(dial.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
  // A press focuses it without the attribute.
  pointer(dial, "pointerdown", { clientY: 10 });
  expect(document.activeElement).toBe(dial);
  expect(dial.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
  pointer(dial, "pointerup");
  // Keys after the click show it.
  key(dial, "ArrowUp");
  expect(dial.hasAttribute(KEYBOARD_FOCUS)).toBe(true);
  detach();
  expect(dial.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
});

test("modifier keys and shortcuts do not show keyboard focus", () => {
  setup();
  const dial = element("div");
  dial.setPointerCapture = () => {};
  attachDial(dial, param("gain"));
  pointer(dial, "pointerdown", { clientY: 10 });
  pointer(dial, "pointerup");
  for (const init of [{ key: "Shift", shiftKey: true }, { key: "z", metaKey: true }]) {
    dial.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  }
  expect(dial.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
  // Nor does focus that comes after them.
  dial.blur();
  dial.focus();
  expect(dial.hasAttribute(KEYBOARD_FOCUS)).toBe(false);
});

test("a switch flips with each click, one undo step each", () => {
  const runtime = setup();
  const button = element("button");
  button.textContent = "Bypass";
  const detach = attachSwitch(button, param("bypass"));
  expect(button.getAttribute("role")).toBe("switch");
  expect(button.getAttribute("aria-checked")).toBe("false");
  expect(button.hasAttribute("aria-label")).toBe(false);
  button.click();
  expect(button.getAttribute("aria-checked")).toBe("true");
  button.click();
  expect(runtime.calls).toEqual(["begin bypass", "set bypass 1", "end bypass", "begin bypass", "set bypass 0", "end bypass"]);
  runtime.push("bypass", 1);
  expect(button.getAttribute("aria-checked")).toBe("true");
  detach();
  expect(button.hasAttribute("role")).toBe(false);
  expect(button.hasAttribute("aria-checked")).toBe(false);
  button.click();
  expect(runtime.calls).toHaveLength(6);
});

function choiceButtons(group: HTMLElement, names: string[]) {
  for (const name of names) {
    const button = document.createElement("button");
    button.textContent = name;
    group.append(button);
  }
  return [...group.children] as HTMLButtonElement[];
}

test("choices are a radio group with one tab stop on the checked choice", async () => {
  setup();
  const group = element("div");
  const buttons = choiceButtons(group, ["Peak", "RMS", "Hold"]);
  attachChoices(group, param("mode"));
  expect(group.getAttribute("role")).toBe("radiogroup");
  expect(group.getAttribute("aria-label")).toBe("Mode");
  expect(buttons.map((button) => button.getAttribute("role"))).toEqual(["radio", "radio", "radio"]);
  expect(buttons.map((button) => button.getAttribute("aria-checked"))).toEqual(["true", "false", "false"]);
  expect(buttons.map((button) => button.getAttribute("tabindex"))).toEqual(["0", "-1", "-1"]);
});

test("a click sets a choice, a click on the checked one sets nothing", () => {
  const runtime = setup();
  const group = element("div");
  const buttons = choiceButtons(group, ["Peak", "RMS", "Hold"]);
  attachChoices(group, param("mode"));
  buttons[2].click();
  buttons[2].click();
  expect(runtime.calls).toEqual(["begin mode", "set mode 1", "end mode"]);
  expect(buttons.map((button) => button.getAttribute("aria-checked"))).toEqual(["false", "false", "true"]);
  expect(buttons.map((button) => button.getAttribute("tabindex"))).toEqual(["-1", "-1", "0"]);
});

test("arrow keys check the next choice, move the focus and wrap", () => {
  const runtime = setup();
  const group = element("div");
  const buttons = choiceButtons(group, ["Peak", "RMS", "Hold"]);
  attachChoices(group, param("mode"));
  buttons[0].focus();
  key(buttons[0], "ArrowRight");
  expect(document.activeElement).toBe(buttons[1]);
  key(buttons[1], "ArrowDown");
  key(buttons[2], "ArrowRight");
  expect(document.activeElement).toBe(buttons[0]);
  key(buttons[0], "ArrowLeft");
  expect(document.activeElement).toBe(buttons[2]);
  expect(runtime.calls.filter((call) => call.startsWith("set"))).toEqual(["set mode 0.5", "set mode 1", "set mode 0", "set mode 1"]);
  // Shortcuts belong to the page.
  key(buttons[2], "ArrowLeft", { metaKey: true });
  expect(document.activeElement).toBe(buttons[2]);
});

test("choices rendered later get their roles, removed ones lose them", async () => {
  const runtime = setup(false);
  const group = element("div");
  const detach = attachChoices(group, param("mode"));
  runtime.init();
  const buttons = choiceButtons(group, ["Peak", "RMS", "Hold"]);
  // The observer reports after the change.
  await Promise.resolve();
  expect(buttons.map((button) => button.getAttribute("role"))).toEqual(["radio", "radio", "radio"]);
  buttons[2].remove();
  await Promise.resolve();
  expect(buttons[2].hasAttribute("role")).toBe(false);
  detach();
  expect(buttons[0].hasAttribute("aria-checked")).toBe(false);
  expect(group.hasAttribute("role")).toBe(false);
});

test("a value field shows the value and opens on a click with the text selected", () => {
  setup();
  const input = element("input");
  attachValueEntry(input, param("gain"));
  expect(input.readOnly).toBe(true);
  expect(input.value).toBe("-30.0 dB");
  expect(input.getAttribute("aria-label")).toBe("Gain value");
  input.click();
  expect(input.readOnly).toBe(false);
  expect(input.hasAttribute("data-typing")).toBe(true);
  expect(input.value).toBe("-30.0");
  expect(document.activeElement).toBe(input);
  expect(input.selectionStart).toBe(0);
  expect(input.selectionEnd).toBe(5);
});

test("Enter sets the typed value and keeps the focus", async () => {
  const runtime = setup();
  runtime.parsed.set("-12", 0.8);
  const input = element("input");
  attachValueEntry(input, param("gain"));
  input.click();
  input.value = "-12";
  key(input, "Enter");
  expect(input.readOnly).toBe(true);
  expect(input.hasAttribute("data-typing")).toBe(false);
  expect(document.activeElement).toBe(input);
  await Promise.resolve();
  await Promise.resolve();
  expect(runtime.calls).toEqual(["parse -12", "begin gain", "set gain 0.8", "end gain"]);
  expect(input.value).toBe("-12.0 dB");
});

test("a click elsewhere sets the typed value, Escape and an unchanged text set nothing", () => {
  const runtime = setup();
  const input = element("input");
  attachValueEntry(input, param("gain"));
  input.click();
  input.value = "-6";
  input.blur();
  expect(runtime.calls).toEqual(["parse -6"]);
  input.click();
  input.value = "-3";
  key(input, "Escape");
  input.click();
  input.blur();
  expect(runtime.calls).toEqual(["parse -6"]);
  expect(input.value).toBe("-30.0 dB");
});

test("Enter or Space opens a closed field, keys of an input method are left alone", () => {
  setup();
  const input = element("input");
  attachValueEntry(input, param("gain"));
  input.focus();
  key(input, " ");
  expect(input.readOnly).toBe(false);
  input.value = "-1";
  key(input, "Enter", { isComposing: true });
  expect(input.readOnly).toBe(false);
  key(input, "Escape");
  key(input, "Enter");
  expect(input.readOnly).toBe(false);
});

test("a value field can open on a double click or from script, with a text of its own", () => {
  setup();
  const input = element("input");
  const detach = attachValueEntry(input, param("gain"), { open: "double-click", text: (gain) => `[${gain.displayText()}]` });
  expect(input.value).toBe("[-30.0]");
  input.click();
  expect(input.readOnly).toBe(true);
  input.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  expect(input.readOnly).toBe(false);
  key(input, "Escape");
  openValueEntry(input);
  expect(input.readOnly).toBe(false);
  // Detaching closes the field without setting what was typed.
  detach();
  expect(input.readOnly).toBe(false);
  expect(input.hasAttribute("data-typing")).toBe(false);
});
