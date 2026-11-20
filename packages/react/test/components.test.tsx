import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterAll, beforeAll, beforeEach, expect, test } from "bun:test";
import { parameter } from "@helpermedia/beamer-api";
import type { Parameter } from "@helpermedia/beamer-api";
import { openTextField } from "@helpermedia/beamer-controls";
import type { DialOptions, SliderOptions, TextFieldOptions } from "@helpermedia/beamer-controls";
import type { DialState, SliderState } from "../src/index";
import { calls, install, push } from "./runtime";

beforeAll(() => GlobalRegistrator.register());
afterAll(() => GlobalRegistrator.unregister());

// React reads the DOM globals when it loads, so it loads after happy-dom.
const { act, createRef, useEffect, useLayoutEffect, useRef } = await import("react");
const { createRoot } = await import("react-dom/client");
const { Dial, RadioGroup, Slider, Switch, TextField } = await import("../src/index");

// React warns outside its test environment unless told it is one.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function mount() {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  return {
    container,
    render: (element: React.ReactNode) => act(() => root.render(element)),
    unmount: () => act(() => root.unmount()),
  };
}

// Whether two lists hold the very same items. `toEqual` compares elements
// by what they look like, so two empty divs would count as equal.
function same(seen: readonly unknown[], expected: readonly unknown[]): boolean {
  return seen.length === expected.length && seen.every((item, index) => item === expected[index]);
}

beforeEach(() => install());

test("Dial is a div with the dial's input that hands its values to the drawing", () => {
  const seen: DialState[] = [];
  const ref = createRef<HTMLDivElement>();
  const { container, render, unmount } = mount();
  render(
    <Dial parameter={parameter("mix")} ref={ref} className="dial" data-look="classic">
      {(state) => {
        seen.push(state);
        return <svg />;
      }}
    </Dial>,
  );
  const element = container.firstElementChild as HTMLElement;
  expect(element.tagName).toBe("DIV");
  expect(ref.current).toBe(element as HTMLDivElement);
  expect(element.className).toBe("dial");
  expect(element.dataset.look).toBe("classic");
  expect(element.getAttribute("role")).toBe("slider");
  expect(element.getAttribute("aria-valuetext")).toBe("50%");
  expect(element.querySelector("svg")).not.toBeNull();
  // The default arc: from 135 degrees, 270 of them.
  expect(seen.at(-1)).toEqual({ value: 0.5, angle: 270, text: "50%" });

  act(() => push.parameter(0.8, "80"));
  expect(seen.at(-1)?.text).toBe("80%");
  expect(seen.at(-1)?.angle).toBeCloseTo(351);

  act(() => {
    element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true }));
  });
  expect(calls).toEqual(["begin", "set 0.81", "end"]);

  // An arc of its own turns the angle too.
  render(
    <Dial parameter={parameter("mix")} arcStart={90} arcSweep={180}>
      {(state) => {
        seen.push(state);
        return null;
      }}
    </Dial>,
  );
  expect(seen.at(-1)?.angle).toBeCloseTo(90 + 0.81 * 180);
  unmount();
  expect(element.hasAttribute("role")).toBe(false);
});

test("Slider hands the thumb's position and follows its orientation", () => {
  const seen: SliderState[] = [];
  const { container, render, unmount } = mount();
  const slider = (orientation?: "vertical" | "horizontal") => (
    <Slider parameter={parameter("mix")} thumbLength={0.2} orientation={orientation}>
      {(state) => {
        seen.push(state);
        return null;
      }}
    </Slider>
  );
  render(slider());
  const element = container.firstElementChild as HTMLElement;
  expect(element.getAttribute("role")).toBe("slider");
  expect(element.getAttribute("aria-orientation")).toBe("vertical");
  // Half the thumb in from the low end, the rest of the travel by the value.
  expect(seen.at(-1)?.position).toBeCloseTo(0.1 + 0.5 * 0.8);
  act(() => push.parameter(1, "100"));
  expect(seen.at(-1)?.position).toBeCloseTo(0.9);
  render(slider("horizontal"));
  expect(element.getAttribute("aria-orientation")).toBe("horizontal");
  unmount();
});

