import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { mockApi, terminalSettings, vaultStatus } from "../testing";
import type { SessionEvent } from "../types";
import { VaultStore } from "../vault.svelte";

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

const handlers: ((e: SessionEvent | ArrayBuffer) => void)[] = [];

function setup(hidden = false, count = 2) {
  handlers.length = 0;
  const mock = mockApi([], {
    openSession: async (_n, onEvent) => {
      handlers.push(onEvent);
      return `sid-${handlers.length}`;
    },
    vaultStatus: async () => vaultStatus({ state: "locked" }),
  });
  const sessions = new SessionStore(mock.api, (id) => id);
  for (let i = 0; i < count; i++) sessions.open(String.fromCharCode(97 + i));
  mounted = mount(SessionArea, {
    target: document.body,
    props: { sessions, vault: new VaultStore(mock.api), settings: terminalSettings(), hidden },
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

test("a_prompt_covers_only_its_own_tab_and_the_terminal_stays_mounted_and_gets_focus_back", async () => {
  const sessions = setup();
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  expect(handlers).toHaveLength(2);
  const dialogs = () => document.querySelectorAll('[role="dialog"]');
  expect(dialogs()).toHaveLength(0);

  // a prompt for the background tab does not cover the active one
  handlers[0]!({
    kind: "prompt",
    id: 1,
    prompt: { kind: "keyPassphrase" },
  });
  flushSync();
  const panes = [...document.querySelectorAll<HTMLElement>(".pane")];
  // it waits until its tab comes to the front
  expect(dialogs()).toHaveLength(0);
  expect(panes[0]!.classList.contains("inactive")).toBe(true);

  // activate it: the prompt shows over its terminal, which stays mounted
  sessions.activate(sessions.tabs[0]!.id);
  flushSync();
  await tick();
  expect(views()).toHaveLength(2);
  expect(h.made.some((c) => c.calls.includes("dispose"))).toBe(false);
  const input = document.querySelector<HTMLInputElement>('[role="dialog"] input')!;
  expect(document.activeElement).toBe(input);
  const focusCalls = () => h.made[0]!.calls.filter((c) => c === "focus").length;
  const before = focusCalls();

  // answering closes the prompt, and the terminal takes the focus back
  document
    .querySelector('[role="dialog"]')!
    .dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  flushSync();
  await tick();
  expect(dialogs()).toHaveLength(0);
  expect(focusCalls()).toBeGreaterThan(before);
});

test("after_the_vault_prompt_the_terminal_takes_the_focus_when_the_tab_connects", async () => {
  const sessions = setup(false, 1);
  await tick();
  await new Promise((r) => setTimeout(r, 0));
  handlers[0]!({ kind: "state", state: "needsVault", detail: null });
  flushSync();
  await tick();
  expect(document.querySelector('[role="dialog"]')).not.toBeNull();
  const before = h.made[0]!.calls.filter((c) => c === "focus").length;
  handlers[0]!({ kind: "state", state: "connecting", detail: null });
  flushSync();
  await tick();
  expect(document.querySelector('[role="dialog"]')).toBeNull();
  expect(h.made[0]!.calls.filter((c) => c === "focus").length).toBeGreaterThan(before);
  expect(sessions.tabs[0]!.state).toBe("connecting");
});
