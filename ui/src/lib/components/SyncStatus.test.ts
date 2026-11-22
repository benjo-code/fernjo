import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SyncStore } from "../sync.svelte";
import { TreeStore } from "../store.svelte";
import { mockApi, syncStatus } from "../testing";
import type { SyncStatus as Status } from "../types";
import { SessionStore } from "../sessions.svelte";
import { VaultStore } from "../vault.svelte";
import Sidebar from "./Sidebar.svelte";
import NoticeBar from "./NoticeBar.svelte";
import SyncStatusLine from "./SyncStatus.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const settle = async () => {
  await new Promise((r) => setTimeout(r, 0));
  await tick();
  flushSync();
};

async function setup(status: Status) {
  const m = mockApi([], { syncStatus: async () => status });
  const sync = new SyncStore(m.api);
  await sync.init();
  const tree = new TreeStore(m.api);
  const onreseed = vi.fn();
  const ondetails = vi.fn();
  mounted = mount(SyncStatusLine, { target: document.body, props: { sync, onreseed, ondetails } });
  await settle();
  return { ...m, sync, tree, onreseed, ondetails };
}

const button = (label: string) =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!;

describe("SyncStatus", () => {
  test("offline_shows_the_pending_count_and_no_error_bar", async () => {
    const { sync, tree } = await setup(syncStatus({ state: "offline", pendingChanges: 3 }));
    expect(document.body.textContent).toContain("Offline, 3 changes waiting");
    expect(document.querySelector('[role="alert"]')).toBeNull();
    expect(document.querySelector(".attention")).toBeNull();
    // the notice bar stays free of it too
    unmount(mounted!);
    mounted = mount(NoticeBar, { target: document.body, props: { store: tree, sync } });
    await settle();
    expect(document.querySelector('[role="alert"]')).toBeNull();
    expect(document.body.textContent).not.toContain("Offline");
  });

  test("offline_with_one_change_uses_the_singular", async () => {
    await setup(syncStatus({ state: "offline", pendingChanges: 1 }));
    expect(document.body.textContent).toContain("Offline, 1 change waiting");
  });

  test("waiting_for_unlock_shows_its_text", async () => {
    await setup(syncStatus({ state: "waiting_for_unlock" }));
    expect(document.body.textContent).toContain("Sync paused: unlock the vault");
  });

  test("synced_shows_the_time", async () => {
    const iso = "2026-11-22T12:03:00Z";
    await setup(syncStatus({ state: "ok", lastSyncedAt: iso }));
    const expected = new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    expect(document.body.textContent).toContain(`Synced ${expected}`);
  });

  test("synced_without_a_time_just_says_synced", async () => {
    await setup(syncStatus({ state: "ok" }));
    expect(document.body.textContent?.trim()).toBe("Synced");
  });

  test("syncing_while_a_run_is_in_progress", async () => {
    await setup(syncStatus({ state: "ok", running: true }));
    expect(document.body.textContent).toContain("Syncing…");
  });

  test("is_hidden_while_off", async () => {
    await setup(syncStatus());
    expect(document.body.textContent?.trim()).toBe("");
  });

  test("rolled_back_offers_reseed", async () => {
    const { onreseed } = await setup(syncStatus({ state: "rolled_back" }));
    expect(document.body.textContent).toContain("Sync: hub older than this device");
    button("Re-seed…").click();
    expect(onreseed).toHaveBeenCalled();
  });

  test("error_says_sync_problem_with_the_code_text_in_the_tooltip_and_details_opens_settings", async () => {
    const { ondetails } = await setup(
      syncStatus({ state: "error", error: { code: "hub_unauthorized", message: "no" } }),
    );
    expect(document.body.textContent).toContain("Sync problem");
    expect(document.querySelector("[title]")!.getAttribute("title")).toBe(
      "The hub did not accept the secret.",
    );
    button("Details").click();
    expect(ondetails).toHaveBeenCalled();
  });

  test("the_line_is_a_polite_live_region_never_an_alert", async () => {
    await setup(syncStatus({ state: "offline", pendingChanges: 2 }));
    expect(document.querySelector('[role="status"]')?.textContent).toContain("Offline");
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });

  test("the_sidebar_lets_the_tree_shrink_while_the_sync_line_is_shown", async () => {
    const m = mockApi([], { syncStatus: async () => syncStatus({ state: "ok" }) });
    const sync = new SyncStore(m.api);
    await sync.init();
    const store = new TreeStore(m.api);
    const vault = new VaultStore(m.api);
    const sessions = new SessionStore(
      m.api,
      (id) => id,
      () => true,
    );
    mounted = mount(Sidebar, {
      target: document.body,
      props: { store, vault, sessions, onOpenVault: () => {}, sync },
    });
    await settle();
    expect(document.querySelector("aside")!.classList.contains("bars-open")).toBe(true);
    m.emitSync(syncStatus({ state: "off" }));
    await settle();
    expect(document.querySelector("aside")!.classList.contains("bars-open")).toBe(false);
  });
});
