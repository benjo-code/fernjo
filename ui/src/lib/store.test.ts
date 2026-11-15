import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { TreeStore } from "./store.svelte";
import { apiError, conn, deferred, folder, mockApi, snapshot } from "./testing";
import type { Mutation } from "./types";

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
  test("init_loads_snapshot_and_notice", async () => {
    const notice = { kind: "restoredFromBackup", badFile: "x" } as const;
    const mock = mockApi([], { listTree: async () => snapshot(tree(), notice) });
    const store = new TreeStore(mock.api);
    await store.init();
    expect(store.children.map((n) => n.name)).toEqual(["Homelab", "web-01"]);
    expect(store.notice).toEqual(notice);
    store.dismissNotice();
    expect(store.notice).toBeNull();
    expect(store.loadError).toBeNull();
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

  test("filter_and_rows_are_derived", async () => {
    const { store } = await started();
    expect(store.rows.map((r) => r.node.name)).toEqual(["Homelab", "web-01"]);
    store.setFilter("pve");
    expect(store.rows.map((r) => r.node.name)).toEqual(["Homelab", "pve-01"]);
    store.setFilter("");
    store.expand("Homelab");
    expect(store.rows).toHaveLength(4);
  });
});
