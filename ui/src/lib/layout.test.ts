import { afterEach, expect, test, vi } from "vitest";
import {
  LAYOUT_KEY,
  LayoutStore,
  SIDEBAR_DEFAULT,
  SIDEBAR_MAX,
  SIDEBAR_MIN,
  clampPanelHeight,
  clampSidebarWidth,
} from "./layout.svelte";

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

test("clampPanelHeight leaves the tree its minimum rows (23 rows at 1280x800)", () => {
  const treeMin = Math.min(24 * 23, 800 - 120);
  expect(treeMin).toBe(552);
  const available = 800 - 90; // sidebar height minus filter, notices and footer
  const tallest = clampPanelHeight(5000, available, treeMin);
  expect(tallest).toBe(available - treeMin);
  expect(treeMin / 24).toBeGreaterThanOrEqual(23);
  expect(clampPanelHeight(5, available, treeMin)).toBe(26);
  expect(clampPanelHeight(5000, 100, treeMin)).toBe(26);
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
