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

// The value of a control. A click turns it into a text field. Enter or a
// click elsewhere sets what the user typed, read by the parameter's own
// parser, so "-12", "-12 dB" and "250ms" all work. Escape leaves the value
// as it was. A text left as it was sets nothing, so no undo step.
function ValueText({ param, text, name }: ValueTextProps) {
  const [typing, setTyping] = useState(false);
  // Whether the value takes the focus back when the field closes: after
  // Enter or Escape a keyboard user goes on from there.
  const refocus = useRef(false);
  // The text the field opened with.
  const opened = useRef("");
  // Whether the field was closed already: a field that Enter or Escape
  // closed may still lose the focus after.
  const closed = useRef(false);

  const open = () => {
    opened.current = param.displayText();
    closed.current = false;
    setTyping(true);
  };
  const close = (typed: string | null) => {
    if (closed.current) return;
    closed.current = true;
    setTyping(false);
    if (typed !== null && typed.trim() !== opened.current) param.typeValue(typed);
  };

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
    // A key that ends a composition of an input method (WebKit sends its
    // Enter with key code 229) belongs to the composition.
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (event.key !== "Enter" && event.key !== "Escape") return;
    // The key ends here. Let through, the Enter would reach the value
    // once it has the focus back and open the field again.
    event.preventDefault();
    refocus.current = true;
    close(event.key === "Enter" ? event.currentTarget.value : null);
  };

  const shared = "w-full rounded-[0.1875rem] border px-0 py-[0.0625rem] text-center text-xs tabular-nums";
  if (typing) {
    return (
      <input
        ref={focusInput}
        defaultValue={opened.current}
        aria-label={`${name} value`}
        onKeyDown={onKeyDown}
        onBlur={(event) => close(event.currentTarget.value)}
        className={`${shared} border-brand-blue bg-well text-paper outline-none select-text`}
      />
    );
  }
  return (
    <button
      ref={focusButton}
      type="button"
      title="Type a value"
      onClick={open}
      className={`${shared} border-transparent font-medium text-paper outline-none hover:border-neutral-700 focus-visible:border-brand-blue`}
    >
      {withMinus(text)}
    </button>
  );
}

export default ValueText;
