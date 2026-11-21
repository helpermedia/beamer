// Type tests with the generated file of a model that declares nothing.
// Checked by tsc, never run.

import { parameter, runtime } from "@helpermedia/beamer-api";

export async function empty(): Promise<void> {
  // @ts-expect-error A model without parameters takes no id.
  parameter("gain");
  // @ts-expect-error No events either.
  runtime().on("sampleRate", () => {});
  // The built-in method is there with every file.
  await runtime().invoke("_beamer/parameterTextToNormalized", 1, "-6");
}
