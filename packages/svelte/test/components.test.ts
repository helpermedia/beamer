import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterAll, afterEach, beforeAll, expect, test } from "bun:test";
import { parameter } from "@helpermedia/beamer-api";
import type { Parameter } from "@helpermedia/beamer-api";
import { fallingLevels, openTextField } from "@helpermedia/beamer-controls";
import type { DialOptions, SliderOptions, TextFieldOptions } from "@helpermedia/beamer-controls";
import type { Component } from "svelte";
import type { DialState, RadioGroupState, SliderState } from "../src/index";
import { installRuntime } from "../../controls/test/runtime";
import type { FakeRuntime } from "../../controls/test/runtime";
import "./svelte";

beforeAll(() => GlobalRegistrator.register());
afterAll(() => GlobalRegistrator.unregister());

// Svelte reads the DOM globals when it loads, so it loads after happy-dom.
const { flushSync, mount, tick, unmount } = await import("svelte");
const { default: Harness, control } = await import("./fixtures/Harness.svelte");
const { Dial, Slider, TextField } = await import("../src/index");
const { default: DialDrawing } = await import("./fixtures/DialDrawing.svelte");
const { default: SliderDrawing } = await import("./fixtures/SliderDrawing.svelte");
const { default: SwitchName } = await import("./fixtures/SwitchName.svelte");
const { default: RadioGroupButtons } = await import("./fixtures/RadioGroupButtons.svelte");
const { default: Field } = await import("./fixtures/Field.svelte");
const { default: StoreText } = await import("./fixtures/StoreText.svelte");
const { default: ViewFields } = await import("./fixtures/ViewFields.svelte");
const { default: ViewList } = await import("./fixtures/ViewList.svelte");

// The parameters of the stand-in runtime: a percentage and three choices.
const PARAMETERS = [
  { info: { stringId: "mix", name: "Mix", units: "%", value: 0.5, defaultValue: 0.25 }, text: (v: number) => String(Math.round(v * 100)) },
  { info: { stringId: "gain", name: "Gain", units: "dB", value: 0.2 }, text: (v: number) => (v * 10).toFixed(1) },
  { info: { stringId: "mode", name: "Mode", steps: 2, value: 0.5, valueTexts: ["A", "B", "C"] }, text: (v: number) => ["A", "B", "C"][Math.round(v * 2)] },
];

let mounted: Array<Record<string, unknown>> = [];

// Render `component` with `props` that `control.set` can change. Each
// fixture has props of its own, so the harness takes any.
function render(component: Component<any>, props: Record<string, unknown>): HTMLElement {
  const target = document.createElement("div");
  document.body.append(target);
  mounted.push(mount(Harness, { target, props: { component, initial: props } }));
  flushSync();
  return target;
}

// Unmount what the test rendered. A view lets go of its parameter in a
// microtask (on Svelte 5.29 one that `flushSync` does not run), so a test
// of a view awaits `tick` after it.
function unmountAll() {
  for (const app of mounted) unmount(app);
  mounted = [];
  flushSync();
}

afterEach(unmountAll);

function install(ready = true): FakeRuntime {
  return installRuntime(PARAMETERS, ready);
}

test("Dial is a div with the dial's input that hands its values to the drawing", () => {
  const host = install();
  const seen: DialState[] = [];
  const doubleClicks: string[] = [];
  const target = render(DialDrawing, {
    parameter: parameter("mix"),
    record: (state: DialState) => seen.push(state),
    class: "dial",
    ondblclick: () => doubleClicks.push("double click"),
  });
  const element = target.firstElementChild as HTMLElement;
  expect(element.tagName).toBe("DIV");
  expect(element.className).toBe("dial");
  expect(element.getAttribute("role")).toBe("slider");
  expect(element.getAttribute("aria-valuetext")).toBe("50%");
  expect(element.querySelector("svg")).not.toBeNull();
  // The default arc: from 135 degrees, 270 of them.
  expect(seen.at(-1)).toEqual({ value: 0.5, angle: 270, text: "50%" });

  host.push("mix", 0.8);
  flushSync();
  expect(seen.at(-1)?.text).toBe("80%");
  expect(seen.at(-1)?.angle).toBeCloseTo(351);

  element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true }));
  expect(host.calls).toEqual(["begin mix", "set mix 0.81", "end mix"]);
  // A double click reaches the caller's handler and resets the dial to its
  // default, 0.25.
  element.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  expect(doubleClicks).toEqual(["double click"]);

  // An arc of its own turns the angle too.
  control.set({ arcStart: 90, arcSweep: 180 });
  flushSync();
  expect(seen.at(-1)?.angle).toBeCloseTo(90 + 0.25 * 180);
});

