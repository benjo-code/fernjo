import { expect, test, vi } from "vitest";
import { SessionStore, type RemoteBridge } from "./sessions.svelte";
import { mockApi } from "./testing";
import type { Api, SessionEvent } from "./types";

/** A region message as Rust builds it: header (u16 LE x, y, w, h) then RGBA. */
function region(x: number, y: number, w: number, h: number, fill = 7): ArrayBuffer {
  const buffer = new ArrayBuffer(8 + w * h * 4);
  const view = new DataView(buffer);
  [x, y, w, h].forEach((v, i) => view.setUint16(i * 2, v, true));
  new Uint8Array(buffer, 8).fill(fill);
  return buffer;
}

function bridge(width = 1280, height = 720) {
  const painted: {
    rect: { x: number; y: number; width: number; height: number };
    bytes: number[];
  }[] = [];
  const sizes: [number, number][] = [];
  const texts: string[] = [];
  const b: RemoteBridge = {
    paint: (rect, rgba) => painted.push({ rect, bytes: [...rgba] }),
    setDesktop: (w, h) => sizes.push([w, h]),
    clipboard: (t) => texts.push(t),
    size: () => ({ width, height }),
  };
  return { b, painted, sizes, texts };
}

async function rdpTab(impl: Partial<Api> = {}) {
  let handler: (e: SessionEvent | ArrayBuffer) => void = () => {};
  const mock = mockApi([], {
    ...impl,
    openSession: async (_nodeId, onEvent) => {
      handler = onEvent;
      return "sid";
    },
  });
  const sessions = new SessionStore(
    mock.api,
    (id) => id,
    () => true,
    (id) => (id.startsWith("rdp") ? "rdp" : "ssh"),
  );
  sessions.open("rdp1");
  const tab = sessions.tabs[0]!;
  const remote = bridge();
  sessions.attachRemote(tab.id, remote.b);
  await new Promise((r) => setTimeout(r, 0));
  const emit = (e: SessionEvent | ArrayBuffer) => handler(e);
  const state = (s: "connecting" | "connected" | "disconnected") =>
    emit({ kind: "state", state: s, detail: null, exitStatus: null, endedCleanly: false });
  return { sessions, tab, emit, state, remote, ...mock };
}

test("an rdp tab has protocol rdp and the default is ssh", () => {
  const mock = mockApi([]);
  const sessions = new SessionStore(
    mock.api,
    (id) => id,
    () => true,
    (id) => (id === "r" ? "rdp" : "ssh"),
  );
  sessions.open("r");
  sessions.open("s");
  expect(sessions.tabs.map((t) => t.protocol)).toEqual(["rdp", "ssh"]);
  const plain = new SessionStore(
    mock.api,
    (id) => id,
    () => true,
  );
  plain.open("x");
  expect(plain.tabs[0]!.protocol).toBe("ssh");
});

test("the session starts at the canvas box size in pixels", async () => {
  const t = await rdpTab();
  expect(t.calls.openSession).toHaveLength(1);
  const [nodeId, , cols, rows] = t.calls.openSession[0]!;
  expect([nodeId, cols, rows]).toEqual(["rdp1", 1280, 720]);
});

test("rdp region messages go to the remote bridge and not the terminal", async () => {
  const t = await rdpTab();
  t.emit(region(10, 20, 2, 3, 9));
  expect(t.remote.painted).toHaveLength(1);
  expect(t.remote.painted[0]!.rect).toEqual({ x: 10, y: 20, width: 2, height: 3 });
  expect(t.remote.painted[0]!.bytes).toEqual(new Array(24).fill(9));
  // acknowledged by its whole length, header included
  await vi.waitFor(() => expect(t.calls.sessionAck).toEqual([["sid", 8 + 24]]));
});

test("a region with a bad length is acknowledged but not painted", async () => {
  const t = await rdpTab();
  const bad = region(0, 0, 2, 2);
  new DataView(bad).setUint16(4, 3, true); // claims width 3
  t.emit(bad);
  t.emit(new ArrayBuffer(5)); // shorter than a header
  expect(t.remote.painted).toHaveLength(0);
  await vi.waitFor(() =>
    expect(t.calls.sessionAck).toEqual([
      ["sid", 8 + 16],
      ["sid", 5],
    ]),
  );
});

test("regions that arrive without a canvas are acknowledged and dropped", async () => {
  const t = await rdpTab();
  t.sessions.detachRemote(t.tab.id);
  t.emit(region(0, 0, 1, 1));
  expect(t.remote.painted).toHaveLength(0);
  await vi.waitFor(() => expect(t.calls.sessionAck).toEqual([["sid", 12]]));
});

