// Type tests of beamer-controls with the generated file of beamer-api's
// type tests: the controls' own sources compile under it and falling
// levels read the sender's type. Checked by tsc, never run.

import { fallingLevels } from "@helpermedia/beamer-controls";

export function levels(): void {
  // The value is Meters without an annotation.
  fallingLevels("meters", { read: (value) => value?.reductionDb ?? 0 });
  // @ts-expect-error Meters has no field `reduction`.
  fallingLevels("meters", { read: (value) => value?.reduction ?? 0 });
  // @ts-expect-error A typo in the sender.
  fallingLevels("meter", { read: () => 0 });
}
