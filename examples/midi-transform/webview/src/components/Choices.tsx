import { Choices as ChoicesControl, useParameter } from "@helpermedia/beamer-react";

interface ChoicesProps {
  parameterId: string;
  // Names for the choices in place of the parameter's own, from the lowest
  // value up: for an on and off parameter shown as two choices.
  labels?: string[];
}

// A button for each choice of an enum parameter, a radio group through
// beamer-react's Choices. The names come from the parameter, so they match
// what the host shows, unless `labels` gives others. Each button is as
// wide as its name, with the room left over shared.
function Choices({ parameterId, labels }: ChoicesProps) {
  const { parameter } = useParameter(parameterId);

  return (
    <ChoicesControl parameter={parameter} className="flex gap-0.5 rounded-lg bg-well p-0.5 ring-1 ring-neutral-800 ring-inset">
      {({ choices }) =>
        (labels ?? choices).map((name) => (
          <button
            key={name}
            type="button"
            className="flex-auto rounded-md px-2.5 whitespace-nowrap py-1.5 text-xs font-medium tabular-nums text-neutral-400 outline-none hover:text-paper data-[keyboard-focus]:outline-2 data-[keyboard-focus]:outline-solid data-[keyboard-focus]:outline-brand-blue aria-checked:bg-brand-blue aria-checked:text-white"
          >
            {name}
          </button>
        ))
      }
    </ChoicesControl>
  );
}

export default Choices;
