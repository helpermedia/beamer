import type { Param } from "@helpermedia/beamer-api";
import type { SliderOptions } from "./gestures.js";
import { attachSliderRole } from "./slider-role.js";

/**
 * Make `element` a slider for `target`: a control that moves a value along
 * a line, upright by default. It draws nothing; the look stays with the
 * caller.
 *
 * The element gets ARIA's `role="slider"` and `tabindex="0"` unless it has
 * its own. It always gets `aria-orientation` from `orientation`. It gets
 * `aria-label` from the parameter's name unless it has an `aria-label` or
 * `aria-labelledby`. `aria-valuenow`, `aria-valuemin` and `aria-valuemax`
 * follow the parameter in its plain units, `aria-valuetext` is its text
 * with the unit.
 *
 * Input, each change one undo step in the host: a press on the track
 * brings the thumb to the pointer and a press on the thumb grabs it. The
 * thumb then follows the pointer along the slider (finer with shift). Also
 * the wheel, arrow keys, Page Up and Page Down, Home and End. A double
 * click or a modifier click resets it. A press focuses the element; while
 * it has focus from the keyboard it has the `data-keyboard-focus`
 * attribute.
 *
 * The track is the element's length: the thumb's center travels it from
 * end to end, or less half of `thumbLength` at each end. Draw the thumb
 * where `thumbPosition(value, thumbLength)` puts it for the pointer to
 * stay on it.
 *
 * Returns the function that detaches it and removes what it set.
 */
export function attachSlider(element: HTMLElement | SVGElement, target: Param, options: SliderOptions = {}): () => void {
  const { orientation = "vertical", thumbLength = 0, ...input } = options;
  return attachSliderRole(element, target, input, { orientation, thumbLength });
}
