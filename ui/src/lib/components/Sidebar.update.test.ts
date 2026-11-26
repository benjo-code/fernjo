import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test, vi } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { TreeStore } from "../store.svelte";
import { mockApi, updateStatus } from "../testing";
import { UpdateStore } from "../update.svelte";
import { VaultStore } from "../vault.svelte";
import Sidebar from "./Sidebar.svelte";

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

describe("Sidebar update marker", () => {
  test("footer_marker_only_when_available_or_failed", async () => {
    const m = mockApi();
    const update = new UpdateStore(m.api);
    await update.init();
    const store = new TreeStore(m.api);
    const vault = new VaultStore(m.api, () => {});
    const sessions = new SessionStore(
      m.api,
      (id) => id,
      () => true,
    );
    const onupdate = vi.fn();
    mounted = mount(Sidebar, {
      target: document.body,
      props: { store, vault, sessions, onOpenVault: () => {}, update, onupdate },
    });
    await settle();
    const marker = () => document.querySelector<HTMLButtonElement>("[data-update-marker]");
    for (const state of ["idle", "checking", "upToDate"] as const) {
      m.emitUpdate(updateStatus({ state }));
      await settle();
      expect(marker(), state).toBeNull();
    }
    m.emitUpdate(updateStatus({ state: "available", version: "0.5.0" }));
    await settle();
    expect(marker()?.textContent?.trim()).toBe("Update available");
    expect(marker()?.title).toContain("0.5.0");
    expect(document.querySelector("aside")!.classList.contains("bars-open")).toBe(true);
    marker()!.click();
    expect(onupdate).toHaveBeenCalledTimes(1);
    m.emitUpdate(updateStatus({ state: "failed", error: "feed_unreachable" }));
    await settle();
    expect(marker()?.textContent?.trim()).toBe("Update failed");
    m.emitUpdate(updateStatus({ state: "upToDate" }));
    await settle();
    expect(marker()).toBeNull();
  });
});
