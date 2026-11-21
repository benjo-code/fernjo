import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App.svelte";
import { conn, mockApi, vaultStatus } from "./lib/testing";
import type { SessionEvent } from "./lib/types";

const h = vi.hoisted(() => ({ rdp: [] as { calls: string[] }[] }));

vi.mock("./lib/rdp/controller", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./lib/rdp/controller")>()),
  RdpController: class {
    calls: string[] = [];
    constructor() {
      h.rdp.push(this);
    }
    setDesktop() {}
    boxChanged() {}
    paint() {}
    keyDown(e: { code: string }) {
      this.calls.push(`down:${e.code}`);
      return true;
    }
    keyUp(e: { code: string }) {
      this.calls.push(`up:${e.code}`);
      return true;
    }
    pointerMove() {}
    pointerButton() {}
    wheel() {}
    releaseAll() {}
    blur() {}
    dispose() {}
  },
}));

vi.mock("./lib/terminal/controller", () => ({
  TerminalController: class {
    cols = 80;
    rows = 24;
    mount() {}
    setVisible() {}
    fit() {}
    focus() {}
    write() {}
    setOptions() {}
    refreshTheme() {}
    dispose() {}
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
const handlers: ((e: SessionEvent | ArrayBuffer) => void)[] = [];

afterEach(() => {
  if (app) unmount(app);
  app = null;
  document.body.innerHTML = "";
  h.rdp.length = 0;
});

const settle = () => new Promise((r) => setTimeout(r, 0));

async function start() {
  handlers.length = 0;
  const mock = mockApi([conn("web-01"), conn("DC01", "rdp")], {
    vaultStatus: async () => vaultStatus({ state: "unlocked" }),
    openSession: async (_n, onEvent) => {
      handlers.push(onEvent);
      return `sid-${handlers.length}`;
    },
  });
  app = mount(App, { target: document.body, props: { api: mock.api } });
  await settle();
  return mock;
}

const row = (id: string) =>
  [...document.querySelectorAll<HTMLElement>('[role="treeitem"]')].find(
    (el) => el.dataset.nodeId === id,
  )!;
const open = async (id: string) => {
  row(id).dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true }));
  await settle();
};
const state = (i: number, s: "connecting" | "connected" | "disconnected") => {
  handlers[i]!({ kind: "state", state: s, detail: null, exitStatus: null, endedCleanly: false });
  flushSync();
};
const shell = () => document.querySelector(".shell")!;
const strip = () => document.querySelector('[role="tablist"]');
const canvas = () => document.querySelector<HTMLCanvasElement>("canvas[data-remote-desktop]")!;
const press = (target: Element, init: KeyboardEventInit) => {
  const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  flushSync();
  return e;
};
const fullscreenChord = { key: "F11", code: "F11", ctrlKey: true, shiftKey: true };

test("fullscreen hides the sidebar and tab strip and the chord brings them back", async () => {
  await start();
  await open("DC01");
  state(0, "connected");
  expect(strip()).not.toBeNull();
  expect(shell().classList.contains("fullscreen")).toBe(false);

  press(canvas(), fullscreenChord);
  expect(shell().classList.contains("fullscreen")).toBe(true);
  expect(strip()).toBeNull();
  expect((shell() as HTMLElement).style.gridTemplateColumns).toBe("1fr");
  // the sidebar stays mounted (the tree keeps its scroll position), only hidden by the stylesheet
  expect(document.querySelector('[role="treeitem"]')).not.toBeNull();
  expect(document.querySelector(".hint")?.textContent).toBe(
    "Press Ctrl+Shift+F11 to leave full screen",
  );

  press(canvas(), fullscreenChord);
  expect(shell().classList.contains("fullscreen")).toBe(false);
  expect(strip()).not.toBeNull();
  expect(document.querySelector(".hint")).toBeNull();
});

test("the chord is the app's: the remote desktop never sees it", async () => {
  await start();
  await open("DC01");
  state(0, "connected");
  press(canvas(), fullscreenChord);
  expect(h.rdp[0]!.calls).toEqual([]);
});

test("other chords reach the remote desktop and do not close the tab", async () => {
  await start();
  await open("DC01");
  state(0, "connected");
  const close = press(canvas(), { key: "W", code: "KeyW", ctrlKey: true, shiftKey: true });
  expect(h.rdp[0]!.calls).toEqual(["down:KeyW"]);
  expect(close.defaultPrevented).toBe(true);
  expect(document.querySelector('[aria-modal="true"]')).toBeNull();
  press(canvas(), { key: "K", code: "KeyK", ctrlKey: true, shiftKey: true });
  expect(h.rdp[0]!.calls).toEqual(["down:KeyW", "down:KeyK"]);
});

test("a held fullscreen chord flips once", async () => {
  await start();
  await open("DC01");
  state(0, "connected");
  press(canvas(), fullscreenChord);
  press(canvas(), { ...fullscreenChord, repeat: true });
  expect(shell().classList.contains("fullscreen")).toBe(true);
});

test("a dropped session leaves fullscreen", async () => {
  await start();
  await open("DC01");
  state(0, "connected");
  press(canvas(), fullscreenChord);
  expect(shell().classList.contains("fullscreen")).toBe(true);
  state(0, "disconnected");
  expect(shell().classList.contains("fullscreen")).toBe(false);
  expect(strip()).not.toBeNull();
  // reconnecting does not bring full screen back by itself
  state(0, "connected");
  expect(shell().classList.contains("fullscreen")).toBe(false);
});

test("a terminal tab does not enter fullscreen", async () => {
  await start();
  await open("web-01");
  state(0, "connected");
  press(document.body, fullscreenChord);
  expect(shell().classList.contains("fullscreen")).toBe(false);
});

test("a connecting rdp tab does not enter fullscreen", async () => {
  await start();
  await open("DC01");
  press(canvas(), fullscreenChord);
  expect(shell().classList.contains("fullscreen")).toBe(false);
});

test("the tab switch chord still works from the remote desktop", async () => {
  await start();
  await open("web-01");
  await open("DC01");
  state(1, "connected");
  press(canvas(), { key: "Tab", code: "Tab", ctrlKey: true });
  expect(document.querySelector('[role="tab"][aria-selected="true"]')?.textContent).toContain(
    "web-01",
  );
  // the key was the app's, not the remote's
  expect(h.rdp[0]!.calls).toEqual([]);
});
