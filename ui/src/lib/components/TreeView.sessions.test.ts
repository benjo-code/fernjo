import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { TreeStore } from "../store.svelte";
import { conn, folder, mockApi } from "../testing";
import type { TreeNode } from "../types";
import TreeView from "./TreeView.svelte";

let mounted: ReturnType<typeof mount> | null = null;

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

const sample = (): TreeNode[] => [
  folder("Homelab", [conn("pve-01"), conn("DC01", "rdp")]),
  conn("web-01"),
];

async function setup() {
  const mock = mockApi(sample());
  const store = new TreeStore(mock.api);
  await store.init();
  store.expand("Homelab");
  const sessions = new SessionStore(
    mock.api,
    (id) => id,
    () => true,
  );
  mounted = mount(TreeView, { target: document.body, props: { store, sessions } });
  flushSync();
  return { store, sessions, ...mock };
}

const row = (id: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="treeitem"]')].find(
    (el) => el.dataset.nodeId === id,
  )!;
const menuLabels = () =>
  [...document.querySelectorAll('[role="menuitem"]')].map((el) =>
    el.querySelector(".label")?.textContent?.trim(),
  );
const item = (label: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
    (el) => el.querySelector(".label")?.textContent?.trim() === label,
  )!;

function contextMenu(el: Element) {
  el.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 20, clientY: 30 }),
  );
  flushSync();
}

async function key(el: Element, k: string, init: KeyboardEventInit = {}) {
  el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

function doubleClick(el: Element) {
  el.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true }));
  flushSync();
}

describe("TreeView sessions", () => {
  test("double_click_opens_a_connection", async () => {
    const { sessions } = await setup();
    const open = vi.spyOn(sessions, "open");
    doubleClick(row("web-01"));
    expect(open).toHaveBeenCalledWith("web-01");
    expect(sessions.tabs.map((t) => t.nodeId)).toEqual(["web-01"]);
  });

  test("enter_opens_the_selected_connection_and_f2_still_renames", async () => {
    const { store, sessions } = await setup();
    const open = vi.spyOn(sessions, "open");
    store.select("web-01");
    flushSync();
    await key(row("web-01"), "Enter");
    expect(open).toHaveBeenCalledWith("web-01");
    expect(store.renamingId).toBeNull();
    await key(row("web-01"), "F2");
    expect(store.renamingId).toBe("web-01");
    expect(open).toHaveBeenCalledTimes(1);
  });

  test("context_menu_has_connect_and_open_another_only_when_open", async () => {
    const { sessions } = await setup();
    contextMenu(row("web-01"));
    expect(menuLabels()[0]).toBe("Connect");
    expect(menuLabels()).not.toContain("Open another session");
    item("Connect").click();
    flushSync();
    expect(sessions.tabs).toHaveLength(1);

    contextMenu(row("web-01"));
    expect(menuLabels().slice(0, 2)).toEqual(["Connect", "Open another session"]);
    item("Open another session").click();
    flushSync();
    expect(sessions.tabs.map((t) => t.title)).toEqual(["web-01", "web-01 (2)"]);

    // a folder never offers either
    contextMenu(row("Homelab"));
    expect(menuLabels()).not.toContain("Connect");
    expect(menuLabels()).not.toContain("Open another session");
  });

  test("an_rdp_connection_opens_a_tab_like_any_other", async () => {
    const { store, sessions } = await setup();
    doubleClick(row("DC01"));
    expect(sessions.tabs.map((t) => t.nodeId)).toEqual(["DC01"]);
    expect(store.error).toBeNull();

    contextMenu(row("DC01"));
    item("Connect").click();
    flushSync();
    // the second Connect only brings the existing tab to the front
    expect(sessions.tabs).toHaveLength(1);
    expect(store.error).toBeNull();
  });

  test("row_shows_the_lamp_for_the_strongest_session_state", async () => {
    const { sessions } = await setup();
    expect(row("web-01").querySelector(".lamp")).toBeNull();
    expect(row("pve-01").querySelector(".lamp")).toBeNull();

    sessions.open("web-01");
    flushSync();
    expect(row("web-01").querySelector(".lamp")?.getAttribute("data-state")).toBe("connecting");

    sessions.open("web-01", { another: true });
    sessions.tabs[0]!.state = "connected";
    sessions.tabs[1]!.state = "disconnected";
    flushSync();
    expect(row("web-01").querySelector(".lamp")?.getAttribute("data-state")).toBe("live");

    sessions.tabs[0]!.state = "disconnected";
    flushSync();
    expect(row("web-01").querySelector(".lamp")?.getAttribute("data-state")).toBe("dropped");

    // folders and other connections stay without a lamp
    expect(row("Homelab").querySelector(".lamp")).toBeNull();
    expect(row("pve-01").querySelector(".lamp")).toBeNull();
  });

  test("folder_double_click_still_toggles", async () => {
    const { sessions } = await setup();
    const open = vi.spyOn(sessions, "open");
    expect(row("Homelab").getAttribute("aria-expanded")).toBe("true");
    doubleClick(row("Homelab"));
    expect(row("Homelab").getAttribute("aria-expanded")).toBe("false");
    doubleClick(row("Homelab"));
    expect(row("Homelab").getAttribute("aria-expanded")).toBe("true");
    expect(open).not.toHaveBeenCalled();
  });
});
