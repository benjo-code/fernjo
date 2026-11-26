import { describe, expect, test } from "vitest";
import { deferred, mockApi, updateStatus } from "./testing";
import { UpdateStore } from "./update.svelte";

describe("UpdateStore", () => {
  test("store_follows_update_status_events", async () => {
    const m = mockApi();
    const store = new UpdateStore(m.api);
    await store.init();
    expect(store.status?.state).toBe("idle");
    m.emitUpdate(updateStatus({ state: "available", version: "0.5.0" }));
    expect(store.status?.state).toBe("available");
    expect(store.status?.version).toBe("0.5.0");
    store.dispose();
    m.emitUpdate(updateStatus({ state: "failed" }));
    expect(store.status?.state).toBe("available");
  });

  test("ignores_second_action_while_busy", async () => {
    const gate = deferred<ReturnType<typeof updateStatus>>();
    const m = mockApi([], { updateCheck: () => gate.promise });
    const store = new UpdateStore(m.api);
    await store.init();
    const first = store.check();
    expect(store.busy).toBe(true);
    await store.check();
    await store.install();
    await store.skip();
    expect(m.calls.updateCheck).toHaveLength(1);
    expect(m.calls.updateInstall).toHaveLength(0);
    expect(m.calls.updateSkip).toHaveLength(0);
    gate.resolve(updateStatus({ state: "upToDate" }));
    await first;
    expect(store.busy).toBe(false);
    expect(store.status?.state).toBe("upToDate");
  });

  test("the_automatic_check_switch_works_while_a_download_is_running", async () => {
    const gate = deferred<ReturnType<typeof updateStatus>>();
    let progress = () => {};
    const m = mockApi([], {
      updateInstall: () => gate.promise,
      // a progress event overtakes the answer of the switch
      setCheckForUpdates: async (on: boolean) => {
        progress();
        return updateStatus({ state: "downloading", version: "0.5.0", checkEnabled: on });
      },
    });
    progress = () =>
      m.emitUpdate(
        updateStatus({ state: "downloading", version: "0.5.0", received: 5, total: 10 }),
      );
    const store = new UpdateStore(m.api);
    await store.init();
    const download = store.install();
    expect(store.busy).toBe(true);
    expect(await store.setEnabled(false)).toBe(true);
    expect(m.calls.setCheckForUpdates).toEqual([[false]]);
    expect(store.status?.checkEnabled).toBe(false);
    // the answer of the switch did not replace the newer progress that an event brought
    expect(store.status?.received).toBe(5);
    // the other commands still wait for the one in flight, and the switch did not end its busy
    expect(store.busy).toBe(true);
    await store.check();
    expect(m.calls.updateCheck).toHaveLength(0);
    gate.resolve(updateStatus({ state: "ready", version: "0.5.0" }));
    await download;
    expect(store.busy).toBe(false);
  });

  test("init_reads_status_once", async () => {
    const m = mockApi();
    const store = new UpdateStore(m.api);
    await store.init();
    await store.init();
    expect(m.calls.updateStatus).toHaveLength(1);
    expect(m.calls.onUpdateStatus).toHaveLength(1);
  });

  test("check_is_always_manual", async () => {
    const m = mockApi();
    const store = new UpdateStore(m.api);
    await store.check();
    expect(m.calls.updateCheck).toEqual([[true]]);
  });

  test("a_failed_command_sets_the_error_and_the_next_action_clears_it", async () => {
    let fail = true;
    const m = mockApi([], {
      updateSkip: async () => {
        if (fail) throw { code: "settings_unusable", message: "no" };
        return updateStatus({ state: "available", skipped: true });
      },
    });
    const store = new UpdateStore(m.api);
    expect(await store.skip()).toBe(false);
    expect(store.error?.code).toBe("settings_unusable");
    fail = false;
    expect(await store.skip()).toBe(true);
    expect(store.error).toBeNull();
  });

  test("set_enabled_sends_the_flag", async () => {
    const m = mockApi();
    const store = new UpdateStore(m.api);
    await store.setEnabled(false);
    expect(m.calls.setCheckForUpdates).toEqual([[false]]);
    expect(store.status?.checkEnabled).toBe(false);
  });
});

const sources = import.meta.glob("../**/*.svelte", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

describe("UI hardening", () => {
  test("no_html_injection_in_ui", () => {
    const files = Object.keys(sources);
    expect(files.length).toBeGreaterThan(10);
    const offenders = files.filter((f) => /\{\s*@html/.test(sources[f]!));
    expect(offenders).toEqual([]);
  });

  // The webview capability grants only event listening (src-tauri/capabilities/default.json,
  // pinned in src-tauri/tests/tauri_conf.rs). A UI import of any other Tauri module (window,
  // webview, plugins) would need a new permission, so it must be noticed and decided on.
  test("ui_imports_only_the_tauri_modules_the_capability_covers", async () => {
    const all = import.meta.glob("../../src/**/*.{ts,svelte}", {
      query: "?raw",
      import: "default",
      eager: true,
    }) as Record<string, string>;
    const allowed = new Set(["@tauri-apps/api/core", "@tauri-apps/api/event"]);
    const files = Object.keys(all).filter((f) => !/\.test\.ts$/.test(f));
    expect(files.length).toBeGreaterThan(10);
    const offenders: string[] = [];
    for (const f of files) {
      for (const m of all[f]!.matchAll(/["'](@tauri-apps\/[^"']+)["']/g)) {
        if (!allowed.has(m[1]!)) offenders.push(`${f}: ${m[1]}`);
      }
      if (/__TAURI/.test(all[f]!)) offenders.push(`${f}: __TAURI`);
    }
    expect(offenders).toEqual([]);
  });
});
