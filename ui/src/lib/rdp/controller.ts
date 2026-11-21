// The canvas side of an RDP tab: painting regions and turning DOM input into RDP input events.
// It knows nothing about sessions; the component wires `send` to the `SessionStore`.
import type { RdpInputEvent } from "../types";
import { scancodeFor } from "./keymap";
import { buttonOf, toDesktop, type DesktopSize, type RdpButton } from "./pointer";

export interface RdpControllerOptions {
  canvas: HTMLCanvasElement;
  /** Receives batches of at most `MAX_EVENTS_PER_SEND` events, in order. */
  send(events: RdpInputEvent[]): void;
  requestAnimationFrame?: (callback: () => void) => number;
  cancelAnimationFrame?: (handle: number) => void;
  /** Asks Rust for a new desktop size; called once after the box has stopped changing. */
  resize?(width: number, height: number): void;
  /** `true` for a fixed resolution: the desktop never follows the box, the canvas scales. */
  fixed?(): boolean;
  setTimeout?: (callback: () => void, ms: number) => number;
  clearTimeout?: (handle: number) => void;
}

/** The box must stay unchanged this long before a resize is sent. */
export const RESIZE_DEBOUNCE_MS = 150;
/** Smallest and largest desktop side asked for (Rust clamps the same way). */
export const MIN_SIDE = 200;
export const MAX_SIDE = 8192;

/**
 * The desktop size for a box: both sides rounded down to an even number and clamped to
 * `MIN_SIDE..=MAX_SIDE`. `null` for a box that is empty or not a number (a hidden tab), which
 * must never be sent: a zero-sized desktop is refused by the server.
 */
export function fitSize(width: number, height: number): { width: number; height: number } | null {
  if (!(width >= 1) || !(height >= 1)) return null;
  const side = (v: number) => Math.min(MAX_SIDE, Math.max(MIN_SIDE, Math.floor(v) & ~1));
  return { width: side(width), height: side(height) };
}

/**
 * The size the canvas is shown at inside a box: the whole desktop, scaled up or down with the
 * aspect ratio kept. `null` while the box or the desktop is empty.
 */
export function displaySize(
  box: { width: number; height: number },
  desktop: { width: number; height: number },
): { width: number; height: number } | null {
  if (!(box.width >= 1) || !(box.height >= 1) || desktop.width < 1 || desktop.height < 1) {
    return null;
  }
  const scale = Math.min(box.width / desktop.width, box.height / desktop.height);
  return {
    width: Math.max(1, Math.floor(desktop.width * scale)),
    height: Math.max(1, Math.floor(desktop.height * scale)),
  };
}

/** Rust refuses more events than this in one call (`MAX_INPUT_EVENTS`). */
export const MAX_EVENTS_PER_SEND = 256;

/** RDP counts a wheel notch as 120; a Chromium notch is 100 pixels. */
const WHEEL_PER_PIXEL = 1.2;
/** `deltaMode` 1 (lines) and 2 (pages) in pixels. */
const LINE_PIXELS = 40;
const PAGE_PIXELS = 800;
const WHEEL_LIMIT = 32767;

export interface Region {
  x: number;
  y: number;
  width: number;
  height: number;
}

export class RdpController {
  #canvas: HTMLCanvasElement;
  #send: (events: RdpInputEvent[]) => void;
  #raf: (callback: () => void) => number;
  #caf: (handle: number) => void;
  #desktop: DesktopSize = { width: 0, height: 0 };
  #ctx: CanvasRenderingContext2D | null = null;
  #queue: RdpInputEvent[] = [];
  #frame: number | null = null;
  /** `KeyboardEvent.code` of the keys that are down, so a lost focus can release them. */
  #keys = new Set<string>();
  #buttons = new Set<RdpButton>();
  #lastMove: { x: number; y: number } | null = null;
  #wheelRest = { x: 0, y: 0 };
  #resize: ((width: number, height: number) => void) | undefined;
  #fixed: () => boolean;
  #setTimeout: (callback: () => void, ms: number) => number;
  #clearTimeout: (handle: number) => void;
  #timer: number | null = null;

