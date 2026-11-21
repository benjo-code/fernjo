import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import SessionArea from "../components/SessionArea.svelte";
import { SessionStore } from "../sessions.svelte";
import { mockApi, terminalSettings, vaultStatus } from "../testing";
import type { SessionEvent } from "../types";
import { VaultStore } from "../vault.svelte";

const h = vi.hoisted(() => ({
  made: [] as { calls: string[]; options: { send(events: unknown[]): void } }[],
}));

vi.mock("./controller", () => ({
  RdpController: class {
    calls: string[] = [];
    options: { send(events: unknown[]): void };
    constructor(options: { send(events: unknown[]): void }) {
      this.options = options;
      h.made.push(this);
    }
    setDesktop(w: number, hh: number) {
      this.calls.push(`desktop:${w}x${hh}`);
    }
    paint() {
      this.calls.push("paint");
    }
    keyDown(e: { code: string }) {
      this.calls.push(`down:${e.code}`);
      return e.code !== "Unknown";
    }
    keyUp(e: { code: string }) {
      this.calls.push(`up:${e.code}`);
      return e.code !== "Unknown";
    }
    pointerMove() {
      this.calls.push("move");
    }
    pointerButton(e: { button: number }, pressed: boolean) {
      this.calls.push(`button:${e.button}:${pressed}`);
    }
    wheel() {
      this.calls.push("wheel");
    }
    releaseAll() {
      this.calls.push("release");
    }
    blur() {
      this.calls.push("blur");
    }
    dispose() {
      this.calls.push("dispose");
    }
  },
}));

vi.mock("../terminal/controller", () => ({
  TerminalController: class {
    mount() {}
    setVisible() {}
    fit() {}
    focus() {}
    write() {}
    setOptions() {}
    refreshTheme() {}
    dispose() {}
    cols = 80;
    rows = 24;
  },
}));

vi.stubGlobal(
  "ResizeObserver",
  class {
    observe() {}
    disconnect() {}
  },
);

let mounted: ReturnType<typeof mount> | null = null;
const handlers: ((e: SessionEvent | ArrayBuffer) => void)[] = [];

afterEach(() => {
  if (mounted) unmount(mounted);
  mounted = null;
  document.body.innerHTML = "";
  h.made.length = 0;
});

function setup(count = 1) {
  handlers.length = 0;
  const mock = mockApi([], {
    openSession: async (_n, onEvent) => {
      handlers.push(onEvent);
      return `sid-${handlers.length}`;
    },
    vaultStatus: async () => vaultStatus({ state: "locked" }),
  });
  const sessions = new SessionStore(
    mock.api,
    (id) => id,
    () => true,
    () => "rdp",
  );
  for (let i = 0; i < count; i++) sessions.open(`r${i}`);
  mounted = mount(SessionArea, {
    target: document.body,
    props: { sessions, vault: new VaultStore(mock.api), settings: terminalSettings() },
  });
  flushSync();
  return { sessions, ...mock };
}

const canvases = () => [...document.querySelectorAll<HTMLCanvasElement>("canvas")];
const state = (i: number, s: "connecting" | "connected" | "disconnected") => {
  handlers[i]!({ kind: "state", state: s, detail: null, exitStatus: null, endedCleanly: false });
  flushSync();
};

test("an rdp tab renders a canvas, not a terminal", () => {
  setup(2);
  expect(canvases()).toHaveLength(2);
  expect(document.querySelectorAll(".term")).toHaveLength(0);
  expect(h.made).toHaveLength(2);
  expect(canvases()[0]!.getAttribute("tabindex")).toBe("0");
  expect(canvases()[0]!.getAttribute("aria-label")).toBe("Remote desktop of r0");
});

test("keys with a scancode are forwarded and kept from the browser; others are not", () => {
  setup();
  const canvas = canvases()[0]!;
  const handled = new KeyboardEvent("keydown", { code: "KeyA", bubbles: true, cancelable: true });
  canvas.dispatchEvent(handled);
  const unknown = new KeyboardEvent("keyup", { code: "Unknown", bubbles: true, cancelable: true });
  canvas.dispatchEvent(unknown);
  expect(handled.defaultPrevented).toBe(true);
  expect(unknown.defaultPrevented).toBe(false);
  expect(h.made[0]!.calls).toContain("down:KeyA");
  expect(h.made[0]!.calls).toContain("up:Unknown");
});

test("blur releases the keys", () => {
  setup();
  canvases()[0]!.dispatchEvent(new FocusEvent("blur"));
  expect(h.made[0]!.calls).toContain("blur");
});

test("a dropped session and leaving the tab release the keys", () => {
  const t = setup(2);
  const first = t.sessions.tabs[0]!.id;
  const second = t.sessions.tabs[1]!.id;
  t.sessions.activate(first);
  state(0, "connected");
  flushSync();
  h.made[0]!.calls.length = 0;
  state(0, "disconnected");
  expect(h.made[0]!.calls).toContain("release");

  state(0, "connected");
  h.made[0]!.calls.length = 0;
  t.sessions.activate(second);
  flushSync();
  expect(h.made[0]!.calls).toContain("release");
});

test("the pointer is forwarded with the button and captured", () => {
  setup();
  const canvas = canvases()[0]!;
  const capture = vi.fn();
  canvas.setPointerCapture = capture;
  const down = new MouseEvent("pointerdown", { button: 2, bubbles: true, cancelable: true });
  Object.defineProperty(down, "pointerId", { value: 7 });
  canvas.dispatchEvent(down);
  canvas.dispatchEvent(new MouseEvent("pointermove", { bubbles: true }));
  canvas.dispatchEvent(new MouseEvent("pointerup", { button: 2, bubbles: true }));
  expect(h.made[0]!.calls).toEqual(
    expect.arrayContaining(["button:2:true", "move", "button:2:false"]),
  );
  expect(capture).toHaveBeenCalledWith(7);
  expect(down.defaultPrevented).toBe(true);
});

test("the wheel is forwarded and the page does not scroll", () => {
  setup();
  const wheel = new WheelEvent("wheel", { deltaY: 100, bubbles: true, cancelable: true });
  canvases()[0]!.dispatchEvent(wheel);
  expect(wheel.defaultPrevented).toBe(true);
  expect(h.made[0]!.calls).toContain("wheel");
});

test("the context menu of the browser is suppressed", () => {
  setup();
  const menu = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  canvases()[0]!.dispatchEvent(menu);
  expect(menu.defaultPrevented).toBe(true);
});

test("input events go through the store to the connected session only", async () => {
  const t = setup();
  await new Promise((r) => setTimeout(r, 0));
  const send = h.made[0]!.options.send;
  const key = { kind: "key", scancode: 30, extended: false, pressed: true };
  send([key]);
  expect(t.calls.rdpInput).toHaveLength(0);
  state(0, "connected");
  send([key]);
  expect(t.calls.rdpInput).toEqual([["sid-1", [key]]]);
});

test("a desktop event reaches the controller", async () => {
  setup();
  await new Promise((r) => setTimeout(r, 0));
  handlers[0]!({ kind: "desktop", width: 1920, height: 1080 });
  expect(h.made[0]!.calls).toContain("desktop:1920x1080");
});

test("connecting shows a status line that goes away", () => {
  setup();
  expect(document.querySelector(".status")?.textContent).toBe("Connecting to r0…");
  state(0, "connected");
  expect(document.querySelector(".status")).toBeNull();
});

test("unmounting disposes the controller", () => {
  setup();
  unmount(mounted!);
  mounted = null;
  expect(h.made[0]!.calls).toContain("dispose");
});
