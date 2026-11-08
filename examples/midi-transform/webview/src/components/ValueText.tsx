import { useCallback, useRef, useState } from "react";
import type { KeyboardEvent } from "react";
import type { Param } from "@helpermedia/beamer-api";
import { withMinus } from "../format";

interface ValueTextProps {
  param: Param;
  // The value with its unit.
  text: string;
  // The parameter's name, for the label of the text field.
  name: string;
}

// The value of a control. A click turns it into a text field: Enter sets
// what the user typed, read by the parameter's own parser, so "-12", "-12
// dB" and "250ms" all work. Escape or a click elsewhere leaves it.
function ValueText({ param, text, name }: ValueTextProps) {
  const [typing, setTyping] = useState(false);
  // Whether the value takes the focus back when the field closes: after
  // Enter or Escape a keyboard user goes on from there.
  const refocus = useRef(false);

  const focusInput = useCallback((input: HTMLInputElement | null) => {
    input?.focus();
    input?.select();
  }, []);
  const focusButton = useCallback((button: HTMLButtonElement | null) => {
    if (button && refocus.current) {
      refocus.current = false;
      button.focus();
    }
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key !== "Enter" && event.key !== "Escape") return;
    refocus.current = true;
    setTyping(false);
    if (event.key === "Enter") param.typeValue(event.currentTarget.value);
  };

  const shared = "w-full rounded-[0.1875rem] border px-0 py-[0.0625rem] text-center text-xs tabular-nums";
  if (typing) {
    return (
      <input
        ref={focusInput}
        defaultValue={param.displayText()}
        aria-label={`${name} value`}
        onKeyDown={onKeyDown}
        onBlur={() => setTyping(false)}
        className={`${shared} border-brand-blue bg-well text-paper outline-none select-text`}
      />
    );
  }
  return (
    <button
      ref={focusButton}
      type="button"
      title="Type a value"
      onClick={() => setTyping(true)}
      className={`${shared} border-transparent font-medium text-paper outline-none hover:border-neutral-700 focus-visible:border-brand-blue`}
    >
      {withMinus(text)}
    </button>
  );
}

export default ValueText;