  constructor(options: RdpControllerOptions) {
    this.#canvas = options.canvas;
    this.#send = options.send;
    this.#resize = options.resize;
    this.#fixed = options.fixed ?? (() => false);
    this.#setTimeout = options.setTimeout ?? ((cb, ms) => window.setTimeout(cb, ms));
    this.#clearTimeout = options.clearTimeout ?? ((h) => window.clearTimeout(h));
    this.#raf = options.requestAnimationFrame ?? ((cb) => requestAnimationFrame(cb));
    this.#caf = options.cancelAnimationFrame ?? ((h) => cancelAnimationFrame(h));
  }

  get desktop(): DesktopSize {
    return this.#desktop;
  }

  // --- fit to the tab ----------------------------------------------------------------------

  /**
   * The box around the canvas has this size now. After it stops changing (the debounce) the
   * desktop is asked to follow, unless the resolution is fixed, the box is empty, or the desktop
   * already has the size.
   */
  boxChanged(width: number, height: number): void {
    if (this.#timer !== null) this.#clearTimeout(this.#timer);
    this.#timer = this.#setTimeout(() => {
      this.#timer = null;
      if (this.#fixed()) return;
      const wanted = fitSize(width, height);
      if (!wanted) return;
      if (wanted.width === this.#desktop.width && wanted.height === this.#desktop.height) return;
      this.#resize?.(wanted.width, wanted.height);
    }, RESIZE_DEBOUNCE_MS);
  }

  // --- painting ----------------------------------------------------------------------------

  /** The remote desktop has this size; the canvas takes it (which also clears it). */
  setDesktop(width: number, height: number): void {
    if (!Number.isInteger(width) || !Number.isInteger(height) || width < 1 || height < 1) return;
    this.#desktop = { width, height };
    this.#canvas.width = width;
    this.#canvas.height = height;
    this.#ctx = this.#canvas.getContext("2d");
    this.#lastMove = null;
  }

  /** Writes the pixels of one region (`rgba` is exactly `width * height * 4` bytes, row by row). */
  paint(rect: Region, rgba: Uint8Array): void {
    const { x, y, width, height } = rect;
    if (!this.#ctx || width < 1 || height < 1) return;
    if (rgba.byteLength !== width * height * 4) return;
    const pixels = new Uint8ClampedArray(
      rgba.buffer as ArrayBuffer,
      rgba.byteOffset,
      rgba.byteLength,
    );
    this.#ctx.putImageData(new ImageData(pixels, width, height), x, y);
  }

  // --- input -------------------------------------------------------------------------------

  /** `true` when the key has a scancode and was sent (the caller then stops the browser). */
  keyDown(e: Pick<KeyboardEvent, "code">): boolean {
    return this.#key(e.code, true);
  }

  /** Whether the key's press was forwarded and its release is still to come. */
  isDown(code: string): boolean {
    return this.#keys.has(code);
  }

  keyUp(e: Pick<KeyboardEvent, "code">): boolean {
    return this.#key(e.code, false);
  }

  #key(code: string, pressed: boolean): boolean {
    const sc = scancodeFor(code);
    if (!sc) return false;
    if (pressed) this.#keys.add(code);
    else this.#keys.delete(code);
    // a key goes out at once, behind whatever mouse input is already waiting
    this.#queue.push({ kind: "key", scancode: sc.scancode, extended: sc.extended, pressed });
    this.flush();
    return true;
  }

  pointerMove(e: Pick<PointerEvent, "clientX" | "clientY">): void {
    if (!this.#moveTo(e)) return;
    this.#schedule();
  }

  pointerButton(e: Pick<PointerEvent, "clientX" | "clientY" | "button">, pressed: boolean): void {
    const button = buttonOf(e.button);
    if (!button) return;
    // the click lands where the pointer is, even when no move came first
    this.#moveTo(e);
    if (pressed) this.#buttons.add(button);
    else this.#buttons.delete(button);
    this.#queue.push({ kind: "mouseButton", button, pressed });
    this.#schedule();
  }

  /** Queues a move to the pointer's desktop pixel; the newest queued move replaces an older one. */
  #moveTo(e: Pick<PointerEvent, "clientX" | "clientY">): boolean {
    if (this.#desktop.width < 1) return false;
    const { x, y } = toDesktop(
      e.clientX,
      e.clientY,
      this.#canvas.getBoundingClientRect(),
      this.#desktop,
    );
    if (this.#lastMove?.x === x && this.#lastMove.y === y) return true;
    this.#lastMove = { x, y };
    const last = this.#queue[this.#queue.length - 1];
    if (last?.kind === "mouseMove") {
      last.x = x;
      last.y = y;
    } else {
      this.#queue.push({ kind: "mouseMove", x, y });
    }
    return true;
  }

  wheel(e: Pick<WheelEvent, "deltaX" | "deltaY" | "deltaMode">): void {
    const unit = e.deltaMode === 1 ? LINE_PIXELS : e.deltaMode === 2 ? PAGE_PIXELS : 1;
    // scrolling down (positive deltaY) turns the RDP wheel away from the user (negative)
    this.#wheelAxis("y", -e.deltaY * unit * WHEEL_PER_PIXEL, true);
    this.#wheelAxis("x", e.deltaX * unit * WHEEL_PER_PIXEL, false);
    this.#schedule();
  }

  /** Small trackpad deltas add up until they make a whole unit. */
  #wheelAxis(axis: "x" | "y", amount: number, vertical: boolean): void {
    if (!Number.isFinite(amount)) return;
    const total = this.#wheelRest[axis] + amount;
    const whole = Math.trunc(total);
    this.#wheelRest[axis] = total - whole;
    if (whole === 0) return;
    // one frame's turns on an axis make one event
    const last = this.#queue[this.#queue.length - 1];
    if (last?.kind === "wheel" && last.vertical === vertical) {
      last.delta = Math.max(-WHEEL_LIMIT, Math.min(WHEEL_LIMIT, last.delta + whole));
      return;
    }
    this.#queue.push({
      kind: "wheel",
      vertical,
      delta: Math.max(-WHEEL_LIMIT, Math.min(WHEEL_LIMIT, whole)),
    });
  }

