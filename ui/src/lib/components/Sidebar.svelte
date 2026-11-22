<script lang="ts">
  import type { Snippet } from "svelte";
  import type { SessionStore } from "../sessions.svelte";
  import type { TreeStore } from "../store.svelte";
  import type { SyncStore } from "../sync.svelte";
  import type { VaultStore } from "../vault.svelte";
  import { LayoutStore, SIDEBAR_MIN, sidebarMax } from "../layout.svelte";
  import { syncLine } from "../sync-line";
  import FilterBox from "./FilterBox.svelte";
  import Icon from "./Icon.svelte";
  import NoticeBar from "./NoticeBar.svelte";
  import Splitter from "./Splitter.svelte";
  import SyncStatusLine from "./SyncStatus.svelte";
  import TreeView from "./TreeView.svelte";
  import VaultChip from "./VaultChip.svelte";
  import VaultNote from "./VaultNote.svelte";

  let {
    store,
    vault,
    sessions,
    onOpenVault,
    onsettings,
    onimport,
    sync,
    onsyncdetails,
    onreseed,
    restartPending = false,
    onRestart,
    layout = new LayoutStore(),
    children,
  }: {
    store: TreeStore;
    vault: VaultStore;
    sessions: SessionStore;
    /** The chip was clicked while the vault is unlocked. */
    onOpenVault: () => void;
    /** The settings button was clicked. */
    onsettings?: () => void;
    /** The "Import / Export" button was clicked. */
    onimport?: () => void;
    /** Its status line sits above the footer buttons and its findings join the notice bar. */
    sync?: SyncStore;
    /** "Details" or the status line was clicked: open Settings at the Sync section. */
    onsyncdetails?: () => void;
    /** "Re-seed…" of a hub that is older than this device. */
    onreseed?: () => void;
    restartPending?: boolean;
    onRestart?: () => void;
    layout?: LayoutStore;
    children?: Snippet;
  } = $props();

  let filterBox: FilterBox | undefined = $state();

  /** Focuses the tree filter (the Ctrl+Shift+K shortcut). */
  export function focusFilter() {
    filterBox?.focus();
  }

  /** Opens and focuses the vault form of the note (create or unlock); the Credential row calls it. */
  export function focusVaultForm() {
    void note?.focusForm();
  }

  let note: VaultNote | undefined = $state();
  let noteOpen = $state(false);

  // Any bar above the tree or the sync line below the panel takes height the 23-row minimum has
  // no room for, so the tree may shrink to 6 rows and the properties panel stays reachable.
  const barsOpen = $derived(
    store.loadError !== null ||
      store.error !== null ||
      store.changedOnDisk ||
      store.notices.length > 0 ||
      restartPending ||
      (sync?.status?.findings.length ?? 0) > 0 ||
      syncLine(sync?.status ?? null) !== null,
  );
</script>

<aside
  class="sidebar"
  class:note-open={noteOpen}
  class:bars-open={barsOpen}
  class:panel-sized={layout.propertiesHeight !== null}
  aria-label="Connections"
>
  <div class="head">
    <div class="filter"><FilterBox bind:this={filterBox} {store} /></div>
    <VaultChip {vault} onOpenManager={onOpenVault} onFocusUnlock={() => void note?.focusForm()} />
  </div>
  <VaultNote bind:this={note} bind:open={noteOpen} {vault} />
  <NoticeBar {store} {sync} {restartPending} {onRestart} />
  <div class="tree-slot"><TreeView {store} {sessions} {onimport} /></div>
  {@render children?.()}
  {#if sync}
    <SyncStatusLine {sync} {onreseed} ondetails={onsyncdetails} />
  {/if}
  <div class="foot">
    <button
      type="button"
      class="foot-btn"
      aria-label="Settings"
      title="Settings"
      onclick={onsettings}
    >
      <Icon name="gear" /><span>Settings</span>
    </button>
    <button
      type="button"
      class="foot-btn"
      aria-label="Import / Export"
      title="Import, export & backup"
      onclick={onimport}
    >
      <Icon name="import" /><span>Import / Export</span>
    </button>
  </div>
  <Splitter
    orientation="vertical"
    label="Resize sidebar"
    value={layout.effectiveSidebarWidth}
    min={SIDEBAR_MIN}
    max={sidebarMax(layout.viewportWidth)}
    onchange={(px) => layout.setSidebarWidth(px)}
    onreset={() => layout.resetSidebarWidth()}
  />
</aside>

<style>
  .sidebar {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--color-sidebar);
    border-right: 1px solid var(--color-border);
  }

  .head {
    display: flex;
    gap: var(--space-3);
    padding: var(--space-3);
  }

  .filter {
    flex: 1;
    min-width: 0;
  }

  .foot {
    display: flex;
    flex: none;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-top: 1px solid var(--color-border);
  }

  .foot-btn {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    height: 28px;
    padding: 0 var(--space-3);
    font-size: var(--text-sm);
    white-space: nowrap;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-muted);
    cursor: default;
  }

  .foot-btn:hover {
    background: var(--color-hover);
    color: var(--color-text);
  }

  /* 23 rows stay visible when the window allows it; the properties panel gives way first */
  .tree-slot {
    display: flex;
    flex: 1 1 0;
    flex-direction: column;
    min-height: min(calc(var(--row-height-tree) * 23), calc(100vh - 120px));
    overflow-y: auto;
  }

  /* the vault form is open: it is taller than the compact note, so the tree may shrink; the same
     when the user has set the properties panel's height (TREE_ROWS_SIZED in layout.svelte.ts)
     and while a notice bar or the sync line is shown */
  .note-open .tree-slot,
  .bars-open .tree-slot,
  .panel-sized .tree-slot {
    min-height: calc(var(--row-height-tree) * 6);
  }
</style>
