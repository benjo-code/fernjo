import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { TreeStore } from "../store.svelte";
import { apiError, conn, folder, mockApi, snapshot } from "../testing";
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
  // jsdom has no layout engine; tests install their own elementFromPoint
  delete (document as { elementFromPoint?: unknown }).elementFromPoint;
});

// root: Homelab (open: pve-01, DC01), web-01
const sample = (): TreeNode[] => [
  folder("Homelab", [conn("pve-01"), conn("DC01", "rdp")]),
  conn("web-01"),
];

async function setup(impl: Partial<Api> = {}) {
  const mock = mockApi(sample(), impl);
  const store = new TreeStore(mock.api);
  await store.init();
  store.expand("Homelab");
  mounted = mount(TreeView, {
    target: document.body,
    props: { store, sessions: new SessionStore(mock.api, (id) => id) },
  });
  flushSync();
  return { store, ...mock };
}

const tree = () => document.querySelector<HTMLElement>('[role="tree"]')!;
const row = (id: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="treeitem"]')].find(
    (el) => el.dataset.nodeId === id,
  )!;

/** Pretends every row is 24 px high, stacked from y = 0, and that `over` is under the pointer. */
function layout(over: () => Element | null) {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    const rows = [...document.querySelectorAll('[role="treeitem"]')];
    const i = rows.indexOf(this);
    const top = i < 0 ? 0 : i * 24;
    return {
      top,
      bottom: top + 24,
      left: 0,
      right: 300,
      width: 300,
      height: 24,
      x: 0,
      y: top,
    } as DOMRect;
  });
  (document as { elementFromPoint?: unknown }).elementFromPoint = () => over();
}

function pointer(type: string, el: Element, x: number, y: number, init: PointerEventInit = {}) {
  const Ctor = typeof PointerEvent === "function" ? PointerEvent : MouseEvent;
  el.dispatchEvent(
    new Ctor(type, {
      bubbles: true,
      cancelable: true,
      clientX: x,
      clientY: y,
      button: 0,
      buttons: type === "pointermove" ? 1 : 0,
      ...init,
    }),
  );
  flushSync();
}

async function settle() {
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  flushSync();
}

