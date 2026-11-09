import type { Param } from "@helpermedia/beamer-api";
import { Attributes } from "./attributes.js";
import { trackKeyboardFocus } from "./focus.js";

/** How a value field shows the value and opens for typing. */
export interface ValueEntryOptions {
  /**
   * The text the closed field shows, called whenever the value changes.
   * Default the value with its unit, `param.text()`.
   */
  text?: (param: Param) => string;
  /** What opens the field with the pointer: a click (the default) or a double click. */
  open?: "click" | "double-click";
}

// The function that opens each attached field, for `openValueEntry`.
const openers = new WeakMap<HTMLInputElement, () => void>();

const ATTRIBUTE_TYPING = "data-typing";

/**
 * Make `input` the field that shows the value of `target` and takes a
 * typed one. It draws nothing; the look stays with the caller.
 *
 * Closed, the field is read-only and shows the value (`options.text`). A
 * click (or a double click, with `open: "double-click"`), Enter or Space
 * opens it: it shows the value without the unit, all selected, with the
 * `data-typing` attribute for its style. Enter or a click elsewhere
 * sets what the user typed, read by the parameter's own parser, so "-12",
 * "-12 dB" and "250ms" all work. A text left as it was sets nothing, so no
 * undo step. Escape leaves the value as it was. The focus stays in the
 * field, so a keyboard user goes on from there.
 *
 * The field gets `aria-label` from the parameter's name unless it has an
 * `aria-label` or `aria-labelledby`. While it has focus from the keyboard
 * it has the `data-keyboard-focus` attribute.
 *
 * Returns the function that detaches it; an open field closes without
 * setting what was typed.
 */
export function attachValueEntry(input: HTMLInputElement, target: Param, options: ValueEntryOptions = {}): () => void {
  const text = options.text ?? ((param: Param) => param.text());
  const attributes = new Attributes();
  const readOnly = input.readOnly;
  input.readOnly = true;

  let typing = false;
  // The text the field opened with.
  let opened = "";

  const show = () => {
    const info = target.info;
    if (info && !input.hasAttribute("aria-labelledby")) attributes.fallback(input, "aria-label", `${info.name} value`);
    if (!typing) input.value = text(target);
  };
  const open = () => {
    if (typing || !target.info) return;
    typing = true;
    opened = target.displayText();
    input.readOnly = false;
    input.setAttribute(ATTRIBUTE_TYPING, "");
    input.value = opened;
    input.focus();
    input.select();
  };
  // Close the field and set `typed`, unless it is null or the text the
  // field opened with.
  const close = (typed: string | null) => {
    if (!typing) return;
    typing = false;
    input.readOnly = true;
    input.removeAttribute(ATTRIBUTE_TYPING);
    show();
    if (typed !== null && typed.trim() !== opened.trim()) void target.typeValue(typed);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    // A key that ends a composition of an input method (WebKit sends its
    // Enter with key code 229) belongs to the composition.
    if (event.isComposing || event.keyCode === 229) return;
    if (!typing) {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      open();
      return;
    }
    if (event.key !== "Enter" && event.key !== "Escape") return;
    event.preventDefault();
    close(event.key === "Enter" ? input.value : null);
  };
  const onBlur = () => close(input.value);
  const openOn = options.open === "double-click" ? "dblclick" : "click";

  const unsubscribe = target.subscribe(show);
  const stopFocus = trackKeyboardFocus(input);
  input.addEventListener("keydown", onKeyDown);
  input.addEventListener("blur", onBlur);
  input.addEventListener(openOn, open);
  openers.set(input, open);

  return () => {
    openers.delete(input);
    input.removeEventListener("keydown", onKeyDown);
    input.removeEventListener("blur", onBlur);
    input.removeEventListener(openOn, open);
    close(null);
    stopFocus();
    unsubscribe();
    attributes.clear();
    input.readOnly = readOnly;
  };
}

/**
 * Open the value field `input` for typing, as a click on it would, for
 * another way in such as a double click on the dial next to it. Does
 * nothing for a field that `attachValueEntry` did not attach.
 */
export function openValueEntry(input: HTMLInputElement): void {
  openers.get(input)?.();
}
