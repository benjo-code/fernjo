<script lang="ts">
  import type { Snippet } from "svelte";
  import type { SessionStore } from "../sessions.svelte";
  import type { TreeStore } from "../store.svelte";
  import type { VaultStore } from "../vault.svelte";
  import { LayoutStore, SIDEBAR_MIN, sidebarMax } from "../layout.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Icon from "./Icon.svelte";
  import NoticeBar from "./NoticeBar.svelte";
  import Splitter from "./Splitter.svelte";
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
</script>

<aside class="sidebar" class:note-open={noteOpen} aria-label="Connections">
  <div class="head">
    <div class="filter"><FilterBox bind:this={filterBox} {store} /></div>
    <VaultChip {vault} onOpenManager={onOpenVault} onFocusUnlock={() => void note?.focusForm()} />
  </div>
  <VaultNote bind:this={note} bind:open={noteOpen} {vault} />
  <NoticeBar {store} {restartPending} {onRestart} />
  <div class="tree-slot"><TreeView {store} {sessions} {onimport} /></div>
  {@render children?.()}
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

  /* the vault form is open: it is taller than the compact note, so the tree may shrink */
  .note-open .tree-slot {
    min-height: calc(var(--row-height-tree) * 6);
  }
</style>
