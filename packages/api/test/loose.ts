// Type tests without a generated file: every name passes, every value is
// unknown, never any. Checked by tsc with the other tests, never run.

import { parameter, runtime, senderValue, stateField } from "@helpermedia/beamer-api";

export async function loose(): Promise<void> {
  parameter("anything");
  stateField("anything").set({ any: true });
  // @ts-expect-error A State value is unknown, not any.
  stateField("anything").get().field;
  // @ts-expect-error A sender's value is unknown, not any.
  senderValue("anything").get().peak;
  // @ts-expect-error An event's data is unknown, not any.
  runtime().on("anything", (data) => data.length);
  // @ts-expect-error A method's result is unknown, not any.
  (await runtime().invoke("anything")).field;
  // The built-in method keeps its types.
  const normalized: number | null = await runtime().invoke("_beamer/parameterTextToNormalized", 1, "-6");
  runtime().emit("anything");
  void normalized;
}
