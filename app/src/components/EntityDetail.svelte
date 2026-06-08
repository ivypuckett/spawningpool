<script lang="ts">
  import { selection } from "../lib/stores";
  import { showEntity } from "../lib/api";

  let definition = $state<unknown>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);

  $effect(() => {
    const sel = $selection;
    if (!sel) {
      definition = null;
      error = null;
      loading = false;
      return;
    }
    let cancelled = false;
    loading = true;
    error = null;
    definition = null;
    showEntity(sel.kind, sel.name)
      .then((value) => {
        if (!cancelled) {
          definition = value;
          loading = false;
        }
      })
      .catch((e) => {
        if (!cancelled) {
          error = String(e);
          loading = false;
        }
      });
    return () => {
      cancelled = true;
    };
  });
</script>

<div class="entity-detail">
  {#if $selection === null}
    <p class="placeholder">Select an item to see its definition</p>
  {:else}
    <h2>{$selection.kind}: {$selection.name}</h2>
    {#if loading}
      <p class="loading">Loading…</p>
    {:else if error !== null}
      <p class="error">{error}</p>
    {:else if definition !== null}
      <pre>{JSON.stringify(definition, null, 2)}</pre>
    {/if}
  {/if}
</div>

<style>
  .entity-detail {
    padding: 1rem;
    height: 100%;
    overflow-y: auto;
  }

  .placeholder {
    color: #888;
    font-style: italic;
  }

  .loading {
    color: #888;
    font-style: italic;
  }

  .error {
    color: #c00;
  }

  h2 {
    margin-top: 0;
    font-size: 1.1rem;
  }

  pre {
    font-size: 0.85rem;
    white-space: pre-wrap;
    word-break: break-all;
  }
</style>
