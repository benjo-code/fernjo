import { flushSync } from "svelte";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { OpenTabsStore, SAVE_DELAY_MS } from "./open-tabs.svelte";
import { SessionStore } from "./sessions.svelte";
import { deferred, mockApi, openTabsInfo } from "./testing";
import type { OpenTabsInfo, QuickTarget, RestorableTab } from "./types";

const quick: QuickTarget = { protocol: "ssh", host: "h.example", port: 2222, user: "root" };

const nodeTab = (id: string, title = id): RestorableTab => ({
  target: { node: id },
  protocol: "ssh",
  title,
  usesVault: false,
});

let stores: OpenTabsStore[] = [];

function setup(info: OpenTabsInfo = openTabsInfo(), impl: Parameters<typeof mockApi>[1] = {}) {
  const m = mockApi([], { getOpenTabs: async () => info, ...impl });
  const sessions = new SessionStore(
    m.api,
    (id) => `name-${id}`,
    () => true,
  );
  const openTabs = new OpenTabsStore(m.api, sessions);
  stores.push(openTabs);
  return { ...m, sessions, openTabs };
}

/** Lets the effect run, then the debounce pass. */
const settle = (ms = SAVE_DELAY_MS) => {
  flushSync();
  vi.advanceTimersByTime(ms);
};

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  for (const s of stores) s.dispose();
  stores = [];
  vi.useRealTimers();
});

