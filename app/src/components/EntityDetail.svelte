<script lang="ts">
  import { selection } from "../lib/stores";
</script>

<div class="entity-detail">
  {#if $selection === null}
    <p class="placeholder">Select an item to see its definition</p>
  {:else}
    <h2>{$selection.kind}: {$selection.name}</h2>
    <p class="coming-soon">Definition coming soon</p>
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

  .coming-soon {
    color: #888;
    font-style: italic;
  }

  h2 {
    margin-top: 0;
    font-size: 1.1rem;
  }
</style>
