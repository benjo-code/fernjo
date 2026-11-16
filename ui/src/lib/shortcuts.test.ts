import { describe, expect, test } from "vitest";
import { routeKey } from "./shortcuts";

const k = (
  key: string,
  mods: Partial<Record<"ctrl" | "shift" | "alt" | "meta", boolean>> = {},
) => ({
  key,
  ctrlKey: !!mods.ctrl,
  shiftKey: !!mods.shift,
  altKey: !!mods.alt,
  metaKey: !!mods.meta,
});

describe("routeKey", () => {
  test("plain_ctrl_letters_and_alt_letters_reach_the_terminal", () => {
    for (const mac of [false, true]) {
      expect(routeKey(k("c", { ctrl: true }), mac, true)).toBeNull();
      expect(routeKey(k("w", { ctrl: true }), mac, true)).toBeNull();
      expect(routeKey(k("b", { ctrl: true }), mac, true)).toBeNull();
      expect(routeKey(k("b", { alt: true }), mac, true)).toBeNull();
    }
  });

  test("escape_is_never_an_app_shortcut", () => {
    for (const mac of [false, true]) {
      expect(routeKey(k("Escape"), mac, true)).toBeNull();
      expect(routeKey(k("Escape", { ctrl: true, shift: true }), mac, false)).toBeNull();
    }
  });

  test("ctrl_tab_and_ctrl_shift_tab_switch_tabs_everywhere", () => {
    for (const mac of [false, true]) {
      for (const inTerminal of [false, true]) {
        expect(routeKey(k("Tab", { ctrl: true }), mac, inTerminal)).toBe("nextTab");
        expect(routeKey(k("Tab", { ctrl: true, shift: true }), mac, inTerminal)).toBe("prevTab");
      }
    }
  });

  test("ctrl_shift_chords_map_to_actions", () => {
    const chord = (key: string) => routeKey(k(key, { ctrl: true, shift: true }), false, true);
    expect(chord("W")).toBe("closeTab");
    expect(chord("K")).toBe("focusFilter");
    expect(chord("F")).toBe("terminalSearch");
    expect(chord("C")).toBe("copy");
    expect(chord("V")).toBe("paste");
  });

  test("cmd_chords_only_on_mac", () => {
    expect(routeKey(k("w", { meta: true }), true, true)).toBe("closeTab");
    expect(routeKey(k("k", { meta: true }), true, true)).toBe("focusFilter");
    expect(routeKey(k("f", { meta: true }), true, true)).toBe("terminalSearch");
    expect(routeKey(k("c", { meta: true }), true, true)).toBe("copy");
    expect(routeKey(k("v", { meta: true }), true, true)).toBe("paste");
    expect(routeKey(k("w", { meta: true }), false, true)).toBeNull();
    expect(routeKey(k("k", { meta: true }), false, false)).toBeNull();
  });

  test("ctrl_shift_w_is_not_close_on_mac", () => {
    expect(routeKey(k("W", { ctrl: true, shift: true }), true, true)).toBeNull();
  });

  test("copy_and_paste_only_inside_the_terminal", () => {
    expect(routeKey(k("C", { ctrl: true, shift: true }), false, false)).toBeNull();
    expect(routeKey(k("V", { ctrl: true, shift: true }), false, false)).toBeNull();
    expect(routeKey(k("c", { meta: true }), true, false)).toBeNull();
    expect(routeKey(k("v", { meta: true }), true, false)).toBeNull();
    // the other chords work outside the terminal too
    expect(routeKey(k("W", { ctrl: true, shift: true }), false, false)).toBe("closeTab");
  });
});
