import { beforeEach, expect, test, vi } from "vitest";
import type { RdpInputEvent } from "../types";
import { MAX_EVENTS_PER_SEND, RdpController } from "./controller";

class FakeImageData {
  constructor(
    public data: Uint8ClampedArray,
    public width: number,
    public height: number,
  ) {}
}
vi.stubGlobal("ImageData", FakeImageData);

let frames: (() => void)[] = [];
let sent: RdpInputEvent[][] = [];
let put: { data: FakeImageData; x: number; y: number }[] = [];

function setup() {
  const ctx = {
    putImageData: (data: FakeImageData, x: number, y: number) => put.push({ data, x, y }),
  };
  const canvas = {
    width: 0,
    height: 0,
    getContext: () => ctx,
    getBoundingClientRect: () => ({ left: 100, top: 50, width: 960, height: 540 }),
  } as unknown as HTMLCanvasElement;
  const controller = new RdpController({
    canvas,
    send: (events) => sent.push(events),
    requestAnimationFrame: (cb) => frames.push(cb),
    cancelAnimationFrame: () => {},
  });
  controller.setDesktop(1920, 1080);
  return { controller, canvas };
}

const frame = () => {
  const due = frames;
  frames = [];
  for (const cb of due) cb();
};

beforeEach(() => {
  frames = [];
  sent = [];
  put = [];
});

test("setDesktop sizes the canvas", () => {
  const { canvas } = setup();
  expect(canvas.width).toBe(1920);
  expect(canvas.height).toBe(1080);
});

test("setDesktop refuses a zero, fractional or negative size", () => {
  const { controller, canvas } = setup();
  controller.setDesktop(0, 600);
  controller.setDesktop(800.5, 600);
  controller.setDesktop(-1, 600);
  expect(canvas.width).toBe(1920);
  expect(controller.desktop).toEqual({ width: 1920, height: 1080 });
});

test("paint writes pixels at the rect", () => {
  const { controller } = setup();
  const rgba = new Uint8Array(2 * 3 * 4).map((_, i) => i);
  controller.paint({ x: 10, y: 20, width: 2, height: 3 }, rgba);
  expect(put).toHaveLength(1);
  expect(put[0]!.x).toBe(10);
  expect(put[0]!.y).toBe(20);
  expect(put[0]!.data.width).toBe(2);
  expect(put[0]!.data.height).toBe(3);
  expect([...put[0]!.data.data]).toEqual([...rgba]);
});

test("paint works on a view into a larger buffer", () => {
  const { controller } = setup();
  const whole = new Uint8Array(8 + 4).map((_, i) => i);
  controller.paint({ x: 0, y: 0, width: 1, height: 1 }, new Uint8Array(whole.buffer, 8));
  expect([...put[0]!.data.data]).toEqual([8, 9, 10, 11]);
});

test("paint ignores a message whose length does not match its rect", () => {
  const { controller } = setup();
  controller.paint({ x: 0, y: 0, width: 2, height: 2 }, new Uint8Array(15));
  controller.paint({ x: 0, y: 0, width: 0, height: 2 }, new Uint8Array(0));
  expect(put).toHaveLength(0);
});

test("mouse moves are coalesced per frame", () => {
  const { controller } = setup();
  for (let i = 0; i < 20; i++) controller.pointerMove({ clientX: 100 + i * 10, clientY: 50 });
  expect(sent).toHaveLength(0);
  expect(frames).toHaveLength(1);
  frame();
  expect(sent).toEqual([[{ kind: "mouseMove", x: 380, y: 0 }]]);
  // nothing moved: no frame, no send
  controller.pointerMove({ clientX: 290, clientY: 50 });
  expect(frames).toHaveLength(0);
});

test("a click carries its position and goes out with the frame", () => {
  const { controller } = setup();
  controller.pointerButton({ clientX: 580, clientY: 320, button: 0 }, true);
  controller.pointerButton({ clientX: 580, clientY: 320, button: 0 }, false);
  frame();
  expect(sent).toEqual([
    [
      { kind: "mouseMove", x: 960, y: 540 },
      { kind: "mouseButton", button: "left", pressed: true },
      { kind: "mouseButton", button: "left", pressed: false },
    ],
  ]);
});

test("an unknown mouse button is ignored", () => {
  const { controller } = setup();
  controller.pointerButton({ clientX: 100, clientY: 50, button: 7 }, true);
  frame();
  expect(sent).toEqual([]);
});

