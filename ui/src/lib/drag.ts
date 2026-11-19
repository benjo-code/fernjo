// Pointer-drag state machine. It never touches the DOM: the view feeds it pointer positions and
// what is under the pointer, and reads `state` / `target` back to draw the indicator.
import { resolveDropMany, type DropPosition } from "./drop-target";
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
  onMove: (dragIds: string[], parentId: string | null, index: number) => void;
}

export class DragController {
  state: "idle" | "pending" | "dragging" = "idle";
  /** The row that was pressed. */
  dragId: string | null = null;
  /** Every node that moves: the whole selection, or just the pressed row. */
  dragIds: string[] = [];
  target: DragTarget | null = null;

  #opts: DragOptions;
  #startX = 0;
  #startY = 0;

  constructor(opts: DragOptions) {
    this.#opts = opts;
  }

  /** `ids` is the selection (tops) to drag along; fewer than two means only the pressed row. */
  pointerDown(id: string, x: number, y: number, ids?: string[]) {
    this.#reset();
    this.state = "pending";
    this.dragId = id;
    this.dragIds = ids && ids.length > 1 ? [...ids] : [id];
    this.#startX = x;
    this.#startY = y;
  }

  pointerMove(x: number, y: number, hit: Hit | null) {
    if (this.state === "idle" || this.dragId === null) return;
    if (this.state === "pending") {
      if (Math.hypot(x - this.#startX, y - this.#startY) < this.#opts.threshold) return;
      this.state = "dragging";
    }
    const drop = hit
      ? resolveDropMany(this.#opts.rows(), this.dragIds, hit.rowId, hit.position)
      : null;
    this.target = hit && drop ? { ...hit, ...drop } : null;
  }

  /** Ends the gesture. Returns `true` if it was a drag (so the click that follows must be ignored). */
  pointerUp(): boolean {
    const wasDrag = this.state === "dragging";
    const { dragId, dragIds, target } = this;
    this.#reset();
    if (wasDrag && dragId !== null && target) {
      this.#opts.onMove(dragIds, target.parentId, target.index);
    }
    return wasDrag;
  }

  cancel() {
    this.#reset();
  }

  #reset() {
    this.state = "idle";
    this.dragId = null;
    this.dragIds = [];
    this.target = null;
  }
}
