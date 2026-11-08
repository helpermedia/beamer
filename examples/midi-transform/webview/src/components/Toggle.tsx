import { useParam } from "@helpermedia/beamer-react";

interface ToggleProps {
  paramId: string;
  // The text on the switch, the parameter's own name by default.
  label?: string;
}

// A switch for an on and off parameter, with a light that is blue while
// it is on. Each click is one undo step.
function Toggle({ paramId, label }: ToggleProps) {
  const { param, value, info } = useParam(paramId);
  const on = value >= 0.5;
  const name = label ?? info?.name ?? paramId;

  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      onClick={() => param.edit(on ? 0 : 1)}
      className="flex items-center gap-2 rounded-lg bg-well px-2.5 py-1.5 text-xs font-medium text-paper ring-1 ring-neutral-800 ring-inset outline-none hover:bg-neutral-800 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand-blue"
    >
      <span
        aria-hidden="true"
        className={`size-2 rounded-full ${on ? "bg-brand-blue shadow-[0_0_0.375rem_var(--color-brand-blue)]" : "bg-neutral-700"}`}
      />
      {name}
    </button>
  );
}

export default Toggle;
