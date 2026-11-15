import { describe, expect, test } from "vitest";
import { dropPosition, resolveDrop } from "./drop-target";
import { flattenRows } from "./rows";
import { conn, folder } from "./testing";

const rect = { top: 100, height: 24 };

describe("dropPosition", () => {
  test("connection_row_halves", () => {
    expect(dropPosition(100, rect, false)).toBe("before");
    expect(dropPosition(111, rect, false)).toBe("before");
    expect(dropPosition(112, rect, false)).toBe("after");
    expect(dropPosition(123, rect, false)).toBe("after");
  });

  test("folder_row_quarters_and_middle", () => {
    expect(dropPosition(100, rect, true)).toBe("before");
    expect(dropPosition(105, rect, true)).toBe("before");
    expect(dropPosition(106, rect, true)).toBe("inside");
    expect(dropPosition(117, rect, true)).toBe("inside");
    expect(dropPosition(118, rect, true)).toBe("after");
    expect(dropPosition(123, rect, true)).toBe("after");
  });
});

// root: A (open: a1, a2), B, C (empty folder)
const tree = () => [folder("A", [conn("a1"), conn("a2")]), conn("B"), folder("C")];
const rows = () => flattenRows(tree(), new Set(["A"]), "");

describe("resolveDrop", () => {
  test("before_B_from_A_is_root_index_1", () => {
    // a1 leaves A, B is then root child 1
    expect(resolveDrop(rows(), "a1", "B", "before")).toEqual({ parentId: null, index: 1 });
  });

  test("before_B_from_C_lands_at_root_index_1", () => {
    expect(resolveDrop(rows(), "C", "B", "before")).toEqual({ parentId: null, index: 1 });
  });

  test("after_C_is_root_end", () => {
    expect(resolveDrop(rows(), "a1", "C", "after")).toEqual({ parentId: null, index: 3 });
    // B leaves first, so the end is index 2 (A, C, B)
    expect(resolveDrop(rows(), "B", "C", "after")).toEqual({ parentId: null, index: 2 });
  });

  test("inside_C_is_C_index_0", () => {
    expect(resolveDrop(rows(), "B", "C", "inside")).toEqual({ parentId: "C", index: 0 });
  });

  test("inside_A_appends_after_a2", () => {
    expect(resolveDrop(rows(), "B", "A", "inside")).toEqual({ parentId: "A", index: 2 });
  });

  test("after_expanded_A_means_inside_A_index_0", () => {
    expect(resolveDrop(rows(), "B", "A", "after")).toEqual({ parentId: "A", index: 0 });
  });

  test("after_a_collapsed_folder_is_a_sibling_slot", () => {
    const collapsed = flattenRows(tree(), new Set(), "");
    expect(resolveDrop(collapsed, "C", "A", "after")).toEqual({ parentId: null, index: 1 });
  });

  test("reorder_a1_after_a2_in_A_is_A_index_1", () => {
    expect(resolveDrop(rows(), "a1", "a2", "after")).toEqual({ parentId: "A", index: 1 });
  });

  test("reorder_a2_before_a1_in_A_is_A_index_0", () => {
    expect(resolveDrop(rows(), "a2", "a1", "before")).toEqual({ parentId: "A", index: 0 });
  });

  test("dropping_on_itself_is_null", () => {
    for (const position of ["before", "after", "inside"] as const) {
      expect(resolveDrop(rows(), "A", "A", position)).toBeNull();
      expect(resolveDrop(rows(), "B", "B", position)).toBeNull();
    }
  });

  test("dropping_A_inside_a1_is_null", () => {
    expect(resolveDrop(rows(), "A", "a1", "inside")).toBeNull();
  });

  test("dropping_a_folder_next_to_its_own_descendant_is_null", () => {
    expect(resolveDrop(rows(), "A", "a1", "before")).toBeNull();
    expect(resolveDrop(rows(), "A", "a2", "after")).toBeNull();
  });

  test("dropping_in_place_is_null", () => {
    // a1 before a2 is exactly where a1 already is
    expect(resolveDrop(rows(), "a1", "a2", "before")).toBeNull();
    // a2 after a1 likewise
    expect(resolveDrop(rows(), "a2", "a1", "after")).toBeNull();
    // B between A and C stays between them
    expect(resolveDrop(rows(), "B", "C", "before")).toBeNull();
    expect(resolveDrop(rows(), "B", "A", "after")).toEqual({ parentId: "A", index: 0 });
  });

  test("dropping_a_first_child_at_the_top_of_its_own_expanded_folder_is_null", () => {
    expect(resolveDrop(rows(), "a1", "A", "after")).toBeNull();
  });

  test("inside_a_connection_or_an_unknown_row_is_null", () => {
    expect(resolveDrop(rows(), "a1", "B", "inside")).toBeNull();
    expect(resolveDrop(rows(), "a1", "nope", "before")).toBeNull();
    expect(resolveDrop(rows(), "nope", "B", "before")).toBeNull();
  });

  test("inside_the_current_folder_moves_to_its_end", () => {
    expect(resolveDrop(rows(), "a1", "A", "inside")).toEqual({ parentId: "A", index: 1 });
    // a2 is already last: no-op
    expect(resolveDrop(rows(), "a2", "A", "inside")).toBeNull();
  });
});
