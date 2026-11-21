import { Switch as SwitchControl, useParameter } from "@helpermedia/beamer-react";
import type { ParameterId } from "@helpermedia/beamer-api";

interface SwitchProps {
  parameterId: ParameterId;
  // The text on the switch, the parameter's own name by default.
  label?: string;
}

// A switch for an on and off parameter, with a light that is blue while
// it is on: the look over beamer-react's Switch, a button that flips the
// value, each click one undo step.
function Switch({ parameterId, label }: SwitchProps) {
  const { parameter, info } = useParameter(parameterId);
  const name = label ?? info?.name ?? parameterId;

  return (
    <SwitchControl
      parameter={parameter}
      className="group flex items-center gap-2 rounded-lg bg-well px-2.5 py-1.5 text-xs font-medium text-paper ring-1 ring-neutral-800 ring-inset outline-none hover:bg-neutral-800 data-[keyboard-focus]:outline-2 data-[keyboard-focus]:outline-solid data-[keyboard-focus]:outline-offset-2 data-[keyboard-focus]:outline-brand-blue"
    >
      <span
        aria-hidden="true"
        className="size-2 rounded-full bg-neutral-700 group-aria-checked:bg-brand-blue group-aria-checked:shadow-[0_0_0.375rem_var(--color-brand-blue)]"
      />
      {name}
    </SwitchControl>
  );
}

export default Switch;