test("Switch is a button that flips the value and shows whether it is on", () => {
  const { container, render, unmount } = mount();
  render(
    <Switch parameter={parameter("mix")} className="switch">
      {({ checked }) => `Mix ${checked ? "on" : "off"}`}
    </Switch>,
  );
  const button = container.firstElementChild as HTMLButtonElement;
  expect(button.tagName).toBe("BUTTON");
  expect(button.type).toBe("button");
  expect(button.className).toBe("switch");
  expect(button.getAttribute("role")).toBe("switch");
  expect(button.getAttribute("aria-checked")).toBe("true");
  expect(button.textContent).toBe("Mix on");
  act(() => button.click());
  expect(calls).toEqual(["begin", "set 0", "end"]);
  expect(button.getAttribute("aria-checked")).toBe("false");
  expect(button.textContent).toBe("Mix off");
  unmount();
});

test("RadioGroup makes its buttons radios and hands the choices", () => {
  const { container, render, unmount } = mount();
  render(
    <RadioGroup parameter={parameter("mode")}>
      {({ choices, checked }) =>
        choices.map((name, index) => (
          <button key={name} type="button" data-checked={index === checked ? "" : undefined}>
            {name}
          </button>
        ))
      }
    </RadioGroup>,
  );
  const group = container.firstElementChild as HTMLElement;
  const buttons = [...group.children] as HTMLButtonElement[];
  expect(group.getAttribute("role")).toBe("radiogroup");
  expect(buttons.map((button) => button.textContent)).toEqual(["A", "B", "C"]);
  expect(buttons.map((button) => button.getAttribute("aria-checked"))).toEqual(["false", "true", "false"]);
  expect(buttons[1].hasAttribute("data-checked")).toBe(true);
  act(() => buttons[2].click());
  expect(calls).toEqual(["begin", "set mode 1", "end"]);
  expect(buttons[2].hasAttribute("data-checked")).toBe(true);
  expect(buttons[2].getAttribute("aria-checked")).toBe("true");
  unmount();
});

test("TextField is an input that shows the value and opens to type one", () => {
  const ref = createRef<HTMLInputElement>();
  const { container, render, unmount } = mount();
  render(<TextField parameter={parameter("mix")} ref={ref} text={(mix) => `= ${mix.text()}`} className="field" />);
  const input = container.firstElementChild as HTMLInputElement;
  expect(ref.current).toBe(input);
  expect(input.type).toBe("text");
  expect(input.className).toBe("field");
  expect(input.readOnly).toBe(true);
  expect(input.value).toBe("= 50%");
  act(() => openTextField(input));
  expect(input.readOnly).toBe(false);
  expect(input.value).toBe("50");
  unmount();
});

test("the components render again when the parameters arrive with ready", () => {
  install(false);
  const dial: DialState[] = [];
  const radioGroup: Array<{ choices: string[]; checked: number }> = [];
  const { render, unmount } = mount();
  render(
    <>
      <Dial parameter={parameter("mix")}>
        {(state) => {
          dial.push(state);
          return null;
        }}
      </Dial>
      <RadioGroup parameter={parameter("mode")}>
        {(state) => {
          radioGroup.push(state);
          return null;
        }}
      </RadioGroup>
    </>,
  );
  expect(dial.at(-1)?.text).toBe("");
  expect(radioGroup.at(-1)).toEqual({ choices: [], checked: -1 });
  act(() => push.init());
  expect(dial.at(-1)).toEqual({ value: 0.5, angle: 270, text: "50%" });
  expect(radioGroup.at(-1)).toEqual({ choices: ["A", "B", "C"], checked: 1 });
  unmount();
});

