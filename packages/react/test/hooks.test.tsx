import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { afterAll, beforeAll, beforeEach, expect, test } from "bun:test";
import type { DialDrag } from "@helpermedia/beamer-controls";
import { calls, install, push } from "./runtime";

beforeAll(() => GlobalRegistrator.register());
afterAll(() => GlobalRegistrator.unregister());

// React reads the DOM globals when it loads, so it loads after happy-dom.
const { act, useRef } = await import("react");
const { createRoot } = await import("react-dom/client");
const { useChoices, useDial, useEvent, useParameter, useReadable, useReady, useSenderValue, useSlider, useStateField, useSwitch, useValueEntry } =
  await import("../src/index");

// React warns outside its test environment unless told it is one.
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function render(element: React.ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(element));
  return { container, unmount: () => act(() => root.unmount()) };
}

beforeEach(() => install());

test("useParameter follows the value and the text the plugin answers with", () => {
  function Dial() {
    const { value, text } = useParameter("mix");
    return <span>{`${value} ${text}`}</span>;
  }
  const { container, unmount } = render(<Dial />);
  expect(container.textContent).toBe("0.5 50%");
  // The text can change without the value: it renders anyway.
  act(() => push.parameter(0.5, "50.0"));
  expect(container.textContent).toBe("0.5 50.0%");
  act(() => push.parameter(0.8, "80"));
  expect(container.textContent).toBe("0.8 80%");
  unmount();
});

test("useStateField reads and sets a State field", () => {
  let setPanLaw: (value: string) => void = () => {};
  function PanLaw() {
    const [panLaw, set] = useStateField<string>("pan_law");
    setPanLaw = set;
    return <span>{panLaw}</span>;
  }
  const { container, unmount } = render(<PanLaw />);
  expect(container.textContent).toBe("linear");
  act(() => setPanLaw("constant_power"));
  expect(calls).toEqual(["state constant_power"]);
  expect(container.textContent).toBe("constant_power");
  act(() => push.state("linear"));
  expect(container.textContent).toBe("linear");
  unmount();
});

test("useSenderValue shows the newest value", () => {
  function Meter() {
    const levels = useSenderValue<{ peak: number }>("levels");
    return <span>{levels ? levels.peak : "none"}</span>;
  }
  const { container, unmount } = render(<Meter />);
  expect(container.textContent).toBe("none");
  act(() => push.value({ peak: 0.7 }));
  expect(container.textContent).toBe("0.7");
  unmount();
});

test("useEvent calls the newest handler and stops on unmount", () => {
  const seen: unknown[] = [];
  function History({ tag }: { tag: string }) {
    useEvent("history", (data) => seen.push(`${tag} ${data}`));
    return null;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<History tag="a" />));
  act(() => push.event(1));
  act(() => root.render(<History tag="b" />));
  act(() => push.event(2));
  act(() => root.unmount());
  act(() => push.event(3));
  expect(seen).toEqual(["a 1", "b 2"]);
});

test("useReady turns true once the runtime is ready", async () => {
  function Status() {
    return <span>{useReady() ? "ready" : "waiting"}</span>;
  }
  const { container, unmount } = render(<Status />);
  await act(async () => {
    await Promise.resolve();
  });
  expect(container.textContent).toBe("ready");
  unmount();
});

test("useDial attaches while mounted", () => {
  function Dial() {
    const ref = useRef<HTMLDivElement>(null);
    const { parameter } = useParameter("mix");
    useDial(ref, parameter);
    return <div ref={ref} />;
  }
  const { container, unmount } = render(<Dial />);
  const element = container.firstElementChild as HTMLElement;
  expect(element.getAttribute("role")).toBe("slider");
  expect(element.getAttribute("aria-valuetext")).toBe("50%");
  act(() => {
    element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true }));
  });
  expect(calls).toEqual(["begin", "set 0.51", "end"]);
  unmount();
  calls.length = 0;
  element.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", cancelable: true }));
  expect(calls).toEqual([]);
  expect(element.hasAttribute("role")).toBe(false);
});

