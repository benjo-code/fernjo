<script lang="ts">
  import type { Snippet } from "svelte";
  import type { SessionStore } from "../sessions.svelte";
  import type { TreeStore } from "../store.svelte";
  import type { VaultStore } from "../vault.svelte";
  import FilterBox from "./FilterBox.svelte";
  import Icon from "./Icon.svelte";
  import NoticeBar from "./NoticeBar.svelte";
  import TreeView from "./TreeView.svelte";
  import VaultChip from "./VaultChip.svelte";
  import VaultNote from "./VaultNote.svelte";

  let {
    store,
    vault,
    sessions,
    onOpenVault,
    onsettings,
    restartPending = false,
    onRestart,
    children,
  }: {
    store: TreeStore;
    vault: VaultStore;
    sessions: SessionStore;
    /** The chip was clicked while the vault is unlocked. */
    onOpenVault: () => void;
    /** The settings (data folder) button was clicked. */
    onsettings?: () => void;
    restartPending?: boolean;
    onRestart?: () => void;
    children?: Snippet;
  } = $props();

  let note: VaultNote | undefined = $state();
  let noteOpen = $state(false);
</script>

<aside class="sidebar" class:note-open={noteOpen} aria-label="Connections">
  <div class="head">
    <div class="filter"><FilterBox {store} /></div>
    <VaultChip {vault} onOpenManager={onOpenVault} onFocusUnlock={() => void note?.focusForm()} />
  </div>
  <VaultNote bind:this={note} bind:open={noteOpen} {vault} />
  <NoticeBar {store} {restartPending} {onRestart} />
  <div class="tree-slot"><TreeView {store} {sessions} /></div>
  {@render children?.()}
  <div class="foot">
    <button
      type="button"
      class="gear"
      aria-label="Settings: data folder"
      title="Settings: data folder"
      onclick={onsettings}
    >
      <Icon name="gear" />
    </button>
  </div>
</aside>

<style>
  .sidebar {
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
    padding: var(--space-2) var(--space-3);
    border-top: 1px solid var(--color-border);
  }

  .gear {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-muted);
    cursor: default;
  }

  .gear:hover {
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