test("Dial attaches again for another parameter and lets the old one go", () => {
  const host = install();
  const seen: DialState[] = [];
  const target = render(DialDrawing, { parameter: parameter("mix"), record: (state: DialState) => seen.push(state) });
  const element = target.firstElementChild as HTMLElement;
  control.set({ parameter: parameter("gain") });
  flushSync();
  expect(element.getAttribute("aria-label")).toBe("Gain");
  expect(seen.at(-1)?.text).toBe("2.0 dB");
  const count = seen.length;
  host.push("mix", 0.9);
  flushSync();
  expect(seen.length).toBe(count);
  element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true }));
  expect(host.calls).toEqual(["begin gain", "set gain 0.21", "end gain"]);
});

test("a new drag function is called on the next move without attaching again", () => {
  const host = install();
  const picked: string[] = [];
  const pick = (name: string) => () => {
    picked.push(name);
    return "horizontal" as const;
  };
  const target = render(DialDrawing, { parameter: parameter("mix"), record: () => {}, drag: pick("first"), reset: ["double-click"] });
  const element = target.firstElementChild as HTMLElement;
  element.setPointerCapture = () => {};
  element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1, button: 0, clientX: 10 }));
  // A new function and a new list with the same content during the drag: a
  // control that attached again would end the gesture here.
  control.set({ drag: pick("second"), reset: ["double-click"] });
  flushSync();
  element.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerId: 1, clientX: 30 }));
  element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1 }));
  expect(picked).toEqual(["first"]);
  expect(host.calls).toEqual(["begin mix", "set mix 0.6", "end mix"]);
  // The next press picks with the new function.
  element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 2, button: 0, clientX: 30 }));
  element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 2 }));
  expect(picked).toEqual(["first", "second"]);
});

test("a changing style keeps the touch action the gestures need", () => {
  install();
  const target = render(DialDrawing, { parameter: parameter("mix"), record: () => {}, style: "--value: 0.5" });
  const element = target.firstElementChild as HTMLElement;
  control.set({ style: "--value: 0.8" });
  flushSync();
  expect(element.style.touchAction).toBe("none");
});

test("Slider hands the thumb's position and follows its orientation", () => {
  const host = install();
  const seen: SliderState[] = [];
  const target = render(SliderDrawing, { parameter: parameter("mix"), record: (state: SliderState) => seen.push(state), thumbLength: 0.2 });
  const element = target.firstElementChild as HTMLElement;
  expect(element.getAttribute("role")).toBe("slider");
  expect(element.getAttribute("aria-orientation")).toBe("vertical");
  // Half the thumb in from the low end, the rest of the travel by the value.
  expect(seen.at(-1)?.position).toBeCloseTo(0.1 + 0.5 * 0.8);
  host.push("mix", 1);
  flushSync();
  expect(seen.at(-1)?.position).toBeCloseTo(0.9);
  control.set({ orientation: "horizontal" });
  flushSync();
  expect(element.getAttribute("aria-orientation")).toBe("horizontal");
});

test("Switch is a button that flips the value and shows whether it is on", () => {
  const host = install();
  const target = render(SwitchName, { parameter: parameter("mix") });
  const button = target.firstElementChild as HTMLButtonElement;
  expect(button.tagName).toBe("BUTTON");
  expect(button.type).toBe("button");
  expect(button.className).toBe("switch");
  expect(button.getAttribute("role")).toBe("switch");
  expect(button.getAttribute("aria-checked")).toBe("true");
  expect(button.textContent?.trim()).toBe("Mix on");
  button.click();
  expect(host.calls).toEqual(["begin mix", "set mix 0", "end mix"]);
  // The stand-in tells the subscribers at once, as the plugin's answer would.
  flushSync();
  expect(button.getAttribute("aria-checked")).toBe("false");
  expect(button.textContent?.trim()).toBe("Mix off");
});

test("a bound ref gets the element and children can be plain markup", () => {
  install();
  const refs: Array<HTMLButtonElement | null> = [];
  const target = render(SwitchName, { parameter: parameter("mix"), plain: true, onRef: (button: HTMLButtonElement | null) => refs.push(button) });
  const button = target.firstElementChild as HTMLButtonElement;
  expect(refs.at(-1)).toBe(button);
  expect(button.textContent?.trim()).toBe("Mix");
});

