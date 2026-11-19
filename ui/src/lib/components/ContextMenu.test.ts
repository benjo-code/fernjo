import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { TreeStore } from "../store.svelte";
import { conn, folder, mockApi, snapshot } from "../testing";
import type { Api, TreeNode } from "../types";
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

// Homelab holds 4 items: pve-01, DC01, Sub and Sub's c1
const sample = (): TreeNode[] => [
  folder("Homelab", [conn("pve-01"), conn("DC01", "rdp"), folder("Sub", [conn("c1")])]),
  conn("web-01"),
];

async function setup(impl: Partial<Api> = {}) {
  const mock = mockApi(sample(), impl);
  const store = new TreeStore(mock.api);
  await store.init();
  store.expand("Homelab");
  mounted = mount(TreeView, {
    target: document.body,
    props: {
      store,
      sessions: new SessionStore(
        mock.api,
        (id) => id,
        () => true,
      ),
    },
  });
  flushSync();
  return { store, ...mock };
}

const row = (id: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="treeitem"]')].find(
    (el) => el.dataset.nodeId === id,
  )!;
const menu = () => document.querySelector<HTMLElement>('[role="menu"]');
const menuLabels = () =>
  [...document.querySelectorAll('[role="menuitem"]')].map((el) => el.textContent?.trim());
const item = (label: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find(
    (el) => el.textContent?.trim() === label,
  )!;
const dialog = () => document.querySelector<HTMLElement>('[role="alertdialog"]');
const button = (label: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="alertdialog"] button')].find(
    (el) => el.textContent?.trim() === label,
  )!;

function contextMenu(el: Element, x = 20, y = 30) {
  el.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: x, clientY: y }),
  );
  flushSync();
}

