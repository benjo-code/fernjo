import { describe, expect, test } from "vitest";
import { conn, folder } from "./testing";
import { countDescendants, findNode, flattenRows, matchesFilter, selectionTops } from "./rows";

const tree = () => [
  folder("Homelab", [conn("pve-01"), conn("DC01", "rdp", { description: "domain controller" })]),
  conn("web-01"),
];

const names = (rows: { node: { name: string } }[]) => rows.map((r) => r.node.name);

describe("flattenRows", () => {
  test("flatten_hides_children_of_collapsed_folders", () => {
    expect(names(flattenRows(tree(), new Set(), ""))).toEqual(["Homelab", "web-01"]);
    expect(names(flattenRows(tree(), new Set(["Homelab"]), ""))).toEqual([
      "Homelab",
      "pve-01",
      "DC01",
      "web-01",
    ]);
  });

  test("rows_carry_depth_parent_and_index", () => {
    const rows = flattenRows(tree(), new Set(["Homelab"]), "");
    expect(rows.map((r) => [r.node.name, r.depth, r.parentId, r.index])).toEqual([
      ["Homelab", 0, null, 0],
      ["pve-01", 1, "Homelab", 0],
      ["DC01", 1, "Homelab", 1],
      ["web-01", 0, null, 1],
    ]);
    expect(rows[0]).toMatchObject({ expanded: true, hasChildren: true });
    expect(rows[1]).toMatchObject({ expanded: false, hasChildren: false });
  });

  test("filter_matches_name_host_and_description_case_insensitively", () => {
    expect(names(flattenRows(tree(), new Set(), "PVE"))).toEqual(["Homelab", "pve-01"]);
    expect(names(flattenRows(tree(), new Set(), "pve-01.LAB"))).toEqual(["Homelab", "pve-01"]);
    expect(names(flattenRows(tree(), new Set(), "Domain"))).toEqual(["Homelab", "DC01"]);
  });

  test("filter_shows_ancestors_of_matches_force_expanded_and_hides_non_matches", () => {
    const deep = [folder("A", [folder("B", [conn("target"), conn("other")]), conn("noise")])];
    const rows = flattenRows(deep, new Set(), "target");
    expect(names(rows)).toEqual(["A", "B", "target"]);
    expect(rows.map((r) => r.expanded)).toEqual([true, true, false]);
    expect(rows.map((r) => r.depth)).toEqual([0, 1, 2]);
    // real positions in the unfiltered parent list are kept
    expect(rows[2].index).toBe(0);
  });

  test("filter_treats_regex_characters_literally", () => {
    const t = [conn("a[1]"), conn("b(2)"), conn("c.*d"), conn("plain"), conn("cxxd")];
    expect(names(flattenRows(t, new Set(), "["))).toEqual(["a[1]"]);
    expect(names(flattenRows(t, new Set(), "("))).toEqual(["b(2)"]);
    expect(names(flattenRows(t, new Set(), ".*"))).toEqual(["c.*d"]);
  });

  test("filter_with_no_match_gives_no_rows", () => {
    expect(flattenRows(tree(), new Set(), "zzz")).toEqual([]);
  });

  test("whitespace_only_filter_means_no_filter", () => {
    expect(names(flattenRows(tree(), new Set(), "   "))).toEqual(["Homelab", "web-01"]);
  });

  test("query_is_trimmed", () => {
    expect(names(flattenRows(tree(), new Set(), "  web-01 "))).toEqual(["web-01"]);
  });

  test("a_matching_folder_without_matching_children_is_shown_collapsed", () => {
    const rows = flattenRows(tree(), new Set(["Homelab"]), "homelab");
    expect(names(rows)).toEqual(["Homelab"]);
    expect(rows[0]).toMatchObject({ expanded: false, hasChildren: true });
  });

  test("filtering_500_connections_stays_under_100_ms", () => {
    const racks = Array.from({ length: 50 }, (_, s) =>
      folder(
        `Rack ${s}`,
        Array.from({ length: 10 }, (_, c) => conn(`host-${s * 10 + c}`)),
      ),
    );
    const start = performance.now();
    const rows = flattenRows(racks, new Set(), "host-49");
    expect(performance.now() - start).toBeLessThan(100);
    expect(names(rows).filter((n) => n.startsWith("host-49")).length).toBeGreaterThan(0);
  });
});

test("matchesFilter_checks_name_host_description", () => {
  expect(matchesFilter(conn("web-01"), "WEB")).toBe(true);
  expect(matchesFilter(conn("x", "ssh", { host: "10.0.0.5" }), "10.0")).toBe(true);
  expect(matchesFilter(folder("F", [], { description: "Customer A" }), "customer")).toBe(true);
  expect(matchesFilter(conn("web-01"), "nope")).toBe(false);
  expect(matchesFilter(conn("web-01"), "  ")).toBe(true);
});

test("count_descendants_counts_nested_nodes_excluding_self", () => {
  const f = folder("A", [folder("B", [conn("c1")]), conn("c2")]);
  expect(countDescendants(f)).toBe(3);
  expect(countDescendants(conn("x"))).toBe(0);
  expect(countDescendants(folder("empty"))).toBe(0);
});

describe("findNode", () => {
  test("finds_nodes_at_any_depth_and_returns_undefined_for_unknown_ids", () => {
    expect(findNode(tree(), "DC01")?.name).toBe("DC01");
    expect(findNode(tree(), "web-01")?.name).toBe("web-01");
    expect(findNode(tree(), "nope")).toBeUndefined();
  });
});

describe("selectionTops", () => {
  test("selectionTops drops a node whose ancestor is selected", () => {
    const rows = flattenRows(tree(), new Set(["Homelab"]), "");
    expect(selectionTops(rows, new Set(["Homelab", "pve-01", "web-01"]))).toEqual([
      "Homelab",
      "web-01",
    ]);
    expect(selectionTops(rows, new Set(["DC01", "pve-01"]))).toEqual(["pve-01", "DC01"]);
    expect(selectionTops(rows, new Set())).toEqual([]);
  });
});
