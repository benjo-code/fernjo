import { afterEach, expect, test, vi } from "vitest";
import { THEME_KEY, ThemeStore, applyTheme, readStoredTheme } from "./theme.svelte";
import { apiError, mockApi, themeInfo } from "./testing";

afterEach(() => {
  delete document.documentElement.dataset.theme;
  localStorage.clear();
  vi.restoreAllMocks();
});

test("applyTheme_sets_data_theme_for_light_and_dark", () => {
  applyTheme("light");
  expect(document.documentElement.dataset.theme).toBe("light");
  applyTheme("dark");
  expect(document.documentElement.dataset.theme).toBe("dark");
});

test("applyTheme_system_removes_the_attribute", () => {
  applyTheme("dark");
  applyTheme("system");
  expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
});

test("readStoredTheme_ignores_values_that_are_not_theme_names", () => {
  for (const bad of ["banana", "", "{}"]) {
    localStorage.setItem(THEME_KEY, bad);
    expect(readStoredTheme()).toBe("system");
  }
  localStorage.setItem(THEME_KEY, "dark");
  expect(readStoredTheme()).toBe("dark");
});

test("readStoredTheme_survives_a_throwing_localStorage", () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw new Error("blocked");
  });
  expect(readStoredTheme()).toBe("system");
});

test("load_applies_and_mirrors_the_theme_from_rust", async () => {
  const { api } = mockApi([], { getTheme: async () => themeInfo({ theme: "dark" }) });
  const store = new ThemeStore(api);
  await store.load();
  expect(store.info?.theme).toBe("dark");
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(localStorage.getItem(THEME_KEY)).toBe("dark");
});

test("a_rejected_set_leaves_attribute_and_mirror_unchanged", async () => {
  const { api } = mockApi([], {
    getTheme: async () => themeInfo({ theme: "light" }),
    setTheme: async () => {
      throw apiError("settings_write_failed", "could not save the settings");
    },
  });
  const store = new ThemeStore(api);
  await store.load();
  const err = await store.set("dark");
  expect(err?.code).toBe("settings_write_failed");
  expect(document.documentElement.dataset.theme).toBe("light");
  expect(localStorage.getItem(THEME_KEY)).toBe("light");
  expect(store.info?.theme).toBe("light");
});

test("a_successful_set_applies_and_mirrors", async () => {
  const { api } = mockApi([]);
  const store = new ThemeStore(api);
  expect(await store.set("dark")).toBeNull();
  expect(document.documentElement.dataset.theme).toBe("dark");
  expect(localStorage.getItem(THEME_KEY)).toBe("dark");
});
