<script lang="ts">
  import { tick } from "svelte";
  import { DragController, type DragTarget, type Hit } from "../drag";
  import { dropPosition } from "../drop-target";
  import { countDescendants, type Row } from "../rows";
  import type { SessionStore } from "../sessions.svelte";
  import type { TreeStore } from "../store.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import ContextMenu, { type MenuItem } from "./ContextMenu.svelte";
  import TreeRow from "./TreeRow.svelte";

  let {
    store,
    sessions,
    onimport,
  }: {
    store: TreeStore;
    sessions: SessionStore;
    /** Opens the import dialog from the empty tree. */
    onimport?: () => void;
  } = $props();

  let container = $state<HTMLElement>();
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let pendingDelete = $state<{ id: string; title: string } | null>(null);

  const rows = $derived(store.rows);
  const tabbableId = $derived(
    rows.find((r) => r.node.id === store.selectedId)?.node.id ?? rows[0]?.node.id ?? null,
  );
  const query = $derived(store.filter.trim());

  // --- pointer-based drag and drop ---
  const DRAG_THRESHOLD = 4;
  const EDGE_SCROLL = 24;

  let dragView = $state<{ dragId: string | null; target: DragTarget | null }>({
    dragId: null,
    target: null,
  });
  let justDragged = false;

  const drag = new DragController({
    threshold: DRAG_THRESHOLD,
    rows: () => rows,
    onMove: (id, parentId, index) => void store.move(id, parentId, index),
  });

  const dragEnabled = () => query === "" && store.renamingId === null;

  function syncDrag() {
    const active = drag.state === "dragging";
    dragView = { dragId: active ? drag.dragId : null, target: active ? drag.target : null };
  }

  function hitAt(x: number, y: number): Hit | null {
    const el = document.elementFromPoint?.(x, y) ?? null;
    const rowEl = el?.closest<HTMLElement>("[data-node-id]");
    if (!rowEl || !container?.contains(rowEl)) return null;
    const row = rows.find((r) => r.node.id === rowEl.dataset.nodeId);
    if (!row) return null;
    return {
      rowId: row.node.id,
      position: dropPosition(y, rowEl.getBoundingClientRect(), row.node.type === "folder"),
    };
  }

  function autoScroll(y: number) {
    if (!container) return;
    const rect = container.getBoundingClientRect();
    if (y < rect.top + EDGE_SCROLL) container.scrollTop -= 12;
    else if (y > rect.bottom - EDGE_SCROLL) container.scrollTop += 12;
  }

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) {
      cancelDrag();
      return;
    }
    if (!dragEnabled()) return;
    const target = e.target as HTMLElement;
    if (target instanceof HTMLInputElement || target.closest(".chevron")) return;
    const el = target.closest<HTMLElement>("[data-node-id]");
    if (el) drag.pointerDown(el.dataset.nodeId!, e.clientX, e.clientY);
  }

  function onpointermove(e: PointerEvent) {
    if (drag.state === "idle") return;
    if ((e.buttons & 1) === 0) {
      // the release happened where we could not see it: do not follow a button that is up
      cancelDrag();
      return;
    }
    const wasPending = drag.state === "pending";
    drag.pointerMove(e.clientX, e.clientY, hitAt(e.clientX, e.clientY));
    if (wasPending && drag.state === "dragging") {
      try {
        container?.setPointerCapture(e.pointerId);
      } catch {
        // capture is a convenience (keeps events coming outside the tree)
      }
    }
    if (drag.state === "dragging") autoScroll(e.clientY);
    syncDrag();
  }

  function onpointerup(e: PointerEvent) {
    if (e.button !== 0) {
      cancelDrag();
      return;
    }
    const wasDrag = drag.pointerUp();
    syncDrag();
    try {
      container?.releasePointerCapture(e.pointerId);
    } catch {
      // nothing was captured
    }
    if (wasDrag) {
      // the click that follows a drag must not select anything
      justDragged = true;
      setTimeout(() => (justDragged = false), 0);
    }
  }

  function cancelDrag() {
    drag.cancel();
    syncDrag();
  }

  function onwindowkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && drag.state !== "idle") cancelDrag();
  }

  /** A press that ends outside the tree must not stay armed for a later pointer move. */
  function onwindowpointerup() {
    if (drag.state === "pending") cancelDrag();
  }

  /** Gives the keyboard back to a row when its editor or dialog closed and left no focus anywhere. */
  async function focusRowIfLost(id: string) {
    // a macrotask: after Svelte removed the editor, and after a click elsewhere has moved focus
    await new Promise((resolve) => setTimeout(resolve, 0));
    const active = document.activeElement;
    if (!active || active === document.body) rowElement(id)?.focus();
  }

  /** The row to continue from after deleting `row`: the next sibling, else the one above. */
  function neighbourAfterDelete(row: Row): string | null {
    const at = rows.indexOf(row);
    if (at < 0) return null;
    for (let i = at + 1; i < rows.length && rows[i].depth >= row.depth; i++) {
      if (rows[i].depth === row.depth) return rows[i].node.id;
    }
    return at > 0 ? rows[at - 1].node.id : null;
  }

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

  /** Opens a session for a connection; RDP is not implemented yet and says so. */
  function connect(row: Row, another = false) {
    const { node } = row;
    if (node.type !== "connection") return;
    if (node.protocol === "rdp") {
      store.showError("RDP sessions are not available yet");
      return;
    }
    if (another) sessions.open(node.id, { another: true });
    else sessions.open(node.id);
  }

  function menuItemsFor(row: Row | null): MenuItem[] {
    if (!row) {
      return [
        { label: "New folder", action: () => void store.createFolder(null, null) },
        { label: "New connection", action: () => void store.createConnection(null, null) },
      ];
    }
    const { parentId, index } = placement(row);
    const connectItems: MenuItem[] = [];
    if (row.node.type === "connection") {
      connectItems.push({ label: "Connect", action: () => connect(row) });
      if (sessions.tabs.some((t) => t.nodeId === row.node.id)) {
        connectItems.push({ label: "Open another session", action: () => connect(row, true) });
      }
    }
    return [
      ...connectItems,
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

  async function confirmDelete() {
    if (!pendingDelete) return;
    const { id } = pendingDelete;
    pendingDelete = null;
    const row = rows.find((r) => r.node.id === id);
    const neighbour = row ? neighbourAfterDelete(row) : null;
    await store.remove(id);
    if (neighbour && store.selectedId === null && rowElement(neighbour) !== undefined) {
      store.select(neighbour);
    }
    if (neighbour) void focusRowIfLost(neighbour);
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
    if (justDragged) {
      justDragged = false;
      return;
    }
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

  function ondblclick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    if (target instanceof HTMLInputElement || target.closest(".chevron")) return;
    const el = target.closest<HTMLElement>("[data-node-id]");
    if (!el || !container?.contains(el)) return;
    const row = rows.find((r) => r.node.id === el.dataset.nodeId);
    if (!row) return;
    if (row.node.type === "folder") store.toggle(row.node.id);
    else connect(row);
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
      case "Enter":
        if (current?.node.type === "connection") {
          e.preventDefault();
          connect(current);
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

<svelte:window onkeydown={onwindowkeydown} onpointerup={onwindowpointerup} />

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
      {#if onimport}
        <button type="button" class="btn" onclick={onimport}>Import from mRemoteNG…</button>
      {/if}
    </div>
  </div>
{:else if rows.length === 0}
  <p class="empty">No matches for “{query}”</p>
{:else}
  <div
    bind:this={container}
    class="tree"
    class:dragging={dragView.dragId !== null}
    role="tree"
    aria-label="Connections"
    tabindex="-1"
    {onclick}
    {ondblclick}
    {oncontextmenu}
    {onkeydown}
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={cancelDrag}
  >
    {#each rows as row (row.node.id)}
      <TreeRow
        {row}
        selected={row.node.id === store.selectedId}
        tabbable={row.node.id === tabbableId}
        renaming={row.node.id === store.renamingId}
        drop={dragView.target?.rowId === row.node.id ? dragView.target.position : null}
        dragging={dragView.dragId === row.node.id}
        lamp={row.node.type === "connection" ? sessions.lampFor(row.node.id) : null}
        onrename={(value) => {
          if (value.trim() === row.node.name) store.stopRename();
          else void store.rename(row.node.id, value);
          void focusRowIfLost(row.node.id);
        }}
        oncancel={() => {
          store.stopRename();
          void focusRowIfLost(row.node.id);
        }}
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
  .tree.dragging {
    cursor: grabbing;
  }

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