describe("pointer drag and drop", () => {
  test("dragging_a_connection_onto_a_folder_moves_it_inside", async () => {
    const after = [folder("Homelab", [conn("pve-01"), conn("DC01", "rdp"), conn("web-01")])];
    const { calls } = await setup({
      moveNode: async (id) => ({ id, snapshot: snapshot(after) }),
    });
    let over: Element | null = row("Homelab");
    layout(() => over);
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointermove", tree(), 50, 12); // middle of the Homelab row = inside
    expect(row("Homelab").classList.contains("drop-inside")).toBe(true);
    pointer("pointerup", tree(), 50, 12);
    await settle();
    expect(calls.moveNode).toEqual([["web-01", "Homelab", 2]]);
    over = null;
  });

  test("an_insertion_line_shows_before_and_after_a_connection", async () => {
    await setup();
    layout(() => row("pve-01")); // rows: Homelab 0-24, pve-01 24-48
    pointer("pointerdown", row("web-01"), 50, 100);
    pointer("pointermove", tree(), 50, 26); // upper half of pve-01
    expect(row("pve-01").classList.contains("drop-before")).toBe(true);
    pointer("pointermove", tree(), 50, 46); // lower half
    expect(row("pve-01").classList.contains("drop-after")).toBe(true);
  });

  test("a_click_with_a_tiny_wobble_selects_and_never_moves", async () => {
    const { store, calls } = await setup();
    layout(() => row("Homelab"));
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointermove", tree(), 52, 61);
    pointer("pointerup", tree(), 52, 61);
    row("web-01").click();
    flushSync();
    expect(store.selectedId).toBe("web-01");
    expect(calls.moveNode).toHaveLength(0);
  });

  test("the_click_after_a_real_drag_does_not_change_the_selection", async () => {
    const { store } = await setup();
    store.select("pve-01");
    layout(() => row("Homelab"));
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointermove", tree(), 50, 12);
    pointer("pointerup", tree(), 50, 12);
    row("web-01").click();
    flushSync();
    expect(store.selectedId).toBe("pve-01");
  });

  test("dropping_a_folder_on_itself_or_a_descendant_calls_nothing", async () => {
    const { calls } = await setup();
    let over: Element = row("Homelab");
    layout(() => over);
    pointer("pointerdown", row("Homelab"), 50, 12);
    pointer("pointermove", tree(), 50, 14);
    pointer("pointerup", tree(), 50, 12); // on itself
    over = row("pve-01");
    pointer("pointerdown", row("Homelab"), 50, 12);
    pointer("pointermove", tree(), 50, 36);
    pointer("pointerup", tree(), 50, 36); // onto its own child
    await settle();
    expect(calls.moveNode).toHaveLength(0);
  });

  test("releasing_outside_the_tree_calls_nothing", async () => {
    const { calls } = await setup();
    layout(() => document.body);
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointermove", tree(), 500, 400);
    pointer("pointerup", tree(), 500, 400);
    await settle();
    expect(calls.moveNode).toHaveLength(0);
    expect(document.querySelector(".drop-inside, .drop-before, .drop-after")).toBeNull();
  });

  test("escape_mid_drag_cancels", async () => {
    const { calls } = await setup();
    layout(() => row("Homelab"));
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointermove", tree(), 50, 12);
    expect(row("Homelab").classList.contains("drop-inside")).toBe(true);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    flushSync();
    expect(document.querySelector(".drop-inside")).toBeNull();
    pointer("pointerup", tree(), 50, 12);
    await settle();
    expect(calls.moveNode).toHaveLength(0);
  });

  test("dragging_is_off_while_a_filter_is_active", async () => {
    const { store, calls } = await setup();
    store.setFilter("e");
    flushSync();
    layout(() => row("Homelab"));
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointermove", tree(), 50, 12);
    pointer("pointerup", tree(), 50, 12);
    await settle();
    expect(calls.moveNode).toHaveLength(0);
  });

  test("dragging_is_off_while_renaming_and_from_the_chevron_and_with_other_buttons", async () => {
    const { store, calls } = await setup();
    layout(() => row("web-01"));
    const chevron = row("Homelab").querySelector(".chevron")!;
    pointer("pointerdown", chevron, 50, 12);
    pointer("pointermove", tree(), 50, 60);
    pointer("pointerup", tree(), 50, 60);

    pointer("pointerdown", row("pve-01"), 50, 36, { button: 2 });
    pointer("pointermove", tree(), 50, 60);
    pointer("pointerup", tree(), 50, 60);

    store.startRename("web-01");
    flushSync();
    pointer("pointerdown", row("pve-01"), 50, 36);
    pointer("pointermove", tree(), 50, 60);
    pointer("pointerup", tree(), 50, 60);
    await settle();
    expect(calls.moveNode).toHaveLength(0);
  });

  test("a_rust_rejection_shows_the_standard_error", async () => {
    const { store } = await setup({
      moveNode: async () => {
        throw apiError("invalid_move", "A folder cannot be moved into itself");
      },
    });
    layout(() => row("Homelab"));
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointermove", tree(), 50, 12);
    pointer("pointerup", tree(), 50, 12);
    await settle();
    expect(store.error).toBe("A folder cannot be moved into itself");
  });

  test("a_press_released_outside_the_tree_never_becomes_a_ghost_drag", async () => {
    const { calls } = await setup();
    layout(() => row("Homelab"));
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointerup", document.body, 600, 300); // released over the main area
    pointer("pointermove", tree(), 50, 12, { buttons: 0 }); // hovering later, no button
    pointer("pointerup", tree(), 50, 12);
    pointer("pointerdown", row("pve-01"), 50, 36, { button: 2 });
    pointer("pointerup", tree(), 50, 36, { button: 2 }); // a right-click
    await settle();
    expect(calls.moveNode).toHaveLength(0);
  });

  test("a_missed_release_cancels_the_drag_when_no_button_is_down_any_more", async () => {
    const { calls } = await setup();
    layout(() => row("Homelab"));
    pointer("pointerdown", row("web-01"), 50, 60);
    pointer("pointermove", tree(), 50, 12);
    expect(row("Homelab").classList.contains("drop-inside")).toBe(true);
    pointer("pointermove", tree(), 50, 14, { buttons: 0 });
    expect(document.querySelector(".drop-inside")).toBeNull();
    pointer("pointerup", tree(), 50, 14);
    await settle();
    expect(calls.moveNode).toHaveLength(0);
  });
});