test("useDial calls the newest drag function for each press", () => {
  const picked: string[] = [];
  function Dial({ tag }: { tag: string }) {
    const ref = useRef<HTMLDivElement>(null);
    const { parameter } = useParameter("mix");
    useDial(ref, parameter, {
      drag: () => {
        picked.push(tag);
        return "horizontal";
      },
    });
    return <div ref={ref} tabIndex={0} />;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Dial tag="first" />));
  act(() => root.render(<Dial tag="second" />));
  const element = container.firstElementChild as HTMLElement;
  element.setPointerCapture = () => {};
  act(() => {
    element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1, button: 0, clientX: 10 }));
    element.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerId: 1, clientX: 30 }));
    element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1 }));
  });
  expect(picked).toEqual(["second"]);
  expect(calls).toEqual(["begin", "set 0.6", "end"]);
  act(() => root.unmount());
});

test("useDial keeps a running drag across a render", () => {
  const picked: string[] = [];
  function Dial({ tag }: { tag: string }) {
    const ref = useRef<HTMLDivElement>(null);
    const { parameter } = useParameter("mix");
    useDial(ref, parameter, {
      drag: () => {
        picked.push(tag);
        return "horizontal";
      },
    });
    return <div ref={ref} tabIndex={0} />;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Dial tag="first" />));
  const element = container.firstElementChild as HTMLElement;
  element.setPointerCapture = () => {};
  act(() => {
    element.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1, button: 0, clientX: 10 }));
  });
  act(() => root.render(<Dial tag="second" />));
  act(() => {
    element.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerId: 1, clientX: 30 }));
    element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1 }));
  });
  expect(picked).toEqual(["first"]);
  expect(calls).toEqual(["begin", "set 0.6", "end"]);
  act(() => root.unmount());
});

test("useDial reads the newest drag for each press, a fixed one or a function", () => {
  function Dial({ drag }: { drag?: DialDrag | (() => DialDrag) }) {
    const ref = useRef<HTMLDivElement>(null);
    const { parameter } = useParameter("mix");
    useDial(ref, parameter, { drag });
    return <div ref={ref} tabIndex={0} />;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Dial />));
  const element = container.firstElementChild as HTMLElement;
  element.setPointerCapture = () => {};
  // A press that moves 20 pixels to the right and 40 up, from the middle.
  const press = () => {
    act(() => push.parameter(0.5, "50"));
    calls.length = 0;
    act(() => {
      element.dispatchEvent(
        new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1, button: 0, clientX: 10, clientY: 50 }),
      );
      element.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerId: 1, clientX: 30, clientY: 10 }));
      element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1 }));
    });
  };
  // Without a drag the dial takes the default, a vertical one.
  press();
  expect(calls).toEqual(["begin", "set 0.7", "end"]);
  // A function that comes later is asked.
  act(() => root.render(<Dial drag={() => "horizontal"} />));
  press();
  expect(calls).toEqual(["begin", "set 0.6", "end"]);
  // As is a fixed drag after it: both directions add up.
  act(() => root.render(<Dial drag="both" />));
  press();
  expect(calls).toEqual(["begin", "set 0.8", "end"]);
  act(() => root.unmount());
});

test("useDial keeps a running drag when the drag changes", () => {
  function Dial({ drag }: { drag: DialDrag }) {
    const ref = useRef<HTMLDivElement>(null);
    const { parameter } = useParameter("mix");
    useDial(ref, parameter, { drag });
    return <div ref={ref} tabIndex={0} />;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Dial drag="horizontal" />));
  const element = container.firstElementChild as HTMLElement;
  element.setPointerCapture = () => {};
  act(() => {
    element.dispatchEvent(
      new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1, button: 0, clientX: 10, clientY: 50 }),
    );
  });
  act(() => root.render(<Dial drag="vertical" />));
  // The drag goes on as it was picked at the press: 20 pixels to the right.
  act(() => {
    element.dispatchEvent(new PointerEvent("pointermove", { bubbles: true, pointerId: 1, clientX: 30, clientY: 10 }));
    element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 1 }));
  });
  expect(calls).toEqual(["begin", "set 0.6", "end"]);
  act(() => root.unmount());
});