test("a new text for the same value reaches the drawing", () => {
  const dial: DialState[] = [];
  const slider: SliderState[] = [];
  const { render, unmount } = mount();
  render(
    <>
      <Dial parameter={parameter("mix")}>
        {(state) => {
          dial.push(state);
          return null;
        }}
      </Dial>
      <Slider parameter={parameter("mix")}>
        {(state) => {
          slider.push(state);
          return null;
        }}
      </Slider>
    </>,
  );
  act(() => push.parameter(0.5, "50.0"));
  expect(dial.at(-1)?.text).toBe("50.0%");
  expect(slider.at(-1)?.text).toBe("50.0%");
  unmount();
});

test("RadioGroup of a continuous parameter has no choices and none checked", () => {
  let seen: { choices: string[]; checked: number } | undefined;
  const { render, unmount } = mount();
  render(
    <RadioGroup parameter={parameter("mix")}>
      {(state) => {
        seen = state;
        return null;
      }}
    </RadioGroup>,
  );
  expect(seen).toEqual({ choices: [], checked: -1 });
  unmount();
});

test("the options stay off the element", () => {
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
  const { container, render, unmount } = mount();
  render(
    <>
      <Dial parameter={parameter("mix")} {...dialOptions} />
      <Slider parameter={parameter("mix")} {...sliderOptions} />
      <TextField parameter={parameter("mix")} {...textFieldOptions} />
    </>,
  );
  // What the controls set, nothing of the options.
  const allowed = ["role", "tabindex", "aria-label", "aria-valuemin", "aria-valuemax", "aria-valuenow", "aria-valuetext", "aria-orientation", "style", "type", "readonly"];
  for (const element of container.children) {
    const names = [...element.attributes].map((attribute) => attribute.name);
    expect(names.filter((name) => !allowed.includes(name))).toEqual([]);
  }
  unmount();
});

test("a ref that changes gets the element, the old one lets go", () => {
  const first = createRef<HTMLButtonElement>();
  const second = createRef<HTMLButtonElement>();
  const { container, render, unmount } = mount();
  render(<Switch parameter={parameter("mix")} ref={first}>Mix</Switch>);
  const button = container.firstElementChild;
  expect(first.current).toBe(button as HTMLButtonElement);
  // Children as plain markup.
  expect(button?.textContent).toBe("Mix");
  render(<Switch parameter={parameter("mix")} ref={second}>Mix</Switch>);
  expect(first.current).toBeNull();
  expect(second.current).toBe(button as HTMLButtonElement);
  unmount();
  expect(second.current).toBeNull();
});

test("a ref holds the element in the first commit, for a layout effect of the parent", () => {
  const seen: Array<Element | null> = [];
  function Parent() {
    const dial = useRef<HTMLDivElement>(null);
    const slider = useRef<HTMLDivElement>(null);
    const onOff = useRef<HTMLButtonElement>(null);
    const radioGroup = useRef<HTMLDivElement>(null);
    const field = useRef<HTMLInputElement>(null);
    useLayoutEffect(() => {
      seen.push(dial.current, slider.current, onOff.current, radioGroup.current, field.current);
    }, []);
    return (
      <>
        <Dial parameter={parameter("mix")} ref={dial} />
        <Slider parameter={parameter("mix")} ref={slider} />
        <Switch parameter={parameter("mix")} ref={onOff} />
        <RadioGroup parameter={parameter("mode")} ref={radioGroup} />
        <TextField parameter={parameter("mix")} ref={field} />
      </>
    );
  }
  const { container, render, unmount } = mount();
  render(<Parent />);
  expect(same(seen, [...container.children])).toBe(true);
  expect(seen.length).toBe(5);
  unmount();
});

test("the controls are attached when an effect of the parent runs on mount", () => {
  const roles: Array<string | null> = [];
  function Parent() {
    const dial = useRef<HTMLDivElement>(null);
    const field = useRef<HTMLInputElement>(null);
    useEffect(() => {
      roles.push(dial.current?.getAttribute("role") ?? null);
      // A field that opens as soon as it mounts.
      if (field.current) openTextField(field.current);
    }, []);
    return (
      <>
        <Dial parameter={parameter("mix")} ref={dial} />
        <TextField parameter={parameter("mix")} ref={field} />
      </>
    );
  }
  const { container, render, unmount } = mount();
  render(<Parent />);
  expect(roles).toEqual(["slider"]);
  const input = container.querySelector("input") as HTMLInputElement;
  expect(input.readOnly).toBe(false);
  expect(input.hasAttribute("data-typing")).toBe(true);
  unmount();
});

