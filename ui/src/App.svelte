<script lang="ts">
  import { onMount } from "svelte";
  import { tauriApi } from "./lib/api";
  import { TreeStore } from "./lib/store.svelte";
  import type { Api } from "./lib/types";

  let { api = tauriApi }: { api?: Api } = $props();

  // The store is created once; swapping the api prop afterwards is not supported.
  // svelte-ignore state_referenced_locally
  const store = new TreeStore(api);

  onMount(() => {
    void store.init();
  });
</script>

<div class="shell">
  <aside class="sidebar" aria-label="Connections">
    <!-- filter, notice, tree and properties panel are added by the tree view tasks -->
    {#if store.loadError}
      <p class="load-error" role="alert">{store.loadError}</p>
    {/if}
  </aside>
  <main class="main">
    <p class="empty">Open a connection from the tree</p>
  </main>
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-width) 1fr;
    height: 100%;
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
  }

  .main {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 0;
    background: var(--color-bg);
  }

  .empty {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .load-error {
    margin: var(--space-3);
    color: var(--color-danger);
    font-size: var(--text-sm);
  }
</style>
