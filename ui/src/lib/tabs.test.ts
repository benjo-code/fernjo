import { describe, expect, test } from "vitest";
import { reorder, tabToClose } from "./tabs";

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

describe("tabToClose", () => {
  test("no_tab_is_closed_while_the_vault_view_is_open", () => {
    expect(tabToClose("vault", "t1")).toBeNull();
    expect(tabToClose("empty", "t1")).toBe("t1");
    expect(tabToClose("empty", null)).toBeNull();
  });
});
