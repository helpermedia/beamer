<!--
  The text field of `parameter`: an `<input>` with `attachTextField`.
  Closed it shows the value (`text`, the value with its unit by default), a
  click (or a double click, with `open="double-click"`) opens it to type
  one. The field owns its text and whether it is read-only. Other props go
  to the `<input>`, `bind:ref` gives it, for `openTextField`.
-->
<script lang="ts">
  import type { HTMLInputAttributes } from "svelte/elements";
  import type { Parameter } from "@helpermedia/beamer-api";
  import type { TextFieldOptions } from "@helpermedia/beamer-controls";
  import { textField } from "./attachments.js";
  import type { EveryKeyOf, NoKeyLeft } from "./option-keys.js";

  type Props = TextFieldOptions &
    Omit<HTMLInputAttributes, "children" | "value" | "defaultValue" | "defaultvalue" | "type" | "readonly"> & {
      parameter: Parameter;
      /** The `<input>`, for `bind:ref`. */
      ref?: HTMLInputElement | null;
    };

  let { parameter, ref = $bindable(null), text, open, ...rest }: Props = $props();
  // Every option is taken above, none goes to the `<input>`.
  type _OptionsTaken = NoKeyLeft<Extract<keyof typeof rest, keyof TextFieldOptions>>;

  // The newest `text` shows at the next change of the value, so a new
  // function does not attach the field again, which would close it.
  const showText = (target: Parameter): string => (text ? text(target) : target.text());
  const attach = $derived(textField(parameter, { text: showText, open } satisfies EveryKeyOf<TextFieldOptions>));
</script>

<input {...rest} type="text" bind:this={ref} {@attach attach} />