test("a callback ref gets the element once and null when it goes", () => {
  const seen: Array<Element | null> = [];
  const ref = (node: HTMLDivElement | null) => {
    seen.push(node);
  };
  const { container, render, unmount } = mount();
  render(<Dial parameter={parameter("mix")} ref={ref} />);
  const element = container.firstElementChild;
  expect(same(seen, [element])).toBe(true);
  // Renders in between leave it alone.
  act(() => push.parameter(0.8, "80"));
  render(<Dial parameter={parameter("mix")} ref={ref} className="dial" />);
  expect(same(seen, [element])).toBe(true);
  unmount();
  expect(same(seen, [element, null])).toBe(true);
});

test("a callback ref with a cleanup gets the element once, then its cleanup runs", () => {
  const seen: Array<Element | string | null> = [];
  const ref = (node: HTMLInputElement | null) => {
    seen.push(node);
    return () => {
      seen.push("cleanup");
    };
  };
  const { container, render, unmount } = mount();
  render(<TextField parameter={parameter("mix")} ref={ref} />);
  const input = container.firstElementChild;
  expect(same(seen, [input])).toBe(true);
  // Renders in between leave it alone.
  act(() => push.parameter(0.8, "80"));
  render(<TextField parameter={parameter("mix")} ref={ref} className="field" />);
  expect(same(seen, [input])).toBe(true);
  // The cleanup stands in for the call with null.
  unmount();
  expect(same(seen, [input, "cleanup"])).toBe(true);
});

test("a ref that is a new function on every render ends no drag", () => {
  const seen: Array<Element | null> = [];
  const { container, render, unmount } = mount();
  const dial = () => <Dial parameter={parameter("mix")} drag="horizontal" ref={(node) => void seen.push(node)} />;
  render(dial());
  const element = container.firstElementChild as HTMLElement;
  element.setPointerCapture = () => {};
  act(() => {
    element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1, button: 0, clientX: 10 }));
  });
  render(dial());
  // The old function lets go and the new one gets the element, as on any
  // element. The control stays where it is.
  expect(same(seen, [element, null, element])).toBe(true);
  act(() => {
    element.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerId: 1, clientX: 30 }));
    element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1 }));
  });
  expect(calls).toEqual(["begin", "set 0.6", "end"]);
  unmount();
});

test("event props reach the element", () => {
  const seen: string[] = [];
  const { container, render, unmount } = mount();
  render(<Dial parameter={parameter("mix")} onDoubleClick={() => seen.push("double click")} onLostPointerCapture={() => seen.push("lost")} />);
  const element = container.firstElementChild as HTMLElement;
  act(() => {
    element.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    element.dispatchEvent(new PointerEvent("lostpointercapture", { bubbles: true, pointerId: 1 }));
  });
  expect(seen).toEqual(["double click", "lost"]);
  unmount();
});

test("TextField opens on a double click when asked to", () => {
  const { container, render, unmount } = mount();
  render(<TextField parameter={parameter("mix")} open="double-click" />);
  const input = container.firstElementChild as HTMLInputElement;
  act(() => input.click());
  expect(input.readOnly).toBe(true);
  act(() => {
    input.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  });
  expect(input.readOnly).toBe(false);
  unmount();
});

test("TextField attaches again when open changes", () => {
  const { container, render, unmount } = mount();
  render(<TextField parameter={parameter("mix")} />);
  const input = container.firstElementChild as HTMLInputElement;
  render(<TextField parameter={parameter("mix")} open="double-click" />);
  act(() => input.click());
  expect(input.readOnly).toBe(true);
  act(() => {
    input.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  });
  expect(input.readOnly).toBe(false);
  unmount();
});
