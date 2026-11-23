import { useId } from "react";
import { Switch as SwitchControl, useParameter } from "@helpermedia/beamer-react";
import type { ParameterId } from "@helpermedia/beamer-api";

interface SwitchProps {
  parameterId: ParameterId;
  // The name of what the switch turns on: the title beside it, or only its
  // spoken name when `titled` is false. The parameter's own name by
  // default.
  label?: string;
  // Whether the name shows beside the switch, as the title of its section.
  // Without it the place on the page says what the switch belongs to.
  titled?: boolean;
}

// An on and off switch: a power button in a well whose symbol is blue
// while it is on, the look over beamer-react's Switch (a button that flips
// the value, each click one undo step). The button carries no name of its
// own, as a power button does not. The title beside it is its label, so a
// click on the title flips it too and a screen reader names the switch by
// it.
function Switch({ parameterId, label, titled = true }: SwitchProps) {
  const { parameter, info } = useParameter(parameterId);
  const name = label ?? info?.name ?? parameterId;
  const id = useId();

  return (
    <div className="flex items-center gap-2">
      <SwitchControl
        parameter={parameter}
        id={id}
        aria-label={titled ? undefined : name}
        className="group grid size-7 shrink-0 place-items-center rounded-lg bg-well text-neutral-500 ring-1 ring-neutral-800 ring-inset outline-none hover:bg-neutral-800 aria-checked:text-brand-blue data-[keyboard-focus]:outline-2 data-[keyboard-focus]:outline-solid data-[keyboard-focus]:outline-offset-2 data-[keyboard-focus]:outline-brand-blue"
      >
        <svg
          viewBox="0 0 16 16"
          aria-hidden="true"
          className="size-3.5 fill-none stroke-current group-aria-checked:drop-shadow-[0_0_0.25rem_var(--color-brand-blue)]"
          strokeWidth={1.75}
          strokeLinecap="round"
        >
          {/* The power symbol: a ring open at the top with a stroke
              through the opening. */}
          <path d="M4.7 4.4a5 5 0 1 0 6.6 0" />
          <path d="M8 2.5v5" />
        </svg>
      </SwitchControl>
      {titled && (
        <label htmlFor={id} className="text-xs font-medium text-paper">
          {name}
        </label>
      )}
    </div>
  );
}

export default Switch;
