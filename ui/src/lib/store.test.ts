import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { TreeStore } from "./store.svelte";
import { SyncStore } from "./sync.svelte";
import {
  apiError,
  conn,
  deferred,
  folder,
  mockApi,
  resolved,
  snapshot,
  syncStatus,
} from "./testing";
import type { Mutation, Notice, ResolvedNode } from "./types";

const KEY = "fernjo.expanded.v1";

const tree = () => [folder("Homelab", [conn("pve-01"), conn("DC01", "rdp")]), conn("web-01")];

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  vi.restoreAllMocks();
});

async function started(initial = tree(), impl = {}) {
  const mock = mockApi(initial, impl);
  const store = new TreeStore(mock.api);
  await store.init();
  return { store, ...mock };
}

describe("TreeStore", () => {
  test("notices_from_the_snapshot_are_kept_and_dismissed_by_index", async () => {
    const notices: Notice[] = [
      { kind: "restoredFromBackup", badFile: "x" },
      { kind: "syncConflicts", files: ["a (conflict).json"] },
    ];
    const mock = mockApi([], { listTree: async () => snapshot(tree(), notices) });
    const store = new TreeStore(mock.api);
    await store.init();
    expect(store.children.map((n) => n.name)).toEqual(["Homelab", "web-01"]);
    expect(store.notices).toEqual(notices);
    store.dismissNotice(0);
    expect(store.notices).toEqual([notices[1]]);
    store.dismissNotice(5);
    expect(store.notices).toHaveLength(1);
    store.dismissNotice(0);
    expect(store.notices).toEqual([]);
    expect(store.loadError).toBeNull();
  });

  test("changed_on_disk shows the reload state, keeps the error bar empty, and reload clears it", async () => {
    const { store, setTree, calls } = await started(tree(), {
      createFolder: async () => {
        throw apiError("changed_on_disk");
      },
    });
    const err = await store.createFolder(null, null);
    expect(err?.code).toBe("changed_on_disk");
    expect(store.changedOnDisk).toBe(true);
    expect(store.error).toBeNull();

    setTree([conn("theirs")]);
    expect(await store.reload()).toBeNull();
    expect(calls.reloadTree).toHaveLength(1);
    expect(store.changedOnDisk).toBe(false);
    expect(store.children.map((n) => n.name)).toEqual(["theirs"]);
  });

  test("a failed reload keeps the tree and the reload state, a later reload clears both", async () => {
    let fail = true;
    const { store, setTree } = await started(tree(), {
      createFolder: async () => {
        throw apiError("changed_on_disk");
      },
      reloadTree: async () => {
        if (fail) throw apiError("reload_failed", "could not read connections.json");
        return snapshot([conn("theirs")]);
      },
    });
    await store.createFolder(null, null);
    const err = await store.reload();
    expect(err?.code).toBe("reload_failed");
    expect(store.changedOnDisk).toBe(true);
    expect(store.loadError).toBeNull();
    expect(store.error).toBe("could not read connections.json");
    expect(store.children.map((n) => n.name)).toEqual(["Homelab", "web-01"]);

    fail = false;
    setTree([conn("theirs")]);
    expect(await store.reload()).toBeNull();
    expect(store.changedOnDisk).toBe(false);
    expect(store.error).toBeNull();
    expect(store.children.map((n) => n.name)).toEqual(["theirs"]);
  });

  test("selection_is_cleared_when_snapshot_no_longer_contains_it", async () => {
    const { store, setTree, api } = await started();
    store.select("web-01");
    expect(store.selectedId).toBe("web-01");
    setTree([folder("Homelab", [conn("pve-01"), conn("DC01", "rdp")])]);
    await store.remove("web-01");
    expect(store.selectedId).toBeNull();
    expect(api).toBeDefined();
  });

  test("stale_expanded_ids_are_pruned", async () => {
    localStorage.setItem(KEY, JSON.stringify(["Homelab", "gone"]));
    const { store } = await started();
    expect([...store.expanded]).toEqual(["Homelab"]);
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual(["Homelab"]);
  });

  test("mutations_run_strictly_in_order", async () => {
    const first = deferred<Mutation>();
    let n = 0;
    const { store, calls } = await started(tree(), {
      moveNode: (id: string) =>
        n++ === 0 ? first.promise : Promise.resolve({ id, snapshot: snapshot(tree()) }),
    });
    const p1 = store.move("web-01", null, 0);
    const p2 = store.move("Homelab", null, 1);
    await new Promise((r) => setTimeout(r, 0));
    expect(calls.moveNode).toHaveLength(1);
    first.resolve({ id: "web-01", snapshot: snapshot(tree()) });
    await Promise.all([p1, p2]);
    expect(calls.moveNode).toHaveLength(2);
    expect(calls.moveNode.map((c) => c[0])).toEqual(["web-01", "Homelab"]);
  });

  test("a_failed_mutation_does_not_block_the_queue", async () => {
    let n = 0;
    const { store, calls } = await started(tree(), {
      moveNode: async (id: string) => {
        if (n++ === 0) throw apiError("invalid_move", "nope");
        return { id, snapshot: snapshot(tree()) };
      },
    });
    const first = await store.move("a", null, 0);
    const second = await store.move("b", null, 0);
    expect(first?.code).toBe("invalid_move");
    expect(second).toBeNull();
    expect(calls.moveNode).toHaveLength(2);
  });

  test("api_error_sets_error_and_keeps_snapshot", async () => {
    const { store } = await started(tree(), {
      moveNode: async () => {
        throw apiError("invalid_move", "A folder cannot be moved into itself");
      },
    });
    const err = await store.move("Homelab", "pve-01", 0);
    expect(err?.code).toBe("invalid_move");
    expect(store.error).toBe("A folder cannot be moved into itself");
    expect(store.children.map((n) => n.name)).toEqual(["Homelab", "web-01"]);
    store.dismissError();
    expect(store.error).toBeNull();
  });

  test("silent_errors_are_returned_but_not_shown_globally", async () => {
    const { store } = await started(tree(), {
      updateNode: async () => {
        throw apiError("invalid_port", "The port must be between 1 and 65535");
      },
    });
    const err = await store.update("web-01", { port: 0 }, { silent: true });
    expect(err?.message).toBe("The port must be between 1 and 65535");
    expect(store.error).toBeNull();
  });

  test("restart_required_is_shown_globally_even_for_silent_calls", async () => {
    const { store } = await started(tree(), {
      updateNode: async () => {
        throw apiError("restart_required", "Restart fernjo first");
      },
    });
    await store.update("web-01", { port: 5 }, { silent: true });
    expect(store.error).toBe("Restart fernjo first");
  });

  test("load_failed_error_sets_load_error", async () => {
    const mock = mockApi([], {
      listTree: async () => {
        throw apiError("load_failed", "connections.json was written by a newer version");
      },
    });
    const store = new TreeStore(mock.api);
    await store.init();
    expect(store.loadError).toBe("connections.json was written by a newer version");
    expect(store.error).toBeNull();
    expect(store.children).toEqual([]);
  });

  test("expansion_persists_to_local_storage_and_restores", async () => {
    const { store } = await started();
    store.toggle("Homelab");
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual(["Homelab"]);
    const again = await started();
    expect(again.store.expanded.has("Homelab")).toBe(true);
    again.store.toggle("Homelab");
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual([]);
  });

  test("broken_local_storage_does_not_crash", async () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    const { store } = await started();
    store.toggle("Homelab");
    expect(store.expanded.has("Homelab")).toBe(true);
  });

  test("garbage_in_local_storage_is_ignored", async () => {
    localStorage.setItem(KEY, "{not json");
    const { store } = await started();
    expect(store.expanded.size).toBe(0);
    localStorage.setItem(KEY, JSON.stringify({ a: 1 }));
    expect((await started()).store.expanded.size).toBe(0);
  });

  test("create_folder_selects_expands_parent_and_starts_rename", async () => {
    const after = [folder("Homelab", [conn("pve-01"), conn("DC01", "rdp"), folder("New folder")])];
    const { store, calls } = await started(tree(), {
      createFolder: async () => ({ id: "New folder", snapshot: snapshot(after) }),
    });
    await store.createFolder("Homelab", null);
    expect(calls.createFolder[0]).toEqual(["Homelab", null, "New folder"]);
    expect(store.selectedId).toBe("New folder");
    expect(store.expanded.has("Homelab")).toBe(true);
    expect(store.renamingId).toBe("New folder");
  });

  test("create_connection_uses_default_name_ssh_and_empty_host", async () => {
    const { store, calls } = await started(tree(), {
      createConnection: async () => ({
        id: "New connection",
        snapshot: snapshot([...tree(), conn("New connection")]),
      }),
    });
    await store.createConnection(null, 2);
    expect(calls.createConnection[0]).toEqual([null, 2, "New connection", "ssh", ""]);
    expect(store.renamingId).toBe("New connection");
  });

  test("rename_sends_only_the_name_and_stops_renaming", async () => {
    const { store, calls } = await started();
    store.startRename("web-01");
    expect(store.renamingId).toBe("web-01");
    await store.rename("web-01", "web-02");
    expect(calls.updateNode[0]).toEqual(["web-01", { name: "web-02" }]);
    expect(store.renamingId).toBeNull();
  });

  test("move_expands_the_new_parent", async () => {
    const { store } = await started();
    await store.move("web-01", "Homelab", 0);
    expect(store.expanded.has("Homelab")).toBe(true);
  });

  test("duplicate_selects_the_copy", async () => {
    const after = [...tree(), conn("web-01 copy")];
    const { store } = await started(tree(), {
      duplicateNode: async () => ({ id: "web-01 copy", snapshot: snapshot(after) }),
    });
    await store.duplicate("web-01");
    expect(store.selectedId).toBe("web-01 copy");
  });

  test("loaded_becomes_true_after_init_even_when_it_fails", async () => {
    const ok = new TreeStore(mockApi(tree()).api);
    expect(ok.loaded).toBe(false);
    await ok.init();
    expect(ok.loaded).toBe(true);
    const bad = new TreeStore(
      mockApi([], {
        listTree: async () => {
          throw apiError("load_failed", "x");
        },
      }).api,
    );
    await bad.init();
    expect(bad.loaded).toBe(true);
  });

  test("filter_and_rows_are_derived", async () => {
    const { store } = await started();
    expect(store.rows.map((r) => r.node.name)).toEqual(["Homelab", "web-01"]);
    store.setFilter("pve");
    expect(store.rows.map((r) => r.node.name)).toEqual(["Homelab", "pve-01"]);
    store.setFilter("");
    store.expand("Homelab");
    expect(store.rows).toHaveLength(4);
  });

  describe("detail", () => {
    const tick = () => new Promise((r) => setTimeout(r, 0));

    test("detail_is_loaded_for_the_selection_and_cleared_with_it", async () => {
      const { store, calls } = await started(tree(), {
        getNode: async (id: string) => resolved({ id, name: id }),
      });
      expect(store.detail).toBeNull();
      store.select("web-01");
      await tick();
      expect(store.detail?.id).toBe("web-01");
      expect(calls.getNode).toEqual([["web-01"]]);
      store.select(null);
      expect(store.detail).toBeNull();
    });

    test("detail_is_refreshed_after_a_mutation_and_before_it_resolves", async () => {
      let username = "ops";
      const { store, calls } = await started(tree(), {
        getNode: async (id: string) =>
          resolved({
            id,
            name: id,
            username: { local: username, effective: username, source: { kind: "local" } },
          }),
        updateNode: async (id: string) => {
          username = "root";
          return { id, snapshot: snapshot(tree()) };
        },
      });
      store.select("web-01");
      await tick();
      expect(store.detail?.username.effective).toBe("ops");
      await store.update("web-01", { username: "root" });
      expect(store.detail?.username.effective).toBe("root");
      expect(calls.getNode).toHaveLength(2);
    });

    test("a_stale_detail_response_is_ignored", async () => {
      const slow = deferred<ResolvedNode>();
      const { store } = await started(tree(), {
        getNode: (id: string) =>
          id === "web-01" ? slow.promise : Promise.resolve(resolved({ id, name: id })),
      });
      store.select("web-01");
      store.select("Homelab");
      await tick();
      slow.resolve(resolved({ id: "web-01", name: "web-01" }));
      await tick();
      expect(store.detail?.id).toBe("Homelab");
    });

    test("a_failing_getNode_leaves_no_detail_and_no_error_bar", async () => {
      const { store } = await started(tree(), {
        getNode: async () => {
          throw apiError("not_found", "gone");
        },
      });
      store.select("web-01");
      await tick();
      expect(store.detail).toBeNull();
      expect(store.error).toBeNull();
    });

    test("detail_disappears_when_the_selected_node_is_deleted", async () => {
      const { store, setTree } = await started(tree(), {
        getNode: async (id: string) => resolved({ id, name: id }),
      });
      store.select("web-01");
      await tick();
      setTree([folder("Homelab", [conn("pve-01"), conn("DC01", "rdp")])]);
      await store.remove("web-01");
      expect(store.selectedId).toBeNull();
      expect(store.detail).toBeNull();
    });
  });
});

