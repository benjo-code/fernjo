import { mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.svelte";
import { apiError, conn, mockApi, vaultStatus } from "./lib/testing";
import type { Api, TreeNode } from "./lib/types";

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
