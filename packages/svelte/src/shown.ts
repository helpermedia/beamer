import { createSubscriber } from "svelte/reactivity";
import type { BeamerParameterInfo, Parameter } from "@helpermedia/beamer-api";

/** What a component shows of a parameter, read reactively. */
export interface Shown {
  /** The normalized value (0 to 1). */
  readonly value: number;
  /** The value with its unit, as a host shows it. */
  readonly text: string;
  /** What the runtime knows of it, undefined before `ready`. */
  readonly info: BeamerParameterInfo | undefined;
}

/**
 * The value, the text and the info of `parameter`, each a reactive read: an
 * effect or a template that reads one follows the parameter while it runs.
 * The text can change while the value stays (the plugin answers an edit),
 * the info arrives with `ready`; both count as a change.
 */
export function shown(parameter: Parameter): Shown {
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
    get value() {
      follow();
      return parameter.get();
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