test("the desktop event sizes the canvas and is replayed to a canvas mounted later", async () => {
  const t = await rdpTab();
  t.emit({ kind: "desktop", width: 1920, height: 1080 });
  expect(t.remote.sizes).toEqual([[1920, 1080]]);
  t.sessions.detachRemote(t.tab.id);
  const second = bridge();
  t.sessions.attachRemote(t.tab.id, second.b);
  expect(second.sizes).toEqual([[1920, 1080]]);
  // a remount does not open a second session
  expect(t.calls.openSession).toHaveLength(1);
});

test("the clipboard event reaches the bridge", async () => {
  const t = await rdpTab();
  t.emit({ kind: "clipboard", text: "from the server" });
  expect(t.remote.texts).toEqual(["from the server"]);
});

test("input is dropped unless connected", async () => {
  const t = await rdpTab();
  const key = { kind: "key" as const, scancode: 30, extended: false, pressed: true };
  t.sessions.rdpInput(t.tab.id, [key]);
  t.sessions.rdpClipboard(t.tab.id, "x");
  expect(t.calls.rdpInput).toHaveLength(0);
  expect(t.calls.rdpClipboard).toHaveLength(0);
  t.state("connected");
  t.sessions.rdpInput(t.tab.id, [key]);
  t.sessions.rdpInput(t.tab.id, []);
  t.sessions.rdpClipboard(t.tab.id, "x");
  expect(t.calls.rdpInput).toEqual([["sid", [key]]]);
  expect(t.calls.rdpClipboard).toEqual([["sid", "x"]]);
  t.state("disconnected");
  t.sessions.rdpInput(t.tab.id, [key]);
  expect(t.calls.rdpInput).toHaveLength(1);
});

test("resize carries pixels and a zero side is not sent", async () => {
  const t = await rdpTab();
  t.sessions.resize(t.tab.id, 1600, 900);
  t.sessions.resize(t.tab.id, 0, 900);
  expect(t.calls.sessionResize).toEqual([["sid", 1600, 900]]);
});

test("reconnect passes the canvas size and writes no terminal separator", async () => {
  const t = await rdpTab();
  t.state("disconnected");
  t.sessions.reconnect(t.tab.id);
  await vi.waitFor(() => expect(t.calls.reconnectSession).toEqual([["sid", 1280, 720]]));
  expect(t.tab.state).toBe("connecting");
});

test("a terminal tab still gets terminal output", async () => {
  let handler: (e: SessionEvent | ArrayBuffer) => void = () => {};
  const mock = mockApi([], {
    openSession: async (_n, onEvent) => {
      handler = onEvent;
      return "sid";
    },
  });
  const sessions = new SessionStore(
    mock.api,
    (id) => id,
    () => true,
    () => "ssh",
  );
  sessions.open("n");
  const written: number[][] = [];
  sessions.attachTerminal(sessions.tabs[0]!.id, {
    write: (b) => written.push([...b]),
    size: () => ({ cols: 80, rows: 24 }),
  });
  await new Promise((r) => setTimeout(r, 0));
  handler(new Uint8Array([1, 2, 3]).buffer);
  expect(written).toEqual([[1, 2, 3]]);
  expect(mock.calls.sessionAck).toHaveLength(0);
});

test("a fixed resolution is read from the connection when the canvas attaches", async () => {
  const node = (mode: "fit" | "fixed") =>
    ({
      rdpResolution: {
        local: null,
        effective: mode === "fixed" ? { mode, width: 1920, height: 1080 } : { mode },
        source: { kind: "default" },
      },
    }) as never;
  const fixed = await rdpTab({ getNode: async () => node("fixed") });
  await vi.waitFor(() => expect(fixed.tab.fixedSize).toBe(true));
  const fit = await rdpTab({ getNode: async () => node("fit") });
  await new Promise((r) => setTimeout(r, 0));
  expect(fit.tab.fixedSize).toBe(false);
  // an unreadable connection leaves the tab fitting
  const unknown = await rdpTab();
  expect(unknown.tab.fixedSize).toBe(false);
});

test("full screen is for a connected rdp tab and ends with any change of tab", async () => {
  const t = await rdpTab();
  t.sessions.toggleFullscreen();
  expect(t.sessions.remoteFullscreen).toBe(false); // still connecting
  t.state("connected");
  t.sessions.toggleFullscreen();
  expect(t.sessions.remoteFullscreen).toBe(true);
  t.sessions.next();
  expect(t.sessions.remoteFullscreen).toBe(false);
  t.sessions.toggleFullscreen();
  expect(t.sessions.remoteFullscreen).toBe(true);
  t.sessions.toggleFullscreen();
  expect(t.sessions.remoteFullscreen).toBe(false);
  t.sessions.toggleFullscreen();
  t.sessions.close(t.tab.id);
  expect(t.sessions.remoteFullscreen).toBe(false);
});