  /** Releases every key and button that is down, for example on losing focus. */
  releaseAll(): void {
    for (const code of this.#keys) {
      const sc = scancodeFor(code);
      if (sc) {
        this.#queue.push({
          kind: "key",
          scancode: sc.scancode,
          extended: sc.extended,
          pressed: false,
        });
      }
    }
    this.#keys.clear();
    for (const button of this.#buttons) {
      this.#queue.push({ kind: "mouseButton", button, pressed: false });
    }
    this.#buttons.clear();
    this.#wheelRest = { x: 0, y: 0 };
    this.flush();
  }

  blur(): void {
    this.releaseAll();
  }

  /** Sends what is queued now (also the end of every animation frame that had input). */
  flush(): void {
    if (this.#frame !== null) {
      this.#caf(this.#frame);
      this.#frame = null;
    }
    const events = this.#queue;
    this.#queue = [];
    for (let i = 0; i < events.length; i += MAX_EVENTS_PER_SEND) {
      this.#send(events.slice(i, i + MAX_EVENTS_PER_SEND));
    }
  }

  #schedule(): void {
    if (this.#frame !== null || this.#queue.length === 0) return;
    this.#frame = this.#raf(() => {
      this.#frame = null;
      this.flush();
    });
  }

  dispose(): void {
    if (this.#timer !== null) this.#clearTimeout(this.#timer);
    this.#timer = null;
    if (this.#frame !== null) this.#caf(this.#frame);
    this.#frame = null;
    this.#queue = [];
    this.#keys.clear();
    this.#buttons.clear();
  }
}
