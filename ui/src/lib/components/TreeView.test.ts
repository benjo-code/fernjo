import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { TreeStore } from "../store.svelte";
import { apiError, conn, folder, mockApi } from "../testing";
import type { Api, TreeNode } from "../types";
import Sidebar from "./Sidebar.svelte";
import TreeView from "./TreeView.svelte";

let mounted: ReturnType<typeof mount> | null = null;

beforeEach(() => {
  localStorage.clear();
});

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
});

const sample = (): TreeNode[] => [
  folder("Homelab", [conn("pve-01"), conn("DC01", "rdp")]),
  conn("web-01"),
];

async function setup(
  children: TreeNode[] = sample(),
  impl: Partial<Api> = {},
  component: typeof TreeView | typeof Sidebar = TreeView,
) {
  const mock = mockApi(children, impl);
  const store = new TreeStore(mock.api);
  await store.init();
  mounted = mount(component, { target: document.body, props: { store } });
  flushSync();
  return { store, ...mock };
}

const items = () => [...document.querySelectorAll<HTMLElement>('[role="treeitem"]')];
const row = (id: string) => items().find((el) => el.dataset.nodeId === id)!;
const names = () => items().map((el) => el.querySelector(".name")?.textContent);

async function key(el: Element, k: string, init: KeyboardEventInit = {}) {
  el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

describe("TreeView", () => {
  test("renders_treeitems_with_level_and_expanded_state", async () => {
    const { store } = await setup();
    store.expand("Homelab");
    flushSync();
    expect(document.querySelector('[role="tree"]')).not.toBeNull();
    expect(names()).toEqual(["Homelab", "pve-01", "DC01", "web-01"]);
    expect(items().map((el) => el.getAttribute("aria-level"))).toEqual(["1", "2", "2", "1"]);
    expect(row("Homelab").getAttribute("aria-expanded")).toBe("true");
    expect(row("pve-01").hasAttribute("aria-expanded")).toBe(false);
    expect(row("web-01").hasAttribute("aria-expanded")).toBe(false);
  });

  test("click_selects_and_chevron_click_toggles", async () => {
    const { store } = await setup();
    row("web-01").click();
    flushSync();
    expect(store.selectedId).toBe("web-01");
    expect(row("web-01").getAttribute("aria-selected")).toBe("true");
    expect(row("Homelab").getAttribute("aria-selected")).toBe("false");

    expect(row("Homelab").getAttribute("aria-expanded")).toBe("false");
    row("Homelab").querySelector<HTMLElement>(".chevron")!.click();
    flushSync();
    expect(row("Homelab").getAttribute("aria-expanded")).toBe("true");
    expect(names()).toContain("pve-01");
    // the chevron does not change the selection
    expect(store.selectedId).toBe("web-01");
  });

  test("arrow_keys_move_selection_and_expand_collapse", async () => {
    const { store } = await setup();
    const tree = document.querySelector('[role="tree"]')!;
    await key(tree, "ArrowDown");
    expect(store.selectedId).toBe("Homelab");
    await key(row("Homelab"), "ArrowDown");
    expect(store.selectedId).toBe("web-01");
    await key(row("web-01"), "ArrowUp");
    expect(store.selectedId).toBe("Homelab");

    await key(row("Homelab"), "ArrowRight"); // expands
    expect(row("Homelab").getAttribute("aria-expanded")).toBe("true");
    expect(store.selectedId).toBe("Homelab");
    await key(row("Homelab"), "ArrowRight"); // into the first child
    expect(store.selectedId).toBe("pve-01");
    await key(row("pve-01"), "ArrowLeft"); // connection: select its parent
    expect(store.selectedId).toBe("Homelab");
    await key(row("Homelab"), "ArrowLeft"); // collapses
    expect(row("Homelab").getAttribute("aria-expanded")).toBe("false");
    expect(store.selectedId).toBe("Homelab");

    await key(row("Homelab"), "End");
    expect(store.selectedId).toBe("web-01");
    await key(row("web-01"), "Home");
    expect(store.selectedId).toBe("Homelab");
  });

  test("focus_follows_keyboard_selection_with_a_roving_tabindex", async () => {
    await setup();
    const tree = document.querySelector('[role="tree"]')!;
    expect(
      items()
        .filter((el) => el.tabIndex === 0)
        .map((el) => el.dataset.nodeId),
    ).toEqual(["Homelab"]);
    await key(tree, "ArrowDown");
    await key(row("Homelab"), "ArrowDown");
    expect(document.activeElement).toBe(row("web-01"));
    expect(
      items()
        .filter((el) => el.tabIndex === 0)
        .map((el) => el.dataset.nodeId),
    ).toEqual(["web-01"]);
  });

  test("f2_starts_rename_enter_commits_escape_cancels", async () => {
    const { store, calls } = await setup();
    row("web-01").click();
    flushSync();
    await key(row("web-01"), "F2");
    const input = document.querySelector<HTMLInputElement>("input.rename")!;
    expect(input).not.toBeNull();
    expect(input.value).toBe("web-01");
    input.value = "web-02";
    await key(input, "Enter");
    expect(calls.updateNode).toEqual([["web-01", { name: "web-02" }]]);
    expect(document.querySelector("input.rename")).toBeNull();
    expect(store.renamingId).toBeNull();

    await key(row("web-01"), "F2");
    const again = document.querySelector<HTMLInputElement>("input.rename")!;
    again.value = "ignored";
    await key(again, "Escape");
    expect(document.querySelector("input.rename")).toBeNull();
    expect(calls.updateNode).toHaveLength(1);
  });

  test("renaming_to_the_same_name_makes_no_call", async () => {
    const { calls } = await setup();
    row("web-01").click();
    flushSync();
    await key(row("web-01"), "F2");
    const input = document.querySelector<HTMLInputElement>("input.rename")!;
    await key(input, "Enter");
    expect(calls.updateNode).toHaveLength(0);
    expect(document.querySelector("input.rename")).toBeNull();
  });

  test("rename_to_whitespace_reverts_and_shows_error", async () => {
    const { store } = await setup(sample(), {
      updateNode: async () => {
        throw apiError("empty_name", "The name must not be empty");
      },
    });
    row("web-01").click();
    flushSync();
    await key(row("web-01"), "F2");
    const input = document.querySelector<HTMLInputElement>("input.rename")!;
    input.value = "   ";
    await key(input, "Enter");
    expect(store.error).toBe("The name must not be empty");
    expect(document.querySelector("input.rename")).toBeNull();
    expect(names()).toContain("web-01");
  });

  test("markup_in_names_is_shown_literally_and_long_names_keep_full_title", async () => {
    const long = "very-long-name-".repeat(20);
    await setup([conn("<b>x</b>"), conn(long)]);
    const first = row("<b>x</b>");
    expect(first.querySelector(".name")!.textContent).toBe("<b>x</b>");
    expect(first.querySelector("b")).toBeNull();
    expect(first.title).toBe("<b>x</b>");
    expect(row(long).title).toBe(long);
    expect(row(long).querySelector(".name")!.textContent).toBe(long);
  });

  test("empty_tree_offers_new_folder_and_new_connection", async () => {
    const { calls } = await setup([]);
    expect(document.querySelector('[role="treeitem"]')).toBeNull();
    const buttons = [...document.querySelectorAll("button")].map((b) => b.textContent?.trim());
    expect(buttons).toEqual(["New folder", "New connection"]);
    document.querySelectorAll("button")[0].click();
    await tick();
    await new Promise((r) => setTimeout(r, 0));
    expect(calls.createFolder).toEqual([[null, null, "New folder"]]);
    document.querySelectorAll("button")[1].click();
    await tick();
    await new Promise((r) => setTimeout(r, 0));
    expect(calls.createConnection).toEqual([[null, null, "New connection", "ssh", ""]]);
  });

  test("no_empty_state_while_loading_or_after_a_load_failure", async () => {
    const mock = mockApi([], {
      listTree: async () => {
        throw apiError("load_failed", "connections.json is from a newer version");
      },
    });
    const store = new TreeStore(mock.api);
    mounted = mount(TreeView, { target: document.body, props: { store } });
    flushSync();
    expect(document.querySelector("button")).toBeNull(); // not loaded yet
    await store.init();
    flushSync();
    expect(document.querySelector("button")).toBeNull(); // failed: no "New folder" offer
  });

  test("filter_without_matches_says_no_matches", async () => {
    const { store } = await setup();
    store.setFilter("zz[");
    flushSync();
    expect(document.querySelector('[role="treeitem"]')).toBeNull();
    expect(document.body.textContent).toContain("No matches for “zz[”");
  });

  test("filter_box_filters_rows_and_clears_with_escape", async () => {
    const { store } = await setup(sample(), {}, Sidebar);
    const input = document.querySelector<HTMLInputElement>(
      'input[placeholder="Filter connections"]',
    )!;
    expect(input).not.toBeNull();
    input.value = "pve";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    flushSync();
    expect(store.filter).toBe("pve");
    expect(names()).toEqual(["Homelab", "pve-01"]);
    await key(input, "Escape");
    expect(store.filter).toBe("");
    expect(input.value).toBe("");
    expect(names()).toEqual(["Homelab", "web-01"]);
  });

  test("sidebar_shows_load_errors_notices_and_errors", async () => {
    const mock = mockApi([], {
      listTree: async () => ({
        children: [conn("a")],
        notices: [
          { kind: "restoredFromBackup", badFile: "C:\\data\\connections.json.corrupt-17" },
          { kind: "syncConflicts", files: ["connections (conflicted copy).json", "vault (1).fjv"] },
          { kind: "settingsUnreadable", message: "could not read settings.json" },
        ],
      }),
      updateNode: async () => {
        throw apiError("save_failed", "could not write the file");
      },
    });
    const store = new TreeStore(mock.api);
    await store.init();
    mounted = mount(Sidebar, { target: document.body, props: { store } });
    flushSync();
    expect(document.body.textContent).toContain("backup was restored");
    expect(document.body.textContent).toContain("connections.json.corrupt-17");
    expect(document.body.textContent).toContain("Sync conflicts found next to your data");
    expect(document.body.textContent).toContain("connections (conflicted copy).json");
    expect(document.body.textContent).toContain("vault (1).fjv");
    expect(document.body.textContent).toContain("could not read settings.json");
    await store.update("a", { name: "b" });
    flushSync();
    const alert = document.querySelector('[role="alert"]')!;
    expect(alert.textContent).toContain("could not write the file");
    alert.querySelector("button")!.click();
    flushSync();
    expect(document.querySelector('[role="alert"]')).toBeNull();
  });
});