test("RadioGroup makes its buttons radios and hands the choices", () => {
  const host = install();
  const target = render(RadioGroupButtons, { parameter: parameter("mode") });
  const group = target.firstElementChild as HTMLElement;
  const buttons = [...group.children] as HTMLButtonElement[];
  expect(group.getAttribute("role")).toBe("radiogroup");
  expect(buttons.map((button) => button.textContent)).toEqual(["A", "B", "C"]);
  expect(buttons.map((button) => button.getAttribute("aria-checked"))).toEqual(["false", "true", "false"]);
  expect(buttons[1].hasAttribute("data-checked")).toBe(true);
  buttons[2].click();
  expect(host.calls).toEqual(["begin mode", "set mode 1", "end mode"]);
  flushSync();
  expect(buttons[2].hasAttribute("data-checked")).toBe(true);
  expect(buttons[2].getAttribute("aria-checked")).toBe("true");
});

test("RadioGroup of a continuous parameter has no choices and none checked", () => {
  install();
  const seen: RadioGroupState[] = [];
  render(RadioGroupButtons, { parameter: parameter("mix"), record: (state: RadioGroupState) => seen.push(state) });
  expect(seen.at(-1)).toEqual({ choices: [], checked: -1 });
});

test("the components render again when the parameters arrive with ready", () => {
  const host = install(false);
  const dial: DialState[] = [];
  const radioGroup: RadioGroupState[] = [];
  render(DialDrawing, { parameter: parameter("mix"), record: (state: DialState) => dial.push(state) });
  render(RadioGroupButtons, { parameter: parameter("mode"), record: (state: RadioGroupState) => radioGroup.push(state) });
  expect(dial.at(-1)?.text).toBe("");
  expect(radioGroup.at(-1)).toEqual({ choices: [], checked: -1 });
  host.init();
  flushSync();
  expect(dial.at(-1)).toEqual({ value: 0.5, angle: 270, text: "50%" });
  expect(radioGroup.at(-1)).toEqual({ choices: ["A", "B", "C"], checked: 1 });
});

test("TextField shows the newest text and opens without closing on a new one", () => {
  const host = install();
  let input: HTMLInputElement | null = null;
  render(Field, { parameter: parameter("mix"), prefix: "a ", onRef: (field: HTMLInputElement | null) => (input = field) });
  const field = input as unknown as HTMLInputElement;
  expect(field.type).toBe("text");
  expect(field.className).toBe("field");
  expect(field.readOnly).toBe(true);
  expect(field.value).toBe("a 50%");
  openTextField(field);
  expect(field.readOnly).toBe(false);
  // A new text function leaves the open field open.
  control.set({ prefix: "b " });
  flushSync();
  expect(field.readOnly).toBe(false);
  field.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true }));
  host.push("mix", 0.8);
  flushSync();
  expect(field.value).toBe("b 80%");
});

test("TextField opens on a double click when asked to", () => {
  install();
  const target = render(Field, { parameter: parameter("mix"), open: "double-click" });
  const field = target.firstElementChild as HTMLInputElement;
  field.click();
  expect(field.readOnly).toBe(true);
  field.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  expect(field.readOnly).toBe(false);
});

test("the options stay off the element", () => {
  install();
  // Every option is set, so a new one has to be added here too.
  const dialOptions = {
    drag: "arc",
    arcStart: 90,
    arcSweep: 180,
    arcDeadZone: 4,
    dragStep: 0.01,
    fineFactor: 0.2,
    keyStep: 0.02,
    pageStep: 0.2,
    wheelCap: 8,
    wheelRestMs: 200,
    reset: ["modifier-click"],
  } satisfies Required<DialOptions>;
  const sliderOptions = {
    orientation: "horizontal",
    thumbLength: 0.1,
    dragStep: 0.01,
    fineFactor: 0.2,
    keyStep: 0.02,
    pageStep: 0.2,
    wheelCap: 8,
    wheelRestMs: 200,
    reset: [],
  } satisfies Required<SliderOptions>;
  const textFieldOptions = {
    text: (mix: Parameter) => mix.text(),
    open: "double-click",
  } satisfies Required<TextFieldOptions>;
  const elements = [
    render(Dial, { parameter: parameter("mix"), ...dialOptions }),
    render(Slider, { parameter: parameter("mix"), ...sliderOptions }),
    render(TextField, { parameter: parameter("mix"), ...textFieldOptions }),
  ].map((target) => target.firstElementChild as HTMLElement);
  // What the controls set, nothing of the options.
  const allowed = ["role", "tabindex", "aria-label", "aria-valuemin", "aria-valuemax", "aria-valuenow", "aria-valuetext", "aria-orientation", "style", "type", "readonly"];
  for (const element of elements) {
    const names = [...element.attributes].map((attribute) => attribute.name);
    expect(names.filter((name) => !allowed.includes(name))).toEqual([]);
  }
});

