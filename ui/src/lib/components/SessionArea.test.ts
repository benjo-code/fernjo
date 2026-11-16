import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { mockApi, terminalSettings } from "../testing";

const h = vi.hoisted(() => ({ made: [] as { calls: string[] }[] }));

vi.mock("../terminal/controller", () => ({
  TerminalController: class {
    calls: string[] = [];
    constructor() {
      h.made.push(this);
    }
    mount() {}
    setVisible(v: boolean) {
      this.calls.push(`visible:${v}`);
    }
    fit() {}
    focus() {
      this.calls.push("focus");
    }
    write() {}
    setOptions() {}
    refreshTheme() {}
    dispose() {
      this.calls.push("dispose");
    }
    cols = 80;
    rows = 24;
  },
}));

import SessionArea from "./SessionArea.svelte";

vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    disconnect() {}
  },
);

let mounted: ReturnType<typeof mount> | null = null;

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
  h.made.length = 0;
});

function setup(hidden = false, count = 2) {
  const mock = mockApi();
  const sessions = new SessionStore(mock.api, (id) => id);
  for (let i = 0; i < count; i++) sessions.open(String.fromCharCode(97 + i));
  mounted = mount(SessionArea, {
    target: document.body,
    props: { sessions, settings: terminalSettings(), hidden },
  });
  flushSync();
  return sessions;
}

const views = () => [...document.querySelectorAll<HTMLElement>(".view")];

test("renders_a_terminal_view_per_tab_and_only_the_active_one_is_visible", async () => {
  const sessions = setup();
  await tick();
  expect(views()).toHaveLength(2);
  expect(h.made).toHaveLength(2);
  expect(views().map((v) => v.classList.contains("hidden"))).toEqual([true, false]);

  sessions.activate(sessions.tabs[0]!.id);
  flushSync();
  expect(views().map((v) => v.classList.contains("hidden"))).toEqual([false, true]);
  // switching never recreates a terminal
  expect(h.made).toHaveLength(2);
  expect(h.made.some((c) => c.calls.includes("dispose"))).toBe(false);

  // a new tab gets its own view
  sessions.open("c");
  flushSync();
  expect(views()).toHaveLength(3);
});

test("vault_view_hides_but_keeps_terminals_mounted", async () => {
  setup(true);
  await tick();
  expect(views()).toHaveLength(2);
  expect(views().every((v) => v.classList.contains("hidden"))).toBe(true);
  expect(h.made.some((c) => c.calls.includes("dispose"))).toBe(false);
  // the surrounding area is hidden as a whole, too
  expect(document.querySelector<HTMLElement>(".area")?.classList.contains("hidden")).toBe(true);
});

test("a_closed_tab_disposes_its_terminal", async () => {
  const sessions = setup();
  await tick();
  sessions.close(sessions.tabs[0]!.id);
  flushSync();
  expect(views()).toHaveLength(1);
  expect(h.made.filter((c) => c.calls.includes("dispose"))).toHaveLength(1);
});
