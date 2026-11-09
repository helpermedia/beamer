import type { Param } from "@helpermedia/beamer-api";
import { Attributes } from "./attributes.js";
import { trackKeyboardFocus } from "./focus.js";
import { attachGestures } from "./gestures.js";
import type { DialOptions, Track } from "./gestures.js";

// A plain value as ARIA takes it: a finite number. A level of minus
// infinity counts as the end of the range it lies beyond.
function ariaNumber(value: number, min: number, max: number): string {
  if (Number.isFinite(value)) return String(value);
  return String(value < 0 ? min : max);
}

/**
 * What a dial and a slider share: ARIA's slider role over the gestures.
 * The element gets `role="slider"` and `tabindex="0"` unless it has its
 * own. It gets `aria-label` from the parameter's name unless it has an
 * `aria-label` or `aria-labelledby`. `aria-valuenow`, `aria-valuemin` and
 * `aria-valuemax` follow the parameter in its plain units,
 * `aria-valuetext` is its text with the unit. A slider passes its `track`:
 * the drag follows the pointer along it and the element gets
 * `aria-orientation`. Returns the function that detaches it and removes
 * what it set.
 */
export function attachSliderRole(
  element: HTMLElement | SVGElement,
  target: Param,
  options: DialOptions,
  track?: Track,
): () => void {
  const attributes = new Attributes();
  attributes.fallback(element, "role", "slider");
  attributes.fallback(element, "tabindex", "0");
  if (track) attributes.own(element, "aria-orientation", track.orientation);

  const show = () => {
    const info = target.info;
    if (!info) return;
    if (!element.hasAttribute("aria-labelledby")) attributes.fallback(element, "aria-label", info.name);
    attributes.own(element, "aria-valuemin", String(info.min));
    attributes.own(element, "aria-valuemax", String(info.max));
    attributes.own(element, "aria-valuenow", ariaNumber(target.plain(), info.min, info.max));
    attributes.own(element, "aria-valuetext", target.text());
  };
  const unsubscribe = target.subscribe(show);
  const stopFocus = trackKeyboardFocus(element);
  const detachGestures = attachGestures(element, target, options, track);

  return () => {
    detachGestures();
    stopFocus();
    unsubscribe();
    attributes.clear();
  };
}