test("key events are sent at once, behind the mouse input that waits", () => {
  const { controller } = setup();
  controller.pointerMove({ clientX: 100, clientY: 50 });
  controller.pointerMove({ clientX: 200, clientY: 100 });
  expect(controller.keyDown({ code: "KeyA" })).toBe(true);
  expect(sent).toEqual([
    [
      { kind: "mouseMove", x: 200, y: 100 },
      { kind: "key", scancode: 0x1e, extended: false, pressed: true },
    ],
  ]);
  // the frame that was waiting has nothing left to send
  frame();
  expect(sent).toHaveLength(1);
  expect(controller.keyUp({ code: "KeyA" })).toBe(true);
  expect(sent[1]).toEqual([{ kind: "key", scancode: 0x1e, extended: false, pressed: false }]);
});

test("a key without a scancode is not sent and not handled", () => {
  const { controller } = setup();
  expect(controller.keyDown({ code: "MediaPlayPause" })).toBe(false);
  expect(controller.keyUp({ code: "Pause" })).toBe(false);
  expect(sent).toEqual([]);
});

test("blur releases pressed keys and buttons, and only those", () => {
  const { controller } = setup();
  controller.keyDown({ code: "ControlLeft" });
  controller.keyDown({ code: "ArrowUp" });
  controller.keyDown({ code: "KeyC" });
  controller.keyUp({ code: "KeyC" });
  controller.pointerButton({ clientX: 100, clientY: 50, button: 2 }, true);
  frame();
  sent = [];
  controller.blur();
  expect(sent.flat()).toEqual([
    { kind: "key", scancode: 0x1d, extended: false, pressed: false },
    { kind: "key", scancode: 0x48, extended: true, pressed: false },
    { kind: "mouseButton", button: "right", pressed: false },
  ]);
  // a second blur has nothing left to release
  sent = [];
  controller.releaseAll();
  expect(sent).toEqual([]);
});

test("a pending frame is flushed by a release so the order is kept", () => {
  const { controller } = setup();
  controller.pointerButton({ clientX: 100, clientY: 50, button: 0 }, true);
  controller.releaseAll();
  expect(sent.flat().map((e) => e.kind)).toEqual(["mouseMove", "mouseButton", "mouseButton"]);
  frame();
  expect(sent).toHaveLength(1);
});

test("wheel: down is negative, notches become 120 and small deltas add up", () => {
  const { controller } = setup();
  controller.wheel({ deltaX: 0, deltaY: 100, deltaMode: 0 });
  frame();
  expect(sent.flat()).toEqual([{ kind: "wheel", vertical: true, delta: -120 }]);
  sent = [];
  // a trackpad: nothing is sent until a whole unit has collected
  controller.wheel({ deltaX: 0, deltaY: -0.4, deltaMode: 0 });
  frame();
  expect(sent).toEqual([]);
  for (let i = 0; i < 4; i++) controller.wheel({ deltaX: 0, deltaY: -0.5, deltaMode: 0 });
  frame();
  expect(sent.flat()).toEqual([{ kind: "wheel", vertical: true, delta: 2 }]);
  sent = [];
  controller.wheel({ deltaX: 50, deltaY: 0, deltaMode: 0 });
  frame();
  expect(sent.flat()).toEqual([{ kind: "wheel", vertical: false, delta: 60 }]);
});

test("a huge wheel delta is clamped to the 16-bit range", () => {
  const { controller } = setup();
  controller.wheel({ deltaX: 0, deltaY: 1e9, deltaMode: 2 });
  frame();
  expect(sent.flat()).toEqual([{ kind: "wheel", vertical: true, delta: -32767 }]);
});

test("a long burst is sent in batches Rust accepts", () => {
  const { controller } = setup();
  for (let i = 0; i < MAX_EVENTS_PER_SEND + 10; i++) {
    controller.pointerButton({ clientX: 100, clientY: 50, button: 0 }, i % 2 === 0);
  }
  frame();
  expect(sent.map((s) => s.length)).toEqual([MAX_EVENTS_PER_SEND, 10 + 1]);
});

test("input before the desktop size is known is dropped", () => {
  const canvas = {
    getContext: () => ({}),
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 100, height: 100 }),
  } as unknown as HTMLCanvasElement;
  const controller = new RdpController({
    canvas,
    send: (events) => sent.push(events),
    requestAnimationFrame: (cb) => frames.push(cb),
    cancelAnimationFrame: () => {},
  });
  controller.pointerMove({ clientX: 5, clientY: 5 });
  frame();
  expect(sent).toEqual([]);
});
