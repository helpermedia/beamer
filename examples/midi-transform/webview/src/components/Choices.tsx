import { useParam } from "@helpermedia/beamer-react";

interface ChoicesProps {
  paramId: string;
  // Names for the choices in place of the parameter's own, from the lowest
  // value up: for an on and off parameter shown as two choices.
  labels?: string[];
}

// A button for each choice of an enum parameter. The names come from the
// parameter, so they match what the host shows, unless `labels` gives
// others. Each button is as wide as its name, with the room left over
// shared.
function Choices({ paramId, labels }: ChoicesProps) {
  const { param, value, info } = useParam(paramId);
  const steps = info?.steps ?? 0;
  const selected = steps > 0 ? Math.round(value * steps) : -1;

  return (
    <div role="radiogroup" aria-label={info?.name ?? paramId} className="flex gap-0.5 rounded-lg bg-well p-0.5 ring-1 ring-neutral-800 ring-inset">
      {(labels ?? param.choices()).map((name, index) => (
        <button
          key={name}
          type="button"
          role="radio"
          aria-checked={index === selected}
          onClick={() => param.choose(index)}
          className="flex-auto rounded-md px-2.5 whitespace-nowrap py-1.5 text-xs font-medium tabular-nums text-neutral-400 outline-none hover:text-paper focus-visible:outline-2 focus-visible:outline-brand-blue aria-checked:bg-brand-blue aria-checked:text-white"
        >
          {name}
        </button>
      ))}
    </div>
  );
}

export default Choices;
