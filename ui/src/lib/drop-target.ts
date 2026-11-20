// Pure geometry and index maths for dragging a row to a new place in the tree.
// Rust still validates every move; this only decides where a drop would land.
import type { Row } from "./rows";

export type DropPosition = "before" | "after" | "inside";

/** Which part of a row the pointer is over: folders have a middle zone, connections do not. */
export function dropPosition(
  y: number,
  rect: { top: number; height: number },
  isFolder: boolean,
): DropPosition {
  const fraction = (y - rect.top) / rect.height;
  if (isFolder) {
    if (fraction < 0.25) return "before";
    if (fraction >= 0.75) return "after";
    return "inside";
  }
  return fraction < 0.5 ? "before" : "after";
}

export interface Drop {
  parentId: string | null;
  /** Counted among the new parent's children after the dragged node was taken out. */
  index: number;
}

/**
 * Where dropping `dragId` on `targetId` would put it, or `null` when nothing would change or
 * the drop is invalid (on itself, into its own subtree).
 */
export function resolveDrop(
  rows: Row[],
  dragId: string,
  targetId: string,
  position: DropPosition,
): Drop | null {
  if (dragId === targetId) return null;
  const byId = new Map(rows.map((r) => [r.node.id, r]));
  const drag = byId.get(dragId);
  const target = byId.get(targetId);
  if (!drag || !target) return null;

  // a node can never go into (or next to) something inside itself
  for (let at: Row | undefined = target; at; at = at.parentId ? byId.get(at.parentId) : undefined) {
    if (at.node.id === dragId) return null;
  }

  const isFolder = target.node.type === "folder";
  let drop: Drop;
  if (
    position === "inside" ||
    (position === "after" && isFolder && target.expanded && target.hasChildren)
  ) {
    if (!isFolder || target.node.type !== "folder") return null;
    const count = target.node.children.length - (drag.parentId === target.node.id ? 1 : 0);
    drop = { parentId: target.node.id, index: position === "inside" ? count : 0 };
  } else {
    let index = position === "before" ? target.index : target.index + 1;
    if (drag.parentId === target.parentId && drag.index < index) index -= 1;
    drop = { parentId: target.parentId, index };
  }

  if (drop.parentId === drag.parentId && drop.index === drag.index) return null;
  return drop;
}

/**
 * Where dropping the nodes `dragIds` (given in the order they will end up in) on `targetId` would
 * put them. `index` counts the new parent's children with every dragged node taken out. `null`
 * when the target is a dragged node or inside a dragged subtree, or when nothing would change.
 * `dragIds` are expected to be selection tops: none lies inside another's subtree.
 */
export function resolveDropMany(
  rows: Row[],
  dragIds: string[],
  targetId: string,
  position: DropPosition,
): Drop | null {
  if (dragIds.length === 0) return null;
  const byId = new Map(rows.map((r) => [r.node.id, r]));
  const target = byId.get(targetId);
  if (!target) return null;
  const dragged = new Set(dragIds);
  const drags: Row[] = [];
  for (const id of dragIds) {
    const row = byId.get(id);
    if (!row) return null;
    drags.push(row);
  }

  for (let at: Row | undefined = target; at; at = at.parentId ? byId.get(at.parentId) : undefined) {
    if (dragged.has(at.node.id)) return null;
  }

  const isFolder = target.node.type === "folder";
  let drop: Drop;
  if (
    position === "inside" ||
    (position === "after" && isFolder && target.expanded && target.hasChildren)
  ) {
    if (target.node.type !== "folder") return null;
    const leaving = drags.filter((d) => d.parentId === target.node.id).length;
    drop = {
      parentId: target.node.id,
      index: position === "inside" ? target.node.children.length - leaving : 0,
    };
  } else {
    const raw = position === "before" ? target.index : target.index + 1;
    const leaving = drags.filter((d) => d.parentId === target.parentId && d.index < raw).length;
    drop = { parentId: target.parentId, index: raw - leaving };
  }

  // already there: the same parent, side by side, in this order, starting at the slot
  const inPlace = drags.every((d, i) => d.parentId === drop.parentId && d.index === drop.index + i);
  return inPlace ? null : drop;
}
