import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test } from "vitest";
import { TreeStore } from "../store.svelte";
import { SyncStore } from "../sync.svelte";
import { apiError, conn, mockApi, syncStatus } from "../testing";
import NoticeBar from "./NoticeBar.svelte";

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

async function settle() {
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

describe("NoticeBar", () => {
  test("shows the reload bar and the button calls reload", async () => {
    const mock = mockApi([conn("a")], {
      createFolder: async () => {
        throw apiError("changed_on_disk");
      },
    });
    const store = new TreeStore(mock.api);
    await store.init();
    mounted = mount(NoticeBar, { target: document.body, props: { store } });
    await settle();
    expect(document.body.textContent).not.toContain("changed by another program");

    await store.createFolder(null, null);
    await settle();
    const bar = document.body.querySelector('[role="alert"]');
    expect(bar?.textContent).toContain(
      "connections.json was changed by another program or device. Your last edit was not saved.",
    );
    const button = [...document.body.querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === "Reload",
    );
    expect(button).toBeDefined();
    button!.click();
    await settle();
    expect(mock.calls.reloadTree).toHaveLength(1);
    expect(store.changedOnDisk).toBe(false);
    expect(document.body.textContent).not.toContain("changed by another program");
  });

  test("tells that the device folder lies inside the data folder", async () => {
    const store = new TreeStore(mockApi([conn("a")]).api);
    await store.init();
    store.notices = [{ kind: "deviceFolderInsideDataDir", path: "/sync/fernjo/dev" }];
    mounted = mount(NoticeBar, { target: document.body, props: { store } });
    await settle();
    const text = document.body.textContent ?? "";
    expect(text).toContain("/sync/fernjo/dev");
    expect(text).toContain("device-local folder");
    expect(text).toContain("data folder");
  });

  test("findings_are_listed_and_dismissable", async () => {
    const mock = mockApi([conn("a")], {
      syncStatus: async () =>
        syncStatus({
          state: "ok",
          findings: [
            {
              id: "n1",
              name: "web",
              kind: "conflictCopy",
              message: "web was edited on two devices: kept both.",
            },
            { id: "n2", name: "db", kind: "moveUndone", message: "Your move of db was undone." },
          ],
        }),
    });
    const store = new TreeStore(mock.api);
    await store.init();
    store.showError("boom");
    store.changedOnDisk = true;
    const sync = new SyncStore(mock.api);
    await sync.init();
    mounted = mount(NoticeBar, { target: document.body, props: { store, sync } });
    await settle();
    const text = () => document.body.textContent ?? "";
    expect(text()).toContain("web was edited on two devices: kept both.");
    expect(text()).toContain("Your move of db was undone.");
    // the other bars are still there
    expect(text()).toContain("boom");
    expect(text()).toContain("connections.json was changed by another program");

    const dismiss = document.body.querySelector<HTMLButtonElement>(
      "button[data-dismiss-findings]",
    )!;
    dismiss.click();
    await settle();
    expect(mock.calls.syncDismissFindings).toHaveLength(1);
    expect(text()).not.toContain("kept both");
    expect(text()).toContain("boom");
  });
});
