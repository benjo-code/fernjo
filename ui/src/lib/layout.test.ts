import { afterEach, expect, test, vi } from "vitest";
import {
  LAYOUT_KEY,
  LayoutStore,
  PANEL_MIN,
  SIDEBAR_CHROME,
  SIDEBAR_DEFAULT,
  SIDEBAR_MAX,
  SIDEBAR_MIN,
  TREE_RESERVE,
  TREE_ROWS_DEFAULT,
  TREE_ROWS_SIZED,
  clampPanelHeight,
  clampSidebarWidth,
  treeMinHeight,
} from "./layout.svelte";
import sidebarSource from "./components/Sidebar.svelte?raw";

afterEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
});

test("clampSidebarWidth keeps the width between the minimum and half the window", () => {
  expect(clampSidebarWidth(100, 1280)).toBe(SIDEBAR_MIN);
  expect(clampSidebarWidth(900, 1280)).toBe(SIDEBAR_MAX);
  expect(clampSidebarWidth(500, 800)).toBe(400);
  expect(clampSidebarWidth(500, 300)).toBe(SIDEBAR_MIN);
  expect(clampSidebarWidth(NaN, 1280)).toBe(SIDEBAR_DEFAULT);
});

test("clampPanelHeight leaves the tree its minimum rows (21 rows at 1280x800)", () => {
  const treeMin = Math.min(24 * 21, 800 - TREE_RESERVE);
  expect(treeMin).toBe(504);
  const available = 800 - 117; // sidebar height minus filter, notices and footer
  const tallest = clampPanelHeight(5000, available, treeMin);
  expect(tallest).toBe(available - treeMin);
  expect(treeMin / 24).toBeGreaterThanOrEqual(21);
  expect(clampPanelHeight(5, available, treeMin)).toBe(26);
  expect(clampPanelHeight(5000, 100, treeMin)).toBe(26);
});

test("a panel the user sized may grow past the 21-row default and leaves the tree 6 rows", () => {
  const available = 800 - 117;
  const row = 24;
  const byDefault = clampPanelHeight(5000, available, treeMinHeight(false, row, 800));
  const sized = clampPanelHeight(5000, available, treeMinHeight(true, row, 800));
  expect(treeMinHeight(false, row, 800)).toBe(row * TREE_ROWS_DEFAULT);
  expect(treeMinHeight(true, row, 800)).toBe(row * TREE_ROWS_SIZED);
  expect(sized).toBeGreaterThan(byDefault);
  expect(sized).toBe(available - row * 6);
  // a very short window never asks for more rows than fit
  expect(treeMinHeight(false, row, 400)).toBe(400 - TREE_RESERVE);
});

test("a corrupt stored value falls back to the defaults", () => {
  for (const raw of [
    "not json",
    "null",
    "42",
    '{"sidebarWidth":"wide","propertiesHeight":"tall"}',
    '{"sidebarWidth":99999,"propertiesHeight":-5}',
    '{"sidebarWidth":null,"propertiesHeight":1e999}',
  ]) {
    localStorage.setItem(LAYOUT_KEY, raw);
    const layout = new LayoutStore();
    expect(layout.sidebarWidth).toBe(SIDEBAR_DEFAULT);
    expect(layout.propertiesHeight).toBeNull();
  }
});

test("sizes survive a new LayoutStore (restart)", () => {
  const first = new LayoutStore();
  first.setSidebarWidth(420);
  first.setPropertiesHeight(180);
  const second = new LayoutStore();
  expect(second.sidebarWidth).toBe(420);
  expect(second.propertiesHeight).toBe(180);
  second.resetSidebarWidth();
  second.resetPropertiesHeight();
  const third = new LayoutStore();
  expect(third.sidebarWidth).toBe(SIDEBAR_DEFAULT);
  expect(third.propertiesHeight).toBeNull();
});

test("a stored width from a larger window is held inside the current one", () => {
  localStorage.setItem(LAYOUT_KEY, '{"sidebarWidth":560,"propertiesHeight":null}');
  const layout = new LayoutStore();
  layout.viewportWidth = 700;
  expect(layout.effectiveSidebarWidth).toBe(350);
});

test("blocked storage does not throw", () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw new Error("blocked");
  });
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("blocked");
  });
  const layout = new LayoutStore();
  expect(layout.sidebarWidth).toBe(SIDEBAR_DEFAULT);
  expect(() => layout.setSidebarWidth(400)).not.toThrow();
  expect(layout.sidebarWidth).toBe(400);
});

test("the tree reserve covers the fixed sidebar chrome plus the panel header", () => {
  const chrome = Object.values(SIDEBAR_CHROME).reduce((a, b) => a + b, 0);
  // a new fixed row in the sidebar must raise SIDEBAR_CHROME and TREE_RESERVE together
  expect(TREE_RESERVE).toBeGreaterThanOrEqual(chrome + PANEL_MIN);
  // the panel keeps at least the room it had before quick connect (81 px chrome, 23 rows) at 1280x800
  const room = 800 - chrome - treeMinHeight(false, 24, 800);
  expect(room).toBeGreaterThanOrEqual(800 - 81 - 24 * 23);
});

test("Sidebar.svelte's tree-slot minimum uses the same rows and reserve", () => {
  expect(sidebarSource).toContain(
    `min(calc(var(--row-height-tree) * ${TREE_ROWS_DEFAULT}), calc(100vh - ${TREE_RESERVE}px))`,
  );
  expect(sidebarSource).toContain(`min-height: calc(var(--row-height-tree) * ${TREE_ROWS_SIZED})`);
});
