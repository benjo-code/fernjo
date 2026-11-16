import { flushSync, mount, tick, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import { SessionStore } from "../sessions.svelte";
import { mockApi, terminalSettings } from "../testing";

const h = vi.hoisted(() => ({
  made: [] as {
    deps: { onData(b: Uint8Array): void; onResize(c: number, r: number): void };
    calls: string[];
    size: { cols: number; rows: number };
  }[],
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
});

test("terminal_view_registers_the_bridge_and_disposes_on_destroy", async () => {
  const { api } = mockApi();
  const store = new SessionStore(api, () => "host");
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
