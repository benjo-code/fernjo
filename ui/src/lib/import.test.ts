import { describe, expect, test } from "vitest";
import { ImportStore } from "./import.svelte";
import { apiError, conn, deferred, mockApi, preview, report } from "./testing";
import type { ApplyResult, ImportPreview, Snapshot } from "./types";

function setup(impl: Parameters<typeof mockApi>[1] = {}) {
  const mock = mockApi([], impl);
  const snapshots: Snapshot[] = [];
  const restores: boolean[] = [];
  const store = new ImportStore(mock.api, (s, restored) => {
    snapshots.push(s);
    restores.push(restored);
  });
  return { store, snapshots, restores, ...mock };
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

  test("a_double_apply_calls_rust_once_and_ends_in_done", async () => {
    const d = deferred<ApplyResult>();
    const { store, calls } = setup({ importApply: () => d.promise });
    await store.start("mremoteng");
    const first = store.apply({ withoutPasswords: false });
    const second = store.apply({ withoutPasswords: false });
    d.resolve({ snapshot: { children: [], notices: [] }, report: report(), restored: null });
    await Promise.all([first, second]);
    expect(calls.importApply).toHaveLength(1);
    expect(store.step).toBe("done");
    expect(store.error).toBeNull();
  });

  test("a_double_submit_secret_calls_rust_once", async () => {
    const d = deferred<ImportPreview>();
    const { store, calls } = setup({
      importInspect: (secret) =>
        secret === null ? Promise.reject(apiError("key_needed")) : d.promise,
    });
    await store.start("mremoteng");
    const first = store.submitSecret("k");
    const second = store.submitSecret("k");
    d.resolve(preview());
    await Promise.all([first, second]);
    expect(calls.importInspect).toEqual([[null], ["k"]]);
    expect(store.step).toBe("preview");
  });

  test("cancel_during_an_inflight_inspect_stays_idle", async () => {
    const d = deferred<ImportPreview>();
    const { store, calls } = setup({ importInspect: () => d.promise });
    const started = store.start("mremoteng");
    await new Promise((r) => setTimeout(r, 0));
    expect(store.busy).toBe(true);
    await store.cancel();
    expect(calls.importCancel).toHaveLength(1);
    d.resolve(preview());
    await started;
    expect(store.step).toBe("idle");
    expect(store.preview).toBeNull();
    expect(store.fileName).toBe("");
    expect(store.busy).toBe(false);
  });

  test("cancel_during_an_inflight_pick_stays_idle", async () => {
    const d = deferred<{ fileName: string } | null>();
    const { store, calls } = setup({ importPick: () => d.promise });
    const started = store.start("mremoteng");
    await store.cancel();
    d.resolve({ fileName: "x.xml" });
    await started;
    expect(store.step).toBe("idle");
    expect(calls.importInspect).toHaveLength(0);
  });

  test("a_second_start_while_the_first_is_pending_is_ignored", async () => {
    const d = deferred<{ fileName: string } | null>();
    const { store, calls } = setup({ importPick: () => d.promise });
    const first = store.start("mremoteng");
    const second = store.start("backup");
    d.resolve({ fileName: "a.xml" });
    await Promise.all([first, second]);
    expect(calls.importPick).toEqual([["mremoteng"]]);
    expect(store.kind).toBe("mremoteng");
    expect(store.step).toBe("preview");
  });

  test("busy_is_true_during_a_call_and_false_after_success_and_error", async () => {
    const d = deferred<ImportPreview>();
    const { store } = setup({ importInspect: () => d.promise });
    const started = store.start("mremoteng");
    await new Promise((r) => setTimeout(r, 0));
    expect(store.busy).toBe(true);
    d.resolve(preview());
    await started;
    expect(store.busy).toBe(false);

    const failing = setup({
      importApply: async () => {
        throw apiError("damaged");
      },
    });
    await failing.store.start("mremoteng");
    await failing.store.apply({ withoutPasswords: false });
    expect(failing.store.step).toBe("error");
    expect(failing.store.busy).toBe(false);
  });

  test("a_cancelled_pick_keeps_the_view_of_a_previous_import", async () => {
    let picks = 0;
    const { store } = setup({
      importPick: async () => (picks++ === 0 ? { fileName: "a.xml" } : null),
    });
    await store.start("mremoteng");
    expect(store.step).toBe("preview");
    await store.start("mremoteng");
    expect(store.step).toBe("preview");
    expect(store.fileName).toBe("a.xml");
  });

  test("exports_are_refused_while_an_import_is_pending_and_allowed_from_done", async () => {
    const { store, calls } = setup();
    await store.start("mremoteng");
    expect(store.step).toBe("preview");
    await store.exportConnections();
    await store.exportBackup("pw-123456");
    expect(calls.exportConnections).toHaveLength(0);
    expect(calls.exportBackup).toHaveLength(0);
    expect(store.step).toBe("preview");

    await store.apply({ withoutPasswords: false });
    expect(store.step).toBe("done");
    await store.exportConnections();
    expect(calls.exportConnections).toHaveLength(1);
    expect(store.exportResult).not.toBeNull();
  });

  test("the_snapshot_callback_says_whether_the_apply_was_a_restore", async () => {
    const restoredResult: ApplyResult = {
      snapshot: { children: [], notices: [] },
      report: null,
      restored: { createdAt: "2026-11-01T10:00:00Z", folders: 0, connections: 0, credentials: 0 },
    };
    const { store, restores } = setup();
    await store.start("mremoteng");
    await store.apply({ withoutPasswords: false });
    expect(restores).toEqual([false]);

    const restore = setup({ importApply: async () => restoredResult });
    await restore.store.start("backup");
    await restore.store.apply({ withoutPasswords: false });
    expect(restore.restores).toEqual([true]);
  });

  test("a_late_inspect_after_cancel_is_cancelled_again_in_rust", async () => {
    // cancel() while the file is being inspected: Rust may install the plan after the cancel
    const inspecting = deferred<ImportPreview>();
    const { store, calls } = setup({ importInspect: () => inspecting.promise });
    const started = store.start("mremoteng");
    await Promise.resolve();
    await Promise.resolve();
    await store.cancel();
    expect(calls.importCancel).toHaveLength(1);
    inspecting.resolve(preview());
    await started;
    expect(store.step).toBe("idle");
    expect(store.preview).toBeNull();
    expect(calls.importCancel).toHaveLength(2);
  });

  test("a_late_failed_inspect_after_cancel_is_cancelled_again_too", async () => {
    const inspecting = deferred<ImportPreview>();
    const { store, calls } = setup({ importInspect: () => inspecting.promise });
    const started = store.start("mremoteng");
    await Promise.resolve();
    await Promise.resolve();
    await store.cancel();
    inspecting.reject(apiError("key_needed", "needs a key"));
    await started;
    expect(store.step).toBe("idle");
    expect(calls.importCancel).toHaveLength(2);
  });

  test("a_late_result_does_not_cancel_a_newer_import", async () => {
    const first = deferred<ImportPreview>();
    let calledOnce = false;
    const { store, calls } = setup({
      importInspect: () => {
        if (!calledOnce) {
          calledOnce = true;
          return first.promise;
        }
        return Promise.resolve(preview());
      },
    });
    const old = store.start("mremoteng");
    await Promise.resolve();
    await Promise.resolve();
    await store.cancel();
    await store.start("mremoteng"); // a newer import reaches the preview
    expect(store.step).toBe("preview");
    const cancels = calls.importCancel.length;
    first.resolve(preview());
    await old;
    expect(store.step).toBe("preview");
    expect(calls.importCancel).toHaveLength(cancels);
  });

  test("a_throwing_snapshot_callback_does_not_turn_a_successful_apply_into_an_error", async () => {
    const mock = mockApi([]);
    const store = new ImportStore(mock.api, () => {
      throw new Error("tree blew up");
    });
    await store.start("mremoteng");
    await expect(store.apply({ withoutPasswords: false })).rejects.toThrow("tree blew up");
    expect(store.step).toBe("done");
    expect(store.result).not.toBeNull();
    expect(store.error).toBeNull();
  });
});