describe("TreeStore.applySnapshot", () => {
  test("adopts_an_externally_returned_snapshot", async () => {
    const { store } = await started();
    store.select("web-01");
    await store.applySnapshot(snapshot([conn("other")]));
    expect(store.children.map((n) => n.name)).toEqual(["other"]);
    expect(store.selectedId).toBeNull();
  });
});

describe("TreeStore selection", () => {
  const tick = () => new Promise((r) => setTimeout(r, 0));
  const open = async () => {
    const r = await started(tree(), { getNode: async (id: string) => resolved({ id, name: id }) });
    r.store.expand("Homelab");
    return r;
  };

  test("selectOnly replaces the selection", async () => {
    const { store } = await open();
    store.selectOnly("pve-01");
    store.toggleSelect("web-01");
    store.selectOnly("DC01");
    expect([...store.selectedIds]).toEqual(["DC01"]);
    expect(store.selectedId).toBe("DC01");
    expect(store.anchorId).toBe("DC01");
    store.selectOnly(null);
    expect(store.selectedIds.size).toBe(0);
    expect(store.selectedId).toBeNull();
    expect(store.anchorId).toBeNull();
  });

  test("toggleSelect adds and removes one id and keeps the anchor sensible", async () => {
    const { store } = await open();
    store.selectOnly("pve-01");
    store.toggleSelect("web-01");
    expect([...store.selectedIds].sort()).toEqual(["pve-01", "web-01"]);
    expect(store.anchorId).toBe("web-01");
    expect(store.selectedId).toBe("web-01");
    expect(store.isSelected("pve-01")).toBe(true);
    store.toggleSelect("web-01");
    expect([...store.selectedIds]).toEqual(["pve-01"]);
    expect(store.selectedId).toBe("pve-01");
    expect(store.anchorId).toBe("pve-01");
    store.toggleSelect("pve-01");
    expect(store.selectedIds.size).toBe(0);
    expect(store.selectedId).toBeNull();
    expect(store.anchorId).toBeNull();
  });

  test("selectRange selects the visible rows between anchor and target in either direction", async () => {
    const { store } = await open();
    store.selectOnly("pve-01");
    store.selectRange("web-01");
    expect([...store.selectedIds].sort()).toEqual(["DC01", "pve-01", "web-01"]);
    expect(store.anchorId).toBe("pve-01");
    store.selectOnly("web-01");
    store.selectRange("Homelab");
    expect([...store.selectedIds].sort()).toEqual(["DC01", "Homelab", "pve-01", "web-01"]);
    expect(store.anchorId).toBe("web-01");
    store.selectOnly("DC01");
    store.selectRange("pve-01");
    expect([...store.selectedIds].sort()).toEqual(["DC01", "pve-01"]);
  });

  test("selectRange without an anchor behaves like selectOnly", async () => {
    const { store } = await open();
    store.selectRange("DC01");
    expect([...store.selectedIds]).toEqual(["DC01"]);
    expect(store.anchorId).toBe("DC01");
  });

  test("selectRange skips children of collapsed folders", async () => {
    const { store } = await started();
    store.selectOnly("Homelab");
    store.selectRange("web-01");
    expect([...store.selectedIds].sort()).toEqual(["Homelab", "web-01"]);
  });

  test("selectionTops drops a node whose ancestor is selected", async () => {
    const { store } = await open();
    store.selectOnly("Homelab");
    store.toggleSelect("pve-01");
    store.toggleSelect("web-01");
    expect(store.selectionTops()).toEqual(["Homelab", "web-01"]);
  });

  test("deleting nodes prunes the selection", async () => {
    const { store, setTree } = await open();
    store.selectOnly("pve-01");
    store.toggleSelect("web-01");
    setTree([folder("Homelab", [conn("pve-01"), conn("DC01", "rdp")])]);
    await store.remove("web-01");
    expect([...store.selectedIds]).toEqual(["pve-01"]);
    expect(store.selectedId).toBe("pve-01");
    store.selectOnly("DC01");
    setTree([]);
    await store.remove("Homelab");
    expect(store.selectedIds.size).toBe(0);
    expect(store.anchorId).toBeNull();
    expect(store.selectedId).toBeNull();
  });

  test("detail is cleared while several nodes are selected", async () => {
    const { store } = await open();
    store.select("web-01");
    await tick();
    expect(store.detail?.id).toBe("web-01");
    store.toggleSelect("pve-01");
    await tick();
    expect(store.detail).toBeNull();
    store.toggleSelect("pve-01");
    await tick();
    expect(store.detail?.id).toBe("web-01");
  });
  test("collapsing_a_folder_prunes_hidden_selected_nodes", async () => {
    const { store } = await open();
    store.selectOnly("Homelab");
    store.toggleSelect("pve-01");
    store.toggleSelect("DC01");
    store.toggleSelect("web-01");
    expect(store.selectedIds.size).toBe(4);
    store.toggle("Homelab"); // collapses: pve-01 and DC01 are no longer visible
    expect([...store.selectedIds].sort()).toEqual(["Homelab", "web-01"]);
    // the count the user sees is what delete would remove
    expect(store.selectionTops()).toEqual(["Homelab", "web-01"]);
    expect(store.selectedIds.size).toBe(store.selectionTops().length);
  });

  test("collapsing_the_folder_of_the_primary_hands_the_primary_to_a_visible_node", async () => {
    const { store } = await open();
    store.selectOnly("web-01");
    store.toggleSelect("pve-01");
    expect(store.selectedId).toBe("pve-01");
    store.toggle("Homelab");
    expect([...store.selectedIds]).toEqual(["web-01"]);
    expect(store.selectedId).toBe("web-01");
  });

  test("filtering_prunes_hidden_selected_nodes", async () => {
    const { store } = await open();
    store.selectOnly("pve-01");
    store.toggleSelect("DC01");
    store.toggleSelect("web-01");
    store.setFilter("web");
    expect([...store.selectedIds]).toEqual(["web-01"]);
    expect(store.selectionTops()).toEqual(["web-01"]);
  });

  test("a_single_selection_survives_a_filter_that_hides_it", async () => {
    const { store } = await open();
    store.selectOnly("pve-01");
    store.setFilter("web");
    expect([...store.selectedIds]).toEqual(["pve-01"]);
    expect(store.selectedId).toBe("pve-01");
  });

  test("ctrl_click_after_a_collapse_does_not_bring_the_hidden_node_back_into_the_count", async () => {
    const { store, calls } = await open();
    store.selectOnly("pve-01");
    store.toggle("Homelab"); // a single selection stays although it is hidden now
    expect([...store.selectedIds]).toEqual(["pve-01"]);
    store.toggleSelect("web-01");
    expect([...store.selectedIds]).toEqual(["web-01"]);
    expect(store.selectedId).toBe("web-01");
    expect(store.anchorId).toBe("web-01");
    expect(store.selectedIds.size).toBe(store.selectionTops().length);
    await store.removeMany([...store.selectedIds]);
    expect(calls.deleteNode).toEqual([["web-01"]]);
  });

  test("ctrl_click_after_a_filter_hid_everything_selected_starts_from_the_new_row", async () => {
    const { store } = await open();
    store.selectOnly("pve-01");
    store.toggleSelect("DC01");
    store.setFilter("web"); // both hidden: the selection is left alone
    expect([...store.selectedIds].sort()).toEqual(["DC01", "pve-01"]);
    store.toggleSelect("web-01");
    expect([...store.selectedIds]).toEqual(["web-01"]);
    expect(store.selectedIds.size).toBe(store.selectionTops().length);
  });

  test("moveMany sends one batch command and applies the snapshot", async () => {
    const after = [folder("Homelab", [conn("pve-01"), conn("DC01", "rdp"), conn("web-01")])];
    const { store, calls } = await started(tree(), {
      moveNodes: async () => ({ id: null, snapshot: snapshot(after) }),
    });
    await store.moveMany(["web-01", "pve-01"], "Homelab", 0);
    expect(calls.moveNodes).toEqual([[["web-01", "pve-01"], "Homelab", 0]]);
    expect(store.children).toEqual(after);
    expect(store.expanded.has("Homelab")).toBe(true);
  });

  test("moveMany with no ids sends nothing", async () => {
    const { store, calls } = await started();
    await store.moveMany([], null, 0);
    expect(calls.moveNodes).toEqual([]);
  });

  test("removeMany deletes the tops in row order, one command each", async () => {
    const { store, calls } = await open();
    await store.removeMany(["web-01", "pve-01", "Homelab"]);
    expect(calls.deleteNode).toEqual([["Homelab"], ["web-01"]]);
  });
});

