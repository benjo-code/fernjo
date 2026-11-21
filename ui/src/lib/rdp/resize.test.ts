import { beforeEach, expect, test, vi } from "vitest";
import {
  displaySize,
  fitSize,
  MAX_SIDE,
  MIN_SIDE,
  RESIZE_DEBOUNCE_MS,
  RdpController,
} from "./controller";

vi.stubGlobal(
  "ImageData",
  class {
    constructor(
      public data: Uint8ClampedArray,
      public width: number,
      public height: number,
    ) {}
  },
);

let timers: { cb: () => void; ms: number; handle: number }[] = [];
let resizes: [number, number][] = [];
let nextHandle = 1;

function setup(fixed = false) {
  const canvas = {
    width: 0,
    height: 0,
    getContext: () => ({ putImageData() {} }),
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 800, height: 600 }),
  } as unknown as HTMLCanvasElement;
  const controller = new RdpController({
    canvas,
    send() {},
    requestAnimationFrame: () => 0,
    cancelAnimationFrame() {},
    resize: (w, h) => resizes.push([w, h]),
    fixed: () => fixed,
    setTimeout: (cb, ms) => {
      const handle = nextHandle++;
      timers.push({ cb, ms, handle });
      return handle;
    },
    clearTimeout: (handle) => {
      timers = timers.filter((t) => t.handle !== handle);
    },
  });
  controller.setDesktop(1280, 800);
  return controller;
}

/** Lets the last pending timer fire, as the clock would after the debounce. */
function settle() {
  const due = timers;
  timers = [];
  for (const t of due) t.cb();
}

beforeEach(() => {
  timers = [];
  resizes = [];
});

test("sizes are even and clamped", () => {
  expect(fitSize(1281, 801)).toEqual({ width: 1280, height: 800 });
  expect(fitSize(1920.9, 1080.9)).toEqual({ width: 1920, height: 1080 });
  expect(fitSize(10, 20)).toEqual({ width: MIN_SIDE, height: MIN_SIDE });
  expect(fitSize(100_000, 9000)).toEqual({ width: MAX_SIDE, height: MAX_SIDE });
  expect(fitSize(MAX_SIDE + 1, 999.5)).toEqual({ width: MAX_SIDE, height: 998 });
});

test("zero or hidden box sends no resize", () => {
  expect(fitSize(0, 600)).toBeNull();
  expect(fitSize(800, 0)).toBeNull();
  expect(fitSize(Number.NaN, 600)).toBeNull();
  expect(fitSize(-5, -5)).toBeNull();
  const c = setup();
  c.boxChanged(0, 0);
  settle();
  c.boxChanged(0, 600);
  settle();
  expect(resizes).toEqual([]);
});

test("a drag through many sizes sends one resize after it settles", () => {
  const c = setup();
  for (let w = 900; w < 1100; w += 7) c.boxChanged(w, 700);
  expect(resizes).toEqual([]);
  expect(timers).toHaveLength(1);
  expect(timers[0]!.ms).toBe(RESIZE_DEBOUNCE_MS);
  c.boxChanged(1101, 701);
  settle();
  expect(resizes).toEqual([[1100, 700]]);
});

test("a box that already has the desktop's size sends nothing", () => {
  const c = setup();
  c.boxChanged(1280, 800);
  settle();
  c.boxChanged(1281, 801);
  settle();
  expect(resizes).toEqual([]);
});

test("fixed mode never sends a resize and scales", () => {
  const c = setup(true);
  c.boxChanged(1000, 500);
  settle();
  expect(resizes).toEqual([]);
  // the canvas scales into the box with the aspect ratio kept
  expect(displaySize({ width: 1000, height: 500 }, { width: 1280, height: 800 })).toEqual({
    width: 800,
    height: 500,
  });
});

test("a pending resize is dropped when the controller is disposed", () => {
  const c = setup();
  c.boxChanged(1000, 700);
  c.dispose();
  settle();
  expect(resizes).toEqual([]);
});

test("the display size keeps the aspect ratio, up and down, and is empty without room", () => {
  expect(displaySize({ width: 640, height: 400 }, { width: 1280, height: 800 })).toEqual({
    width: 640,
    height: 400,
  });
  expect(displaySize({ width: 2560, height: 1000 }, { width: 1280, height: 800 })).toEqual({
    width: 1600,
    height: 1000,
  });
  expect(displaySize({ width: 0, height: 500 }, { width: 1280, height: 800 })).toBeNull();
  expect(displaySize({ width: 500, height: 500 }, { width: 0, height: 0 })).toBeNull();
  // never rounds up past the box
  const shown = displaySize({ width: 1001, height: 1001 }, { width: 3, height: 7 })!;
  expect(shown.width).toBeLessThanOrEqual(1001);
  expect(shown.height).toBeLessThanOrEqual(1001);
});