test("a parameter and falling levels read as Svelte stores", () => {
  const host = install();
  host.send("level", 3);
  const levels = fallingLevels("level", { read: (value) => (typeof value === "number" ? value : 0) });
  const target = render(StoreText, { parameter: parameter("mix"), levels });
  expect(target.textContent).toBe("0.5 3");
  host.push("mix", 0.25);
  host.send("level", 5);
  flushSync();
  expect(target.textContent).toBe("0.25 5");
});

test("unmounting detaches the controls", () => {
  install();
  const target = render(DialDrawing, { parameter: parameter("mix"), record: () => {} });
  const element = target.firstElementChild as HTMLElement;
  expect(element.getAttribute("role")).toBe("slider");
  unmountAll();
  // Detaching removes what the control set.
  expect(element.hasAttribute("role")).toBe(false);
  expect(element.hasAttribute("aria-valuenow")).toBe(false);
});

test("a view follows a new text for the same value and the parameters that arrive with ready", () => {
  // The plugin's text for the value, which its answer to an edit can change.
  let answer = "1.0";
  const host = installRuntime([{ info: { stringId: "time", name: "Time", units: "s", value: 0.5, displayText: "1.0" }, text: () => answer }], false);
  const seen: Record<string, unknown> = {};
  render(ViewFields, { parameter: parameter("time"), record: (field: string, value: unknown) => (seen[field] = value) });
  expect(seen).toEqual({ handle: true, value: 0, displayText: "", text: "", info: undefined });

  host.init();
  flushSync();
  expect(seen).toEqual({ handle: true, value: 0.5, displayText: "1.0", text: "1.0 s", info: { name: "Time", displayText: "1.0" } });

  // The plugin answers with a new text for the same value, as the runtime
  // does: the info first, then the subscribers.
  answer = "1.00";
  const info = parameter("time").info;
  if (info) info.displayText = "1.00";
  host.push("time", 0.5);
  flushSync();
  expect(seen).toEqual({ handle: true, value: 0.5, displayText: "1.00", text: "1.00 s", info: { name: "Time", displayText: "1.00" } });
});

// Count the runtime's live listeners per parameter from now on.
function countListeners(): Record<string, number> {
  const counts: Record<string, number> = {};
  const parameters = globalThis.__BEAMER__!.parameters;
  const on = parameters.on;
  parameters.on = (id, callback) => {
    counts[id] = (counts[id] ?? 0) + 1;
    const off = on(id, callback);
    return () => {
      counts[id] -= 1;
      off();
    };
  };
  return counts;
}

test("a view of a parameter prop follows the new parameter and lets the old one go", async () => {
  const host = install();
  const listeners = countListeners();
  const seen: Array<[string, unknown]> = [];
  const record = (field: string, value: unknown) => seen.push([field, value]);
  render(ViewFields, { parameter: parameter("mix"), record });
  expect(Object.fromEntries(seen)).toMatchObject({ handle: true, value: 0.5, text: "50%" });
  expect(listeners).toEqual({ mix: 1 });

  control.set({ parameter: parameter("gain") });
  flushSync();
  await tick();
  expect(Object.fromEntries(seen)).toMatchObject({ handle: true, value: 0.2, displayText: "2.0", text: "2.0 dB" });
  expect(listeners).toEqual({ mix: 0, gain: 1 });

  // A change of the old parameter reaches no field, one of the new does.
  seen.length = 0;
  host.push("mix", 0.8);
  flushSync();
  expect(seen).toEqual([]);
  host.push("gain", 0.3);
  flushSync();
  expect(Object.fromEntries(seen)).toMatchObject({ value: 0.3, text: "3.0 dB" });

  unmountAll();
  await tick();
  expect(listeners).toEqual({ mix: 0, gain: 0 });
});

test("a view made per item of an each block follows its parameter while the item is there", async () => {
  const host = install();
  const listeners = countListeners();
  const target = render(ViewList, { parameters: [parameter("mix"), parameter("gain")] });
  expect(target.textContent).toBe("50%2.0 dB");
  expect(listeners).toEqual({ mix: 1, gain: 1 });
  host.push("gain", 0.25);
  flushSync();
  expect(target.textContent).toBe("50%2.5 dB");

  control.set({ parameters: [parameter("mix")] });
  flushSync();
  await tick();
  expect(target.textContent).toBe("50%");
  expect(listeners).toEqual({ mix: 1, gain: 0 });

  unmountAll();
  await tick();
  expect(listeners).toEqual({ mix: 0, gain: 0 });
});
