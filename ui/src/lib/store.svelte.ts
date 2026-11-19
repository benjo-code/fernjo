// View state and the bridge to the Rust-owned tree. The snapshot from the last command is the
// only source of the tree; every mutation goes through one queue so commands never overlap.
import { SvelteSet } from "svelte/reactivity";
import { toApiError } from "./api";
import { collectIds, flattenRows, selectionTops, type Row } from "./rows";
import type { Api, ApiError, NodePatch, Notice, ResolvedNode, Snapshot, TreeNode } from "./types";

const STORAGE_KEY = "fernjo.expanded.v1";

function loadExpanded(): string[] {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");
    if (Array.isArray(parsed)) return parsed.filter((v): v is string => typeof v === "string");
  } catch {
    // unreadable or blocked storage: start collapsed
  }
  return [];
}

function saveExpanded(ids: Iterable<string>) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify([...ids]));
  } catch {
    // blocked storage: expansion just is not remembered
  }
}

interface RunOptions {
  /** Return the error to the caller without showing it in the global error bar. */
  silent?: boolean;
}

export class TreeStore {
  children = $state<TreeNode[]>([]);
  notices = $state<Notice[]>([]);
  /** The single primary node: drives `detail` and the properties panel. */
  selectedId = $state<string | null>(null);
  /** Every selected node; `selectedId` is always one of them (or `null` when empty). */
  selectedIds = new SvelteSet<string>();
  /** Start of a Shift range: the node last selected without Shift. */
  anchorId = $state<string | null>(null);
  expanded = new SvelteSet<string>();
  filter = $state("");
  renamingId = $state<string | null>(null);
  error = $state<string | null>(null);
  loadError = $state<string | null>(null);
  /** connections.json changed under us and the last edit was refused; `reload()` clears it. */
  changedOnDisk = $state(false);
  /** The selected node with effective values and sources, as resolved by Rust. */
  detail = $state<ResolvedNode | null>(null);
  /** True once the first load has finished, whether it worked or not. */
  loaded = $state(false);
  rows: Row[] = $derived(flattenRows(this.children, this.expanded, this.filter));

  #api: Api;
  #queue: Promise<unknown> = Promise.resolve();
  #detailToken = 0;

  constructor(api: Api) {
    this.#api = api;
  }

