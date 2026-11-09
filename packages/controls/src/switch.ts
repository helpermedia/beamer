import type { Param } from "@helpermedia/beamer-api";
import { Attributes } from "./attributes.js";
import { trackKeyboardFocus } from "./focus.js";

/**
 * Make `button` a switch for the on and off parameter `target`: on from
 * the middle of its range up. It draws nothing; the look stays with the
 * caller.
 *
 * The button gets `role="switch"` unless it has its own role.
 * `aria-checked` follows the parameter, for a style of the on state. Its
 * text is its name. Each click, also from Space or Enter, flips the value
 * as one undo step in the host. While the button has focus from the
 * keyboard it has the `data-keyboard-focus` attribute.
 *
 * Returns the function that detaches it and removes what it set.
 */
export function attachSwitch(button: HTMLButtonElement, target: Param): () => void {
  const attributes = new Attributes();
  attributes.fallback(button, "role", "switch");

  const isOn = () => target.get() >= 0.5;
  const show = () => attributes.own(button, "aria-checked", String(isOn()));
  const unsubscribe = target.subscribe(show);
  const stopFocus = trackKeyboardFocus(button);
  const onClick = () => target.edit(isOn() ? 0 : 1);
  button.addEventListener("click", onClick);

  return () => {
    button.removeEventListener("click", onClick);
    stopFocus();
    unsubscribe();
    attributes.clear();
  };
}
