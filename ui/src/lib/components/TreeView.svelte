<script lang="ts">
  import { tick } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { DragController, type DragTarget, type Hit } from "../drag";
  import { dropPosition } from "../drop-target";
  import { countDescendants, countKinds, findNode, type Row } from "../rows";
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
  let pendingDelete = $state<{
    ids: string[];
    title: string;
    detail: string;
  } | null>(null);

  const rows = $derived(store.rows);
  const tabbableId = $derived(
    rows.find((r) => r.node.id === store.selectedId)?.node.id ?? rows[0]?.node.id ?? null,
  );
  const query = $derived(store.filter.trim());

  // --- pointer-based drag and drop ---
  const DRAG_THRESHOLD = 4;
  const EDGE_SCROLL = 24;

  let dragView = $state<{ dragIds: string[]; target: DragTarget | null }>({
    dragIds: [],
    target: null,
  });
  let justDragged = false;

  const drag = new DragController({
    threshold: DRAG_THRESHOLD,
    rows: () => rows,
    onMove: (ids, parentId, index) => {
      if (ids.length === 1) void store.move(ids[0], parentId, index);
      else void store.moveMany(ids, parentId, index);
    },
  });

  const dragEnabled = () => query === "" && store.renamingId === null;

  function syncDrag() {
    const active = drag.state === "dragging";
    dragView = { dragIds: active ? drag.dragIds : [], target: active ? drag.target : null };
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
    if (!el) return;
    const id = el.dataset.nodeId!;
    // a press on a selected row of a multi-selection drags all of it; any other row drags alone
    const along =
      store.selectedIds.size > 1 && store.isSelected(id) ? store.selectionTops() : undefined;
    drag.pointerDown(id, e.clientX, e.clientY, along);
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

  /**
   * The row to continue from after deleting `ids`: for one node the next sibling, else the one
   * above; for several the first surviving row below the first of them, else the last above.
   */
  function neighbourAfterDelete(ids: string[]): string | null {
    const first = rows.findIndex((r) => r.node.id === ids[0]);
    if (first < 0) return null;
    if (ids.length === 1) {
      const row = rows[first];
      for (let i = first + 1; i < rows.length && rows[i].depth >= row.depth; i++) {
        if (rows[i].depth === row.depth) return rows[i].node.id;
      }
      return first > 0 ? rows[first - 1].node.id : null;
    }
    const gone = new SvelteSet<string>();
    for (const r of rows) {
      if (ids.includes(r.node.id) || (r.parentId !== null && gone.has(r.parentId))) {
        gone.add(r.node.id);
      }
    }
    for (let i = first + 1; i < rows.length; i++) {
      if (!gone.has(rows[i].node.id)) return rows[i].node.id;
    }
    for (let i = first - 1; i >= 0; i--) {
      if (!gone.has(rows[i].node.id)) return rows[i].node.id;
    }
    return null;
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

  async function extendAndFocus(id: string) {
    store.selectRange(id);
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

  // The keys `onkeydown` handles: Enter connects a connection row, F2 renames, Delete deletes.
  const DELETE_KEY = { hint: "Del", keys: "Delete" };

  function menuItemsFor(row: Row | null): MenuItem[] {
    if (row && store.selectedIds.size > 1 && store.isSelected(row.node.id)) {
      // Rename and Duplicate need a single node
      return [{ label: "Delete", action: () => requestDelete(row), ...DELETE_KEY }];
    }
    if (!row) {
      return [
        { label: "New folder", action: () => void store.createFolder(null, null) },
        { label: "New connection", action: () => void store.createConnection(null, null) },
      ];
    }
    const { parentId, index } = placement(row);
    const connectItems: MenuItem[] = [];
    if (row.node.type === "connection") {
      connectItems.push({
        label: "Connect",
        action: () => connect(row),
        hint: "Enter",
        keys: "Enter",
      });
      if (sessions.tabs.some((t) => t.nodeId === row.node.id)) {
        connectItems.push({ label: "Open another session", action: () => connect(row, true) });
      }
    }
    return [
      ...connectItems,
      { label: "New folder", action: () => void store.createFolder(parentId, index) },
      { label: "New connection", action: () => void store.createConnection(parentId, index) },
      { label: "Duplicate", action: () => void store.duplicate(row.node.id) },
      { label: "Rename", action: () => store.startRename(row.node.id), hint: "F2", keys: "F2" },
      { label: "Delete", action: () => requestDelete(row), ...DELETE_KEY },
    ];
  }

  function requestDelete(row: Row) {
    if (store.selectedIds.size > 1 && store.isSelected(row.node.id)) {
      const tops = store.selectionTops();
      const { connections, folders } = countKinds(
        tops.flatMap((id) => findNode(store.children, id) ?? []),
      );
      const parts = [
        connections > 0 ? `${connections} ${connections === 1 ? "connection" : "connections"}` : "",
        folders > 0 ? `${folders} ${folders === 1 ? "folder" : "folders"}` : "",
      ].filter(Boolean);
      pendingDelete = {
        ids: tops,
        title: `Delete ${store.selectedIds.size} items?`,
        detail: `This removes ${parts.join(" and ")}${folders > 0 ? ", including everything inside" : ""}. This cannot be undone.`,
      };
      return;
    }
    const { name } = row.node;
    const count = countDescendants(row.node);
    const inside = count > 0 ? ` and ${count} ${count === 1 ? "item" : "items"} inside` : "";
    pendingDelete = {
      ids: [row.node.id],
      title: `Delete “${name}”${inside}?`,
      detail: "This cannot be undone.",
    };
  }

  async function confirmDelete() {
    if (!pendingDelete) return;
    const { ids } = pendingDelete;
    pendingDelete = null;
    const neighbour = neighbourAfterDelete(ids);
    if (ids.length === 1) await store.remove(ids[0]);
    else await store.removeMany(ids);
    if (neighbour && store.selectedId === null && rowElement(neighbour) !== undefined) {
      store.select(neighbour);
    }
    if (neighbour) void focusRowIfLost(neighbour);
  }

  function oncontextmenu(e: MouseEvent) {
    e.preventDefault();
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-node-id]");
    const row = el ? rows.find((r) => r.node.id === el.dataset.nodeId) : undefined;
    // a right click inside a multi-selection keeps it; elsewhere it selects just that row
    if (row && !store.isSelected(row.node.id)) store.select(row.node.id);
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
    if (e.ctrlKey || e.metaKey) store.toggleSelect(id);
    else if (e.shiftKey) store.selectRange(id);
    else store.select(id);
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
      const id = rows[Math.max(0, Math.min(rows.length - 1, to))].node.id;
      if (e.shiftKey && index >= 0) void extendAndFocus(id);
      else void selectAndFocus(id);
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
        if (e.shiftKey) break;
        if (current?.node.type === "folder" && current.hasChildren) {
          e.preventDefault();
          if (!current.expanded) store.expand(current.node.id);
          else go(index + 1);
        }
        break;
      case "ArrowLeft":
        if (!current || e.shiftKey) break;
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
    class:dragging={dragView.dragIds.length > 0}
    role="tree"
    aria-multiselectable="true"
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
        selected={store.isSelected(row.node.id)}
        tabbable={row.node.id === tabbableId}
        renaming={row.node.id === store.renamingId}
        drop={dragView.target?.rowId === row.node.id ? dragView.target.position : null}
        dragging={dragView.dragIds.includes(row.node.id)}
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
    detail={pendingDelete.detail}
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

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }
</style>