  /** Loads the tree and the persisted expansion state. */
  async init(): Promise<void> {
    for (const id of loadExpanded()) this.expanded.add(id);
    await this.#run(async () => {
      const snapshot = await this.#api.listTree();
      this.#apply(snapshot.children, snapshot.notices);
    });
    this.loaded = true;
  }

  select(id: string | null) {
    this.selectOnly(id);
  }

  /** Makes `id` the whole selection (and the range anchor); `null` clears it. */
  selectOnly(id: string | null) {
    const before = this.#detailTarget();
    this.selectedIds.clear();
    if (id !== null) this.selectedIds.add(id);
    this.selectedId = id;
    this.anchorId = id;
    this.#detailMaybeChanged(before);
  }

  /** Ctrl/Cmd click: adds or removes one node. An added node becomes anchor and primary. */
  toggleSelect(id: string) {
    const before = this.#detailTarget();
    if (this.selectedIds.delete(id)) {
      if (this.selectedId === id) {
        const rest = [...this.selectedIds];
        this.selectedId = rest.length > 0 ? rest[rest.length - 1] : null;
      }
      if (this.anchorId === id) this.anchorId = this.selectedId;
    } else {
      this.selectedIds.add(id);
      this.selectedId = id;
      this.anchorId = id;
    }
    this.#detailMaybeChanged(before);
  }

  /** Shift click: the visible rows from the anchor to `id` become the selection; the anchor stays. */
  selectRange(id: string) {
    const anchor = this.anchorId;
    const from = anchor === null ? -1 : this.rows.findIndex((r) => r.node.id === anchor);
    const to = this.rows.findIndex((r) => r.node.id === id);
    if (from < 0 || to < 0) {
      this.selectOnly(id);
      return;
    }
    const before = this.#detailTarget();
    this.selectedIds.clear();
    const [lo, hi] = from <= to ? [from, to] : [to, from];
    for (let i = lo; i <= hi; i++) this.selectedIds.add(this.rows[i].node.id);
    this.selectedId = id;
    this.#detailMaybeChanged(before);
  }

  isSelected(id: string): boolean {
    return this.selectedIds.has(id);
  }

  /** The selected nodes whose ancestors are not selected, in row order. */
  selectionTops(): string[] {
    return selectionTops(this.rows, this.selectedIds);
  }

  toggle(id: string) {
    if (!this.expanded.delete(id)) this.expanded.add(id);
    saveExpanded(this.expanded);
  }

  expand(id: string) {
    if (this.expanded.has(id)) return;
    this.expanded.add(id);
    saveExpanded(this.expanded);
  }

  setFilter(query: string) {
    this.filter = query;
  }

  startRename(id: string) {
    this.renamingId = id;
  }

  stopRename() {
    this.renamingId = null;
  }

  dismissNotice(index: number) {
    this.notices = this.notices.filter((_, i) => i !== index);
  }

  dismissError() {
    this.error = null;
  }

  /** Shows a message in the global error bar (for errors other stores want surfaced). */
  showError(message: string) {
    this.error = message;
  }

  createFolder(parentId: string | null, index: number | null) {
    return this.#run(async () => {
      const m = await this.#api.createFolder(parentId, index, "New folder");
      this.#apply(m.snapshot.children, m.snapshot.notices);
      this.#afterCreate(parentId, m.id);
    });
  }

  createConnection(parentId: string | null, index: number | null) {
    return this.#run(async () => {
      const m = await this.#api.createConnection(parentId, index, "New connection", "ssh", "");
      this.#apply(m.snapshot.children, m.snapshot.notices);
      this.#afterCreate(parentId, m.id);
    });
  }

  /** Closes the inline editor at once; the name only changes if Rust accepts it. */
  rename(id: string, name: string) {
    this.stopRename();
    return this.update(id, { name });
  }

  update(id: string, patch: NodePatch, options: RunOptions = {}) {
    return this.#run(async () => {
      const m = await this.#api.updateNode(id, patch);
      this.#apply(m.snapshot.children, m.snapshot.notices);
    }, options);
  }

  move(id: string, parentId: string | null, index: number) {
    return this.#run(async () => {
      const m = await this.#api.moveNode(id, parentId, index);
      this.#apply(m.snapshot.children, m.snapshot.notices);
      if (parentId) this.expand(parentId);
    });
  }

  duplicate(id: string) {
    return this.#run(async () => {
      const m = await this.#api.duplicateNode(id);
      this.#apply(m.snapshot.children, m.snapshot.notices);
      if (m.id) this.select(m.id);
    });
  }

  remove(id: string) {
    return this.#run(async () => {
      const m = await this.#api.deleteNode(id);
      this.#apply(m.snapshot.children, m.snapshot.notices);
    });
  }

  /** Re-reads connections.json (Rust adopts what is on disk) and clears the changed-on-disk state. */
  reload() {
    return this.#run(async () => {
      const snapshot = await this.#api.reloadTree();
      this.#apply(snapshot.children, snapshot.notices);
      this.changedOnDisk = false;
      this.error = null;
    });
  }

  /** Adopts a snapshot that another command returned (for example deleting a credential). */
  applySnapshot(snapshot: Snapshot) {
    return this.#run(async () => {
      this.#apply(snapshot.children, snapshot.notices);
    });
  }

  /** Asks Rust for the selected node's effective values; late answers for an old selection are dropped. */
  async #refreshDetail(): Promise<void> {
    const token = ++this.#detailToken;
    const id = this.#detailTarget();
    if (id === null) {
      this.detail = null;
      return;
    }
    let detail: ResolvedNode | null = null;
    try {
      detail = await this.#api.getNode(id);
    } catch {
      // the node vanished or Rust is unavailable: show the empty state, not an error
    }
    if (token === this.#detailToken) this.detail = detail;
  }

  /** The node whose details are shown: the primary one, only while it is the sole selection. */
  #detailTarget(): string | null {
    return this.selectedIds.size === 1 ? this.selectedId : null;
  }

  #detailMaybeChanged(before: string | null) {
    if (this.#detailTarget() !== before) void this.#refreshDetail();
  }

  #afterCreate(parentId: string | null, id: string | null) {
    this.filter = "";
    if (parentId) this.expand(parentId);
    if (id) {
      this.select(id);
      this.startRename(id);
    }
  }

  /** Adopts a snapshot and drops view state that points at nodes that no longer exist. */
  #apply(children: TreeNode[], notices: Notice[]) {
    this.children = children;
    if (notices.length > 0) this.notices = [...this.notices, ...notices];
    const { all, folders } = collectIds(children);
    for (const id of [...this.selectedIds]) if (!all.has(id)) this.selectedIds.delete(id);
    if (this.selectedId !== null && !this.selectedIds.has(this.selectedId)) {
      const rest = [...this.selectedIds];
      this.selectedId = rest.length > 0 ? rest[rest.length - 1] : null;
    }
    if (this.anchorId !== null && !all.has(this.anchorId)) this.anchorId = this.selectedId;
    if (this.renamingId !== null && !all.has(this.renamingId)) this.renamingId = null;
    let pruned = false;
    for (const id of [...this.expanded]) {
      if (!folders.has(id)) {
        this.expanded.delete(id);
        pruned = true;
      }
    }
    if (pruned) saveExpanded(this.expanded);
  }

  /** Runs `job` after every earlier job finished. Resolves to the error, or `null`. */
  #run(job: () => Promise<void>, options: RunOptions = {}): Promise<ApiError | null> {
    const result = this.#queue.then(async () => {
      try {
        await job();
        await this.#refreshDetail();
        return null;
      } catch (e) {
        const err = toApiError(e);
        if (err.code === "load_failed") this.loadError = err.message;
        // the bar with its Reload button replaces the error bar for this one
        else if (err.code === "changed_on_disk") this.changedOnDisk = true;
        // a pending data folder change blocks every write: always tell the user, even for
        // callers that show their own errors
        else if (!options.silent || err.code === "restart_required") this.error = err.message;
        return err;
      }
    });
    this.#queue = result;
    return result;
  }
}
