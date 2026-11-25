import { flushSync, mount, unmount } from "svelte";
import { afterEach, expect, test, vi } from "vitest";
import SessionArea from "../components/SessionArea.svelte";
import { SessionStore } from "../sessions.svelte";
import { mockApi, terminalSettings, vaultStatus } from "../testing";
import type { Api, SessionEvent } from "../types";
import { VaultStore } from "../vault.svelte";

const h = vi.hoisted(() => ({
  made: [] as { calls: string[]; options: { send(events: unknown[]): void } }[],
}));

vi.mock("./controller", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./controller")>()),
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
    boxChanged() {}
    down = new Set<string>();
    isDown(code: string) {
      return this.down.has(code);
    }
    keyDown(e: { code: string }) {
      this.down.add(e.code);
      this.calls.push(`down:${e.code}`);
      return e.code !== "Unknown";
    }
    keyUp(e: { code: string }) {
      this.down.delete(e.code);
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

function setup(count = 1, impl: Partial<Api> = {}) {
  handlers.length = 0;
  const mock = mockApi([], {
    ...impl,
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
  expect(h.made[0]!.calls.some((c) => c.includes("Unknown"))).toBe(false);
});

test("a key that went down is released even when a modifier turned its release into an app chord", () => {
  setup();
  const canvas = canvases()[0]!;
  h.made[0]!.calls.length = 0;
  canvas.dispatchEvent(
    new KeyboardEvent("keydown", { code: "Tab", key: "Tab", bubbles: true, cancelable: true }),
  );
  // Ctrl was pressed meanwhile: Tab released now would look like Ctrl+Tab
  const up = new KeyboardEvent("keyup", {
    code: "Tab",
    key: "Tab",
    ctrlKey: true,
    bubbles: true,
    cancelable: true,
  });
  canvas.dispatchEvent(up);
  expect(h.made[0]!.calls).toEqual(["down:Tab", "up:Tab"]);
  // a Ctrl+Tab that never went down stays the app's
  h.made[0]!.calls.length = 0;
  canvas.dispatchEvent(
    new KeyboardEvent("keyup", {
      code: "Tab",
      key: "Tab",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    }),
  );
  expect(h.made[0]!.calls).toEqual([]);
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

// ---------- clipboard ----------

function fakeClipboard(read: () => Promise<string>) {
  const clipboard = {
    readText: vi.fn(read),
    writeText: vi.fn<(text: string) => Promise<void>>(async () => {}),
  };
  vi.stubGlobal("navigator", { ...navigator, clipboard });
  return clipboard;
}

const flush = () => new Promise((r) => setTimeout(r, 0));

async function connected(clipboard: boolean) {
  const t = setup(1);
  await flush();
  // the setting comes with the session (the plan it really uses), not from the connection
  handlers[0]!({ kind: "rdpSettings", clipboard, host: "r0" });
  flushSync();
  state(0, "connected");
  await flush();
  return t;
}

test("with the setting off the local clipboard is never touched", async () => {
  const clip = fakeClipboard(async () => "secret");
  const t = await connected(false);
  canvases()[0]!.dispatchEvent(new FocusEvent("focus"));
  canvases()[0]!.dispatchEvent(
    new KeyboardEvent("keydown", { code: "KeyV", ctrlKey: true, bubbles: true, cancelable: true }),
  );
  handlers[0]!({ kind: "clipboard", text: "from the server" });
  await flush();
  expect(clip.readText).not.toHaveBeenCalled();
  expect(clip.writeText).not.toHaveBeenCalled();
  expect(t.calls.rdpClipboard).toEqual([]);
});

test("the local text is offered when the desktop gets the focus", async () => {
  const clip = fakeClipboard(async () => "copied here");
  const t = await connected(true);
  canvases()[0]!.dispatchEvent(new FocusEvent("focus"));
  await flush();
  expect(clip.readText).toHaveBeenCalled();
  expect(t.calls.rdpClipboard).toEqual([["sid-1", "copied here"]]);
  // the same text is not sent again on the next focus
  canvases()[0]!.dispatchEvent(new FocusEvent("focus"));
  await flush();
  expect(t.calls.rdpClipboard).toHaveLength(1);
});

test("text the remote side copied is written locally and not sent back", async () => {
  let local = "";
  const clip = fakeClipboard(async () => local);
  clip.writeText.mockImplementation(async (text: string) => {
    local = text;
  });
  const t = await connected(true);
  handlers[0]!({ kind: "clipboard", text: "from the server" });
  await flush();
  expect(clip.writeText).toHaveBeenCalledWith("from the server");
  canvases()[0]!.dispatchEvent(new FocusEvent("focus"));
  await flush();
  expect(t.calls.rdpClipboard).toEqual([]);
});

const pasteEvent = (text: string) => {
  const e = new Event("paste", { bubbles: true, cancelable: true });
  Object.defineProperty(e, "clipboardData", { value: { getData: () => text } });
  return e;
};

test("a paste chord takes the text from the paste event, without reading the clipboard", async () => {
  const clip = fakeClipboard(async () => "never asked");
  const t = await connected(true);
  clip.readText.mockClear();
  const canvas = canvases()[0]!;
  h.made[0]!.calls.length = 0;
  const key = (type: string, code: string, ctrlKey = true) => {
    const e = new KeyboardEvent(type, { code, ctrlKey, bubbles: true, cancelable: true });
    canvas.dispatchEvent(e);
    return e;
  };
  key("keydown", "ControlLeft");
  const v = key("keydown", "KeyV");
  key("keyup", "KeyV");
  key("keyup", "ControlLeft", false);
  await flush();
  // the chord is left to the browser (so it sends the paste event); the paste key waits for it
  expect(v.defaultPrevented).toBe(false);
  expect(h.made[0]!.calls).toEqual(["down:ControlLeft"]);
  const paste = pasteEvent("copied here");
  canvas.dispatchEvent(paste);
  await flush();
  expect(paste.defaultPrevented).toBe(true);
  // the focus sync on connect sent the older text first
  expect(t.calls.rdpClipboard.at(-1)).toEqual(["sid-1", "copied here"]);
  expect(h.made[0]!.calls).toEqual(["down:ControlLeft", "down:KeyV", "up:KeyV", "up:ControlLeft"]);
  expect(clip.readText).not.toHaveBeenCalled();
});

test("without a paste event the chord reads the clipboard after a short wait", async () => {
  fakeClipboard(async () => "read instead");
  const t = await connected(true);
  const canvas = canvases()[0]!;
  h.made[0]!.calls.length = 0;
  canvas.dispatchEvent(
    new KeyboardEvent("keydown", { code: "KeyV", ctrlKey: true, bubbles: true, cancelable: true }),
  );
  expect(h.made[0]!.calls).toEqual([]);
  await new Promise((r) => setTimeout(r, 450));
  expect(t.calls.rdpClipboard).toContainEqual(["sid-1", "read instead"]);
  expect(h.made[0]!.calls).toEqual(["down:KeyV"]);
});

test("losing the focus while a paste waits drops the waiting keys", async () => {
  let release: (text: string) => void = () => {};
  fakeClipboard(() => new Promise<string>((r) => (release = r)));
  await connected(true);
  const canvas = canvases()[0]!;
  h.made[0]!.calls.length = 0;
  canvas.dispatchEvent(
    new KeyboardEvent("keydown", { code: "KeyV", ctrlKey: true, bubbles: true, cancelable: true }),
  );
  canvas.dispatchEvent(new FocusEvent("blur"));
  release("late");
  await flush();
  expect(h.made[0]!.calls).toEqual(["blur"]);
});

test("a refused read shows no banner", async () => {
  fakeClipboard(async () => {
    throw new Error("denied");
  });
  await connected(true);
  canvases()[0]!.dispatchEvent(new FocusEvent("focus"));
  await flush();
  expect(document.querySelector(".note")).toBeNull();
});

test("a refused write of the remote text shows one line", async () => {
  const clip = fakeClipboard(async () => "");
  clip.writeText.mockRejectedValue(new Error("denied"));
  await connected(true);
  handlers[0]!({ kind: "clipboard", text: "one" });
  handlers[0]!({ kind: "clipboard", text: "two" });
  await flush();
  expect(document.querySelectorAll(".note")).toHaveLength(1);
  expect(document.querySelector(".note")?.textContent).toBe(
    "The clipboard is blocked by the web view",
  );
});

test("a hidden rdp tab never writes the remote text locally; it waits for the tab's focus", async () => {
  // M11 review #31: a background tab must not set what the next paste into another tab pastes
  const clip = fakeClipboard(async () => "");
  const t = setup(2);
  await flush();
  for (const i of [0, 1]) {
    handlers[i]!({ kind: "rdpSettings", clipboard: true, host: `r${i}` });
    state(i, "connected");
  }
  await flush();
  // the second tab is in front; the first one's server copies something
  expect(t.sessions.activeId).toBe(t.sessions.tabs[1]!.id);
  handlers[0]!({ kind: "clipboard", text: "ls\x1b[201~\rrm -rf ~\r" });
  handlers[0]!({ kind: "clipboard", text: "latest" });
  await flush();
  expect(clip.writeText).not.toHaveBeenCalled();
  // the user brings the tab to the front: its desktop takes the focus and the latest text is
  // written then
  t.sessions.activate(t.sessions.tabs[0]!.id);
  flushSync();
  canvases()[0]!.dispatchEvent(new FocusEvent("focus"));
  await flush();
  expect(clip.writeText.mock.calls).toEqual([["latest"]]);
});

test("a visible rdp tab whose desktop lost the focus holds the remote text too", async () => {
  const clip = fakeClipboard(async () => "");
  await connected(true);
  const canvas = canvases()[0]!;
  canvas.blur();
  canvas.dispatchEvent(new FocusEvent("blur"));
  handlers[0]!({ kind: "clipboard", text: "while away" });
  await flush();
  expect(clip.writeText).not.toHaveBeenCalled();
  canvas.focus();
  canvas.dispatchEvent(new FocusEvent("focus"));
  await flush();
  expect(clip.writeText).toHaveBeenCalledWith("while away");
});