test("useSwitch and useChoices attach to the markup", () => {
  function Controls() {
    const onOff = useRef<HTMLButtonElement>(null);
    const group = useRef<HTMLDivElement>(null);
    const { parameter } = useParameter("mix");
    useSwitch(onOff, parameter);
    useChoices(group, parameter);
    return (
      <>
        <button ref={onOff}>Mix</button>
        <div ref={group} />
      </>
    );
  }
  const { container, unmount } = render(<Controls />);
  const [button, group] = [...container.children] as HTMLElement[];
  expect(button.getAttribute("role")).toBe("switch");
  expect(button.getAttribute("aria-checked")).toBe("true");
  act(() => button.click());
  expect(calls).toEqual(["begin", "set 0", "end"]);
  expect(group.getAttribute("role")).toBe("radiogroup");
  unmount();
});

test("useValueEntry shows the text of the newest function", () => {
  function Field({ prefix }: { prefix: string }) {
    const ref = useRef<HTMLInputElement>(null);
    const { parameter } = useParameter("mix");
    useValueEntry(ref, parameter, { text: (mix) => `${prefix}${mix.text()}` });
    return <input ref={ref} />;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Field prefix="a " />));
  const input = container.firstElementChild as HTMLInputElement;
  expect(input.value).toBe("a 50%");
  expect(input.readOnly).toBe(true);
  act(() => root.render(<Field prefix="b " />));
  act(() => push.parameter(0.8, "80"));
  expect(input.value).toBe("b 80%");
  act(() => root.unmount());
});

test("useValueEntry keeps an open field open across a render", () => {
  function Field({ prefix }: { prefix: string }) {
    const ref = useRef<HTMLInputElement>(null);
    const { parameter } = useParameter("mix");
    useValueEntry(ref, parameter, { text: (mix) => `${prefix}${mix.text()}` });
    return <input ref={ref} />;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Field prefix="a " />));
  const input = container.firstElementChild as HTMLInputElement;
  act(() => input.click());
  expect(input.readOnly).toBe(false);
  expect(input.value).toBe("50");
  input.value = "70";
  act(() => root.render(<Field prefix="b " />));
  expect(input.readOnly).toBe(false);
  expect(input.hasAttribute("data-typing")).toBe(true);
  expect(input.value).toBe("70");
  // Closed, it shows the text of the newest function.
  act(() => {
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", cancelable: true }));
  });
  expect(input.readOnly).toBe(true);
  expect(input.value).toBe("b 50%");
  act(() => root.unmount());
});

test("useReadable follows a readable", () => {
  let value = 1;
  const callbacks = new Set<(next: number) => void>();
  const readable = {
    get: () => value,
    subscribe: (callback: (next: number) => void) => {
      callbacks.add(callback);
      callback(value);
      return () => callbacks.delete(callback);
    },
  };
  function Show() {
    return <span>{useReadable(readable)}</span>;
  }
  const { container, unmount } = render(<Show />);
  expect(container.textContent).toBe("1");
  act(() => {
    value = 2;
    callbacks.forEach((callback) => callback(value));
  });
  expect(container.textContent).toBe("2");
  unmount();
  expect(callbacks.size).toBe(0);
});

test("useSlider attaches an upright slider and follows its orientation", () => {
  function Slider({ orientation }: { orientation: "vertical" | "horizontal" }) {
    const ref = useRef<HTMLDivElement>(null);
    const { parameter } = useParameter("mix");
    useSlider(ref, parameter, { orientation });
    return <div ref={ref} />;
  }
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Slider orientation="vertical" />));
  const element = container.firstElementChild as HTMLElement;
  expect(element.getAttribute("role")).toBe("slider");
  expect(element.getAttribute("aria-orientation")).toBe("vertical");
  act(() => root.render(<Slider orientation="horizontal" />));
  expect(element.getAttribute("aria-orientation")).toBe("horizontal");
  act(() => root.unmount());
});
