<script lang="ts">
  import { onMount } from "svelte";
  import Shell from "./components/Shell.svelte";
  import { loadRegistry } from "./lib/stores";

  onMount(() => {
    loadRegistry().catch(console.error);
  });
</script>

<Shell />
