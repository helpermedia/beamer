import { untrack } from "svelte";
import type { Attachment } from "svelte/attachments";
import type { Parameter } from "@helpermedia/beamer-api";
import { attachDial, attachRadioGroup, attachSlider, attachSwitch, attachTextField } from "@helpermedia/beamer-controls";
import type { DialOptions, SliderOptions, TextFieldOptions } from "@helpermedia/beamer-controls";

// The controls of beamer-controls as attachments, the layer under the
// components. A component makes one from its parameter and options inside
// a `$derived`, so it attaches again when one of them changes. The attach
// function runs untracked: what it reads (a `text` function that reads
// state, say) must not attach the control again, which would close an
// open text field.

/** `attachDial` as an attachment. */
export function dial(parameter: Parameter, options: DialOptions): Attachment<HTMLElement | SVGElement> {
  return (element) => untrack(() => attachDial(element, parameter, options));
}

/** `attachSlider` as an attachment. */
export function slider(parameter: Parameter, options: SliderOptions): Attachment<HTMLElement | SVGElement> {
  return (element) => untrack(() => attachSlider(element, parameter, options));
}

/** `attachSwitch` as an attachment. */
export function onOffSwitch(parameter: Parameter): Attachment<HTMLButtonElement> {
  return (button) => untrack(() => attachSwitch(button, parameter));
}

/** `attachRadioGroup` as an attachment. */
export function radioGroup(parameter: Parameter): Attachment<HTMLElement> {
  return (container) => untrack(() => attachRadioGroup(container, parameter));
}

/** `attachTextField` as an attachment. */
export function textField(parameter: Parameter, options: TextFieldOptions): Attachment<HTMLInputElement> {
  return (input) => untrack(() => attachTextField(input, parameter, options));
}