describe("OpenTabsStore", () => {
  test("after_a_failed_install_the_tabs_come_back_disconnected_and_nothing_is_saved", async () => {
    const t = setup();
    await t.openTabs.load();
    t.sessions.open("a");
    t.sessions.open("b");
    t.sessions.activate(t.sessions.tabs[0]!.id);
    settle(1000);
    const saves = t.calls.saveOpenTabs.length;
    const before = t.sessions.tabs.map((tab) => tab.id);

    t.sessions.endedByRust();
    settle(1000);

    const tabs = t.sessions.tabs;
    expect(tabs.map((tab) => [tab.nodeId, tab.title])).toEqual([
      ["a", "name-a"],
      ["b", "name-b"],
    ]);
    // new tabs, so their views attach again and ask Rust for a disconnected session
    expect(tabs.some((tab) => before.includes(tab.id))).toBe(false);
    expect(tabs.every((tab) => tab.state === "disconnected" && tab.restored)).toBe(true);
    expect(t.sessions.activeId).toBe(tabs[0]!.id);
    expect(t.calls.saveOpenTabs).toHaveLength(saves);
  });

  test("nothing_is_saved_before_the_restore_has_finished", async () => {
    const answer = deferred<OpenTabsInfo>();
    const t = setup(undefined, { getOpenTabs: () => answer.promise });
    const loading = t.openTabs.load();
    // the user acts while Rust has not answered yet
    t.sessions.open("a");
    settle(1000);
    expect(t.openTabs.ready).toBe(false);
    expect(t.calls.saveOpenTabs).toHaveLength(0);
    answer.resolve(openTabsInfo({ tabs: [nodeTab("b")], active: 0 }));
    await loading;
    settle(1000);
    expect(t.openTabs.ready).toBe(true);
    // the restored list is what is on disk: it is not written back
    expect(t.calls.saveOpenTabs).toHaveLength(0);
  });

  test("a_failed_load_never_saves", async () => {
    const t = setup(undefined, {
      getOpenTabs: async () => {
        throw new Error("boom");
      },
    });
    await t.openTabs.load();
    t.sessions.open("a");
    settle(1000);
    expect(t.openTabs.ready).toBe(false);
    expect(t.calls.saveOpenTabs).toHaveLength(0);
  });

  test("an_empty_list_after_the_restore_is_saved_as_empty_only_when_the_user_closed_the_tabs", async () => {
    const t = setup(openTabsInfo({ tabs: [nodeTab("a"), nodeTab("b")], active: 1 }));
    await t.openTabs.load();
    settle(1000);
    expect(t.calls.saveOpenTabs).toHaveLength(0);
    expect(t.sessions.tabs).toHaveLength(2);

    for (const tab of [...t.sessions.tabs]) t.sessions.close(tab.id);
    settle();
    expect(t.calls.saveOpenTabs).toEqual([[[], null]]);
  });

  test("an_empty_saved_list_stays_unwritten", async () => {
    const t = setup(openTabsInfo({ tabs: [] }));
    await t.openTabs.load();
    settle(1000);
    expect(t.calls.saveOpenTabs).toHaveLength(0);
  });

  test("opening_closing_reordering_and_switching_save_the_list", async () => {
    const t = setup();
    await t.openTabs.load();

    t.sessions.open("a");
    t.sessions.open("b");
    settle();
    expect(t.calls.saveOpenTabs.at(-1)).toEqual([[{ node: "a" }, { node: "b" }], 1]);

    t.sessions.activate(t.sessions.tabs[0]!.id);
    settle();
    expect(t.calls.saveOpenTabs.at(-1)).toEqual([[{ node: "a" }, { node: "b" }], 0]);

    t.sessions.reorder(t.sessions.tabs[0]!.id, null);
    settle();
    expect(t.calls.saveOpenTabs.at(-1)).toEqual([[{ node: "b" }, { node: "a" }], 1]);

    t.sessions.close(t.sessions.tabs[0]!.id);
    settle();
    expect(t.calls.saveOpenTabs.at(-1)).toEqual([[{ node: "a" }], 0]);
    expect(t.calls.saveOpenTabs).toHaveLength(4);
  });

  test("a_tab_closed_because_the_shell_ended_cleanly_drops_out_of_the_list", async () => {
    const t = setup();
    await t.openTabs.load();
    t.sessions.open("a");
    t.sessions.open("b");
    settle();
    const first = t.sessions.tabs[0]!;
    t.sessions.attachTerminal(first.id, { write() {}, size: () => ({ cols: 80, rows: 24 }) });
    await Promise.resolve();
    // the open call recorded the channel callback of the first tab
    const emit = t.calls.openSession[0]![1] as unknown as (e: never) => void;
    emit({
      kind: "state",
      state: "disconnected",
      detail: null,
      exitStatus: 0,
      endedCleanly: true,
    } as never);
    settle();
    expect(t.sessions.tabs).toHaveLength(1);
    expect(t.calls.saveOpenTabs.at(-1)).toEqual([[{ node: "b" }], 0]);
  });

  test("closeAll_on_unload_is_not_saved", async () => {
    const t = setup(openTabsInfo({ tabs: [nodeTab("a"), nodeTab("b")], active: 0 }));
    await t.openTabs.load();
    settle();
    t.sessions.closeAll();
    settle(5000);
    expect(t.sessions.tabs).toHaveLength(0);
    expect(t.calls.saveOpenTabs).toHaveLength(0);

    // a later change by the user is saved again
    t.sessions.open("c");
    settle();
    expect(t.calls.saveOpenTabs).toEqual([[[{ node: "c" }], 0]]);
  });

  test("closeAll_cancels_a_save_that_was_waiting", async () => {
    const t = setup();
    await t.openTabs.load();
    t.sessions.open("a");
    flushSync();
    t.sessions.closeAll();
    settle(5000);
    expect(t.calls.saveOpenTabs).toHaveLength(0);
  });

  test("saves_are_debounced", async () => {
    const t = setup();
    await t.openTabs.load();
    t.sessions.open("a");
    flushSync();
    vi.advanceTimersByTime(SAVE_DELAY_MS - 50);
    t.sessions.open("b");
    flushSync();
    vi.advanceTimersByTime(SAVE_DELAY_MS - 50);
    expect(t.calls.saveOpenTabs).toHaveLength(0);
    vi.advanceTimersByTime(100);
    expect(t.calls.saveOpenTabs).toEqual([[[{ node: "a" }, { node: "b" }], 1]]);
  });

  test("nothing_is_saved_while_reopen_is_off", async () => {
    const t = setup(openTabsInfo({ reopen: false, tabs: [nodeTab("a")] }));
    await t.openTabs.load();
    // off: the saved tabs are not restored either
    expect(t.sessions.tabs).toHaveLength(0);
    t.sessions.open("x");
    settle(1000);
    expect(t.calls.saveOpenTabs).toHaveLength(0);
  });

  test("turning_reopen_on_saves_the_current_tabs", async () => {
    const t = setup(openTabsInfo({ reopen: false }));
    await t.openTabs.load();
    t.sessions.open("x");
    settle(1000);
    expect(await t.openTabs.setReopen(true)).toBeNull();
    settle();
    expect(t.calls.setReopenTabs).toEqual([[true]]);
    expect(t.calls.saveOpenTabs).toEqual([[[{ node: "x" }], 0]]);
  });

  test("quick_tabs_are_saved_as_their_target", async () => {
    const t = setup(undefined, { quickParse: async () => quick });
    await t.openTabs.load();
    await t.sessions.openQuick("root@h.example:2222");
    t.sessions.open("a");
    settle();
    expect(t.calls.saveOpenTabs.at(-1)).toEqual([[{ quick }, { node: "a" }], 1]);
  });

  test("a_failed_save_is_ignored", async () => {
    const t = setup(undefined, {
      saveOpenTabs: async () => {
        throw new Error("disk");
      },
    });
    await t.openTabs.load();
    t.sessions.open("a");
    settle();
    expect(t.calls.saveOpenTabs).toHaveLength(1);
  });
});
