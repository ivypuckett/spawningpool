<script lang="ts">
  import type { EntityKind } from "../lib/types";
  import { selection } from "../lib/stores";

  let { kind, names }: { kind: EntityKind; names: string[] } = $props();

  let filter = $state("");

  let filtered = $derived(
    filter.trim() === ""
      ? names
      : names.filter((n) => n.toLowerCase().includes(filter.toLowerCase()))
  );
</script>

<div class="entity-list">
  <input
    type="text"
    placeholder="Filter…"
    aria-label={`Filter ${kind}s`}
    bind:value={filter}
  />
  {#if names.length === 0}
    <p class="empty">No {kind}s yet</p>
  {:else if filtered.length === 0}
    <p class="empty">No results</p>
  {:else}
    <ul>
      {#each filtered as name}
        <li>
          <button
            class="row"
            aria-current={$selection?.kind === kind && $selection?.name === name ? "true" : undefined}
            onclick={() => selection.set({ kind, name })}
          >
            {name}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .entity-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.5rem;
    height: 100%;
    overflow: hidden;
  }

  input {
    width: 100%;
    padding: 0.25rem 0.5rem;
    box-sizing: border-box;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    flex: 1;
  }

  .row {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    padding: 0.25rem 0.5rem;
    cursor: pointer;
    border-radius: 3px;
  }

  .row:hover {
    background: rgba(0, 0, 0, 0.08);
  }

  .row[aria-current="true"] {
    background: rgba(0, 100, 255, 0.15);
    font-weight: bold;
  }

  .empty {
    color: #888;
    font-style: italic;
    padding: 0.5rem;
  }
</style>
