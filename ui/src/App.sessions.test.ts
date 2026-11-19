import { mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.svelte";
import { apiError, conn, mockApi, preview, snapshot, vaultStatus } from "./lib/testing";
import type { Api, SessionEvent, TreeNode } from "./lib/types";

const h = vi.hoisted(() => ({ made: [] as { calls: string[] }[] }));

vi.mock("./lib/terminal/controller", () => ({
  TerminalController: class {
    calls: string[] = [];
    cols = 80;
    rows = 24;
    constructor() {
      h.made.push(this);
    }
    mount() {}
    setVisible() {}
    fit() {}
    focus() {}
    write() {}
    setOptions() {}
    refreshTheme() {}
    dispose() {
      this.calls.push("dispose");
    }
  },
}));

vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    disconnect() {}
  },
);

let app: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (app) unmount(app);
  app = null;
  document.body.innerHTML = "";
  h.made.length = 0;
});

const settle = () => new Promise((r) => setTimeout(r, 0));

async function start(
  tree: TreeNode[] = [conn("web-01"), conn("DC01", "rdp")],
  impl: Partial<Api> = {},
) {
  const mock = mockApi(tree, {
    vaultStatus: async () => vaultStatus({ state: "unlocked" }),
    ...impl,
  });
  app = mount(App, { target: document.body, props: { api: mock.api } });
  await settle();
  return mock;
}

const row = (id: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="treeitem"]')].find(
    (el) => el.dataset.nodeId === id,
  )!;
const doubleClick = (id: string) =>
  row(id).dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true }));
const tabs = () => document.querySelectorAll('[role="tablist"] [data-tab-id]');
const views = () => [...document.querySelectorAll<HTMLElement>(".view")];

test("opening_a_connection_shows_the_tab_strip_and_a_terminal_view", async () => {
  const { calls } = await start();
  expect(document.querySelector('[role="tablist"]')).toBeNull();
  doubleClick("web-01");
  await settle();
  expect(tabs()).toHaveLength(1);
  expect(views()).toHaveLength(1);
  expect(document.querySelector("main")?.textContent).not.toContain("Open a connection");
  expect(calls.openSession).toHaveLength(1);
  expect(calls.openSession[0]![0]).toBe("web-01");
  expect(document.querySelector("[data-tab-id] .title")?.textContent).toBe("web-01");
});

test("the_vault_view_replaces_the_session_area_while_the_terminals_stay_mounted", async () => {
  await start();
  doubleClick("web-01");
  await settle();
  document.querySelector<HTMLButtonElement>("button[aria-label^='Vault']")!.click();
  await settle();
  expect(document.querySelector("main")?.textContent).toContain("Credentials");
  expect(views()).toHaveLength(1);
  expect(views()[0]!.classList.contains("hidden")).toBe(true);
  expect(h.made[0]!.calls).not.toContain("dispose");

  [...document.querySelectorAll("main button")]
    .find((b) => b.textContent === "Close")
    ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await settle();
  expect(views()[0]!.classList.contains("hidden")).toBe(false);
  expect(h.made).toHaveLength(1);
});

test("a_failed_open_shows_its_message_in_the_error_bar", async () => {
  await start([conn("web-01")], {
    openSession: async () => {
      throw apiError("connect_failed", "Could not reach web-01.lab");
    },
  });
  doubleClick("web-01");
  await settle();
  await settle();
  expect(document.querySelector(".bar.error")?.textContent).toContain("Could not reach web-01.lab");
  expect(tabs()).toHaveLength(0);
});

test("pagehide_ends_every_session", async () => {
  const { calls } = await start();
  doubleClick("web-01");
  await settle();
  expect(calls.closeSession).toHaveLength(0);
  window.dispatchEvent(new Event("pagehide"));
  await settle();
  expect(calls.closeSession).toEqual([["sid"]]);
  expect(tabs()).toHaveLength(0);
});

test("beforeunload_ends_every_session_too", async () => {
  const { calls } = await start();
  doubleClick("web-01");
  await settle();
  window.dispatchEvent(new Event("beforeunload"));
  await settle();
  expect(calls.closeSession).toEqual([["sid"]]);
});

test("the_terminal_settings_come_from_the_api_with_a_fallback", async () => {
  const { calls } = await start([conn("web-01")], {
    getTerminalSettings: async () => {
      throw apiError("internal", "nope");
    },
  });
  expect(calls.getTerminalSettings).toHaveLength(1);
  doubleClick("web-01");
  await settle();
  expect(views()).toHaveLength(1);
});

