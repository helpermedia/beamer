import type { Param } from "@helpermedia/beamer-api";
import type { DialOptions } from "./gestures.js";
import { attachSliderRole } from "./slider-role.js";

/**
 * Make `element` a dial for `target`: a round control that turns a value.
 * It draws nothing; the look stays with the caller.
 *
 * The element gets ARIA's `role="slider"` (ARIA has no role of its own
 * for a dial) and `tabindex="0"` unless it has its own. It gets
 * `aria-label` from the parameter's name unless it has an `aria-label` or
 * `aria-labelledby`. `aria-valuenow`, `aria-valuemin` and `aria-valuemax`
 * follow the parameter in its plain units, `aria-valuetext` is its text
 * with the unit.
 *
 * Input, each change one undo step in the host: drag (vertical by default
 * and finer with shift, or around the center with `drag: "arc"`), the wheel,
 * arrow keys, Page Up and Page Down, Home and End. A double click or a
 * modifier click resets it. A press focuses the element; while it has
 * focus from the keyboard it has the `data-keyboard-focus` attribute.
 *
 * Returns the function that detaches it and removes what it set.
 */
export function attachDial(element: HTMLElement | SVGElement, target: Param, options: DialOptions = {}): () => void {
  return attachSliderRole(element, target, options);
}
