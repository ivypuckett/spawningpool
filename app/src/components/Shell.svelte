<script lang="ts">
  import type { EntityKind } from "../lib/types";
  import { registry } from "../lib/stores";
  import EntityList from "./EntityList.svelte";
  import EntityDetail from "./EntityDetail.svelte";

  let activeKind = $state<EntityKind>("provider");

  const kinds: { kind: EntityKind; label: string }[] = [
    { kind: "provider", label: "Providers" },
    { kind: "model", label: "Models" },
    { kind: "specialist", label: "Specialists" },
    { kind: "tool", label: "Tools" },
  ];

  function countFor(kind: EntityKind): number {
    if ($registry === null) return 0;
    switch (kind) {
      case "provider": return $registry.providers.length;
      case "model": return $registry.models.length;
      case "specialist": return $registry.specialists.length;
      case "tool": return $registry.tools.length;
    }
  }

  function namesFor(kind: EntityKind): string[] {
    if ($registry === null) return [];
    switch (kind) {
      case "provider": return $registry.providers;
      case "model": return $registry.models;
      case "specialist": return $registry.specialists;
      case "tool": return $registry.tools;
    }
  }
</script>

<div class="shell">
  <nav class="left-rail">
    {#each kinds as { kind, label }}
      <button
        class="kind-btn"
        aria-current={activeKind === kind ? "page" : undefined}
        onclick={() => { activeKind = kind; }}
      >
        {label}
        <span class="count">{countFor(kind)}</span>
      </button>
    {/each}
    <footer class="registry-path">
      {#if $registry}
        {$registry.registry_path}
      {:else}
        &mdash;
      {/if}
    </footer>
  </nav>

  <main class="middle-pane">
    <EntityList kind={activeKind} names={namesFor(activeKind)} />
  </main>

  <aside class="right-pane">
    <EntityDetail />
  </aside>
</div>

<style>
  .shell {
    display: flex;
    height: 100vh;
    overflow: hidden;
    font-family: sans-serif;
  }

  .left-rail {
    width: 160px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid #ddd;
    padding: 0.5rem 0;
  }

  .kind-btn {
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: none;
    border: none;
    padding: 0.5rem 0.75rem;
    cursor: pointer;
    text-align: left;
    font-size: 0.9rem;
  }

  .kind-btn:hover {
    background: rgba(0, 0, 0, 0.06);
  }

  .kind-btn[aria-current="page"] {
    background: rgba(0, 100, 255, 0.12);
    font-weight: bold;
  }

  .count {
    font-size: 0.75rem;
    color: #888;
    background: rgba(0, 0, 0, 0.08);
    border-radius: 9px;
    padding: 0 0.4rem;
    min-width: 1.2em;
    text-align: center;
  }

  .registry-path {
    margin-top: auto;
    padding: 0.5rem 0.75rem;
    font-size: 0.65rem;
    color: #aaa;
    word-break: break-all;
    border-top: 1px solid #eee;
  }

  .middle-pane {
    width: 240px;
    flex-shrink: 0;
    border-right: 1px solid #ddd;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .right-pane {
    flex: 1;
    overflow: hidden;
  }
</style>
