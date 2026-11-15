// Pointer-drag state machine. It never touches the DOM: the view feeds it pointer positions and
// what is under the pointer, and reads `state` / `target` back to draw the indicator.
import { resolveDrop, type DropPosition } from "./drop-target";
import type { Row } from "./rows";

export interface Hit {
  rowId: string;
  position: DropPosition;
}

export interface DragTarget extends Hit {
  parentId: string | null;
  index: number;
}

export interface DragOptions {
  /** Pointer travel in px before a press becomes a drag. */
  threshold: number;
  /** The rows currently shown, used to resolve drops. */
  rows: () => Row[];
  onMove: (dragId: string, parentId: string | null, index: number) => void;
}

export class DragController {
  state: "idle" | "pending" | "dragging" = "idle";
  dragId: string | null = null;
  target: DragTarget | null = null;

  #opts: DragOptions;
  #startX = 0;
  #startY = 0;

  constructor(opts: DragOptions) {
    this.#opts = opts;
  }

  pointerDown(id: string, x: number, y: number) {
    this.#reset();
    this.state = "pending";
    this.dragId = id;
    this.#startX = x;
    this.#startY = y;
  }

  pointerMove(x: number, y: number, hit: Hit | null) {
    if (this.state === "idle" || this.dragId === null) return;
    if (this.state === "pending") {
      if (Math.hypot(x - this.#startX, y - this.#startY) < this.#opts.threshold) return;
      this.state = "dragging";
    }
    const drop = hit ? resolveDrop(this.#opts.rows(), this.dragId, hit.rowId, hit.position) : null;
    this.target = hit && drop ? { ...hit, ...drop } : null;
  }

  /** Ends the gesture. Returns `true` if it was a drag (so the click that follows must be ignored). */
  pointerUp(): boolean {
    const wasDrag = this.state === "dragging";
    const { dragId, target } = this;
    this.#reset();
    if (wasDrag && dragId !== null && target) {
      this.#opts.onMove(dragId, target.parentId, target.index);
    }
    return wasDrag;
  }

  cancel() {
    this.#reset();
  }

  #reset() {
    this.state = "idle";
    this.dragId = null;
    this.target = null;
  }
}