describe("TreeStore background refresh", () => {
  test("refresh_keeps_the_error_and_changed_on_disk_flags", async () => {
    const { store, setTree, calls } = await started();
    store.showError("boom");
    store.markChangedOnDisk();
    setTree([conn("remote")]);
    await store.refresh();
    expect(store.children.map((n) => n.name)).toEqual(["remote"]);
    expect(store.error).toBe("boom");
    expect(store.changedOnDisk).toBe(true);
    expect(calls.reloadTree).toHaveLength(0);
  });

  test("refresh_is_quiet_when_listing_fails", async () => {
    const { store } = await started(tree(), {});
    const failing = mockApi(tree(), {
      listTree: async () => {
        throw apiError("load_failed", "nope");
      },
    });
    const other = new TreeStore(failing.api);
    await other.refresh();
    expect(other.error).toBeNull();
    expect(other.loadError).toBeNull();
    expect(other.changedOnDisk).toBe(false);
    expect(store.children.length).toBeGreaterThan(0);
  });
});

describe("SyncStore", () => {
  test("sync_store_ignores_a_new_action_while_busy", async () => {
    const gate = deferred<ReturnType<typeof syncStatus>>();
    const mock = mockApi([], { syncNow: () => gate.promise });
    const sync = new SyncStore(mock.api);
    const first = sync.syncNow();
    expect(sync.busy).toBe(true);
    expect(await sync.syncNow()).toBe(false);
    expect(await sync.leave()).toBe(false);
    expect(mock.calls.syncNow).toHaveLength(1);
    expect(mock.calls.syncLeave).toHaveLength(0);
    gate.resolve(syncStatus({ state: "ok" }));
    expect(await first).toBe(true);
    expect(sync.busy).toBe(false);
  });

  test("a_changed_applied_counter_reloads_tree_and_vault_but_an_unchanged_one_does_not", async () => {
    const mock = mockApi([], { syncStatus: async () => syncStatus({ state: "ok", applied: 0 }) });
    const onApplied = vi.fn();
    const sync = new SyncStore(mock.api, onApplied);
    await sync.init();
    expect(onApplied, "nothing was applied yet").not.toHaveBeenCalled();
    mock.emitSync(syncStatus({ state: "ok", applied: 0 }));
    expect(onApplied).not.toHaveBeenCalled();
    mock.emitSync(syncStatus({ state: "ok", applied: 3 }));
    expect(onApplied).toHaveBeenCalledTimes(1);
    expect(sync.status?.applied).toBe(3);
    mock.emitSync(syncStatus({ state: "ok", applied: 3 }));
    expect(onApplied).toHaveBeenCalledTimes(1);
    sync.dispose();
    mock.emitSync(syncStatus({ state: "ok", applied: 4 }));
    expect(onApplied).toHaveBeenCalledTimes(1);
  });

  test("a_first_status_that_already_counts_applied_runs_reloads_tree_and_vault", async () => {
    // a startup run applied remote changes after the tree was listed, before sync was told
    const mock = mockApi([], { syncStatus: async () => syncStatus({ state: "ok", applied: 2 }) });
    const onApplied = vi.fn();
    const sync = new SyncStore(mock.api, onApplied);
    await sync.init();
    expect(onApplied).toHaveBeenCalledTimes(1);
    mock.emitSync(syncStatus({ state: "ok", applied: 2 }));
    expect(onApplied, "the same counter again changes nothing").toHaveBeenCalledTimes(1);
  });

  test("the_pairing_code_is_dropped_when_the_status_goes_off", async () => {
    const mock = mockApi([], { syncStatus: async () => syncStatus({ state: "ok" }) });
    const sync = new SyncStore(mock.api);
    await sync.init();
    await sync.showPairingCode();
    expect(sync.pairingCode).toBe("PAIRING-CODE");
    mock.emitSync(syncStatus({ state: "off" }));
    expect(sync.pairingCode).toBeNull();
  });

  test("nothing_of_sync_is_written_to_local_storage", async () => {
    localStorage.clear();
    const mock = mockApi([]);
    const sync = new SyncStore(mock.api);
    await sync.init();
    await sync.setup("https://h", "secret", null, "PC");
    expect(localStorage.length).toBe(0);
  });

  test("a_successful_join_calls_on_joined_but_a_confirmation_does_not", async () => {
    const mock = mockApi([], {
      syncJoin: async (_c, _n, confirm) =>
        confirm
          ? syncStatus({ state: "ok" })
          : syncStatus({
              state: "needs_confirmation",
              confirmation: { connections: 1, folders: 0, vaultEntries: null, hasVault: false },
            }),
    });
    const onJoined = vi.fn();
    const sync = new SyncStore(mock.api, () => {}, onJoined);
    expect(await sync.join("c", "d", false)).toBe("confirm");
    expect(onJoined).not.toHaveBeenCalled();
    expect(await sync.join("c", "d", true)).toBe("joined");
    expect(onJoined).toHaveBeenCalledTimes(1);
  });

  test("a_pairing_code_that_arrives_after_hide_is_not_stored", async () => {
    const gate = deferred<string>();
    const mock = mockApi([], { syncPairingCode: () => gate.promise });
    const sync = new SyncStore(mock.api);
    const shown = sync.showPairingCode();
    sync.hidePairingCode();
    gate.resolve("LATE-CODE");
    await shown;
    expect(sync.pairingCode).toBeNull();
  });

  test("a_pairing_code_that_arrives_after_dispose_is_not_stored", async () => {
    const gate = deferred<string>();
    const mock = mockApi([], { syncPairingCode: () => gate.promise });
    const sync = new SyncStore(mock.api);
    const shown = sync.showPairingCode();
    sync.dispose();
    gate.resolve("LATE-CODE");
    await shown;
    expect(sync.pairingCode).toBeNull();
  });

  test("a_pairing_code_that_arrives_after_leave_is_not_stored", async () => {
    const gate = deferred<string>();
    const mock = mockApi([], { syncPairingCode: () => gate.promise });
    const sync = new SyncStore(mock.api);
    const shown = sync.showPairingCode();
    await sync.leave();
    gate.resolve("LATE-CODE");
    await shown;
    expect(sync.pairingCode).toBeNull();
  });

  test("closing_the_panel_while_setup_runs_drops_the_code_it_fetches_afterwards", async () => {
    const gate = deferred<ReturnType<typeof syncStatus>>();
    const mock = mockApi([], {
      syncSetup: () => gate.promise,
      syncPairingCode: async () => "CODE",
    });
    const sync = new SyncStore(mock.api);
    const running = sync.setup("https://h", "s", null, "PC");
    sync.hidePairingCode(); // the Settings dialog was closed
    gate.resolve(syncStatus({ state: "ok" }));
    expect(await running).toBe(true);
    expect(sync.pairingCode).toBeNull();
  });
});
