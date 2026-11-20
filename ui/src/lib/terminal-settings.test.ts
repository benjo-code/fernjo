import { expect, test } from "vitest";
import { TerminalSettingsStore } from "./terminal-settings.svelte";
import { apiError, mockApi, terminalSettings } from "./testing";

test("load_fills_info", async () => {
  const { api, calls } = mockApi([], {
    getTerminalSettings: async () => terminalSettings({ fontSize: 17, fontFamily: "Fira Code" }),
  });
  const store = new TerminalSettingsStore(api);
  expect(store.info).toBeNull();
  await store.load();
  expect(calls.getTerminalSettings).toHaveLength(1);
  expect(store.info?.fontSize).toBe(17);
  expect(store.info?.fontFamily).toBe("Fira Code");
});

test("save_updates_info_and_surfaces_rust_error_codes", async () => {
  const { api, calls } = mockApi([], {
    setTerminalSettings: async (s) => {
      if (s.fontSize > 32) throw apiError("invalid_font_size", "Font size must be 8 to 32.");
      return terminalSettings({ ...s, persistent: false });
    },
  });
  const store = new TerminalSettingsStore(api);
  const ok = await store.save({
    fontFamily: null,
    fontSize: 20,
    scrollback: 5000,
    closeOnExit: true,
  });
  expect(ok).toBeNull();
  expect(calls.setTerminalSettings[0]).toEqual([
    { fontFamily: null, fontSize: 20, scrollback: 5000, closeOnExit: true },
  ]);
  expect(store.info).toMatchObject({ fontSize: 20, scrollback: 5000, persistent: false });

  const failed = await store.save({
    fontFamily: null,
    fontSize: 99,
    scrollback: 5000,
    closeOnExit: true,
  });
  expect(failed?.code).toBe("invalid_font_size");
  // the last good values stay
  expect(store.info?.fontSize).toBe(20);
});
