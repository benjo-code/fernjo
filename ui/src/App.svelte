<script lang="ts">
  import { onDestroy, onMount, untrack } from "svelte";
  import { tauriApi, toApiError } from "./lib/api";
  import ConfirmDialog from "./lib/components/ConfirmDialog.svelte";
  import SettingsDialog from "./lib/components/SettingsDialog.svelte";
  import ImportDialog from "./lib/components/ImportDialog.svelte";
  import PropertiesPanel from "./lib/components/PropertiesPanel.svelte";
  import SessionArea from "./lib/components/SessionArea.svelte";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import TabStrip from "./lib/components/TabStrip.svelte";
  import VaultManager from "./lib/components/VaultManager.svelte";
  import { startActivityHeartbeat } from "./lib/activity";
  import { ImportStore } from "./lib/import.svelte";
  import { OpenTabsStore } from "./lib/open-tabs.svelte";
  import { LayoutStore, SIDEBAR_DEFAULT } from "./lib/layout.svelte";
  import { findNode } from "./lib/rows";
  import { SessionStore } from "./lib/sessions.svelte";
  import { isMac, routeKey } from "./lib/shortcuts";
  import { tabToClose } from "./lib/tabs";
  import { TreeStore } from "./lib/store.svelte";
  import { SyncStore } from "./lib/sync.svelte";
  import { UpdateStore } from "./lib/update.svelte";
  import UpdateDialog from "./lib/components/UpdateDialog.svelte";
  import { syncErrorText } from "./lib/sync-errors";
  import { RESEED_CONFIRM } from "./lib/sync-line";
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
  const terminal = new TerminalSettingsStore(api);

  // svelte-ignore state_referenced_locally
  const sessions = new SessionStore(
    api,
    (id) => findNode(store.children, id)?.name ?? "Session",
    () => (terminal.info ?? DEFAULT_TERMINAL).closeOnExit,
    protocolOf,
  );

  // svelte-ignore state_referenced_locally
  const openTabs = new OpenTabsStore(api, sessions);

  // svelte-ignore state_referenced_locally
  const theme = new ThemeStore(api);

  // A run that wrote remote changes locally: fetch the tree and the vault status again. Reload
  // goes through the same path as the "changed on disk" bar.
  // svelte-ignore state_referenced_locally
  const sync = new SyncStore(
    api,
    () => {
      void store.refresh();
      void vault.init();
    },
    () => {
      // a join ended every session in Rust without an event: drop the tabs, or they would stay
      // "connected" over a frozen terminal (the same as after a restore)
      sessions.closeAll();
      void store.refresh();
      void vault.init();
    },
  );

  // svelte-ignore state_referenced_locally
  const update = new UpdateStore(api);
  let updateOpen = $state(false);

  const layout = new LayoutStore();
  // nothing stored: the grid keeps the stylesheet's token, so the default layout is unchanged
  const gridColumns = $derived(
    layout.effectiveSidebarWidth === SIDEBAR_DEFAULT
      ? null
      : `${layout.effectiveSidebarWidth}px 1fr`,
  );

  /** Until Rust answers (or when it cannot), terminals use these. */
  const DEFAULT_TERMINAL: TerminalSettings = {
    fontFamily: null,
    fontSize: 13,
    scrollback: 10000,
    closeOnExit: true,
    persistent: true,
    unsavedReason: null,
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

  // "Reconnect all" asks for the vault at most once. The unlock form is the one in the sidebar;
  // this waits until the vault is unlocked (answers true) or until the question is asked again
  // or the vault turns out to be unusable (answers false).
  let vaultWaiter: ((unlocked: boolean) => void) | null = null;

  function ensureVault(): Promise<boolean> {
    vaultWaiter?.(false);
    vaultWaiter = null;
    const state = vault.status?.state;
    if (state === "unlocked") return Promise.resolve(true);
    if (state !== "locked" && state !== "none") return Promise.resolve(false);
    sidebar?.focusVaultForm();
    return new Promise((resolve) => (vaultWaiter = resolve));
  }

  $effect(() => {
    const state = vault.status?.state;
    if (!vaultWaiter) return;
    if (state === "unlocked") {
      vaultWaiter(true);
      vaultWaiter = null;
    } else if (state === "unusable") {
      vaultWaiter(false);
      vaultWaiter = null;
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
  let settingsAtSync = $state(false);
  let reseedAsk = $state(false);

  function openSettings(atSync: boolean) {
    settingsAtSync = atSync;
    folderOpen = true;
  }

  async function reseed() {
    reseedAsk = false;
    if (!(await sync.reseed()) && sync.error) store.showError(syncErrorText(sync.error).text);
  }
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

  // Asks the manager to start on the new-credential form; a fresh token each time, so it also
  // works while the manager is open. Opening it any other way clears the request.
  let newCredentialToken = $state<number | null>(null);
  let newCredentialCount = 0;

  function openVault() {
    newCredentialToken = null;
    view = "vault";
  }

  function newCredential() {
    newCredentialToken = ++newCredentialCount;
    view = "vault";
  }

  /**
   * Only the app chords are handled here: tab switching, full screen, closing and the filter. Copy,
   * paste and search belong to the terminal controller, which leaves the app chords unhandled so
   * they get here. A remote desktop keeps every key except full screen and the tab switch.
   */
  function onkeydown(e: KeyboardEvent) {
    // a modal (settings, any confirm dialog) owns the keyboard; so does anything that handled the key
    if (e.defaultPrevented || folderOpen || importOpen || sessions.closeRequest) return;
    if (document.querySelector("[aria-modal='true']")) return;
    const target = e.target instanceof Element ? e.target : null;
    const focus = target?.closest(".xterm")
      ? "terminal"
      : target?.closest("[data-remote-desktop]")
        ? "remote"
        : "none";
    const action = routeKey(e, isMac(), focus);
    if (action === "fullscreen") {
      // a held key must not flip it back and forth
      if (!e.repeat) sessions.toggleFullscreen();
    } else if (action === "nextTab" || action === "prevTab") {
      if (sessions.tabs.length === 0) return;
      if (action === "nextTab") sessions.next();
      else sessions.previous();
    } else if (action === "closeTab") {
      const closing = tabToClose(view, sessions.activeId);
      if (closing === null) return;
      // a held key must not close one tab after the other
      if (e.repeat) {
        e.preventDefault();
        return;
      }
      sessions.requestClose(closing);
    } else if (action === "focusFilter") {
      sidebar?.focusFilter();
    } else if (action === "quickConnect") {
      sidebar?.focusQuickConnect();
    } else {
      return;
    }
    e.preventDefault();
  }

  onMount(() => {
    // the saved tabs come back once the tree is there (their names and protocols follow it)
    void store.init().finally(() => void openTabs.load());
    void vault.init();
    void vault.listen();
    void refreshRestart();
    void terminal.load();
    void theme.load();
    void sync.init();
    void update.init();
  });

  // Rust owns the idle clock for the vault auto-lock; this only reports that the user is there.
  onMount(() =>
    startActivityHeartbeat(
      () => void api.noteActivity().catch(() => {}),
      window,
      () => Date.now(),
    ),
  );

  onDestroy(() => {
    openTabs.dispose();
    sync.dispose();
    update.dispose();
    vault.dispose();
  });
</script>

<svelte:window
  onresize={() => (layout.viewportWidth = window.innerWidth)}
  onpagehide={endSessions}
  onbeforeunload={endSessions}
  {onkeydown}
/>

<div
  class="shell"
  class:fullscreen={sessions.remoteFullscreen}
  style:grid-template-columns={sessions.remoteFullscreen ? "1fr" : gridColumns}
>
  <Sidebar
    bind:this={sidebar}
    {store}
    {vault}
    {sessions}
    onOpenVault={openVault}
    onsettings={() => openSettings(false)}
    onimport={() => (importOpen = true)}
    {sync}
    onsyncdetails={() => openSettings(true)}
    onreseed={() => (reseedAsk = true)}
    {update}
    onupdate={() => (updateOpen = true)}
    {restartPending}
    onRestart={() => void restart()}
    {layout}
  >
    <PropertiesPanel
      {store}
      {vault}
      {layout}
      onOpenVault={openVault}
      onFocusVaultForm={() => sidebar?.focusVaultForm()}
      onNewCredential={newCredential}
    />
  </Sidebar>
  <main class="main">
    {#if sessions.tabs.length > 0 && !sessions.remoteFullscreen}
      <TabStrip {sessions} {protocolOf} onreconnectall={() => sessions.reconnectAll(ensureVault)} />
    {/if}
    <div class="content">
      {#if view === "vault"}
        <VaultManager
          {vault}
          tree={store}
          newToken={newCredentialToken}
          onclose={() => (view = "empty")}
        />
      {/if}
      {#if sessions.tabs.length > 0}
        <SessionArea {sessions} {vault} settings={terminalSettings} hidden={view === "vault"} />
      {:else if view !== "vault"}
        {#if store.loadError || !store.loaded}
          <!-- nothing to say until the tree is there (the notice bar explains a load failure) -->
        {:else if store.children.length === 0}
          <ol class="start">
            <li>
              <span>Add a connection</span>
              <div class="actions">
                <button
                  type="button"
                  class="btn"
                  onclick={() => void store.createConnection(null, null)}
                >
                  New connection
                </button>
                <button type="button" class="btn" onclick={() => (importOpen = true)}>
                  Import from mRemoteNG…
                </button>
              </div>
            </li>
            {#if vault.status?.state === "none"}
              <li>
                <span>Optional: create a vault to store logins</span>
                <div class="actions">
                  <button type="button" class="btn" onclick={() => sidebar?.focusVaultForm()}>
                    Create vault
                  </button>
                </div>
              </li>
            {/if}
          </ol>
        {:else}
          <p class="empty">Double-click a connection (or press Enter) to open it.</p>
        {/if}
      {/if}
    </div>
  </main>
</div>

{#if folderOpen}
  <SettingsDialog
    {api}
    {terminal}
    {theme}
    {sync}
    {openTabs}
    {update}
    onshowupdate={() => {
      folderOpen = false;
      updateOpen = true;
    }}
    scrollToSync={settingsAtSync}
    onexport={() => (importOpen = true)}
    onclose={() => (folderOpen = false)}
    onchange={(info) => (restartPending = info.restartRequired)}
  />
{/if}

{#if updateOpen}
  <UpdateDialog {update} onclose={() => (updateOpen = false)} />
{/if}

{#if reseedAsk}
  <ConfirmDialog
    title={RESEED_CONFIRM.title}
    detail={RESEED_CONFIRM.detail}
    confirmLabel={RESEED_CONFIRM.confirmLabel}
    onConfirm={() => void reseed()}
    onCancel={() => (reseedAsk = false)}
  />
{/if}

{#if importOpen}
  <ImportDialog store={importer} {vault} tree={store} {sync} onclose={() => (importOpen = false)} />
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: var(--sidebar-width) 1fr;
    height: 100%;
  }

  /* Full screen hides the sidebar without unmounting it, so the tree keeps its scroll position */
  .shell.fullscreen > :global(:first-child) {
    display: none;
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

  .start {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    margin: auto;
    padding: 0 0 0 var(--space-6);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .start li {
    padding-left: var(--space-2);
  }

  .start li > span {
    display: block;
    margin-bottom: var(--space-3);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
</style>
