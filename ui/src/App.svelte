<script lang="ts">
  import { onMount } from "svelte";
  import { tauriApi, toApiError } from "./lib/api";
  import DataFolderDialog from "./lib/components/DataFolderDialog.svelte";
  import PropertiesPanel from "./lib/components/PropertiesPanel.svelte";
  import SessionArea from "./lib/components/SessionArea.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import TabStrip from "./lib/components/TabStrip.svelte";
  import VaultManager from "./lib/components/VaultManager.svelte";
  import { findNode } from "./lib/rows";
  import { SessionStore } from "./lib/sessions.svelte";
  import { TreeStore } from "./lib/store.svelte";
  import type { Api, Protocol, TerminalSettings } from "./lib/types";
  import { VaultStore } from "./lib/vault.svelte";

  let { api = tauriApi }: { api?: Api } = $props();

  // The stores are created once; swapping the api prop afterwards is not supported.
  // svelte-ignore state_referenced_locally
  const store = new TreeStore(api);
  // svelte-ignore state_referenced_locally
  const vault = new VaultStore(api, (message) => store.showError(message));

  // svelte-ignore state_referenced_locally
  const sessions = new SessionStore(api, (id) => findNode(store.children, id)?.name ?? "Session");

  /** Until Rust answers (or when it cannot), terminals use these. */
  let terminalSettings = $state<TerminalSettings>({
    fontFamily: null,
    fontSize: 13,
    scrollback: 10000,
    persistent: true,
  });

  async function loadTerminalSettings() {
    try {
      terminalSettings = await api.getTerminalSettings();
    } catch {
      // the defaults above stay in effect
    }
  }

  function protocolOf(nodeId: string): Protocol {
    const node = findNode(store.children, nodeId);
    return node?.type === "connection" ? node.protocol : "ssh";
  }

  // a session that could not start reports through the global error bar
  $effect(() => {
    if (sessions.error !== null) {
      store.showError(sessions.error);
      sessions.error = null;
    }
  });

  /** A reload or closing window must not leave sessions running in Rust without a UI. */
  function endSessions() {
    sessions.closeAll();
  }

  let folderOpen = $state(false);
  let restartPending = $state(false);

  async function refreshRestart() {
    try {
      restartPending = (await api.getDataFolder()).restartRequired;
    } catch {
      // the dialog reports data folder problems itself
    }
  }

  async function restart() {
    try {
      await api.restartApp();
    } catch (e) {
      store.showError(toApiError(e).message);
    }
  }

  let view = $state<"empty" | "vault">("empty");

  // the manager only exists while the vault is unlocked
  $effect(() => {
    if (view === "vault" && vault.status?.state !== "unlocked") view = "empty";
  });

  onMount(() => {
    void store.init();
    void vault.init();
    void refreshRestart();
    void loadTerminalSettings();
  });
</script>

<svelte:window onpagehide={endSessions} onbeforeunload={endSessions} />

<div class="shell">
  <Sidebar
    {store}
    {vault}
    {sessions}
    onOpenVault={() => (view = "vault")}
    onsettings={() => (folderOpen = true)}
    {restartPending}
    onRestart={() => void restart()}
  >
    <PropertiesPanel {store} {vault} />
  </Sidebar>
  <main class="main">
    {#if sessions.tabs.length > 0}
      <TabStrip {sessions} {protocolOf} />
    {/if}
    <div class="content">
      {#if view === "vault"}
        <VaultManager {vault} tree={store} onclose={() => (view = "empty")} />
      {/if}
      {#if sessions.tabs.length > 0}
        <SessionArea {sessions} {vault} settings={terminalSettings} hidden={view === "vault"} />
      {:else if view !== "vault"}
        <p class="empty">Open a connection from the tree</p>
      {/if}
    </div>
  </main>
</div>

{#if folderOpen}
  <DataFolderDialog
    {api}
    onclose={() => (folderOpen = false)}
    onchange={(info) => (restartPending = info.restartRequired)}
  />
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-width) 1fr;
    height: 100%;
  }

  .main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--color-bg);
  }

  .content {
    display: flex;
    flex: 1 1 0;
    min-height: 0;
    min-width: 0;
  }

  .empty {
    margin: auto;
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }
</style>
