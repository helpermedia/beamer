// Type tests of beamer-react with the generated file of beamer-api's type
// tests. Checked by tsc, never run.

import type { ParameterId } from "@helpermedia/beamer-api";
import { useParameter, usePluginEvent, useSenderValue, useStateField } from "@helpermedia/beamer-react";
import type { Meters, Tab } from "../../../api/test/typed/full/model-types";

function Dial({ parameterId }: { parameterId: ParameterId }) {
  const { text } = useParameter(parameterId);
  return <span>{text}</span>;
}

export function Editor() {
  useParameter("gain");
  // @ts-expect-error A typo in a parameter id.
  useParameter("gian");

  const [tab, setTab] = useStateField("tab");
  const shown: Tab | undefined = tab;
  setTab("mixer");
  // @ts-expect-error A value that is no Tab.
  setTab("drums");
  // @ts-expect-error A typo in a State path.
  useStateField("tabs");

  const meters: Meters | undefined = useSenderValue("meters");
  // @ts-expect-error A typo in a sender name.
  useSenderValue("meter");

  usePluginEvent("transformed", (all) => all.filter((item) => item.type === "noteOn"));
  usePluginEvent("sampleRate", (hz) => hz.toFixed(0));
  // @ts-expect-error A typo in an event name.
  usePluginEvent("sampleRates", () => {});

  return (
    <div>
      <Dial parameterId="mode" />
      {/* @ts-expect-error A typo in a prop that takes a parameter id. */}
      <Dial parameterId="mdoe" />
      {shown}
      {meters?.keyDb}
    </div>
  );
}
