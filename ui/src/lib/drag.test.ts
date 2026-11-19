import { describe, expect, test, vi } from "vitest";
import { DragController } from "./drag";
import { flattenRows } from "./rows";
import { conn, folder } from "./testing";

const tree = () => [folder("A", [conn("a1"), conn("a2")]), conn("B"), folder("C")];

function setup() {
  const onMove = vi.fn();
  const rows = flattenRows(tree(), new Set(["A"]), "");
  const ctrl = new DragController({ threshold: 4, rows: () => rows, onMove });
  return { ctrl, onMove };
}

describe("DragController", () => {
  test("small_wobble_is_a_click_not_a_drag", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerDown("B", 10, 10);
    expect(ctrl.state).toBe("pending");
    ctrl.pointerMove(13, 10, { rowId: "C", position: "inside" });
    expect(ctrl.state).toBe("pending");
    expect(ctrl.pointerUp()).toBe(false);
    expect(ctrl.state).toBe("idle");
    expect(onMove).not.toHaveBeenCalled();
  });

  test("crossing_threshold_starts_dragging", () => {
    const { ctrl } = setup();
    ctrl.pointerDown("B", 10, 10);
    ctrl.pointerMove(10, 15, null);
    expect(ctrl.state).toBe("dragging");
    expect(ctrl.dragId).toBe("B");
  });

  test("diagonal_movement_counts_by_distance", () => {
    const { ctrl } = setup();
    ctrl.pointerDown("B", 0, 0);
    ctrl.pointerMove(3, 3, null); // 4.24 px
    expect(ctrl.state).toBe("dragging");
  });

  test("release_over_valid_target_calls_onMove_once", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerDown("B", 10, 10);
    ctrl.pointerMove(10, 40, { rowId: "C", position: "inside" });
    expect(ctrl.pointerUp()).toBe(true);
    expect(onMove).toHaveBeenCalledTimes(1);
    expect(onMove).toHaveBeenCalledWith(["B"], "C", 0);
    expect(ctrl.state).toBe("idle");
    expect(ctrl.pointerUp()).toBe(false);
    expect(onMove).toHaveBeenCalledTimes(1);
  });

  test("dragging a selected row moves the whole selection", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerDown("B", 10, 10, ["B", "C"]);
    ctrl.pointerMove(10, 40, { rowId: "A", position: "inside" });
    expect(ctrl.dragIds).toEqual(["B", "C"]);
    expect(ctrl.target).toEqual({ rowId: "A", position: "inside", parentId: "A", index: 2 });
    ctrl.pointerUp();
    expect(onMove).toHaveBeenCalledWith(["B", "C"], "A", 2);
  });

  test("dragging an unselected row moves only that row", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerDown("B", 10, 10);
    ctrl.pointerMove(10, 40, { rowId: "A", position: "inside" });
    expect(ctrl.dragIds).toEqual(["B"]);
    ctrl.pointerUp();
    expect(onMove).toHaveBeenCalledWith(["B"], "A", 2);
    // a one-element selection is just that row
    ctrl.pointerDown("C", 10, 10, ["C"]);
    expect(ctrl.dragIds).toEqual(["C"]);
  });

  test("a_target_among_the_dragged_rows_is_no_target", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerDown("B", 10, 10, ["B", "C"]);
    ctrl.pointerMove(10, 40, { rowId: "C", position: "inside" });
    expect(ctrl.target).toBeNull();
    ctrl.pointerUp();
    expect(onMove).not.toHaveBeenCalled();
  });

  test("target_is_exposed_for_the_indicator", () => {
    const { ctrl } = setup();
    ctrl.pointerDown("B", 10, 10);
    ctrl.pointerMove(10, 40, { rowId: "A", position: "inside" });
    expect(ctrl.target).toEqual({ rowId: "A", position: "inside", parentId: "A", index: 2 });
    ctrl.pointerMove(10, 60, null);
    expect(ctrl.target).toBeNull();
  });

  test("release_outside_the_tree_cancels", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerDown("B", 10, 10);
    ctrl.pointerMove(300, 10, null);
    expect(ctrl.pointerUp()).toBe(true); // it was a drag, so the click must be swallowed
    expect(onMove).not.toHaveBeenCalled();
    expect(ctrl.state).toBe("idle");
  });

  test("escape_cancel_returns_to_idle_without_call", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerDown("B", 10, 10);
    ctrl.pointerMove(10, 40, { rowId: "C", position: "inside" });
    ctrl.cancel();
    expect(ctrl.state).toBe("idle");
    expect(ctrl.target).toBeNull();
    expect(ctrl.dragId).toBeNull();
    ctrl.pointerUp();
    expect(onMove).not.toHaveBeenCalled();
  });

  test("release_over_invalid_target_calls_nothing", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerDown("A", 10, 10);
    ctrl.pointerMove(10, 40, { rowId: "a1", position: "inside" });
    expect(ctrl.target).toBeNull();
    ctrl.pointerUp();
    expect(onMove).not.toHaveBeenCalled();
  });

  test("moving_without_pressing_does_nothing", () => {
    const { ctrl, onMove } = setup();
    ctrl.pointerMove(50, 50, { rowId: "C", position: "inside" });
    expect(ctrl.state).toBe("idle");
    expect(ctrl.pointerUp()).toBe(false);
    expect(onMove).not.toHaveBeenCalled();
  });
});
