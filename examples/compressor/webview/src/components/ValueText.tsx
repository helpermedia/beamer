import { useRef } from "react";
import type { Param } from "@helpermedia/beamer-api";
import { useValueEntry } from "@helpermedia/beamer-react";
import { withMinus } from "../format";

interface ValueTextProps {
  param: Param;
  // The name of the control, for the label of the field, when it is not
  // the parameter's own.
  label?: string;
}

// The value of a control, a field that beamer-controls opens on a click.
// Enter or a click elsewhere sets what the user typed, read by the
// parameter's own parser, so "-12", "-12 dB" and "250ms" all work. Escape
// leaves the value as it was. A text left as it was sets nothing, so no
// undo step.
function ValueText({ param, label }: ValueTextProps) {
  const ref = useRef<HTMLInputElement>(null);
  useValueEntry(ref, param, { text: (shown) => withMinus(shown.text()) });

  return (
    <input
      ref={ref}
      title="Type a value"
      aria-label={label === undefined ? undefined : `${label} value`}
      className="w-full cursor-default rounded-[0.1875rem] border border-transparent px-0 py-[0.0625rem] text-center text-xs font-medium tabular-nums text-paper outline-none hover:border-neutral-700 data-[keyboard-focus]:border-brand-blue data-[typing]:cursor-text data-[typing]:border-brand-blue data-[typing]:bg-well data-[typing]:font-normal data-[typing]:select-text"
    />
  );
}

export default ValueText;