async function settle() {
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

async function key(el: Element, k: string, init: KeyboardEventInit = {}) {
  el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
  await settle();
}

describe("context menu", () => {
  test("row_context_menu_lists_items_in_order_and_selects_the_row", async () => {
    const { store } = await setup();
    contextMenu(row("web-01"));
    expect(menuLabels()).toEqual([
      "Connect",
      "New folder",
      "New connection",
      "Duplicate",
      "Rename",
      "Delete",
    ]);
    expect(store.selectedId).toBe("web-01");
  });

  test("empty_space_menu_only_offers_new_folder_and_new_connection_at_root", async () => {
    const { calls } = await setup();
    contextMenu(document.querySelector('[role="tree"]')!);
    expect(menuLabels()).toEqual(["New folder", "New connection"]);
    item("New folder").click();
    await settle();
    expect(calls.createFolder).toEqual([[null, null, "New folder"]]);
    expect(menu()).toBeNull();
  });

  test("new_folder_on_folder_appends_inside_and_starts_rename", async () => {
    const after = sample();
    (after[0] as Extract<TreeNode, { type: "folder" }>).children.push(folder("New folder"));
    const { store, calls } = await setup({
      createFolder: async () => ({ id: "New folder", snapshot: snapshot(after) }),
    });
    contextMenu(row("Homelab"));
    item("New folder").click();
    await settle();
    expect(calls.createFolder).toEqual([["Homelab", null, "New folder"]]);
    expect(store.renamingId).toBe("New folder");
    expect(document.querySelector("input.rename")).not.toBeNull();
    expect(row("Homelab").getAttribute("aria-expanded")).toBe("true");
  });

  test("new_connection_on_connection_lands_after_it_in_the_same_parent", async () => {
    const { calls } = await setup();
    contextMenu(row("pve-01"));
    item("New connection").click();
    await settle();
    expect(calls.createConnection).toEqual([["Homelab", 1, "New connection", "ssh", ""]]);
  });

  test("new_folder_on_top_level_connection_lands_after_it_at_the_top", async () => {
    const { calls } = await setup();
    contextMenu(row("web-01"));
    item("New folder").click();
    await settle();
    expect(calls.createFolder).toEqual([[null, 2, "New folder"]]);
  });

  test("duplicate_selects_the_copy", async () => {
    const after = [...sample(), conn("web-01 copy")];
    const { store, calls } = await setup({
      duplicateNode: async () => ({ id: "web-01 copy", snapshot: snapshot(after) }),
    });
    contextMenu(row("web-01"));
    item("Duplicate").click();
    await settle();
    expect(calls.duplicateNode).toEqual([["web-01"]]);
    expect(store.selectedId).toBe("web-01 copy");
  });

  test("rename_item_starts_inline_rename", async () => {
    const { store } = await setup();
    contextMenu(row("web-01"));
    item("Rename").click();
    await settle();
    expect(store.renamingId).toBe("web-01");
    expect(document.querySelector("input.rename")).not.toBeNull();
  });

  test("delete_asks_with_descendant_count", async () => {
    await setup();
    contextMenu(row("Homelab"));
    item("Delete").click();
    await settle();
    expect(dialog()!.querySelector(".title")!.textContent).toBe(
      "Delete “Homelab” and 4 items inside?",
    );
    button("Cancel").click();
    await settle();
    expect(dialog()).toBeNull();

    contextMenu(row("web-01"));
    item("Delete").click();
    await settle();
    expect(dialog()!.querySelector(".title")!.textContent).toBe("Delete “web-01”?");
  });

  test("a_folder_with_one_item_says_item_not_items", async () => {
    await setup();
    contextMenu(row("Sub"));
    item("Delete").click();
    await settle();
    expect(dialog()!.querySelector(".title")!.textContent).toBe("Delete “Sub” and 1 item inside?");
  });

  test("confirm_calls_delete_and_cancel_or_escape_does_not", async () => {
    const { calls } = await setup();
    contextMenu(row("web-01"));
    item("Delete").click();
    await settle();
    button("Cancel").click();
    await settle();
    expect(calls.deleteNode).toHaveLength(0);

    contextMenu(row("web-01"));
    item("Delete").click();
    await settle();
    await key(dialog()!, "Escape");
    expect(dialog()).toBeNull();
    expect(calls.deleteNode).toHaveLength(0);

    contextMenu(row("web-01"));
    item("Delete").click();
    await settle();
    button("Delete").click();
    await settle();
    expect(calls.deleteNode).toEqual([["web-01"]]);
    expect(dialog()).toBeNull();
  });

  test("enter_on_dialog_does_not_confirm_by_default_focus_is_cancel", async () => {
    const { calls } = await setup();
    contextMenu(row("web-01"));
    item("Delete").click();
    await settle();
    expect(document.activeElement).toBe(button("Cancel"));
    await key(dialog()!, "Enter");
    expect(calls.deleteNode).toHaveLength(0);
  });

  test("tab_stays_inside_the_dialog", async () => {
    await setup();
    contextMenu(row("web-01"));
    item("Delete").click();
    await settle();
    await key(button("Cancel"), "Tab");
    expect(document.activeElement).toBe(button("Delete"));
    await key(button("Delete"), "Tab");
    expect(document.activeElement).toBe(button("Cancel"));
    await key(button("Cancel"), "Tab", { shiftKey: true });
    expect(document.activeElement).toBe(button("Delete"));
  });

  test("delete_key_on_selected_row_opens_the_dialog", async () => {
    const { store } = await setup();
    store.select("web-01");
    flushSync();
    await key(row("web-01"), "Delete");
    expect(dialog()).not.toBeNull();
    expect(dialog()!.querySelector(".title")!.textContent).toBe("Delete “web-01”?");
  });

  test("menu_closes_on_escape_outside_click_and_after_an_action", async () => {
    await setup();
    contextMenu(row("web-01"));
    expect(menu()).not.toBeNull();
    await key(menu()!, "Escape");
    expect(menu()).toBeNull();

    contextMenu(row("web-01"));
    expect(menu()).not.toBeNull();
    document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    flushSync();
    expect(menu()).toBeNull();

    contextMenu(row("web-01"));
    item("Rename").click();
    await settle();
    expect(menu()).toBeNull();
  });

  test("a_click_inside_the_menu_does_not_close_it_before_the_action_runs", async () => {
    const { calls } = await setup();
    contextMenu(row("web-01"));
    const target = item("Duplicate");
    target.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
    flushSync();
    expect(menu()).not.toBeNull();
    target.click();
    await settle();
    expect(calls.duplicateNode).toHaveLength(1);
  });

  test("menu_stays_inside_the_viewport", async () => {
    await setup();
    Object.defineProperty(window, "innerWidth", { value: 400, configurable: true });
    Object.defineProperty(window, "innerHeight", { value: 300, configurable: true });
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: 160,
      height: 100,
      top: 0,
      left: 0,
      right: 160,
      bottom: 100,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    });
    contextMenu(row("web-01"), 390, 290);
    await settle();
    expect(parseFloat(menu()!.style.left)).toBeLessThanOrEqual(240);
    expect(parseFloat(menu()!.style.top)).toBeLessThanOrEqual(200);
    expect(parseFloat(menu()!.style.left)).toBeGreaterThanOrEqual(0);
  });

  test("arrow_keys_move_through_items_and_enter_activates", async () => {
    const { calls } = await setup();
    // a folder row: five items, "New folder" first (connections start with "Connect")
    contextMenu(row("Homelab"));
    await settle();
    expect(document.activeElement).toBe(item("New folder"));
    await key(menu()!, "ArrowDown");
    expect(document.activeElement).toBe(item("New connection"));
    await key(menu()!, "ArrowUp");
    await key(menu()!, "ArrowUp"); // wraps to the last item
    expect(document.activeElement).toBe(item("Delete"));
    await key(menu()!, "ArrowDown"); // wraps to the first
    await key(menu()!, "ArrowDown");
    await key(menu()!, "ArrowDown");
    expect(document.activeElement).toBe(item("Duplicate"));
    await key(document.activeElement!, "Enter");
    expect(calls.duplicateNode).toHaveLength(1);
    expect(menu()).toBeNull();
  });

  test("shift_f10_and_the_context_menu_key_open_the_menu_for_the_selected_row", async () => {
    const { store } = await setup();
    store.select("web-01");
    flushSync();
    await key(row("web-01"), "F10", { shiftKey: true });
    expect(menuLabels()).toHaveLength(6);
    await key(menu()!, "Escape");
    await key(row("web-01"), "ContextMenu");
    expect(menuLabels()).toHaveLength(6);
  });

  test("focus_returns_to_the_row_after_rename_commit_and_cancel", async () => {
    await setup();
    contextMenu(row("web-01"));
    item("Rename").click();
    await settle();
    const input = document.querySelector<HTMLInputElement>("input.rename")!;
    input.value = "web-02";
    await key(input, "Enter");
    expect(document.activeElement).toBe(row("web-01"));

    contextMenu(row("web-01"));
    item("Rename").click();
    await settle();
    await key(document.querySelector("input.rename")!, "Escape");
    expect(document.activeElement).toBe(row("web-01"));
  });

  test("focus_is_not_stolen_when_a_rename_ends_because_the_user_clicked_elsewhere", async () => {
    await setup();
    const other = document.createElement("button");
    document.body.append(other);
    contextMenu(row("web-01"));
    item("Rename").click();
    await settle();
    other.focus();
    document.querySelector("input.rename")!.dispatchEvent(new FocusEvent("blur"));
    await settle();
    expect(document.activeElement).toBe(other);
  });

  test("after_deleting_the_neighbour_is_selected_and_focused", async () => {
    const { store, setTree } = await setup();
    // rows: Homelab, pve-01, DC01, Sub (collapsed), web-01
    store.select("web-01");
    flushSync();
    setTree([
      folder("Homelab", [conn("pve-01"), conn("DC01", "rdp"), folder("Sub", [conn("c1")])]),
    ]);
    await key(row("web-01"), "Delete");
    button("Delete").click();
    await settle();
    await settle();
    expect(store.selectedId).toBe("Sub");
    expect(document.activeElement).toBe(row("Sub"));
  });

  test("deleting_a_folder_selects_the_next_row_after_its_subtree", async () => {
    const { store, setTree } = await setup();
    setTree([conn("web-01")]);
    store.select("Homelab");
    flushSync();
    await key(row("Homelab"), "Delete");
    button("Delete").click();
    await settle();
    await settle();
    expect(store.selectedId).toBe("web-01");
    expect(document.activeElement).toBe(row("web-01"));
  });
});
