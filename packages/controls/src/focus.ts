/** The attribute a control has while it has focus from the keyboard. */
export const KEYBOARD_FOCUS = "data-keyboard-focus";

// Whether the page's last input came from the keyboard, so that focus can
// tell Tab from a click. Listened for once per page, before any element
// handles the input.
let keyboardInput = false;
let inputListened = false;

// A modifier on its own or a shortcut is no keyboard navigation.
function isNavigation(event: KeyboardEvent): boolean {
  const modifier = event.key === "Shift" || event.key === "Meta" || event.key === "Control" || event.key === "Alt";
  return !modifier && !event.metaKey && !event.ctrlKey && !event.altKey;
}

function listenForInput(): void {
  if (inputListened) return;
  inputListened = true;
  document.addEventListener(
    "keydown",
    (event) => {
      if (isNavigation(event)) keyboardInput = true;
    },
    true,
  );
  document.addEventListener("pointerdown", () => (keyboardInput = false), true);
}

/**
 * Give the element within `root` that has focus the `data-keyboard-focus`
 * attribute while that focus comes from the keyboard: Tab, or keys after a
 * click. `:focus-visible` cannot tell, because a dial or a slider focuses
 * itself from script on a press, which WebKit counts as keyboard focus.
 * Returns the function that stops it and removes the attribute.
 */
export function trackKeyboardFocus(root: HTMLElement | SVGElement): () => void {
  listenForInput();
  let marked: Element | null = null;
  const mark = (element: Element | null) => {
    if (marked === element) return;
    marked?.removeAttribute(KEYBOARD_FOCUS);
    marked = element;
    marked?.setAttribute(KEYBOARD_FOCUS, "");
  };
  const onFocusIn = (event: Event) => {
    if (keyboardInput) mark(event.target as Element);
  };
  const onFocusOut = () => mark(null);
  const onPointerDown = () => mark(null);
  // Keys that drive the control show that it has the focus.
  const onKeyDown = (event: KeyboardEvent) => {
    const focused = document.activeElement;
    if (isNavigation(event) && focused && root.contains(focused)) mark(focused);
  };
  root.addEventListener("focusin", onFocusIn);
  root.addEventListener("focusout", onFocusOut);
  root.addEventListener("pointerdown", onPointerDown);
  root.addEventListener("keydown", onKeyDown as EventListener);
  return () => {
    root.removeEventListener("focusin", onFocusIn);
    root.removeEventListener("focusout", onFocusOut);
    root.removeEventListener("pointerdown", onPointerDown);
    root.removeEventListener("keydown", onKeyDown as EventListener);
    mark(null);
  };
}
