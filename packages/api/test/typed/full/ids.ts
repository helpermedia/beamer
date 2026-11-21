// Type tests with a generated file that holds every section: each name is
// one the model declares, each value has the model's type. Checked by tsc,
// never run.

import { parameter, runtime, senderValue, stateField } from "@helpermedia/beamer-api";
import type { BeamerParameterInfo, MethodName, ParameterId, StatePath, ValueSenderName } from "@helpermedia/beamer-api";
import type { Info, Meters, Tab, Transformed } from "./model-types";

export function parameters(info: BeamerParameterInfo): void {
  parameter("gain");
  parameter("note/enabled");
  // @ts-expect-error A typo in a parameter id.
  parameter("gian");
  const id: ParameterId = parameter("mode").id;
  const stringId: "gain" | "mode" | "note/enabled" = info.stringId;
  runtime().parameters.get("gain");
  // @ts-expect-error The runtime's own API takes the model's ids too.
  runtime().parameters.get("gian");
  void id;
  void stringId;
}

export function state(): void {
  const tab = stateField("tab");
  const shown: Tab | undefined = tab.get();
  tab.set("mixer");
  // @ts-expect-error A value that is no Tab.
  tab.set("drums");
  // @ts-expect-error A typo in a State path.
  stateField("tabs");
  const paths: StatePath[] = runtime().state.ids();
  // @ts-expect-error The value is a Tab, not a number.
  const wrong: number | undefined = runtime().state.get("tab");
  void shown;
  void paths;
  void wrong;
}

export function values(): void {
  const meters: Meters | undefined = senderValue("meters").get();
  const pressed: number | undefined = runtime().values.get("pressed");
  // @ts-expect-error A typo in a sender name.
  senderValue("meter");
  // @ts-expect-error The value is Meters, not a number.
  const wrong: number | undefined = senderValue("meters").get();
  const name: ValueSenderName = "pressed";
  void meters;
  void pressed;
  void wrong;
  void name;
}

export function events(): void {
  runtime().on("sampleRate", (hz) => hz.toFixed(0));
  // @ts-expect-error The data is a number, not a string.
  runtime().on("sampleRate", (hz: string) => hz);
  // A stream sender's data is an array of its values.
  runtime().on("transformed", (all: Transformed[]) => all.length);
  // @ts-expect-error A typo in an event name.
  runtime().on("sampleRates", () => {});
  runtime().emit("padHit", { drum: 0, velocity: 0.8 });
  runtime().emit("reset");
  // @ts-expect-error The wrong data for a page event.
  runtime().emit("padHit", { pad: 0 });
  // @ts-expect-error A page event with data needs it.
  runtime().emit("padHit");
  // @ts-expect-error A typo in a page event.
  runtime().emit("padHits", { drum: 0, velocity: 1 });
}

export async function methods(): Promise<void> {
  const info: Info = await runtime().invoke("info");
  const done: boolean = await runtime().invoke("setGain", -6);
  // @ts-expect-error The result is Info, not a string.
  const wrong: string = await runtime().invoke("info");
  // @ts-expect-error A typo in a method name.
  await runtime().invoke("infos");
  // @ts-expect-error A missing argument.
  await runtime().invoke("setGain");
  // @ts-expect-error An argument of the wrong type.
  await runtime().invoke("setGain", "-6");
  // The built-in method is there with every file.
  const normalized: number | null = await runtime().invoke("_beamer/parameterTextToNormalized", 1, "-6");
  const name: MethodName = "_beamer/parameterTextToNormalized";
  void info;
  void done;
  void wrong;
  void normalized;
  void name;
}
