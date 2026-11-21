<!--
  Type tests of beamer-svelte's components and beamer-api in a Svelte
  component, with the generated file of beamer-api's type tests. Checked
  by svelte-check, never rendered. A template takes no `@ts-expect-error`,
  so the typos are in the script; the template shows that the typed names
  pass there.
-->
<script lang="ts">
  import { parameter, stateField } from "@helpermedia/beamer-api";
  import type { ParameterId } from "@helpermedia/beamer-api";
  import { Dial, parameterView } from "@helpermedia/beamer-svelte";
  import type { Tab } from "../../../api/test/typed/full/model-types";

  const tab = stateField("tab");
  const shown: Tab = $derived($tab ?? "pads");
  const ids: ParameterId[] = ["gain", "mode"];
  const view = parameterView(parameter("gain"));

  // @ts-expect-error A typo in a parameter id.
  parameter("mdoe");
  // @ts-expect-error A value that is no Tab.
  tab.set("drums");
</script>

<p>{shown} {view.text} {parameter("mode").text()}</p>
<button onclick={() => tab.set("mixer")}>Mixer</button>
{#each ids as id (id)}
  <Dial parameter={parameter(id)} />
{/each}
