import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { mockApi, terminalSettings } from "../testing";
import type { SessionEvent } from "../types";

const h = vi.hoisted(() => ({
  made: [] as {
    deps: { onData(b: Uint8Array): void; onResize(c: number, r: number): void };
    calls: string[];
    size: { cols: number; rows: number };
  }[],
  pasteFails: false,
}));

vi.mock("./controller", () => ({
  TerminalController: class {
    calls: string[] = [];
    size = { cols: 91, rows: 33 };
    deps: (typeof h.made)[number]["deps"];
    constructor(_opts: unknown, deps: (typeof h.made)[number]["deps"]) {
      this.deps = deps;
      h.made.push(this);
    }
    mount() {
      this.calls.push("mount");
    }
    setVisible(v: boolean) {
      this.calls.push(`visible:${v}`);
    }
    fit() {
      this.calls.push("fit");
    }
    focus() {
      this.calls.push("focus");
    }
    write() {}
    setOptions() {
      this.calls.push("setOptions");
    }
    refreshTheme() {}
    async paste() {
      this.calls.push("paste");
      if (h.pasteFails) throw new Error("clipboard denied");
    }
    async copySelection() {
      this.calls.push("copy");
    }
    clearSearch() {}
    dispose() {
      this.calls.push("dispose");
    }
    get cols() {
      return this.size.cols;
    }
    get rows() {
      return this.size.rows;
    }
  },
}));

import TerminalView from "./TerminalView.svelte";

vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    disconnect() {}
  },
);

afterEach(() => {
  document.body.innerHTML = "";
  h.made.length = 0;
  h.pasteFails = false;
});

/** A store with one tab whose session events the test can inject. */
function openTab() {
  const mock = mockApi();
  const store = new SessionStore(
    mock.api,
    () => "host",
    () => true,
  );
  store.open("n1");
  const tab = store.tabs[0]!;
  const view = (visible = true) =>
    mount(TerminalView, {
      target: document.body,
      props: { tab, sessions: store, settings: terminalSettings(), visible },
    });
  // the session channel callback the store handed to the api on attach
  const emit = (event: SessionEvent) =>
    (mock.calls.openSession[0]![1] as (e: SessionEvent) => void)(event);
  return { store, tab, view, emit };
}

const focusCalls = () => h.made[0]!.calls.filter((c) => c === "focus").length;

test("terminal_view_registers_the_bridge_and_disposes_on_destroy", async () => {
  const { api } = mockApi();
  const store = new SessionStore(
    api,
    () => "host",
    () => true,
  );
  store.open("n1");
  const tab = store.tabs[0]!;
  const attach = vi.spyOn(store, "attachTerminal");
  const detach = vi.spyOn(store, "detachTerminal");
  const input = vi.spyOn(store, "input");

  const view = mount(TerminalView, {
    target: document.body,
    props: { tab, sessions: store, settings: terminalSettings(), visible: true },
  });
  await tick();
  flushSync();

  expect(h.made).toHaveLength(1);
  const c = h.made[0]!;
  expect(attach).toHaveBeenCalledTimes(1);
  expect(attach.mock.calls[0]![0]).toBe(tab.id);
  expect(c.calls).toContain("visible:true");

  // the bridge reports the controller's size, and typed bytes reach the store
  expect(attach.mock.calls[0]![1].size()).toEqual({ cols: 91, rows: 33 });
  c.deps.onData(new Uint8Array([1]));
  expect(input).toHaveBeenCalledWith(tab.id, new Uint8Array([1]));

  unmount(view);
  expect(detach).toHaveBeenCalledWith(tab.id);
  expect(c.calls).toContain("dispose");
});

test("terminal_takes_focus_only_for_a_visible_connecting_or_connected_tab_without_a_prompt", async () => {
  const { view, emit } = openTab();
  const mounted = view();
  await tick();
  flushSync();
  // connecting, visible, no prompt: the terminal gets the keyboard
  expect(focusCalls()).toBe(1);

  // the move to connected does not steal focus again
  emit({ kind: "state", state: "connected", detail: null, exitStatus: null, endedCleanly: false });
  await tick();
  flushSync();
  expect(focusCalls()).toBe(1);
  unmount(mounted);
});

test("an_open_prompt_keeps_the_terminal_from_taking_focus_until_it_is_gone", async () => {
  const { view, tab } = openTab();
  // the prompt is already there when the terminal appears
  tab.prompt = { id: 1, prompt: { kind: "password", user: "root", host: "h" } };
  const mounted = view();
  await tick();
  flushSync();
  expect(tab.prompt).not.toBeNull();
  expect(focusCalls()).toBe(0);

  // answering clears the prompt: now the terminal may take focus
  tab.prompt = null;
  await tick();
  flushSync();
  expect(focusCalls()).toBe(1);
  unmount(mounted);
});

test("a_tab_waiting_for_the_vault_does_not_take_focus", async () => {
  const { view, tab } = openTab();
  tab.state = "needsVault";
  const mounted = view();
  await tick();
  flushSync();
  expect(focusCalls()).toBe(0);
  tab.state = "connecting";
  await tick();
  flushSync();
  expect(focusCalls()).toBe(1);
  unmount(mounted);
});

test("a_hidden_tab_never_takes_focus", async () => {
  const { view } = openTab();
  const mounted = view(false);
  await tick();
  flushSync();
  expect(focusCalls()).toBe(0);
  unmount(mounted);
});

// The test runs on Node; the UI tsconfig carries no Node types.
const nodeProcess = (
  globalThis as unknown as {
    process: {
      on(e: "unhandledRejection", f: (r: unknown) => void): void;
      off(e: "unhandledRejection", f: (r: unknown) => void): void;
    };
  }
).process;

test("a_failing_menu_paste_is_swallowed", async () => {
  const unhandled: unknown[] = [];
  const onUnhandled = (reason: unknown) => unhandled.push(reason);
  nodeProcess.on("unhandledRejection", onUnhandled);
  try {
    h.pasteFails = true;
    const { view } = openTab();
    const mounted = view();
    await tick();
    flushSync();
    document
      .querySelector(".view")!
      .dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    flushSync();
    [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')]
      .find((el) => el.textContent?.trim() === "Paste")!
      .click();
    flushSync();
    await new Promise((r) => setTimeout(r, 10));
    expect(h.made[0]!.calls).toContain("paste");
    expect(unhandled).toEqual([]);
    unmount(mounted);
  } finally {
    nodeProcess.off("unhandledRejection", onUnhandled);
  }
});
