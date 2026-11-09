import { useRef } from "react";
import { useParam, useSwitch } from "@helpermedia/beamer-react";

interface SwitchProps {
  paramId: string;
  // The text on the switch, the parameter's own name by default.
  label?: string;
}

// A switch for an on and off parameter, with a light that is blue while
// it is on. beamer-controls makes the button a switch: each click is one
// undo step.
function Switch({ paramId, label }: SwitchProps) {
  const { param, info } = useParam(paramId);
  const ref = useRef<HTMLButtonElement>(null);
  useSwitch(ref, param);
  const name = label ?? info?.name ?? paramId;

  return (
    <button
      ref={ref}
      type="button"
      className="group flex items-center gap-2 rounded-lg bg-well px-2.5 py-1.5 text-xs font-medium text-paper ring-1 ring-neutral-800 ring-inset outline-none hover:bg-neutral-800 data-[keyboard-focus]:outline-2 data-[keyboard-focus]:outline-offset-2 data-[keyboard-focus]:outline-brand-blue"
    >
      <span
        aria-hidden="true"
        className="size-2 rounded-full bg-neutral-700 group-aria-checked:bg-brand-blue group-aria-checked:shadow-[0_0_0.375rem_var(--color-brand-blue)]"
      />
      {name}
    </button>
  );
}

export default Switch;
