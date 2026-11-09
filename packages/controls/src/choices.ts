import type { Param } from "@helpermedia/beamer-api";
import { Attributes } from "./attributes.js";
import { trackKeyboardFocus } from "./focus.js";

/**
 * Make `container` a radio group for the discrete parameter `target`: one
 * choice per element child, in order, the first for the lowest value. The
 * children should be buttons, with the names of the choices
 * (`target.choices()`) or names of their own. It draws nothing; the look
 * stays with the caller.
 *
 * The container gets `role="radiogroup"` unless it has its own role and
 * `aria-label` from the parameter's name unless it has an `aria-label` or
 * `aria-labelledby`. Each child gets `role="radio"` unless it has its own,
 * `aria-checked` for the choice the parameter is at (for its style) and a
 * tabindex: only the checked child is in the tab order. Children added
 * later, such as the buttons a framework renders once the choices are
 * known, get the same.
 *
 * A click on a child sets its choice as one undo step in the host, a click
 * on the checked one sets nothing. The arrow keys check the next or
 * previous choice and move the focus there, from the last back to the
 * first. While a child has focus from the keyboard it has the
 * `data-keyboard-focus` attribute.
 *
 * Returns the function that detaches it and removes what it set.
 */
export function attachChoices(container: HTMLElement, target: Param): () => void {
  const attributes = new Attributes();
  attributes.fallback(container, "role", "radiogroup");

  const radios = () => [...container.children];
  // The choices the parameter has, 0 for a continuous parameter or before
  // `ready`.
  const count = () => {
    const steps = target.info?.steps ?? 0;
    return steps > 0 ? steps + 1 : 0;
  };
  const checked = () => (count() > 0 ? target.choice() : -1);
  // The child `node` belongs to, or -1.
  const indexOf = (node: EventTarget | null) => radios().findIndex((radio) => node instanceof Node && radio.contains(node));

  // The children this control has seen, so that removed ones lose what it
  // set.
  let seen: Element[] = [];
  const show = () => {
    const info = target.info;
    if (info && !container.hasAttribute("aria-labelledby")) attributes.fallback(container, "aria-label", info.name);
    const current = radios();
    for (const gone of seen) {
      if (!current.includes(gone)) attributes.forget(gone);
    }
    seen = current;
    const selected = checked();
    // With no choice checked, the first child takes the tab stop.
    const tabStop = selected >= 0 && selected < current.length ? selected : 0;
    current.forEach((radio, index) => {
      attributes.fallback(radio, "role", "radio");
      attributes.own(radio, "aria-checked", String(index === selected));
      attributes.own(radio, "tabindex", index === tabStop ? "0" : "-1");
    });
  };

  const choose = (index: number) => {
    if (index < 0 || index >= count() || index === checked()) return;
    target.choose(index);
  };
  const onClick = (event: MouseEvent) => choose(indexOf(event.target));
  const onKeyDown = (event: KeyboardEvent) => {
    // Shortcuts with Command, Control or Alt belong to the page.
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const step = event.key === "ArrowRight" || event.key === "ArrowDown" ? 1 : event.key === "ArrowLeft" || event.key === "ArrowUp" ? -1 : 0;
    if (step === 0) return;
    const current = radios();
    const total = Math.min(current.length, count());
    if (total === 0) return;
    event.preventDefault();
    const from = indexOf(event.target);
    const next = ((from < 0 ? checked() : from) + step + total) % total;
    (current[next] as HTMLElement | SVGElement).focus();
    choose(next);
  };

  const unsubscribe = target.subscribe(show);
  const observer = new MutationObserver(show);
  observer.observe(container, { childList: true });
  const stopFocus = trackKeyboardFocus(container);
  container.addEventListener("click", onClick);
  container.addEventListener("keydown", onKeyDown);

  return () => {
    container.removeEventListener("click", onClick);
    container.removeEventListener("keydown", onKeyDown);
    stopFocus();
    observer.disconnect();
    unsubscribe();
    attributes.clear();
  };
}
