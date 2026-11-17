<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { tauriApi, toApiError } from "./lib/api";
  import SettingsDialog from "./lib/components/SettingsDialog.svelte";
  import ImportDialog from "./lib/components/ImportDialog.svelte";
  import PropertiesPanel from "./lib/components/PropertiesPanel.svelte";
  import SessionArea from "./lib/components/SessionArea.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import TabStrip from "./lib/components/TabStrip.svelte";
  import VaultManager from "./lib/components/VaultManager.svelte";
  import { ImportStore } from "./lib/import.svelte";
  import { findNode } from "./lib/rows";
  import { SessionStore } from "./lib/sessions.svelte";
  import { isMac, routeKey } from "./lib/shortcuts";
  import { TreeStore } from "./lib/store.svelte";
  import { TerminalSettingsStore } from "./lib/terminal-settings.svelte";
  import { ThemeStore } from "./lib/theme.svelte";
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

  // svelte-ignore state_referenced_locally
  const terminal = new TerminalSettingsStore(api);

  // svelte-ignore state_referenced_locally
  const theme = new ThemeStore(api);

  /** Until Rust answers (or when it cannot), terminals use these. */
  const DEFAULT_TERMINAL: TerminalSettings = {
    fontFamily: null,
    fontSize: 13,
    scrollback: 10000,
    persistent: true,
  };
  const terminalSettings = $derived(terminal.info ?? DEFAULT_TERMINAL);

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

  // svelte-ignore state_referenced_locally
  const importer = new ImportStore(api, (snapshot, restored) => {
    // a restore ended every session in Rust without an event: drop the tabs, or they would
    // stay "connected" over a frozen terminal
    if (restored) sessions.closeAll();
    void store.applySnapshot(snapshot);
    // an import adds credentials and a restore replaces the vault: reload its status and list
    void vault.init();
  });

  let folderOpen = $state(false);
  let importOpen = $state(false);
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

  // Opening or activating a tab brings the session area back; opening the manager or closing a
  // tab does not count. `view` is read untracked so the effect only reacts to the counter.
  $effect(() => {
    void sessions.revealed;
    untrack(() => {
      if (view === "vault") view = "empty";
    });
  });

  let sidebar: Sidebar | undefined = $state();

  /**
   * Only the app chords are handled here: tab switching, closing and the filter. Copy, paste and
   * search belong to the terminal controller, which leaves the app chords unhandled so they get here.
   */
  function onkeydown(e: KeyboardEvent) {
    // a modal (settings, any confirm dialog) owns the keyboard; so does anything that handled the key
    if (e.defaultPrevented || folderOpen || importOpen || sessions.closeRequest) return;
    if (document.querySelector("[aria-modal='true']")) return;
    const inTerminal = e.target instanceof Element && e.target.closest(".xterm") !== null;
    const action = routeKey(e, isMac(), inTerminal);
    if (action === "nextTab" || action === "prevTab") {
      if (sessions.tabs.length === 0) return;
      if (action === "nextTab") sessions.next();
      else sessions.previous();
    } else if (action === "closeTab") {
      if (sessions.activeId === null) return;
      // a held key must not close one tab after the other
      if (e.repeat) {
        e.preventDefault();
        return;
      }
      sessions.requestClose(sessions.activeId);
    } else if (action === "focusFilter") {
      sidebar?.focusFilter();
    } else {
      return;
    }
    e.preventDefault();
  }

  onMount(() => {
    void store.init();
    void vault.init();
    void refreshRestart();
    void terminal.load();
    void theme.load();
  });
</script>

<svelte:window onpagehide={endSessions} onbeforeunload={endSessions} {onkeydown} />

<div class="shell">
  <Sidebar
    bind:this={sidebar}
    {store}
    {vault}
    {sessions}
    onOpenVault={() => (view = "vault")}
    onsettings={() => (folderOpen = true)}
    onimport={() => (importOpen = true)}
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
  <SettingsDialog
    {api}
    {terminal}
    {theme}
    onclose={() => (folderOpen = false)}
    onchange={(info) => (restartPending = info.restartRequired)}
  />
{/if}

{#if importOpen}
  <ImportDialog store={importer} {vault} tree={store} onclose={() => (importOpen = false)} />
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