const chip = () => document.querySelector<HTMLButtonElement>("button[aria-label^='Vault']")!;
const manager = () => document.querySelector("section.manager");
const tabButton = (i: number) =>
  document.querySelectorAll<HTMLElement>("[data-tab-id] [role=tab]")[i]!;
const visibleViews = () => views().filter((v) => !v.classList.contains("hidden"));

async function openVault() {
  chip().click();
  await settle();
  expect(manager()).not.toBeNull();
}

test("opening_a_connection_while_the_vault_manager_is_open_shows_the_new_tab", async () => {
  await start();
  await openVault();
  doubleClick("web-01");
  await settle();
  expect(manager()).toBeNull();
  expect(visibleViews()).toHaveLength(1);
  expect(tabs()).toHaveLength(1);
});

test("activating_a_tab_or_reopening_the_active_one_leaves_the_vault_view", async () => {
  await start([conn("web-01"), conn("db-01")]);
  doubleClick("web-01");
  await settle();
  doubleClick("db-01");
  await settle();
  // a click on another tab
  await openVault();
  tabButton(0).click();
  await settle();
  expect(manager()).toBeNull();
  expect(tabButton(0).getAttribute("aria-selected")).toBe("true");
  expect(visibleViews()).toHaveLength(1);
  // a click on the tab that is already active
  await openVault();
  tabButton(0).click();
  await settle();
  expect(manager()).toBeNull();
  // double-click on a connection whose tab is already active
  await openVault();
  doubleClick("web-01");
  await settle();
  expect(manager()).toBeNull();
  expect(tabs()).toHaveLength(2);
});

test("opening_the_vault_manager_with_tabs_hides_the_session_area_and_keeps_it_hidden", async () => {
  await start();
  doubleClick("web-01");
  await settle();
  await openVault();
  await settle();
  // the unchanged active tab must not close the manager again
  expect(manager()).not.toBeNull();
  expect(visibleViews()).toHaveLength(0);
  expect(views()).toHaveLength(1);
  expect(h.made[0]!.calls).not.toContain("dispose");
});

test("closing_a_tab_while_the_vault_manager_is_open_keeps_the_vault_view", async () => {
  await start([conn("web-01"), conn("db-01")]);
  doubleClick("web-01");
  await settle();
  doubleClick("db-01");
  await settle();
  await openVault();
  // closing the active tab changes the active id, closing the other one does not
  for (const label of ["db-01", "web-01"]) {
    document.querySelector<HTMLButtonElement>(`button[aria-label='Close ${label}']`)!.click();
    await settle();
    // a connected tab asks first
    [...document.querySelectorAll<HTMLButtonElement>("[role=alertdialog] button")]
      .find((b) => b.textContent?.trim() === "Close")
      ?.click();
    await settle();
    expect(manager()).not.toBeNull();
  }
  expect(tabs()).toHaveLength(0);
});

// --- app shortcuts ---------------------------------------------------------------------------

const handlers: ((e: SessionEvent | ArrayBuffer) => void)[] = [];

/** A started app whose sessions report "connected" as soon as they open. */
async function startConnected(tree: TreeNode[] = [conn("web-01"), conn("db-01")]) {
  handlers.length = 0;
  const mock = await start(tree, {
    openSession: async (_id, onEvent) => {
      handlers.push(onEvent);
      queueMicrotask(() =>
        onEvent({
          kind: "state",
          state: "connected",
          detail: null,
          exitStatus: null,
          endedCleanly: false,
        }),
      );
      return `sid-${handlers.length}`;
    },
  });
  return mock;
}

function press(init: KeyboardEventInit, target: EventTarget = document.body): KeyboardEvent {
  const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e;
}

/** An element the terminal controller would own: a keydown from it counts as "in the terminal". */
function xtermIn(view: HTMLElement): HTMLElement {
  const el = document.createElement("textarea");
  el.className = "xterm-helper-textarea";
  const host = document.createElement("div");
  host.className = "xterm";
  host.append(el);
  view.append(host);
  return el;
}

const activeTitle = () =>
  document.querySelector("[role=tab][aria-selected=true] .title")?.textContent ?? null;
const confirm = () => document.querySelector<HTMLElement>("[role=alertdialog]");

