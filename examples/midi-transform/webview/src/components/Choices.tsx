import { useRef } from "react";
import { useChoices, useParam } from "@helpermedia/beamer-react";

interface ChoicesProps {
  paramId: string;
  // Names for the choices in place of the parameter's own, from the lowest
  // value up: for an on and off parameter shown as two choices.
  labels?: string[];
}

// A button for each choice of an enum parameter, a radio group through
// beamer-controls. The names come from the parameter, so they match what
// the host shows, unless `labels` gives others. Each button is as wide as
// its name, with the room left over shared.
function Choices({ paramId, labels }: ChoicesProps) {
  const { param } = useParam(paramId);
  const ref = useRef<HTMLDivElement>(null);
  useChoices(ref, param);

  return (
    <div ref={ref} className="flex gap-0.5 rounded-lg bg-well p-0.5 ring-1 ring-neutral-800 ring-inset">
      {(labels ?? param.choices()).map((name) => (
        <button
          key={name}
          type="button"
          className="flex-auto rounded-md px-2.5 whitespace-nowrap py-1.5 text-xs font-medium tabular-nums text-neutral-400 outline-none hover:text-paper data-[keyboard-focus]:outline-2 data-[keyboard-focus]:outline-brand-blue aria-checked:bg-brand-blue aria-checked:text-white"
        >
          {name}
        </button>
      ))}
    </div>
  );
}

export default Choices;
