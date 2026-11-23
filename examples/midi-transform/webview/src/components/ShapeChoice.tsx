import { useRef } from "react";
import type { KeyboardEvent } from "react";
import type { CurveShape } from "../model-types";

const SHAPES: { id: CurveShape; label: string }[] = [
  { id: "smooth", label: "Smooth" },
  { id: "linear", label: "Linear" },
];

interface ShapeChoiceProps {
  shape: CurveShape;
  onChange: (shape: CurveShape) => void;
}

// The shape of the velocity curve as two choices, with the look of the
// RadioGroup of the parameters. One tab stop; the arrow keys choose the
// shape before or after, as in a group of radio buttons.
function ShapeChoice({ shape, onChange }: ShapeChoiceProps) {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  // Shortcuts with Command, Control or Alt belong to the page.
  function onKeyDown(event: KeyboardEvent, index: number) {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const forward = event.key === "ArrowRight" || event.key === "ArrowDown";
    const back = event.key === "ArrowLeft" || event.key === "ArrowUp";
    if (!forward && !back) return;
    event.preventDefault();
    const next = (index + (forward ? 1 : SHAPES.length - 1)) % SHAPES.length;
    onChange(SHAPES[next].id);
    buttons.current[next]?.focus();
  }

  return (
    <div role="radiogroup" aria-label="Shape" className="flex gap-0.5 rounded-lg bg-well p-0.5 ring-1 ring-neutral-800 ring-inset">
      {SHAPES.map(({ id, label }, index) => (
        <button
          key={id}
          ref={(element) => {
            buttons.current[index] = element;
          }}
          type="button"
          role="radio"
          aria-checked={shape === id}
          tabIndex={shape === id ? 0 : -1}
          className="flex-auto rounded-md px-2.5 py-1.5 text-xs font-medium text-neutral-400 outline-none hover:text-paper focus-visible:outline-2 focus-visible:outline-solid focus-visible:outline-brand-blue aria-checked:bg-brand-blue aria-checked:text-white"
          onClick={() => onChange(id)}
          onKeyDown={(event) => onKeyDown(event, index)}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

export default ShapeChoice;
