import { createSubscriber } from "svelte/reactivity";
import type { BeamerParameterInfo, Parameter } from "@helpermedia/beamer-api";

/** A parameter as a component shows it, each field but the handle a reactive read. */
export interface ParameterView {
  /** The handle, for changes: `parameter.edit()`, `parameter.reset()`, `parameter.typeValue()`. */
  readonly parameter: Parameter;
  /** The normalized value (0 to 1). */
  readonly value: number;
  /** The value as the parameter formats it, without the unit. */
  readonly displayText: string;
  /** The value with its unit, as a host shows it. */
  readonly text: string;
  /** What the runtime knows of it, undefined before `ready`. */
  readonly info: BeamerParameterInfo | undefined;
}

/**
 * `parameter` as a component shows it. The value, the display text, the
 * text and the info are reactive reads: a template, an effect or a
 * `$derived` that reads one follows the parameter while it runs. The text
 * can change while the value stays (the plugin answers an edit), the info
 * arrives with `ready`; both count as a change. A read outside an effect
 * returns the value now.
 */
export function parameterView(parameter: Parameter): ParameterView {
  const follow = createSubscriber((update) => {
    // `subscribe` calls back at once with the value now, which is no change.
    let started = false;
    const unsubscribe = parameter.subscribe(() => {
      if (started) update();
    });
    started = true;
    return unsubscribe;
  });
  return {
    parameter,
    get value() {
      follow();
      return parameter.get();
    },
    get displayText() {
      follow();
      return parameter.displayText();
    },
    get text() {
      follow();
      return parameter.text();
    },
    get info() {
      follow();
      return parameter.info;
    },
  };
}
