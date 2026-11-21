// Type tests with a generated file that holds only some sections. Checked
// by tsc, never run.

import { parameter, runtime, senderValue, stateField } from "@helpermedia/beamer-api";
import type { Info } from "./model-types";

export async function sections(): Promise<void> {
  // @ts-expect-error The parameters are declared: a typo fails.
  parameter("gian");
  // @ts-expect-error An empty section allows no name at all.
  stateField("tab");
  const info: Info = await runtime().invoke("info");
  // @ts-expect-error The methods are declared: a typo fails.
  await runtime().invoke("infos");

  // Events are declared, so streams count as empty.
  runtime().on("sampleRate", (hz) => hz.toFixed(0));
  // @ts-expect-error A stream name with events declared and no streams.
  runtime().on("history", () => {});

  // Sections the file does not hold take any name, values unknown.
  // @ts-expect-error The value is unknown, not any.
  senderValue("anything").get().peak;
  runtime().emit("anything", { any: true });
  runtime().emit("anything");
  void info;
}