test("ctrl_shift_w_closes_the_active_tab_with_confirm_for_a_connected_one", async () => {
  const { calls } = await startConnected();
  doubleClick("web-01");
  await settle();
  doubleClick("db-01");
  await settle();
  expect(tabs()).toHaveLength(2);

  // connected: asks first, Cancel has focus, Escape keeps the tab
  const e = press({ key: "W", ctrlKey: true, shiftKey: true });
  await settle();
  expect(e.defaultPrevented).toBe(true);
  expect(confirm()).not.toBeNull();
  expect(confirm()!.textContent).toContain("db-01");
  expect(document.activeElement?.textContent?.trim()).toBe("Cancel");
  confirm()!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  await settle();
  expect(confirm()).toBeNull();
  expect(tabs()).toHaveLength(2);
  expect(calls.closeSession).toHaveLength(0);

  // confirmed: closes the active tab only
  press({ key: "W", ctrlKey: true, shiftKey: true });
  await settle();
  [...confirm()!.querySelectorAll("button")]
    .find((b) => b.textContent?.trim() === "Close")!
    .click();
  await settle();
  expect(tabs()).toHaveLength(1);
  expect(activeTitle()).toBe("web-01");
  expect(calls.closeSession).toEqual([["sid-2"]]);

  // a tab that is not connected closes at once
  document.querySelector<HTMLButtonElement>("button[aria-label='Close web-01']")!.click();
  await settle();
  confirm()?.querySelector("button")?.click();
});

test("a_tab_that_is_not_connected_closes_at_once_and_no_tab_means_no_effect", async () => {
  await start([conn("web-01")]);
  // no active tab: not handled, not prevented
  const idle = press({ key: "W", ctrlKey: true, shiftKey: true });
  expect(idle.defaultPrevented).toBe(false);
  doubleClick("web-01");
  await settle();
  // the mock session never reports connected, so the tab is still "connecting"
  const e = press({ key: "W", ctrlKey: true, shiftKey: true });
  await settle();
  expect(e.defaultPrevented).toBe(true);
  expect(confirm()).toBeNull();
  expect(tabs()).toHaveLength(0);
});

test("ctrl_tab_cycles_tabs_and_plain_ctrl_w_inside_the_terminal_is_untouched", async () => {
  const { calls } = await startConnected([conn("a"), conn("b"), conn("c")]);
  for (const id of ["a", "b", "c"]) {
    doubleClick(id);
    await settle();
  }
  expect(activeTitle()).toBe("c");
  const inside = xtermIn(views()[2]!);

  const next = press({ key: "Tab", ctrlKey: true }, inside);
  await settle();
  expect(next.defaultPrevented).toBe(true);
  expect(activeTitle()).toBe("a");
  press({ key: "Tab", ctrlKey: true, shiftKey: true }, inside);
  await settle();
  expect(activeTitle()).toBe("c");

  // plain Ctrl+W belongs to the shell: neither handled nor prevented
  const plain = press({ key: "w", ctrlKey: true }, inside);
  await settle();
  expect(plain.defaultPrevented).toBe(false);
  expect(tabs()).toHaveLength(3);
  expect(confirm()).toBeNull();

  // Ctrl+Shift+C and +F are the controller's: the app handler leaves them alone
  const copy = press({ key: "C", ctrlKey: true, shiftKey: true }, inside);
  const find = press({ key: "F", ctrlKey: true, shiftKey: true }, inside);
  expect(copy.defaultPrevented).toBe(false);
  expect(find.defaultPrevented).toBe(false);

  // none of the app chords reaches the shell as input
  press({ key: "W", ctrlKey: true, shiftKey: true }, inside);
  press({ key: "K", ctrlKey: true, shiftKey: true }, inside);
  await settle();
  expect(calls.sessionInput).toHaveLength(0);
});

test("ctrl_shift_k_focuses_the_filter", async () => {
  await startConnected([conn("web-01")]);
  doubleClick("web-01");
  await settle();
  const inside = xtermIn(views()[0]!);
  inside.focus();
  const e = press({ key: "K", ctrlKey: true, shiftKey: true }, inside);
  await settle();
  expect(e.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(
    document.querySelector("input[aria-label='Filter connections']"),
  );
  // plain Ctrl+K stays with the page
  const plain = press({ key: "k", ctrlKey: true });
  expect(plain.defaultPrevented).toBe(false);
});

test("saving_terminal_settings_reaches_open_terminals", async () => {
  await startConnected([conn("web-01")]);
  doubleClick("web-01");
  await settle();
  const seen: number[] = [];
  const proto = h.made[0] as unknown as { setOptions: (o: { fontSize: number }) => void };
  proto.setOptions = (o) => seen.push(o.fontSize);
  document.querySelector<HTMLButtonElement>("button[aria-label='Settings']")!.click();
  await settle();
  const size = document.querySelector<HTMLInputElement>("input[aria-label='Font size']")!;
  size.value = "20";
  size.dispatchEvent(new Event("input", { bubbles: true }));
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === "Save")!.click();
  await settle();
  expect(seen).toContain(20);
});

