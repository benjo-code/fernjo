import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, describe, expect, test } from "vitest";
import { TreeStore } from "../store.svelte";
import { apiError, conn, mockApi } from "../testing";
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
});
