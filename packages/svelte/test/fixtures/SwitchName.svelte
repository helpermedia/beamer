<!-- A switch whose name says whether it is on, or a plain name with `ref`. -->
<script lang="ts">
  import type { Parameter } from "@helpermedia/beamer-api";
  import { Switch } from "../../src/index";

  let { parameter, plain = false, onRef }: { parameter: Parameter; plain?: boolean; onRef?: (button: HTMLButtonElement | null) => void } =
    $props();
  let ref: HTMLButtonElement | null = $state(null);
  $effect(() => onRef?.(ref));
</script>

{#if plain}
  <Switch {parameter} class="switch" bind:ref>Mix</Switch>
{:else}
  <Switch {parameter} class="switch">
    {#snippet children({ checked })}Mix {checked ? "on" : "off"}{/snippet}
  </Switch>
{/if}
