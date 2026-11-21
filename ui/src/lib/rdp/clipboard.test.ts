import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { ClipboardSync, isPasteChord, MAX_CLIPBOARD_BYTES } from "./clipboard";

let local = "";
let sent: string[] = [];
let written: string[] = [];
let blocked = 0;

function make(
  over: Partial<{
    read: () => Promise<string>;
    write: (t: string) => Promise<void>;
    timeoutMs: number;
  }> = {},
) {
  return new ClipboardSync({
    read: over.read ?? (async () => local),
    write:
      over.write ??
      (async (t) => {
        written.push(t);
        local = t;
      }),
    send: (t) => sent.push(t),
    timeoutMs: over.timeoutMs,
    onBlocked: () => blocked++,
  });
}

beforeEach(() => {
  local = "";
  sent = [];
  written = [];
  blocked = 0;
});

afterEach(() => {
  vi.useRealTimers();
});

test("text from the remote is written and not sent back", async () => {
  const sync = make();
  await sync.onRemote("from the server");
  expect(written).toEqual(["from the server"]);
  await sync.syncLocal();
  expect(sent).toEqual([]);
});

test("local text that changed is sent once", async () => {
  const sync = make();
  local = "copied here";
  await sync.syncLocal();
  await sync.syncLocal();
  expect(sent).toEqual(["copied here"]);
  local = "copied again";
  await sync.syncLocal();
  expect(sent).toEqual(["copied here", "copied again"]);
});

test("unchanged local text is not sent again after the remote one replaced it", async () => {
  const sync = make();
  local = "one";
  await sync.syncLocal();
  await sync.onRemote("two");
  await sync.syncLocal();
  expect(sent).toEqual(["one"]);
  // copying the first text again locally is a change
  local = "one";
  await sync.syncLocal();
  expect(sent).toEqual(["one", "one"]);
});

test("an empty clipboard sends nothing", async () => {
  const sync = make();
  await sync.syncLocal();
  expect(sent).toEqual([]);
  expect(blocked).toBe(0);
});

test("text over one mebibyte is not sent", async () => {
  const sync = make();
  local = "x".repeat(MAX_CLIPBOARD_BYTES + 1);
  await sync.syncLocal();
  expect(sent).toEqual([]);
  // the limit counts bytes, not characters: 2-byte characters hit it at half the length
  local = "é".repeat(MAX_CLIPBOARD_BYTES / 2 + 1);
  await sync.syncLocal();
  expect(sent).toEqual([]);
  local = "x".repeat(MAX_CLIPBOARD_BYTES);
  await sync.syncLocal();
  expect(sent).toHaveLength(1);
});

test("a slow read times out quietly", async () => {
  vi.useFakeTimers();
  const sync = make({ read: () => new Promise<string>(() => {}), timeoutMs: 1500 });
  const first = sync.syncLocal();
  await vi.advanceTimersByTimeAsync(1499);
  expect(blocked).toBe(0);
  await vi.advanceTimersByTimeAsync(1);
  await first;
  const second = sync.syncLocal();
  await vi.advanceTimersByTimeAsync(2000);
  await second;
  // a permission the user has not answered is not an error worth a banner
  expect(blocked).toBe(0);
  expect(sent).toEqual([]);
});

test("a late answer after the timeout is dropped", async () => {
  vi.useFakeTimers();
  let answer: (t: string) => void = () => {};
  const sync = make({ read: () => new Promise<string>((r) => (answer = r)), timeoutMs: 100 });
  const pending = sync.syncLocal();
  await vi.advanceTimersByTimeAsync(100);
  await pending;
  answer("too late");
  await vi.advanceTimersByTimeAsync(0);
  expect(sent).toEqual([]);
});

test("a read that fails is given up on quietly", async () => {
  const sync = make({
    read: async () => {
      throw new Error("denied");
    },
  });
  await sync.syncLocal();
  expect(blocked).toBe(0);
  expect(sent).toEqual([]);
});

test("text taken from a paste event is sent once, unless empty, unchanged or too big", () => {
  const sync = make();
  sync.offer("");
  sync.offer("from the paste event");
  sync.offer("from the paste event");
  sync.offer("x".repeat(MAX_CLIPBOARD_BYTES + 1));
  expect(sent).toEqual(["from the paste event"]);
  sync.offer("another");
  expect(sent).toEqual(["from the paste event", "another"]);
});

test("a failed write reports blocked", async () => {
  const sync = make({
    write: async () => {
      throw new Error("denied");
    },
  });
  await sync.onRemote("text");
  await sync.onRemote("more");
  expect(blocked).toBe(1);
  // the text still counts as known on both sides, so it is not echoed back
  local = "more";
  await sync.syncLocal();
  expect(sent).toEqual([]);
});

test("the paste chord is Ctrl+V (Cmd+V on a Mac) and Shift+Insert", () => {
  const key = (init: Partial<Parameters<typeof isPasteChord>[0]>) => ({
    code: "KeyV",
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    ...init,
  });
  expect(isPasteChord(key({ ctrlKey: true }), false)).toBe(true);
  expect(isPasteChord(key({ ctrlKey: true, shiftKey: true }), false)).toBe(true);
  expect(isPasteChord(key({ metaKey: true }), true)).toBe(true);
  expect(isPasteChord(key({ ctrlKey: true }), true)).toBe(false);
  expect(isPasteChord(key({ metaKey: true }), false)).toBe(false);
  expect(isPasteChord(key({ code: "KeyV" }), false)).toBe(false);
  expect(isPasteChord(key({ ctrlKey: true, altKey: true }), false)).toBe(false);
  expect(isPasteChord(key({ code: "Insert", shiftKey: true }), false)).toBe(true);
  expect(isPasteChord(key({ code: "Insert" }), false)).toBe(false);
  expect(isPasteChord(key({ code: "KeyC", ctrlKey: true }), false)).toBe(false);
});
