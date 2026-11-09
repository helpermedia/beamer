/**
 * The attributes a control sets on an element, so that detaching removes
 * them again. Two kinds:
 *
 * - `fallback`: set only while the author's markup has none, such as a
 *   role or a label. An attribute that holds another value than the one
 *   the control set is the author's, also when a framework sets it later;
 *   one the author removes gets the fallback again.
 * - `own`: set by the control alone and kept up to date, such as
 *   `aria-valuenow`.
 */
export class Attributes {
  // The value this control set for each attribute, by element.
  #set = new Map<Element, Map<string, string>>();

  /** Set `name` unless the author gave it a value of their own. */
  fallback(element: Element, name: string, value: string): void {
    const current = element.getAttribute(name);
    if (current !== null && current !== this.#set.get(element)?.get(name)) {
      this.#set.get(element)?.delete(name);
      return;
    }
    this.own(element, name, value);
  }

  /** Set `name`, whatever the markup has. */
  own(element: Element, name: string, value: string): void {
    let values = this.#set.get(element);
    if (!values) {
      values = new Map();
      this.#set.set(element, values);
    }
    values.set(name, value);
    if (element.getAttribute(name) !== value) element.setAttribute(name, value);
  }

  /** Remove what this control set on `element` and still holds its value. */
  forget(element: Element): void {
    for (const [name, value] of this.#set.get(element) ?? []) {
      if (element.getAttribute(name) === value) element.removeAttribute(name);
    }
    this.#set.delete(element);
  }

  /** Remove what this control set on every element. */
  clear(): void {
    for (const element of [...this.#set.keys()]) this.forget(element);
  }
}
