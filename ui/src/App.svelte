<script lang="ts">
  import { onMount } from "svelte";
  import { tauriApi } from "./lib/api";
  import PropertiesPanel from "./lib/components/PropertiesPanel.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import VaultManager from "./lib/components/VaultManager.svelte";
  import { TreeStore } from "./lib/store.svelte";
  import type { Api } from "./lib/types";
  import { VaultStore } from "./lib/vault.svelte";

  let { api = tauriApi }: { api?: Api } = $props();

  // The stores are created once; swapping the api prop afterwards is not supported.
  // svelte-ignore state_referenced_locally
  const store = new TreeStore(api);
  // svelte-ignore state_referenced_locally
  const vault = new VaultStore(api, (message) => store.showError(message));

  let view = $state<"empty" | "vault">("empty");

  // the manager only exists while the vault is unlocked
  $effect(() => {
    if (view === "vault" && vault.status?.state !== "unlocked") view = "empty";
  });

  onMount(() => {
    void store.init();
    void vault.init();
  });
</script>

<div class="shell">
  <Sidebar {store} {vault} onOpenVault={() => (view = "vault")}>
    <PropertiesPanel {store} />
  </Sidebar>
  <main class="main">
    {#if view === "vault"}
      <VaultManager {vault} tree={store} onclose={() => (view = "empty")} />
    {:else}
      <p class="empty">Open a connection from the tree</p>
    {/if}
  </main>
</div>

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-width) 1fr;
    height: 100%;
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
</style>
