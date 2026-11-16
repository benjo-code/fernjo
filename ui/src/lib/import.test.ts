import { describe, expect, test } from "vitest";
import { ImportStore } from "./import.svelte";
import { apiError, conn, mockApi, preview, report } from "./testing";
import type { Snapshot } from "./types";

function setup(impl: Parameters<typeof mockApi>[1] = {}) {
  const mock = mockApi([], impl);
  const snapshots: Snapshot[] = [];
  const store = new ImportStore(mock.api, (s) => snapshots.push(s));
  return { store, snapshots, ...mock };
}

/** Every public field of the store as JSON (to prove no secret is kept). */
const held = (store: ImportStore) =>
  JSON.stringify({
    step: store.step,
    kind: store.kind,
    fileName: store.fileName,
    preview: store.preview,
    result: store.result,
    exportResult: store.exportResult,
    error: store.error,
    own: Object.getOwnPropertyNames(store).map((k) => (store as never)[k]),
  });

describe("ImportStore", () => {
  test("start_with_cancelled_dialog_stays_idle", async () => {
    const { store, calls } = setup({ importPick: async () => null });
    await store.start("mremoteng");
    expect(store.step).toBe("idle");
    expect(store.error).toBeNull();
    expect(calls.importInspect).toHaveLength(0);
  });

  test("default_key_file_goes_straight_to_preview", async () => {
    const p = preview({ fileName: "confCons.xml", report: report() });
    const { store, calls } = setup({
      importPick: async () => ({ fileName: "confCons.xml" }),
      importInspect: async () => p,
    });
    await store.start("mremoteng");
    expect(store.step).toBe("preview");
    expect(store.kind).toBe("mremoteng");
    expect(store.fileName).toBe("confCons.xml");
    expect(store.preview).toEqual(p);
    expect(calls.importInspect).toEqual([[null]]);
  });

  test("key_needed_then_wrong_key_keeps_key_step_and_shows_the_error_then_right_key_goes_to_preview", async () => {
    const { store, calls } = setup({
      importInspect: async (secret) => {
        if (secret === null) throw apiError("key_needed", "enter the key");
        if (secret !== "right") throw apiError("wrong_key", "that key is wrong");
        return preview();
      },
    });
    await store.start("mremoteng");
    expect(store.step).toBe("key");
    await store.submitSecret("nope");
    expect(store.step).toBe("key");
    expect(store.error).toEqual(apiError("wrong_key", "that key is wrong"));
    await store.submitSecret("right");
    expect(store.step).toBe("preview");
    expect(store.error).toBeNull();
    expect(calls.importInspect).toEqual([[null], ["nope"], ["right"]]);
  });

  test("backup_asks_for_its_password", async () => {
    const { store, calls } = setup({
      importInspect: async (secret) => {
        if (secret === null) throw apiError("password_needed", "enter the password");
        if (secret !== "pw-ok") throw apiError("wrong_password", "wrong password");
        return preview({ kind: "backup", report: null });
      },
    });
    await store.start("backup");
    expect(store.step).toBe("key");
    expect(store.error).toBeNull();
    await store.submitSecret("pw-bad");
    expect(store.step).toBe("key");
    expect(store.error?.code).toBe("wrong_password");
    await store.submitSecret("pw-ok");
    expect(store.step).toBe("preview");
    expect(calls.importInspect).toHaveLength(3);
    expect(held(store)).not.toContain("pw-ok");
    expect(held(store)).not.toContain("pw-bad");
  });

  test("apply_vault_needed_goes_to_vault_step_and_retry_applies", async () => {
    let first = true;
    const { store, calls } = setup({
      importApply: async () => {
        if (first) {
          first = false;
          throw apiError("vault_needed", "unlock the vault");
        }
        return { snapshot: { children: [], notices: [] }, report: report(), restored: null };
      },
    });
    await store.start("mremoteng");
    await store.apply({ withoutPasswords: false });
    expect(store.step).toBe("vault");
    expect(store.error?.code).toBe("vault_needed");
    await store.apply({ withoutPasswords: false });
    expect(store.step).toBe("done");
    expect(store.error).toBeNull();
    expect(calls.importApply).toEqual([
      [{ withoutPasswords: false }],
      [{ withoutPasswords: false }],
    ]);
  });

  test("apply_success_hands_the_snapshot_to_the_tree_and_ends_in_done", async () => {
    const snap: Snapshot = { children: [conn("a")], notices: [] };
    const result = { snapshot: snap, report: report(), restored: null };
    const { store, snapshots } = setup({ importApply: async () => result });
    await store.start("mremoteng");
    await store.apply({ withoutPasswords: true });
    expect(store.step).toBe("done");
    expect(store.result).toEqual(result);
    expect(snapshots).toEqual([snap]);
  });

  test("cancel_calls_import_cancel_and_resets", async () => {
    const { store, calls } = setup();
    await store.start("mremoteng");
    expect(store.step).toBe("preview");
    await store.cancel();
    expect(calls.importCancel).toHaveLength(1);
    expect(store.step).toBe("idle");
    expect(store.preview).toBeNull();
    expect(store.fileName).toBe("");
  });

  test("export_backup_passes_the_password_once_and_clears_it", async () => {
    const info = { fileName: "b.fjbk", droppedCredentials: 0 };
    const { store, calls } = setup({ exportBackup: async () => info });
    await store.exportBackup("s3cret-pw");
    expect(calls.exportBackup).toEqual([["s3cret-pw"]]);
    expect(store.step).toBe("done");
    expect(store.exportResult).toEqual(info);
    expect(held(store)).not.toContain("s3cret-pw");

    const c = setup({ exportBackup: async () => null });
    await c.store.exportBackup("s3cret-pw");
    expect(c.store.step).toBe("idle");
    expect(c.store.exportResult).toBeNull();
  });

  test("submit_secret_keeps_no_copy", async () => {
    const { store } = setup({
      importInspect: async (secret) => {
        if (secret === null) throw apiError("key_needed");
        throw apiError("wrong_key");
      },
    });
    await store.start("mremoteng");
    await store.submitSecret("hunter2-key");
    expect(held(store)).not.toContain("hunter2-key");
  });

  test("an_api_error_ends_in_error_with_its_message", async () => {
    const a = setup({
      importInspect: async () => {
        throw apiError("not_a_backup", "this is not a fernjo backup");
      },
    });
    await a.store.start("backup");
    expect(a.store.step).toBe("error");
    expect(a.store.error?.message).toBe("this is not a fernjo backup");

    const b = setup({
      importApply: async () => {
        throw apiError("restore_incomplete", "look in vault.fernjo.bak");
      },
    });
    await b.store.start("mremoteng");
    await b.store.apply({ withoutPasswords: false });
    expect(b.store.step).toBe("error");
    expect(b.store.error?.message).toBe("look in vault.fernjo.bak");
    b.store.reset();
    expect(b.store.step).toBe("idle");
    expect(b.store.error).toBeNull();

    const c = setup({
      exportConnections: async () => {
        throw apiError("save_failed", "disk full");
      },
    });
    await c.store.exportConnections();
    expect(c.store.step).toBe("error");
    expect(c.store.error?.code).toBe("save_failed");
  });
});
