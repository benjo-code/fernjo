import { describe, expect, test } from "vitest";
import { reorder } from "./tabs";

const t = (...ids: string[]) => ids.map((id) => ({ id }));
const ids = (tabs: { id: string }[]) => tabs.map((x) => x.id);

describe("reorder", () => {
  test("reorder_moves_before_target", () => {
    expect(ids(reorder(t("a", "b", "c", "d"), "d", "b"))).toEqual(["a", "d", "b", "c"]);
    expect(ids(reorder(t("a", "b", "c", "d"), "a", "c"))).toEqual(["b", "a", "c", "d"]);
  });

  test("reorder_to_end_with_null", () => {
    expect(ids(reorder(t("a", "b", "c"), "a", null))).toEqual(["b", "c", "a"]);
  });

  test("reorder_unknown_id_returns_input", () => {
    const tabs = t("a", "b");
    expect(reorder(tabs, "x", "a")).toBe(tabs);
    expect(reorder(tabs, "a", "x")).toBe(tabs);
  });
});
