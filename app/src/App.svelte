<script lang="ts">
  import { onMount } from "svelte";
  import Shell from "./components/Shell.svelte";
  import { loadRegistry, watchRegistry } from "./lib/stores";

  onMount(() => {
    loadRegistry().catch(console.error);

    let cleanup: (() => void) | null = null;
    let disposed = false;
    watchRegistry()
      .then((stop) => {
        if (disposed) stop();
        else cleanup = stop;
      })
      .catch(console.error);

    return () => {
      disposed = true;
      cleanup?.();
    };
  });
</script>

<Shell />
