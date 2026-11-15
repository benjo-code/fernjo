<script lang="ts">
  import { tick } from "svelte";
  import { countDescendants, type Row } from "../rows";
  import type { TreeStore } from "../store.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import TreeRow from "./TreeRow.svelte";

  let { store }: { store: TreeStore } = $props();

  let container = $state<HTMLElement>();
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let pendingDelete = $state<{ id: string; title: string } | null>(null);

  const rows = $derived(store.rows);
  const tabbableId = $derived(
    rows.find((r) => r.node.id === store.selectedId)?.node.id ?? rows[0]?.node.id ?? null,
  );
  const query = $derived(store.filter.trim());

  function rowElement(id: string): HTMLElement | undefined {
    return [...(container?.querySelectorAll<HTMLElement>("[data-node-id]") ?? [])].find(
      (el) => el.dataset.nodeId === id,
    );
  }

  async function selectAndFocus(id: string) {
    store.select(id);
    await tick();
    rowElement(id)?.focus();
  }

  /** New nodes go inside a folder (at the end) and right after a connection. */
  function placement(row: Row): { parentId: string | null; index: number | null } {
    return row.node.type === "folder"
      ? { parentId: row.node.id, index: null }
      : { parentId: row.parentId, index: row.index + 1 };
  }

  function menuItemsFor(row: Row | null): MenuItem[] {
    if (!row) {
      return [
        { label: "New folder", action: () => void store.createFolder(null, null) },
        { label: "New connection", action: () => void store.createConnection(null, null) },
      ];
    }
    const { parentId, index } = placement(row);
    return [
      { label: "New folder", action: () => void store.createFolder(parentId, index) },
      { label: "New connection", action: () => void store.createConnection(parentId, index) },
      { label: "Duplicate", action: () => void store.duplicate(row.node.id) },
      { label: "Rename", action: () => store.startRename(row.node.id) },
      { label: "Delete", action: () => requestDelete(row) },
    ];
  }

  function requestDelete(row: Row) {
    const { name } = row.node;
    const count = countDescendants(row.node);
    const inside = count > 0 ? ` and ${count} ${count === 1 ? "item" : "items"} inside` : "";
    pendingDelete = { id: row.node.id, title: `Delete “${name}”${inside}?` };
  }

  function confirmDelete() {
    if (!pendingDelete) return;
    const { id } = pendingDelete;
    pendingDelete = null;
    void store.remove(id);
  }

  function oncontextmenu(e: MouseEvent) {
    e.preventDefault();
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-node-id]");
    const row = el ? rows.find((r) => r.node.id === el.dataset.nodeId) : undefined;
    if (row) store.select(row.node.id);
    menu = { x: e.clientX, y: e.clientY, items: menuItemsFor(row ?? null) };
  }

  function openMenuForSelection(current: Row) {
    const rect = rowElement(current.node.id)?.getBoundingClientRect();
    menu = {
      x: (rect?.left ?? 0) + 24,
      y: rect?.bottom ?? 0,
      items: menuItemsFor(current),
    };
  }

  function onclick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    const el = target.closest<HTMLElement>("[data-node-id]");
    if (!el || !container?.contains(el)) return;
    const id = el.dataset.nodeId!;
    if (target.closest(".chevron")) {
      store.toggle(id);
      return;
    }
    store.select(id);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement || e.altKey || e.ctrlKey || e.metaKey) return;
    if (rows.length === 0) return;
    if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) {
      const selected = rows.find((r) => r.node.id === store.selectedId);
      if (selected) {
        e.preventDefault();
        openMenuForSelection(selected);
      }
      return;
    }
    const index = rows.findIndex((r) => r.node.id === store.selectedId);
    const current = index >= 0 ? rows[index] : null;
    const go = (to: number) => {
      e.preventDefault();
      void selectAndFocus(rows[Math.max(0, Math.min(rows.length - 1, to))].node.id);
    };
    switch (e.key) {
      case "ArrowDown":
        go(index < 0 ? 0 : index + 1);
        break;
      case "ArrowUp":
        go(index < 0 ? 0 : index - 1);
        break;
      case "Home":
        go(0);
        break;
      case "End":
        go(rows.length - 1);
        break;
      case "ArrowRight":
        if (current?.node.type === "folder" && current.hasChildren) {
          e.preventDefault();
          if (!current.expanded) store.expand(current.node.id);
          else go(index + 1);
        }
        break;
      case "ArrowLeft":
        if (!current) break;
        e.preventDefault();
        if (current.node.type === "folder" && current.expanded && !query) {
          store.toggle(current.node.id);
        } else if (current.parentId !== null) {
          void selectAndFocus(current.parentId);
        }
        break;
      case "Delete":
        if (current) {
          e.preventDefault();
          requestDelete(current);
        }
        break;
      case "F2":
        if (current) {
          e.preventDefault();
          store.startRename(current.node.id);
        }
        break;
    }
  }
</script>

{#if store.loadError || !store.loaded}
  <!-- nothing to show yet, or the data could not be opened (the notice bar says why) -->
{:else if store.children.length === 0}
  <div class="empty">
    <p>No connections yet</p>
    <div class="actions">
      <button type="button" class="btn" onclick={() => store.createFolder(null, null)}>
        New folder
      </button>
      <button type="button" class="btn" onclick={() => store.createConnection(null, null)}>
        New connection
      </button>
    </div>
  </div>
{:else if rows.length === 0}
  <p class="empty">No matches for “{query}”</p>
{:else}
  <div
    bind:this={container}
    class="tree"
    role="tree"
    aria-label="Connections"
    tabindex="-1"
    {onclick}
    {oncontextmenu}
    {onkeydown}
  >
    {#each rows as row (row.node.id)}
      <TreeRow
        {row}
        selected={row.node.id === store.selectedId}
        tabbable={row.node.id === tabbableId}
        renaming={row.node.id === store.renamingId}
        onrename={(value) => {
          if (value.trim() === row.node.name) store.stopRename();
          else void store.rename(row.node.id, value);
        }}
        oncancel={() => store.stopRename()}
      />
    {/each}
  </div>
{/if}

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => (menu = null)} />
{/if}

{#if pendingDelete}
  <ConfirmDialog
    title={pendingDelete.title}
    detail="This cannot be undone."
    confirmLabel="Delete"
    onConfirm={confirmDelete}
    onCancel={() => (pendingDelete = null)}
  />
{/if}

<style>
  .tree {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    outline: none;
  }

  .empty {
    margin: 0;
    padding: var(--space-5) var(--space-3);
    color: var(--color-text-muted);
    font-size: var(--text-sm);
  }

  .empty p {
    margin: 0 0 var(--space-3);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
</style>