test("shortcuts_do_nothing_while_the_settings_dialog_is_open", async () => {
  const { calls } = await startConnected();
  doubleClick("web-01");
  await settle();
  doubleClick("db-01");
  await settle();
  document.querySelector<HTMLButtonElement>("button[aria-label='Settings']")!.click();
  await settle();
  const close = press({ key: "W", ctrlKey: true, shiftKey: true });
  const filter = press({ key: "K", ctrlKey: true, shiftKey: true });
  const next = press({ key: "Tab", ctrlKey: true });
  await settle();
  expect(close.defaultPrevented).toBe(false);
  expect(filter.defaultPrevented).toBe(false);
  expect(next.defaultPrevented).toBe(false);
  expect(confirm()).toBeNull();
  expect(activeTitle()).toBe("db-01");
  expect(document.activeElement).not.toBe(
    document.querySelector("input[aria-label='Filter connections']"),
  );
  expect(calls.closeSession).toHaveLength(0);
});

test("ctrl_tab_and_ctrl_shift_w_do_nothing_while_the_close_confirm_is_open", async () => {
  await startConnected();
  doubleClick("web-01");
  await settle();
  doubleClick("db-01");
  await settle();
  press({ key: "W", ctrlKey: true, shiftKey: true });
  await settle();
  expect(confirm()!.textContent).toContain("db-01");
  press({ key: "Tab", ctrlKey: true });
  press({ key: "W", ctrlKey: true, shiftKey: true });
  await settle();
  expect(activeTitle()).toBe("db-01");
  expect(confirm()!.textContent).toContain("db-01");
});

test("a_held_ctrl_shift_w_closes_one_tab_only", async () => {
  await start([conn("a"), conn("b"), conn("c")]);
  for (const id of ["a", "b", "c"]) {
    doubleClick(id);
    await settle();
  }
  const first = press({ key: "W", ctrlKey: true, shiftKey: true });
  const held = press({ key: "W", ctrlKey: true, shiftKey: true, repeat: true });
  await settle();
  expect(first.defaultPrevented).toBe(true);
  expect(held.defaultPrevented).toBe(true);
  expect(tabs()).toHaveLength(2);
});

test("a_restore_closes_every_tab_and_the_lamps_go_out", async () => {
  // Rust closed the sessions without telling the page; the tabs must not stay "connected"
  const { calls } = await start([conn("web-01")], {
    importPick: async () => ({ fileName: "b.fjbk" }),
    importInspect: async () =>
      preview({
        kind: "backup",
        fileName: "b.fjbk",
        report: null,
        backup: {
          createdAt: "2026-11-01T10:00:00Z",
          app: "fernjo 0.5.0",
          folders: 0,
          connections: 1,
          credentials: 0,
        },
      }),
    importApply: async () => ({
      // the restored tree still has the connection, so a stale lamp would show next to it
      snapshot: snapshot([conn("web-01")]),
      report: null,
      restored: { createdAt: "2026-11-01T10:00:00Z", folders: 0, connections: 1, credentials: 0 },
    }),
  });
  doubleClick("web-01");
  await settle();
  expect(tabs()).toHaveLength(1);
  expect(row("web-01").querySelector(".lamp")).not.toBeNull();
  expect(calls.closeSession).toHaveLength(0);

  const click = async (label: string) => {
    [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!.click();
    await settle();
  };
  document.querySelector<HTMLButtonElement>("button[aria-label='Import & backup']")!.click();
  await settle();
  await click("Backup file…");
  await click("Restore…");
  await click("Restore");

  expect(calls.importApply).toHaveLength(1);
  expect(tabs()).toHaveLength(0);
  expect(row("web-01").querySelector(".lamp")).toBeNull();
  expect(calls.closeSession).toEqual([["sid"]]);
});
