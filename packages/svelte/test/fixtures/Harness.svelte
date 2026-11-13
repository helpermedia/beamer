<!-- Renders `component` with props a test can change: `control.set`. -->
<script module lang="ts">
  export const control: { set: (next: Record<string, unknown>) => void } = { set: () => {} };
</script>

<script lang="ts">
  import { untrack } from "svelte";
  import type { Component } from "svelte";

  // Each fixture has props of its own.
  let { component: Shown, initial }: { component: Component<any>; initial: Record<string, unknown> } = $props();
  // The props start as given, later only `control.set` changes them.
  const values: Record<string, unknown> = $state(untrack(() => ({ ...initial })));
  control.set = (next) => Object.assign(values, next);
</script>

<Shown {...values} />
