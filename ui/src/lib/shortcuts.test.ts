import { describe, expect, test } from "vitest";
import {
  isMac,
  routeKey,
  SHORTCUTS,
  shortcutEvent,
  shortcutFor,
  shortcutKeys,
  type AppAction,
} from "./shortcuts";

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

describe("isMac", () => {
  test("isMac_reads_userAgentData_then_platform_and_defaults_to_false", () => {
    expect(isMac({ userAgentData: { platform: "macOS" }, platform: "Win32" })).toBe(true);
    expect(isMac({ platform: "MacIntel" })).toBe(true);
    expect(isMac({ platform: "Win32" })).toBe(false);
    expect(isMac({})).toBe(false);
    expect(isMac(undefined)).toBe(false);
  });
});

describe("SHORTCUTS", () => {
  test("every_entry_with_a_chord_routes_to_its_action_on_both_platforms", () => {
    const chords = SHORTCUTS.filter((s) => s.action);
    expect(chords.map((s) => s.action).sort()).toEqual(
      ["closeTab", "copy", "focusFilter", "nextTab", "paste", "prevTab", "terminalSearch"].sort(),
    );
    for (const mac of [false, true]) {
      for (const s of chords) {
        const event = shortcutEvent(s, mac);
        expect(event).not.toBeNull();
        expect(routeKey(event!, mac, true), `${s.label} (mac=${mac})`).toBe(s.action);
      }
    }
  });

  test("the_other_platforms_chord_does_not_route_to_the_action", () => {
    const tabs: (AppAction | undefined)[] = ["nextTab", "prevTab"];
    for (const s of SHORTCUTS.filter((x) => x.action && !tabs.includes(x.action))) {
      expect(routeKey(shortcutEvent(s, true)!, false, true)).toBeNull();
      expect(routeKey(shortcutEvent(s, false)!, true, true)).toBeNull();
    }
  });

  test("labels_name_the_chord_the_platform_really_uses", () => {
    const keys = (action: AppAction, mac: boolean) => shortcutKeys(shortcutFor(action)!, mac);
    expect(keys("focusFilter", false)).toBe("Ctrl+Shift+K");
    expect(keys("focusFilter", true)).toBe("Cmd+K");
    expect(keys("closeTab", false)).toBe("Ctrl+Shift+W");
    expect(keys("closeTab", true)).toBe("Cmd+W");
    expect(keys("terminalSearch", false)).toBe("Ctrl+Shift+F");
    expect(keys("terminalSearch", true)).toBe("Cmd+F");
    expect(keys("nextTab", false)).toBe("Ctrl+Tab");
    expect(keys("nextTab", true)).toBe("Ctrl+Tab");
    expect(keys("prevTab", false)).toBe("Ctrl+Shift+Tab");
    expect(keys("prevTab", true)).toBe("Ctrl+Shift+Tab");
  });

  test("mouse_and_tree_gestures_have_text_but_no_chord", () => {
    const plain = SHORTCUTS.filter((s) => !s.action);
    expect(plain.map((s) => shortcutKeys(s, false))).toEqual([
      "Double-click or Enter",
      "Ctrl+click",
      "Shift+click",
      "F2",
      "Delete",
    ]);
    expect(shortcutKeys(plain[1], true)).toBe("Cmd+click");
    for (const s of plain) expect(shortcutEvent(s, false)).toBeNull();
  });
});
